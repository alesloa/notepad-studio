// Concrete `HostApi` vtable. Each `extern "C"` callback below pulls the
// active `Workspace` + `Context` out of the thread-local set by
// `start_active_call` and acts on them. Plugins are dispatched
// synchronously on the main thread, so the raw pointers stashed in the
// thread-local are valid for the entire `notepad_studio_plugin_run_command` call
// (and never read past it).

use core::ptr;
use core::slice;
use std::cell::RefCell;

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

/// Stash workspace + context pointers in the thread-local for the duration
/// of one `notepad_studio_plugin_run_command` call. Pair every call with
/// `end_active_call`.
pub(crate) fn start_active_call(workspace: &mut Workspace, cx: &mut Context<'_, Workspace>) {
    let workspace_ptr: *mut Workspace = workspace;
    let cx_ptr: *mut () = cx as *mut Context<'_, Workspace> as *mut ();
    CURRENT.with(|cell| {
        *cell.borrow_mut() = Some(ActiveCall {
            workspace: workspace_ptr,
            cx: cx_ptr,
        });
    });
}

/// Clear the thread-local active call. Safe to call when no call is in
/// flight (no-op).
pub(crate) fn end_active_call() {
    CURRENT.with(|cell| *cell.borrow_mut() = None);
}

/// Run `f` with mutable access to the active workspace and context. Returns
/// `None` when called outside a plugin command (i.e. without a matching
/// `start_active_call`), letting the C-ABI callbacks fail gracefully.
fn with_active<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut Workspace, &mut Context<'_, Workspace>) -> R,
{
    CURRENT.with(|cell| {
        let mut guard = cell.borrow_mut();
        let active = guard.as_mut()?;
        if active.workspace.is_null() || active.cx.is_null() {
            return None;
        }
        // SAFETY: `start_active_call` recorded raw pointers that refer to
        // stack-live values for the duration of one synchronous plugin
        // dispatch. Plugins never see this thread-local, so there's no way
        // they can keep references past the dispatch boundary. The cx
        // pointer was erased to `*mut ()` for storage; cast back here
        // — the original lifetime is bounded by the parent dispatcher's
        // stack frame.
        unsafe {
            let ws = &mut *active.workspace;
            let cx = &mut *(active.cx as *mut Context<'_, Workspace>);
            Some(f(ws, cx))
        }
    })
}

unsafe extern "C" fn host_get_buffer_text(out_len: *mut usize) -> *const u8 {
    let bytes = match with_active(|ws, _| ws.buffer().rope.to_string().into_bytes()) {
        Some(b) => b,
        None => {
            if !out_len.is_null() {
                unsafe { *out_len = 0 };
            }
            return ptr::null();
        }
    };
    let len = bytes.len();
    if !out_len.is_null() {
        unsafe { *out_len = len };
    }
    if len == 0 {
        // Stable non-null pointer with zero length so the plugin can still
        // call `free_buffer_text` symmetrically — `Vec::with_capacity(0)` is
        // well-defined and `into_boxed_slice` of an empty Vec yields a
        // dangling-but-aligned pointer.
        let boxed = Vec::<u8>::new().into_boxed_slice();
        return Box::into_raw(boxed) as *const u8;
    }
    let boxed = bytes.into_boxed_slice();
    Box::into_raw(boxed) as *const u8
}

unsafe extern "C" fn host_free_buffer_text(ptr: *const u8, len: usize) {
    if ptr.is_null() {
        return;
    }
    let slice_ptr = unsafe { slice::from_raw_parts_mut(ptr as *mut u8, len) };
    let _ = unsafe { Box::from_raw(slice_ptr as *mut [u8]) };
}

unsafe extern "C" fn host_replace_buffer_text(ptr: *const u8, len: usize) {
    if ptr.is_null() && len > 0 {
        return;
    }
    let bytes = if len == 0 {
        &[][..]
    } else {
        unsafe { slice::from_raw_parts(ptr, len) }
    };
    let Ok(text) = std::str::from_utf8(bytes) else {
        return;
    };
    let owned = text.to_string();
    with_active(|ws, cx| {
        let total = ws.buffer().rope.len_bytes();
        ws.replace_byte_range(0..total, &owned, cx);
    });
}

unsafe extern "C" fn host_get_selection(start: *mut usize, end: *mut usize) {
    let (s, e) = match with_active(|ws, _| {
        let r = ws.selection().ordered();
        (r.start, r.end)
    }) {
        Some(v) => v,
        None => (0, 0),
    };
    if !start.is_null() {
        unsafe { *start = s };
    }
    if !end.is_null() {
        unsafe { *end = e };
    }
}

unsafe extern "C" fn host_replace_selection(ptr: *const u8, len: usize) {
    if ptr.is_null() && len > 0 {
        return;
    }
    let bytes = if len == 0 {
        &[][..]
    } else {
        unsafe { slice::from_raw_parts(ptr, len) }
    };
    let Ok(text) = std::str::from_utf8(bytes) else {
        return;
    };
    let owned = text.to_string();
    with_active(|ws, cx| {
        let r = ws.selection().ordered();
        ws.replace_byte_range(r, &owned, cx);
    });
}

unsafe extern "C" fn host_set_status_message(ptr: *const u8, len: usize) {
    if ptr.is_null() && len > 0 {
        return;
    }
    let bytes = if len == 0 {
        &[][..]
    } else {
        unsafe { slice::from_raw_parts(ptr, len) }
    };
    let Ok(text) = std::str::from_utf8(bytes) else {
        return;
    };
    let owned = text.to_string();
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
