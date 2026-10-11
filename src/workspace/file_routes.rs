// PowerEditor/src/NppCommands.cpp:241-273, 399-405; NppIO.cpp:2097-2265.
use std::{ffi::OsString, path::{Path, PathBuf}, process::Command, sync::Arc};
use gpui::{Context, Window};
use crate::{actions::*, workspace::Workspace};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShellKind { CommandPrompt, PowerShell }
#[derive(Clone, Debug, PartialEq, Eq)]
struct Launch { executable: OsString, arguments: Vec<OsString>, directory: PathBuf }

#[cfg(target_os = "windows")]
fn launch_request(kind: ShellKind, directory: PathBuf) -> Result<Launch, String> {
    let executable = match kind {
        ShellKind::CommandPrompt => std::env::var_os("COMSPEC").filter(|value| !value.is_empty()).unwrap_or_else(|| "cmd.exe".into()),
        ShellKind::PowerShell => registered_powershell()?,
    };
    Ok(Launch { executable, arguments: Vec::new(), directory })
}
#[cfg(target_os = "macos")]
fn launch_request(kind: ShellKind, directory: PathBuf) -> Result<Launch, String> {
    let application = match kind { ShellKind::CommandPrompt => "Terminal", ShellKind::PowerShell => "PowerShell" };
    Ok(Launch { executable: "open".into(), arguments: vec!["-a".into(), application.into(), directory.as_os_str().to_owned()], directory })
}
#[cfg(target_os = "linux")]
fn launch_request(kind: ShellKind, directory: PathBuf) -> Result<Launch, String> {
    let arguments = match kind { ShellKind::CommandPrompt => Vec::new(), ShellKind::PowerShell => vec!["-e".into(), "pwsh".into()] };
    Ok(Launch { executable: "x-terminal-emulator".into(), arguments, directory })
}

// RunDlg/RunDlg.cpp:207-251 expands the executable's environment variables.
#[cfg(target_os = "windows")]
fn registered_powershell() -> Result<OsString, String> {
    #[cfg(test)]
    if let Some(answer) = tests::registry_answer() { return answer.and_then(expand_shell_environment); }
    use winreg::{RegKey, enums::{HKEY_LOCAL_MACHINE, KEY_READ, REG_SZ}};
    let key = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags("SOFTWARE\\Microsoft\\PowerShell\\1\\ShellIds\\Microsoft.PowerShell", KEY_READ)
        .map_err(|error| format!("PowerShell is not registered for this Windows installation: {error}"))?;
    if key.query_info().map_err(|error| error.to_string())?.max_value_len > 1024 {
        return Err("registered PowerShell configuration is too large; repair the Windows PowerShell installation".into());
    }
    let value = key.get_raw_value("Path").map_err(|error| format!("cannot read the registered PowerShell Path: {error}"))?;
    if value.vtype != REG_SZ || value.bytes.len() > 1024 || value.bytes.len() % 2 != 0 {
        return Err("registered PowerShell Path must be a string of at most 512 Windows characters".into());
    }
    use std::os::windows::ffi::OsStringExt;
    let mut words: Vec<u16> = value.bytes.as_chunks::<2>().0.iter()
        .map(|[low, high]| u16::from_le_bytes([*low, *high])).collect();
    while words.last() == Some(&0) { words.pop(); }
    expand_shell_environment(OsString::from_wide(&words))
}

#[cfg(target_os = "windows")]
fn expand_shell_environment(path: OsString) -> Result<OsString, String> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    let input: Vec<u16> = path.encode_wide().collect();
    if input.is_empty() || input.contains(&0) || input.len() > 512 {
        return Err("registered PowerShell Path is empty, invalid, or too long".into());
    }
    let mut output = Vec::new(); let mut offset = 0usize;
    while let Some(&word) = input.get(offset) {
        if word == u16::from(b'%') {
            if let Some(end) = input.get(offset.saturating_add(1)..).and_then(|suffix| suffix.iter().position(|&next| next == u16::from(b'%')))
                .and_then(|length| offset.checked_add(length)?.checked_add(1)) {
                let name = OsString::from_wide(input.get(offset.saturating_add(1)..end).unwrap_or_default());
                if let Some(value) = std::env::var_os(name) {
                    let words: Vec<u16> = value.encode_wide().collect();
                    if output.len().checked_add(words.len()).is_none_or(|total| total > 32767) {
                        return Err("expanded PowerShell Path exceeds the Windows path limit".into());
                    }
                    output.try_reserve(words.len()).map_err(|_| "not enough memory for the PowerShell Path")?;
                    output.extend(words); offset = end.saturating_add(1); continue;
                }
            }
        }
        if output.len() >= 32767 { return Err("expanded PowerShell Path exceeds the Windows path limit".into()); }
        output.try_reserve(1).map_err(|_| "not enough memory for the PowerShell Path")?;
        output.push(word); offset = offset.saturating_add(1);
    }
    Ok(OsString::from_wide(&output))
}

