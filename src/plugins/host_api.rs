// Concrete `HostApi` vtable. Each `extern "C"` callback below pulls the
// active `Workspace` + `Context` out of the thread-local set by
// `start_active_call` and acts on them. Plugins are dispatched
// synchronously on the main thread, so the raw pointers stashed in the
// thread-local are valid for the entire `notepad_studio_plugin_run_command` call
// (and never read past it).
//
// Every argument a plugin passes is hostile input: null or misaligned
// out-pointers, lengths no slice can have, text that is not UTF-8, frees of
// pointers the host never handed out, and calls made outside a dispatch.
// Each callback checks what can be checked and turns the rest into a no-op
// or a status-bar message instead of a crash.

use core::ptr;
use core::slice;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Mutex;

use gpui::Context;
use notepad_studio_plugin::{HostApi, PLUGIN_API_VERSION};

use crate::workspace::Workspace;

/// Snapshot of the currently-active plugin call. Pointers refer to stack
/// locals owned by the action handler that initiated the dispatch — they
/// MUST be cleared before that handler returns.
///
/// `Context<'a, Workspace>` carries a lifetime tying it to the App that
/// produced it; we strip that lifetime for storage by routing through a
/// raw `*mut ()` and re-typing on access. Plugin dispatch is synchronous
/// on the main thread so the pointee outlives the borrow trivially.
struct ActiveCall {
    workspace: *mut Workspace,
    cx: *mut (),
}

thread_local! {
    static CURRENT: RefCell<Option<ActiveCall>> = const { RefCell::new(None) };
}

/// Buffers handed out by `host_get_buffer_text` and not yet freed: address
/// to length. `host_free_buffer_text` frees only addresses found here, with
/// the length recorded here, so a plugin that frees twice, frees a pointer
/// the host never returned, or passes the wrong length leaks a buffer
/// instead of corrupting the heap.
static OUTSTANDING: Mutex<BTreeMap<usize, usize>> = Mutex::new(BTreeMap::new());

/// Stash workspace + context pointers in the thread-local for the duration
/// of one `notepad_studio_plugin_run_command` call. Pair every successful
/// call with `end_active_call`. Returns false (and stores nothing) when the
/// slot is busy, which only happens if a dispatch is re-entered.
pub(crate) fn start_active_call(workspace: &mut Workspace, cx: &mut Context<'_, Workspace>) -> bool {
    let workspace_ptr: *mut Workspace = workspace;
    let cx_ptr: *mut () = cx as *mut Context<'_, Workspace> as *mut ();
    CURRENT
        .try_with(|cell| {
            let Ok(mut slot) = cell.try_borrow_mut() else {
                return false;
            };
            if slot.is_some() {
                return false;
            }
            *slot = Some(ActiveCall {
                workspace: workspace_ptr,
                cx: cx_ptr,
            });
            true
        })
        .unwrap_or(false)
}

/// Clear the thread-local active call. Safe to call when no call is in
/// flight (no-op).
pub(crate) fn end_active_call() {
    let _ = CURRENT.try_with(|cell| {
        if let Ok(mut slot) = cell.try_borrow_mut() {
            *slot = None;
        }
    });
}