fn launch(request: Launch) -> Result<(), String> {
    #[cfg(test)]
    if let Some(answer) = tests::capture_launch(&request) { return answer; }
    let mut command = Command::new(&request.executable);
    command.args(&request.arguments).current_dir(&request.directory);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0000_0010); // CREATE_NEW_CONSOLE: this action explicitly opens an interactive terminal.
    }
    command.spawn().map(|_| ()).map_err(|error| error.to_string())
}

impl Workspace {
    pub(crate) fn file_destination_open_elsewhere(&mut self, target: &Path, saving: usize,
        current_window: gpui::AnyWindowHandle, cx: &mut Context<Self>) -> bool {
        if self.save_target_open_elsewhere(target, saving, cx) { return true; }
        let canonical = std::fs::canonicalize(target).ok(); let identity = file_identity(target);
        let matches = |path: &Path| path == target
            || canonical.as_ref().is_some_and(|target| std::fs::canonicalize(path).ok().as_ref() == Some(target))
            || identity.as_ref().is_some_and(|target| file_identity(path).as_ref() == Some(target));
        if let Some(other) = self.tabs.iter().enumerate().find_map(|(index, tab)| (index != saving
            && tab.buffer.file_path.as_deref().is_some_and(matches)).then_some(index)) {
            self.set_active_tab(other, cx); cx.notify(); return true;
        }
        let current = cx.entity().entity_id();
        for handle in cx.windows() {
            // update_in already leases this window; reading its root would borrow it twice.
            if handle.window_id() == current_window.window_id() { continue; }
            let Some(handle) = handle.downcast::<Workspace>() else { continue; };
            let Ok(entity) = handle.entity(cx) else { continue; };
            if entity.entity_id() == current { continue; }
            if entity.read(cx).tabs.iter().any(|tab| tab.buffer.file_path.as_deref().is_some_and(matches)) { return true; }
        }
        false
    }

    fn containing_directory(&mut self, cx: &mut Context<Self>) -> Option<PathBuf> {
        let Some(path) = self.tabs.get(self.active).and_then(|tab| tab.buffer.file_path.as_ref()) else {
            self.set_status("Current document has no file path"); cx.notify(); return None;
        };
        if path.to_string_lossy().contains('\0') { self.set_status("Current file path is invalid"); cx.notify(); return None; }
        let directory = std::path::absolute(path).ok().and_then(|path| path.parent().map(Path::to_path_buf));
        let Some(directory) = directory.filter(|path| path.is_dir()) else {
            self.set_status("Containing folder does not exist"); cx.notify(); return None;
        };
        Some(directory)
    }

    fn open_file_shell(&mut self, kind: ShellKind, cx: &mut Context<Self>) {
        let Some(directory) = self.containing_directory(cx) else { return; };
        match launch_request(kind, directory.clone()).and_then(launch) {
            Ok(()) => self.set_status(format!("Opened terminal in {}", directory.display())),
            Err(error) => self.set_status(format!("Open terminal failed: {error}")),
        }
        cx.notify();
    }
    pub(crate) fn file_open_containing_folder_cmd(&mut self, _: &FileOpenContainingFolderCmd, _: &mut Window, cx: &mut Context<Self>) {
        self.open_file_shell(ShellKind::CommandPrompt, cx);
    }
    pub(crate) fn file_open_containing_folder_powershell(&mut self, _: &FileOpenContainingFolderPowerShell, _: &mut Window, cx: &mut Context<Self>) {
        self.open_file_shell(ShellKind::PowerShell, cx);
    }
    pub(crate) fn file_containing_folder_as_workspace(&mut self, _: &FileContainingFolderAsWorkspace, _: &mut Window, cx: &mut Context<Self>) {
        let Some(directory) = self.containing_directory(cx) else { return; };
        let count = crate::ui::folder_panel::add_root(directory.clone());
        crate::ui::folder_panel::set_visible(true);
        self.set_status(format!("Added {} to Folder as Workspace ({count} roots)", directory.display())); cx.notify();
    }
    pub(crate) fn file_move_to_trash(&mut self, _: &FileMoveToTrash, window: &mut Window, cx: &mut Context<Self>) {
        self.move_tab_to_trash(self.active, window, cx);
    }

    pub(crate) fn file_rename(&mut self, _: &FileRename, window: &mut Window, cx: &mut Context<Self>) {
        if self.window_closed { return; }
        self.stop_file_move(); cx.notify();
        let Some(tab) = self.tabs.get(self.active) else { return; };
        let Some(path) = tab.buffer.file_path.as_ref().filter(|path| path.is_file()).cloned() else {
            self.open_tab_rename(self.active, cx);
            self.focus_input(crate::tab::InputFocus::TabRename, window, cx); return;
        };
        let id = tab.session_id; let instance = tab.instance.clone();
        let window_id = self.session_window_id; let native_window = window.window_handle();
        let request = uuid::Uuid::new_v4(); self.file_rename_request = Some(request);
        let selected = crate::io::pick_rename_path(&path);
        cx.spawn_in(window, async move |this, cx| {
            let destination = selected.await;
            let _ = this.update_in(cx, |w, window, cx| {
                if w.file_rename_request != Some(request) { return; }
                w.file_rename_request = None;
                let Some(destination) = destination else { return; };
                if w.window_closed || w.session_window_id != window_id || window.window_handle() != native_window { return; }
                let Some(index) = w.tabs.iter().position(|tab| tab.session_id == id
                    && Arc::ptr_eq(&tab.instance, &instance) && tab.buffer.file_path.as_ref() == Some(&path)) else {
                    w.set_status("Rename cancelled: original document or file path changed"); cx.notify(); return;
                };
                w.rename_tab_to_path(index, destination, window, cx);
            });
        }).detach();
    }

    fn rename_tab_to_path(&mut self, index: usize, destination: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.tabs.get(index).and_then(|tab| tab.buffer.file_path.clone()) else { return; };
        if destination == source { return; }
        if destination.to_string_lossy().contains('\0') {
            self.set_status("Rename failed — file path contains a null character"); cx.notify(); return;
        }
        if self.file_destination_open_elsewhere(&destination, index, window.window_handle(), cx) {
            self.set_status(crate::workspace::already_open_message()); cx.notify(); return;
        }
        match rename_on_disk(&source, &destination) {
            Ok(()) => self.finish_file_rename(index, source, destination, cx),
            Err(error) if error.kind() == std::io::ErrorKind::CrossesDevices => self.rename_across_filesystems(index, source, destination, window, cx),
            Err(error) => self.set_status(format!("Rename failed: {error}")),
        }
        cx.notify();
    }

    pub(crate) fn finish_file_rename(&mut self, index: usize, source: PathBuf, destination: PathBuf, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(index) else { return; };
        let id = tab.session_id;
        tab.buffer.file_path = Some(destination.clone());
        tab.buffer.language = crate::syntax::language_for_path(&destination).or(tab.buffer.language);
        self.retire_syntax_highlights(id, cx); self.request_syntax_refresh_for_tab(id, std::time::Duration::ZERO, cx);
        crate::recent::remove_path(&source); crate::recent::record_opened(destination.clone());
        crate::app::menus::rebuild_menus(cx);
        self.set_status(format!("Renamed to {}", destination.display())); self.save_session(cx);
    }

    pub(crate) fn rename_tab_label(&mut self, index: usize, name: String, cx: &mut Context<Self>) {
        let name = name.trim();
        if name.is_empty() || name == "." || name == ".." || name.chars().any(|character| matches!(character, '\0' | '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*')) {
            self.set_status("Rename failed: choose a valid document name"); cx.notify(); return;
        }
        // NppConstants.h:358; NppIO.cpp:2296 reserves the final UTF-16 unit for NUL.
        if name.encode_utf16().count() > 63 {
            self.set_status("Rename failed: document names can contain at most 63 Windows characters"); cx.notify(); return;
        }
        if self.tabs.iter().enumerate().any(|(other, tab)| other != index && tab.buffer.file_path.is_none()
            && tab.buffer.display_short_name().as_ref() == name) {
            self.set_status("Rename failed: that document name is already in use"); cx.notify(); return;
        }
        let Some(tab) = self.tabs.get_mut(index) else { return; };
        tab.buffer.new_label = Some(name.to_owned().into()); tab.buffer.file_path = None;
        self.set_status(format!("Renamed tab to {name}")); self.save_session(cx); cx.notify();
    }
}

#[cfg(target_os = "windows")]
fn file_identity(path: &Path) -> Option<file_id::FileId> { file_id::get_file_id(path).ok() }
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn file_identity(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|metadata| (metadata.dev(), metadata.ino()))
}
fn rename_on_disk(source: &Path, destination: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    if tests::force_cross_device() { return Err(std::io::Error::from(std::io::ErrorKind::CrossesDevices)); }
    std::fs::rename(source, destination)
}

#[cfg(test)]
#[path = "file_routes_tests.rs"]
mod tests;