/// Run `f` with mutable access to the active workspace and context. Returns
/// `None` when called outside a plugin command (i.e. without a matching
/// `start_active_call`), from another thread, or re-entrantly from inside
/// another callback, letting the C-ABI callbacks fail gracefully.
fn with_active<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut Workspace, &mut Context<'_, Workspace>) -> R,
{
    CURRENT
        .try_with(|cell| {
            // Held for the whole callback: a second, nested callback finds the
            // slot borrowed and bails out instead of minting a second
            // `&mut Workspace`.
            let mut guard = cell.try_borrow_mut().ok()?;
            let active = guard.as_mut()?;
            if active.workspace.is_null() || active.cx.is_null() {
                return None;
            }
            // SAFETY: `start_active_call` recorded raw pointers that refer to
            // stack-live values for the duration of one synchronous plugin
            // dispatch on this thread, and `end_active_call` clears them
            // before that dispatch returns. Plugins never see this
            // thread-local, so there's no way they can keep references past
            // the dispatch boundary. The `try_borrow_mut` guard above makes
            // these the only live references derived from the pointers. The
            // cx pointer was erased to `*mut ()` for storage; cast back here
            // — the original lifetime is bounded by the parent dispatcher's
            // stack frame.
            unsafe {
                let ws = &mut *active.workspace;
                let cx = &mut *(active.cx as *mut Context<'_, Workspace>);
                Some(f(ws, cx))
            }
        })
        .ok()
        .flatten()
}

/// Write `value` through a plugin-supplied out-pointer. Null and misaligned
/// pointers are skipped.
///
/// # Safety
/// A non-null, aligned `out` must be valid for a `usize` write, as the
/// `HostApi` contract in `ffi.rs` requires.
unsafe fn write_out(out: *mut usize, value: usize) {
    if out.is_null() || !out.is_aligned() {
        return;
    }
    // SAFETY: non-null and aligned are checked above; writability is the
    // caller's contract.
    unsafe { out.write(value) };
}

/// Borrow `len` bytes at `ptr` from a plugin call. `None` for a null pointer
/// with a non-zero length, or a length larger than any Rust slice can have.
///
/// # Safety
/// When `len > 0` and `ptr` is non-null, `ptr` must be valid for reads of
/// `len` bytes for the rest of the callback, as the `HostApi` contract says.
unsafe fn plugin_bytes<'a>(ptr: *const u8, len: usize) -> Option<&'a [u8]> {
    if len == 0 {
        return Some(&[]);
    }
    if ptr.is_null() || len > isize::MAX as usize {
        return None;
    }
    // SAFETY: non-null, `len` within `isize::MAX`, and readability for `len`
    // bytes is the caller's contract.
    Some(unsafe { slice::from_raw_parts(ptr, len) })
}

/// Decode the text argument of a write callback, reporting a bad argument on
/// the status bar so the user sees why the plugin command did nothing.
///
/// # Safety
/// Same as `plugin_bytes`.
unsafe fn plugin_text(ptr: *const u8, len: usize, callback: &str) -> Option<String> {
    // SAFETY: forwarded caller contract.
    let Some(bytes) = (unsafe { plugin_bytes(ptr, len) }) else {
        report(format!("Plugin passed a null or oversized buffer to {callback}"));
        return None;
    };
    match std::str::from_utf8(bytes) {
        Ok(text) => Some(text.to_owned()),
        Err(_) => {
            report(format!("Plugin passed text that is not valid UTF-8 to {callback}"));
            None
        }
    }
}

fn report(message: String) {
    with_active(|ws, cx| {
        ws.set_status(message);
        cx.notify();
    });
}

/// Hand `bytes` to the plugin as a raw pointer and remember it for
/// `release_buffer`. `None` when the registry is unavailable (poisoned).
fn register_buffer(bytes: Vec<u8>) -> Option<*const u8> {
    let len = bytes.len();
    let raw = Box::into_raw(bytes.into_boxed_slice()) as *mut u8 as *const u8;
    if len == 0 {
        // A zero-length box owns no allocation; nothing to track or free.
        return Some(raw);
    }
    let Ok(mut map) = OUTSTANDING.lock() else {
        // SAFETY: `raw` came from `Box::into_raw` of a `[u8]` of `len` bytes
        // just above and was never shared; rebuilding the box frees it.
        drop(unsafe { Box::from_raw(ptr::slice_from_raw_parts_mut(raw as *mut u8, len)) });
        return None;
    };
    map.insert(raw as usize, len);
    Some(raw)
}

/// Free a buffer handed out by `register_buffer`. Unknown pointers (double
/// free, foreign pointer) are ignored; the recorded length wins over the
/// plugin's.
fn release_buffer(ptr: *const u8) {
    if ptr.is_null() {
        return;
    }
    let Ok(mut map) = OUTSTANDING.lock() else {
        return;
    };
    let Some(len) = map.remove(&(ptr as usize)) else {
        return;
    };
    // SAFETY: the registry only holds pointers produced by `Box::into_raw`
    // of a `[u8]` with exactly the recorded length, and removing the entry
    // above guarantees each one is rebuilt (and freed) once.
    drop(unsafe { Box::from_raw(ptr::slice_from_raw_parts_mut(ptr as *mut u8, len)) });
}

unsafe extern "C" fn host_get_buffer_text(out_len: *mut usize) -> *const u8 {
    let Some(bytes) = with_active(|ws, _| ws.buffer().rope.to_string().into_bytes()) else {
        // SAFETY: `write_out` checks null and alignment; the rest is the
        // plugin's contract.
        unsafe { write_out(out_len, 0) };
        return ptr::null();
    };
    let len = bytes.len();
    // A zero-length result is still a stable non-null pointer so the plugin
    // can call `free_buffer_text` symmetrically.
    let Some(raw) = register_buffer(bytes) else {
        // SAFETY: as above.
        unsafe { write_out(out_len, 0) };
        return ptr::null();
    };
    // SAFETY: as above.
    unsafe { write_out(out_len, len) };
    raw
}

unsafe extern "C" fn host_free_buffer_text(ptr: *const u8, _len: usize) {
    release_buffer(ptr);
}

unsafe extern "C" fn host_replace_buffer_text(ptr: *const u8, len: usize) {
    // SAFETY: `plugin_text` checks null and length; readability of `len`
    // bytes is the plugin's contract.
    let Some(owned) = (unsafe { plugin_text(ptr, len, "replace_buffer_text") }) else {
        return;
    };
    with_active(|ws, cx| {
        let total = ws.buffer().rope.len_bytes();
        ws.replace_byte_range(0..total, &owned, cx);
    });
}

unsafe extern "C" fn host_get_selection(start: *mut usize, end: *mut usize) {
    let (s, e) = with_active(|ws, _| {
        let r = ws.selection().ordered();
        (r.start, r.end)
    })
    .unwrap_or((0, 0));
    // SAFETY: `write_out` checks null and alignment; the rest is the
    // plugin's contract.
    unsafe {
        write_out(start, s);
        write_out(end, e);
    }
}

unsafe extern "C" fn host_replace_selection(ptr: *const u8, len: usize) {
    // SAFETY: `plugin_text` checks null and length; readability of `len`
    // bytes is the plugin's contract.
    let Some(owned) = (unsafe { plugin_text(ptr, len, "replace_selection") }) else {
        return;
    };
    with_active(|ws, cx| {
        let r = ws.selection().ordered();
        ws.replace_byte_range(r, &owned, cx);
    });
}

unsafe extern "C" fn host_set_status_message(ptr: *const u8, len: usize) {
    // SAFETY: `plugin_text` checks null and length; readability of `len`
    // bytes is the plugin's contract.
    let Some(owned) = (unsafe { plugin_text(ptr, len, "set_status_message") }) else {
        return;
    };
    with_active(|ws, _| ws.set_status(owned));
}

/// `'static` vtable handed to every plugin command. Same value reused for
/// every dispatch — function pointers are pure code addresses, no per-call
/// state lives here.
pub(crate) static HOST_API: HostApi = HostApi {
    api_version: PLUGIN_API_VERSION,
    get_buffer_text: host_get_buffer_text,
    free_buffer_text: host_free_buffer_text,
    replace_buffer_text: host_replace_buffer_text,
    get_selection: host_get_selection,
    replace_selection: host_replace_selection,
    set_status_message: host_set_status_message,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freeing_twice_or_freeing_a_foreign_pointer_is_ignored() {
        let raw = register_buffer(b"hello".to_vec()).expect("registry");
        // SAFETY: `raw` points at the 5 live bytes registered above.
        assert_eq!(unsafe { slice::from_raw_parts(raw, 5) }, b"hello");
        // Wrong length from the plugin: the recorded one is used.
        // SAFETY: host callback invoked the way a plugin would.
        unsafe { host_free_buffer_text(raw, 9_999) };
        // Double free used to rebuild the Box a second time.
        // SAFETY: as above; the pointer is no longer registered.
        unsafe { host_free_buffer_text(raw, 5) };
        let local = [1u8, 2, 3];
        // SAFETY: a pointer the host never handed out.
        unsafe { host_free_buffer_text(local.as_ptr(), local.len()) };
        // SAFETY: null pointer.
        unsafe { host_free_buffer_text(ptr::null(), 4) };
    }

    #[test]
    fn empty_buffers_are_non_null_and_free_is_a_no_op() {
        let raw = register_buffer(Vec::new()).expect("registry");
        assert!(!raw.is_null());
        // SAFETY: host callback invoked the way a plugin would.
        unsafe { host_free_buffer_text(raw, 0) };
    }

    #[test]
    fn callbacks_outside_a_dispatch_are_no_ops() {
        let mut start = 7usize;
        let mut end = 9usize;
        let mut len = 3usize;
        let text = b"abc";
        // SAFETY: valid pointers; no active call, so nothing is dereferenced
        // beyond the out-pointers and the 3 readable bytes.
        unsafe {
            host_get_selection(&mut start, &mut end);
            assert_eq!((start, end), (0, 0));
            assert!(host_get_buffer_text(&mut len).is_null());
            assert_eq!(len, 0);
            host_replace_buffer_text(text.as_ptr(), text.len());
            host_replace_selection(text.as_ptr(), text.len());
            host_set_status_message(text.as_ptr(), text.len());
        }
    }

    #[test]
    fn hostile_arguments_are_rejected_before_any_read() {
        // Null with a length, a length past isize::MAX, invalid UTF-8,
        // null and misaligned out-pointers.
        // SAFETY: none of these calls may read through the bad pointers;
        // that is what the test checks (a read would fault).
        unsafe {
            assert!(plugin_bytes(ptr::null(), 4).is_none());
            assert!(plugin_bytes(ptr::dangling::<u8>(), usize::MAX).is_none());
            assert_eq!(plugin_bytes(ptr::null(), 0), Some(&[][..]));
            let bad = [0xffu8, 0xfe];
            assert!(plugin_text(bad.as_ptr(), bad.len(), "test").is_none());
            host_replace_selection(ptr::null(), 10);
            host_set_status_message(ptr::dangling::<u8>(), usize::MAX);
            host_get_selection(ptr::null_mut(), ptr::null_mut());
            let mut words = [0usize; 2];
            let misaligned = (words.as_mut_ptr() as *mut u8).add(1) as *mut usize;
            write_out(misaligned, 42);
            assert_eq!(words, [0, 0]);
            assert!(host_get_buffer_text(ptr::null_mut()).is_null());
        }
    }
}
