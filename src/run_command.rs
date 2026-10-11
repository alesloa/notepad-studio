// PowerEditor/src/WinControls/StaticDialog/RunDlg/RunDlg.cpp:96-202 — variable
// names and command behaviour; original parser and launch implementation.
// Windows argument rules: learn.microsoft.com/en-us/cpp/c-language/parsing-c-command-line-arguments

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};

use gpui::AppContext;
use serde::{Deserialize, Serialize};

use crate::buffer::{Buffer, Selection};

pub(crate) const MAX_COMMAND_BYTES: usize = 32 * 1024;
const MAX_VARIABLE_BYTES: usize = 8192;
const HISTORY_LIMIT: usize = 20;
const MAX_HISTORY_BYTES: u64 = 1024 * 1024;
const MAX_SAVED_COMMANDS: usize = 100;
pub(crate) const MAX_NAME_BYTES: usize = 256;

pub(crate) const VARIABLES: &[&str] = &[
    "FULL_CURRENT_PATH",
    "CURRENT_DIRECTORY",
    "FILE_NAME",
    "NAME_PART",
    "EXT_PART",
    "CURRENT_WORD",
    "NPP_DIRECTORY",
    "NPP_FULL_FILE_PATH",
    "CURRENT_LINE",
    "CURRENT_COLUMN",
    "CURRENT_LINESTR",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Platform {
    Windows,
    Unix,
}

impl Platform {
    pub(crate) fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Unix
        }
    }
}

#[derive(Clone)]
pub(crate) struct RunContext {
    path: Option<PathBuf>,
    name: String,
    rope: ropey::Rope,
    selection: Selection,
    tab_width: usize,
    executable: PathBuf,
}

impl RunContext {
    pub(crate) fn from_buffer(buffer: &Buffer, selection: Selection) -> Result<Self, String> {
        if buffer.loading.is_some() {
            return Err("Wait for the document to finish loading before running a command.".into());
        }
        let (_, tab_width) =
            crate::settings::indent_for_lang(buffer.language.map(|lang| lang.key()));
        Ok(Self {
            path: buffer.file_path.clone(),
            name: buffer.display_short_name().to_string(),
            rope: buffer.rope.clone(),
            selection: selection.clamped(&buffer.rope),
            tab_width,
            executable: std::env::current_exe()
                .map_err(|err| format!("Cannot locate Notepad Studio: {err}"))?,
        })
    }

    fn value(&self, name: &str) -> Result<Option<String>, String> {
        let file = self
            .path
            .as_deref()
            .unwrap_or_else(|| Path::new(&self.name));
        let head = self.selection.head;
        let line = self.rope.byte_to_line(head);
        let value = match name {
            "FULL_CURRENT_PATH" => path_text(file)?,
            "CURRENT_DIRECTORY" => path_text(
                self.path
                    .as_deref()
                    .and_then(Path::parent)
                    .unwrap_or_else(|| Path::new("")),
            )?,
            "FILE_NAME" => path_text(
                file.file_name()
                    .map(Path::new)
                    .unwrap_or_else(|| Path::new("")),
            )?,
            "NAME_PART" => path_text(
                file.file_stem()
                    .map(Path::new)
                    .unwrap_or_else(|| Path::new("")),
            )?,
            "EXT_PART" => match file.extension() {
                Some(ext) => format!(".{}", path_text(Path::new(ext))?),
                None => String::new(),
            },
            "CURRENT_WORD" => self.current_word()?,
            "NPP_DIRECTORY" => {
                path_text(self.executable.parent().unwrap_or_else(|| Path::new("")))?
            }
            "NPP_FULL_FILE_PATH" => path_text(&self.executable)?,
            "CURRENT_LINE" => line.saturating_add(1).to_string(),
            "CURRENT_COLUMN" => {
                crate::ui::status_text::caret_column(&self.rope, head, self.tab_width).to_string()
            }
            "CURRENT_LINESTR" => {
                let Some(text) = self.rope.get_line(line) else {
                    return Ok(Some(String::new()));
                };
                bounded_text(text.chars())?
                    .trim_end_matches(['\r', '\n'])
                    .to_string()
            }
            _ => return Ok(None),
        };
        Ok(Some(value))
    }

    fn current_word(&self) -> Result<String, String> {
        let selected = self.selection.ordered();
        if !self.selection.is_empty() {
            let start = self.rope.byte_to_char(selected.start);
            let end = self.rope.byte_to_char(selected.end);
            return bounded_text(self.rope.chars_at(start).take(end.saturating_sub(start)));
        }
        let head = self.rope.byte_to_char(self.selection.head);
        let word = |c: char| c.is_alphanumeric() || c == '_';
        let mut start = head;
        let mut count = 0usize;
        while start > 0 && self.rope.get_char(start - 1).is_some_and(word) {
            start -= 1;
            count += 1;
            if count > MAX_VARIABLE_BYTES {
                return Err("Current word is too long for a Run variable.".into());
            }
        }
        let mut end = head;
        while self.rope.get_char(end).is_some_and(word) {
            end += 1;
            count += 1;
            if count > MAX_VARIABLE_BYTES {
                return Err("Current word is too long for a Run variable.".into());
            }
        }
        bounded_text(self.rope.chars_at(start).take(end.saturating_sub(start)))
    }
}

fn path_text(path: &Path) -> Result<String, String> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        "This path cannot be represented in a Run command. Browse to a program instead.".into()
    })
}

fn bounded_text(chars: impl Iterator<Item = char>) -> Result<String, String> {
    let mut text = String::new();
    for c in chars {
        if text.len().saturating_add(c.len_utf8()) > MAX_VARIABLE_BYTES {
            return Err(
                "Run variable text exceeds 8192 bytes. Use a smaller selection or line.".into(),
            );
        }
        text.push(c);
    }
    Ok(text)
}

/// Tokenize before expanding variables: a selected quote, space or shell
/// operator stays part of one literal argument. No shell interprets the result.
pub(crate) fn parse(source: &str, platform: Platform) -> Result<Vec<String>, String> {
    if source.len() > MAX_COMMAND_BYTES {
        return Err("Run command exceeds 32 KB.".into());
    }
    if source.contains('\0') {
        return Err("Run command contains a NUL character.".into());
    }
    match platform {
        Platform::Windows => parse_windows(source),
        Platform::Unix => parse_unix(source),
    }
}

fn parse_windows(source: &str) -> Result<Vec<String>, String> {
    let mut chars = source.chars().peekable();
    let mut tokens = Vec::new();
    while chars.peek().is_some() {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }
        let mut token = String::new();
        let mut quoted = false;
        while let Some(&c) = chars.peek() {
            if !quoted && c.is_whitespace() {
                break;
            }
            chars.next();
            if c == '\\' {
                let mut count = 1usize;
                while chars.peek() == Some(&'\\') {
                    chars.next();
                    count += 1;
                }
                if chars.peek() == Some(&'"') {
                    token.extend(std::iter::repeat_n('\\', count / 2));
                    chars.next();
                    if count % 2 == 1 {
                        token.push('"');
                    } else {
                        quoted = !quoted;
                    }
                } else {
                    token.extend(std::iter::repeat_n('\\', count));
                }
            } else if c == '"' {
                if quoted && chars.peek() == Some(&'"') {
                    chars.next();
                    token.push('"');
                } else {
                    quoted = !quoted;
                }
            } else {
                token.push(c);
            }
        }
        if quoted {
            return Err("Close the double quote in the Run command.".into());
        }
        tokens.push(token);
    }
    Ok(tokens)
}

fn parse_unix(source: &str) -> Result<Vec<String>, String> {
    let mut chars = source.chars().peekable();
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    let mut started = false;
    while let Some(c) = chars.next() {
        if c.is_whitespace() && quote.is_none() {
            if started {
                tokens.push(std::mem::take(&mut token));
                started = false;
            }
        } else if (c == '\'' || c == '"') && quote.is_none() {
            quote = Some(c);
            started = true;
        } else if quote == Some(c) {
            quote = None;
        } else if c == '\\' && quote != Some('\'') {
            let Some(next) = chars.next() else {
                return Err("Run command ends with an escape character.".into());
            };
            if quote == Some('"') && !matches!(next, '"' | '\\' | '$' | '`') {
                token.push('\\');
            }
            token.push(next);
            started = true;
        } else {
            token.push(c);
            started = true;
        }
    }
    if quote.is_some() {
        return Err("Close the quote in the Run command.".into());
    }
    if started {
        tokens.push(token);
    }
    Ok(tokens)
}

fn expand(
    source: &str,
    context: &RunContext,
    platform: Platform,
    env: &impl Fn(&str) -> Option<String>,
) -> Result<String, String> {
    let mut rest = source;
    let mut result = String::new();
    while !rest.is_empty() {
        let expansion = if let Some(after) = rest.strip_prefix("$(") {
            after
                .find(')')
                .map(|end| (end + 3, context.value(after.get(..end).unwrap_or_default())))
        } else if platform == Platform::Windows {
            rest.strip_prefix('%').and_then(|after| {
                after
                    .find('%')
                    .map(|end| (end + 2, Ok(env(after.get(..end).unwrap_or_default()))))
            })
        } else {
            rest.strip_prefix("${").and_then(|after| {
                after
                    .find('}')
                    .map(|end| (end + 3, Ok(env(after.get(..end).unwrap_or_default()))))
            })
        };
        if let Some((consumed, value)) = expansion {
            if let Some(value) = value? {
                if value.contains('\0') {
                    return Err("Run variable contains a NUL character.".into());
                }
                if result.len().saturating_add(value.len()) > MAX_COMMAND_BYTES {
                    return Err("Expanded Run command exceeds 32 KB.".into());
                }
                result.push_str(&value);
                rest = rest.get(consumed..).unwrap_or_default();
            } else {
                let Some(c) = rest.chars().next() else {
                    break;
                };
                result.push(c);
                rest = rest.get(c.len_utf8()..).unwrap_or_default();
            }
        } else {
            let Some(c) = rest.chars().next() else {
                break;
            };
            result.push(c);
            rest = rest.get(c.len_utf8()..).unwrap_or_default();
        }
        if result.len() > MAX_COMMAND_BYTES {
            return Err("Expanded Run command exceeds 32 KB.".into());
        }
    }
    Ok(result)
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PreparedCommand {
    pub(crate) program: String,
    pub(crate) arguments: Vec<String>,
    pub(crate) directory: Option<PathBuf>,
}

pub(crate) fn prepare(
    source: &str,
    context: &RunContext,
    platform: Platform,
    env: impl Fn(&str) -> Option<String>,
) -> Result<PreparedCommand, String> {
    let mut tokens = parse(source, platform)?.into_iter();
    let first = tokens.next().ok_or("Enter the program to run.")?;
    let program = expand(&first, context, platform, &env)?;
    if program.is_empty() {
        return Err("Enter the program to run.".into());
    }
    let mut arguments = Vec::new();
    let mut bytes = program.len();
    for token in tokens {
        let value = expand(&token, context, platform, &env)?;
        bytes = bytes.saturating_add(value.len()).saturating_add(1);
        if bytes > MAX_COMMAND_BYTES {
            return Err("Expanded Run command exceeds 32 KB.".into());
        }
        arguments.push(value);
    }
    if platform == Platform::Windows
        && Path::new(program.trim_end_matches(['.', ' ']))
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("bat") || e.eq_ignore_ascii_case("cmd"))
    {
        return Err("Batch files need an explicit interpreter. Run cmd.exe /c with a command you control, or choose an .exe program.".into());
    }
    Ok(PreparedCommand {
        program,
        arguments,
        directory: context
            .path
            .as_deref()
            .and_then(Path::parent)
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf),
    })
}

fn process(prepared: &PreparedCommand) -> Result<Command, String> {
    let entered = Path::new(&prepared.program);
    let directory = prepared
        .directory
        .clone()
        .map(|dir| {
            if dir.is_absolute() {
                Ok(dir)
            } else {
                std::env::current_dir()
                    .map(|cwd| cwd.join(dir))
                    .map_err(|err| format!("Cannot resolve the command directory: {err}"))
            }
        })
        .transpose()?;
    let candidate = directory.as_deref().map(|dir| dir.join(entered));
    let local_program = candidate.as_deref().filter(|path| {
        entered.is_relative() && (entered.components().count() > 1 || path.is_file())
    });
    let mut command = Command::new(local_program.unwrap_or(entered));
    command.args(&prepared.arguments).stdin(Stdio::null());
    if let Some(directory) = directory {
        command.current_dir(directory);
    }
    Ok(command)
}

pub(crate) fn launch(prepared: PreparedCommand) -> Result<(), String> {
    // A no-argument document, directory or URL uses its registered handler.
    let entered_path = Path::new(&prepared.program);
    let resolved_path = if entered_path.is_relative() {
        prepared
            .directory
            .as_deref()
            .map(|dir| dir.join(entered_path))
            .unwrap_or_else(|| entered_path.to_path_buf())
    } else {
        entered_path.to_path_buf()
    };
    let path = resolved_path.as_path();
    let url = prepared.program.contains("://") || prepared.program.starts_with("mailto:");
    let document = path.exists() && !is_executable(path);
    if prepared.arguments.is_empty() && (url || document) {
        let target = if url { entered_path } else { path };
        return open::that_detached(target)
            .map_err(|err| format!("Cannot open {}: {err}", prepared.program));
    }
    let mut child = process(&prepared)?
        .spawn()
        .map_err(|err| format!("Cannot run {}: {err}", prepared.program))?;
    // Reap on a separate thread on Unix; the editor never waits for the child.
    let _ = std::thread::Builder::new()
        .name("run-command-wait".into())
        .spawn(move || {
            let _ = child.wait();
        });
    Ok(())
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(windows)]
    {
        path.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe") || ext.eq_ignore_ascii_case("com"))
    }
}

// PowerEditor/src/WinControls/StaticDialog/RunDlg/RunDlg.cpp:413-454 — Save
// adds a named command to the Run menu. Names and commands remain unexpanded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SavedRunCommand {
    #[serde(default)]
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) command: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct RunHistory {
    #[serde(default)]
    pub(crate) commands: Vec<String>,
    #[serde(default)]
    pub(crate) saved: Vec<SavedRunCommand>,
}

impl RunHistory {
    pub(crate) fn save_named(
        &mut self,
        id: Option<&str>,
        name: &str,
        command: &str,
    ) -> Result<SavedRunCommand, String> {
        let name = name.trim();
        let command = command.trim();
        validate_name(name)?;
        validate_command(command)?;
        let words = parse(command, Platform::current())?;
        if words.first().is_none_or(|word| word.is_empty()) {
            return Err("Enter a program or document to run before saving.".into());
        }
        let existing = match id {
            Some(id) => Some(
                self.saved
                    .iter()
                    .position(|saved| saved.id == id)
                    .ok_or_else(|| {
                        "This saved command was removed. Save it as a new command instead."
                            .to_string()
                    })?,
            ),
            None => None,
        };
        let normalized = name.to_lowercase();
        if self.saved.iter().enumerate().any(|(index, saved)| {
            Some(index) != existing && saved.name.to_lowercase() == normalized
        }) {
            return Err("A saved command already has this name. Choose another name.".into());
        }
        if existing.is_none() && self.saved.len() >= MAX_SAVED_COMMANDS {
            return Err("At most 100 Run commands can be saved. Delete a command first.".into());
        }
        let saved = SavedRunCommand {
            id: id
                .map(str::to_string)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name: name.to_string(),
            command: command.to_string(),
        };
        if let Some(existing) = existing {
            if let Some(slot) = self.saved.get_mut(existing) {
                *slot = saved.clone();
            }
        } else {
            self.saved.push(saved.clone());
        }
        Ok(saved)
    }

    pub(crate) fn remove_named(&mut self, id: &str) -> Result<(), String> {
        let before = self.saved.len();
        self.saved.retain(|saved| saved.id != id);
        if before == self.saved.len() {
            return Err("This saved command was already removed.".into());
        }
        Ok(())
    }

    pub(crate) fn record(&mut self, source: &str) {
        let source = source.trim();
        if source.is_empty()
            || source.len() > MAX_COMMAND_BYTES
            || source.contains(['\0', '\r', '\n'])
        {
            return;
        }
        self.commands.retain(|command| command != source);
        self.commands.insert(0, source.to_string());
        self.commands.truncate(HISTORY_LIMIT);
    }

    pub(crate) fn load(path: &Path) -> Result<Self, String> {
        use std::io::Read;
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(err) => return Err(format!("Cannot read Run history: {err}")),
        };
        let mut bytes = Vec::new();
        file.take(MAX_HISTORY_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|err| format!("Cannot read Run history: {err}"))?;
        if bytes.len() as u64 > MAX_HISTORY_BYTES {
            return Err("Run history exceeds 1 MB; reduce or remove run.toml.".into());
        }
        let loaded = match std::str::from_utf8(&bytes)
            .ok()
            .and_then(|text| toml::from_str::<Self>(text).ok())
        {
            Some(history) => history,
            None => {
                // Preserve the rejected state before a future successful Run.
                if crate::persist::quarantine(path).is_none() {
                    return Err(
                        "Run history is damaged and could not be preserved as run.toml.corrupt."
                            .into(),
                    );
                }
                Self::default()
            }
        };
        validate_saved(&loaded.saved)?;
        let mut normalized = Self {
            saved: loaded.saved,
            ..Self::default()
        };
        for command in loaded.commands.iter().rev() {
            normalized.record(command);
        }
        Ok(normalized)
    }

    pub(crate) fn save(&self, path: &Path) -> Result<(), String> {
        validate_saved(&self.saved)?;
        let data = toml::to_string_pretty(self).map_err(|err| err.to_string())?;
        if data.len() as u64 > MAX_HISTORY_BYTES {
            return Err(
                "Run settings exceed 1 MB. Delete a saved command or shorten its text.".into(),
            );
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        crate::persist::atomic_write(path, data.as_bytes()).map_err(|err| err.to_string())
    }
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > MAX_NAME_BYTES || name.chars().any(char::is_control) {
        return Err(
            "Command names must be nonempty, at most 256 bytes, and contain no control characters."
                .into(),
        );
    }
    Ok(())
}

fn validate_command(command: &str) -> Result<(), String> {
    if command.is_empty()
        || command.len() > MAX_COMMAND_BYTES
        || command.contains(['\0', '\r', '\n'])
    {
        return Err("Saved Run commands must be a single nonempty line of at most 32 KB.".into());
    }
    Ok(())
}

fn validate_saved(saved: &[SavedRunCommand]) -> Result<(), String> {
    if saved.len() > MAX_SAVED_COMMANDS {
        return Err(
            "run.toml contains more than 100 saved commands. Reduce its saved list.".into(),
        );
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut names = std::collections::BTreeSet::new();
    for entry in saved {
        validate_name(&entry.name)?;
        validate_command(&entry.command)?;
        if uuid::Uuid::parse_str(&entry.id).is_err() || !ids.insert(&entry.id) {
            return Err("run.toml has an invalid or repeated saved command id. Correct its saved list before saving.".into());
        }
        if !names.insert(entry.name.to_lowercase()) {
            return Err(
                "run.toml has repeated saved command names. Choose unique names before saving."
                    .into(),
            );
        }
    }
    Ok(())
}

static SAVED_COMMANDS: OnceLock<Mutex<Vec<SavedRunCommand>>> = OnceLock::new();
// Recent-command recording and named-command edits share run.toml. Serialize
// the read/modify/write so one editor cannot overwrite another editor's save.
static RUN_FILE_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn saved_snapshot() -> Vec<SavedRunCommand> {
    SAVED_COMMANDS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .map(|saved| saved.clone())
        .unwrap_or_default()
}

fn publish_saved(saved: &[SavedRunCommand]) -> Result<(), String> {
    let mut current = SAVED_COMMANDS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .map_err(|_| {
            "Saved Run command state is unavailable. Restart Notepad Studio.".to_string()
        })?;
    *current = saved.to_vec();
    Ok(())
}

fn update_at<T>(
    path: &Path,
    update: impl FnOnce(&mut RunHistory) -> Result<T, String>,
) -> Result<(T, RunHistory), String> {
    let mut history = RunHistory::load(path)?;
    let result = update(&mut history)?;
    history.save(path)?;
    Ok((result, history))
}

pub(crate) fn update_config<T>(
    update: impl FnOnce(&mut RunHistory) -> Result<T, String>,
) -> Result<(T, RunHistory), String> {
    let path = history_path()
        .ok_or_else(|| "No configuration directory is available for Run commands.".to_string())?;
    let _guard = RUN_FILE_LOCK
        .lock()
        .map_err(|_| "Run settings are unavailable. Restart Notepad Studio.".to_string())?;
    let result = update_at(&path, update)?;
    publish_saved(&result.1.saved)?;
    Ok(result)
}

pub(crate) fn init_saved_commands(cx: &mut gpui::App) {
    let task = cx.background_spawn(async {
        let _guard = RUN_FILE_LOCK
            .lock()
            .map_err(|_| "Run settings are unavailable.".to_string())?;
        let history = history_path()
            .map(|path| RunHistory::load(&path))
            .unwrap_or_else(|| Ok(RunHistory::default()))?;
        publish_saved(&history.saved)
    });
    cx.spawn(async move |cx| match task.await {
        Ok(()) => {
            cx.update(crate::app::menus::rebuild_menus);
        }
        Err(err) => eprintln!("[run] {err}"),
    })
    .detach();
}

pub(crate) fn history_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join(crate::consts::APP_NAME).join("run.toml"))
}

/// Quote a browsed path for the command input using the platform's grammar.
pub(crate) fn quote_path(path: &str, platform: Platform) -> String {
    if platform == Platform::Unix {
        return format!("'{}'", path.replace('\'', "'\\''"));
    }
    let mut quoted = String::from("\"");
    let mut backslashes = 0usize;
    for c in path.chars() {
        if c == '\\' {
            backslashes += 1;
            continue;
        }
        quoted.extend(std::iter::repeat_n(
            '\\',
            if c == '"' {
                backslashes.saturating_mul(2).saturating_add(1)
            } else {
                backslashes
            },
        ));
        backslashes = 0;
        quoted.push(c);
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes.saturating_mul(2)));
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_context(text: &str, selection: Selection) -> RunContext {
        RunContext {
            path: Some(PathBuf::from("/samples/café.test.rs")),
            name: "café.test.rs".into(),
            rope: ropey::Rope::from_str(text),
            selection: selection.clamped(&ropey::Rope::from_str(text)),
            tab_width: 4,
            executable: PathBuf::from("/app/notepad-studio"),
        }
    }

    #[test]
    fn windows_keeps_paths_empty_arguments_and_escaped_quotes() {
        assert_eq!(
            parse(
                r#""C:\Program Files\tool.exe" "" "a b" C:\folder\file.txt "a\"b""#,
                Platform::Windows
            )
            .unwrap(),
            [
                "C:\\Program Files\\tool.exe",
                "",
                "a b",
                "C:\\folder\\file.txt",
                "a\"b"
            ]
        );
        assert_eq!(
            parse(r#"tool "a""b""#, Platform::Windows).unwrap(),
            ["tool", "a\"b"]
        );
        assert!(parse("tool \"unfinished", Platform::Windows).is_err());
    }

    #[test]
    fn unix_quotes_escapes_and_shell_operators_are_literal() {
        assert_eq!(
            parse(r#"tool 'a b' "" c\ d "a\q" & | >"#, Platform::Unix).unwrap(),
            ["tool", "a b", "", "c d", "a\\q", "&", "|", ">"]
        );
        assert!(parse("tool 'unfinished", Platform::Unix).is_err());
        assert!(parse("tool trailing\\", Platform::Unix).is_err());
    }

    #[test]
    fn browsed_paths_roundtrip_on_both_platforms() {
        for platform in [Platform::Windows, Platform::Unix] {
            for path in [
                r#"C:\Program Files\café & "test"\"#,
                "/tmp/café's tool",
                r#"\\server\share\tool.exe"#,
            ] {
                assert_eq!(
                    parse(&quote_path(path, platform), platform).unwrap(),
                    [path]
                );
            }
        }
    }

    #[test]
    fn variables_use_one_based_lines_visual_columns_and_native_paths() {
        let context = sample_context(
            "first\r\n\t中 café\r\n",
            Selection {
                anchor: 12,
                head: 12,
                is_dragging: false,
            },
        );
        let prepared = prepare("tool $(FILE_NAME) $(NAME_PART) $(EXT_PART) $(CURRENT_LINE) $(CURRENT_COLUMN) $(CURRENT_LINESTR)", &context, Platform::Unix, |_| None).unwrap();
        assert_eq!(
            prepared.arguments,
            ["café.test.rs", "café.test", ".rs", "2", "7", "\t中 café"]
        );
        assert_eq!(
            context.value("NPP_DIRECTORY").unwrap().as_deref(),
            Some("/app")
        );
        assert_eq!(
            context.value("FULL_CURRENT_PATH").unwrap().as_deref(),
            Some("/samples/café.test.rs")
        );
    }

    #[test]
    fn substituted_quotes_and_operators_cannot_add_arguments() {
        let text = "café \" & $(UNKNOWN)";
        let context = sample_context(
            text,
            Selection {
                anchor: 0,
                head: text.len(),
                is_dragging: false,
            },
        );
        for platform in [Platform::Windows, Platform::Unix] {
            let prepared = prepare("tool $(CURRENT_WORD)", &context, platform, |_| None).unwrap();
            assert_eq!(prepared.arguments, [text]);
            let command = process(&prepared).unwrap();
            assert_eq!(
                command.get_args().collect::<Vec<_>>(),
                [std::ffi::OsStr::new(text)]
            );
        }
    }

    #[test]
    fn word_at_caret_unsaved_file_and_stale_offsets_are_safe() {
        let mut context = sample_context(
            "中 café_word!",
            Selection {
                anchor: 9,
                head: 9,
                is_dragging: false,
            },
        );
        assert_eq!(
            context.value("CURRENT_WORD").unwrap().as_deref(),
            Some("café_word")
        );
        context.path = None;
        context.name = "new 1".into();
        assert_eq!(
            context.value("FULL_CURRENT_PATH").unwrap().as_deref(),
            Some("new 1")
        );
        assert_eq!(
            context.value("CURRENT_DIRECTORY").unwrap().as_deref(),
            Some("")
        );
        assert_eq!(context.value("EXT_PART").unwrap().as_deref(), Some(""));
        let context = sample_context(
            "中",
            Selection {
                anchor: usize::MAX,
                head: 1,
                is_dragging: false,
            },
        );
        assert_eq!(
            context.value("CURRENT_WORD").unwrap().as_deref(),
            Some("中")
        );
    }

    #[test]
    fn environment_and_unknown_variables_are_not_reparsed() {
        let context = sample_context("", Selection::default());
        let env = |name: &str| (name == "TOOLS").then(|| "café path & x".into());
        let win = prepare(
            "tool %TOOLS% $(UNKNOWN) $(unclosed",
            &context,
            Platform::Windows,
            env,
        )
        .unwrap();
        assert_eq!(win.arguments, ["café path & x", "$(UNKNOWN)", "$(unclosed"]);
        let unix = prepare("tool ${TOOLS}", &context, Platform::Unix, env).unwrap();
        assert_eq!(unix.arguments, ["café path & x"]);
    }

    #[test]
    fn command_and_variable_limits_and_batch_errors_are_actionable() {
        let context = sample_context("x", Selection::default());
        for source in ["", " \t", "\"\"", "tool \0arg"] {
            assert!(prepare(source, &context, Platform::Windows, |_| None).is_err());
        }
        assert!(parse(&"x".repeat(MAX_COMMAND_BYTES + 1), Platform::Unix).is_err());
        assert!(
            prepare("tool.cmd arg", &context, Platform::Windows, |_| None)
                .unwrap_err()
                .contains("explicit interpreter")
        );
        assert!(prepare("\"tool.CMD. \" arg", &context, Platform::Windows, |_| None).is_err());
        assert!(
            prepare(
                "tool $(CURRENT_WORD)",
                &sample_context(&"x".repeat(MAX_VARIABLE_BYTES + 1), Selection::default()),
                Platform::Unix,
                |_| None
            )
            .is_err()
        );
        assert!(
            prepare("tool ${BIG} ${BIG}", &context, Platform::Unix, |_| Some(
                "x".repeat(MAX_COMMAND_BYTES / 2)
            ))
            .is_err()
        );
    }

    #[test]
    fn recent_commands_are_bounded_unique_and_compatible_with_empty_state() {
        let mut history = RunHistory::default();
        for n in 0..25 {
            history.record(&format!("tool {n}"));
        }
        history.record(" tool 10 ");
        history.record("tool\nnext");
        assert!(
            !history
                .commands
                .iter()
                .any(|command| command.contains('\n'))
        );
        assert_eq!(history.commands.len(), HISTORY_LIMIT);
        assert_eq!(
            history.commands.first().map(String::as_str),
            Some("tool 10")
        );
        assert_eq!(
            history
                .commands
                .iter()
                .filter(|c| c.as_str() == "tool 10")
                .count(),
            1
        );
        assert!(
            toml::from_str::<RunHistory>("")
                .unwrap()
                .commands
                .is_empty()
        );
        let serialized = toml::to_string(&history).unwrap();
        assert_eq!(
            toml::from_str::<RunHistory>(&serialized).unwrap().commands,
            history.commands
        );
    }

    #[test]
    fn history_persists_unicode_commands_and_recovers_corrupt_toml() {
        let directory =
            std::env::temp_dir().join(format!("notepad-studio-run-{}", uuid::Uuid::new_v4()));
        let path = directory.join("run.toml");
        let mut history = RunHistory::default();
        history.record("tool 'café 中' $(CURRENT_WORD)");
        history.save(&path).unwrap();
        assert_eq!(RunHistory::load(&path).unwrap().commands, history.commands);
        std::fs::write(&path, "commands = [").unwrap();
        assert!(RunHistory::load(&path).unwrap().commands.is_empty());
        assert!(directory.join("run.toml.corrupt").exists());
        std::fs::remove_file(directory.join("run.toml.corrupt")).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn oversized_history_is_rejected_without_overwriting_it() {
        let directory =
            std::env::temp_dir().join(format!("notepad-studio-run-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("run.toml");
        std::fs::write(&path, vec![b' '; MAX_HISTORY_BYTES as usize + 1]).unwrap();
        assert!(RunHistory::load(&path).unwrap_err().contains("1 MB"));
        assert_eq!(path.metadata().unwrap().len(), MAX_HISTORY_BYTES + 1);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn named_commands_survive_reopen_and_recent_history_updates() {
        let directory = crate::persist::TempDir::new("saved-run-reopen");
        let path = directory.join("run.toml");
        let saved = SavedRunCommand {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Build café & 中".into(),
            command: "tool $(FULL_CURRENT_PATH) $(CURRENT_WORD)".into(),
        };
        let state = RunHistory {
            saved: vec![saved.clone()],
            ..Default::default()
        };
        state.save(&path).unwrap();
        let mut reopened = RunHistory::load(&path).unwrap();
        assert_eq!(reopened.saved.as_slice(), std::slice::from_ref(&saved));
        reopened.record("another tool");
        reopened.save(&path).unwrap();
        assert_eq!(RunHistory::load(&path).unwrap().saved, [saved]);
    }

    #[test]
    fn names_ids_and_updates_stay_unique_without_losing_order() {
        let mut state = RunHistory::default();
        let first = state
            .save_named(None, " Build café & 中 ", " tool $(CURRENT_WORD) ")
            .unwrap();
        let second = state.save_named(None, "Other", "other").unwrap();
        assert_eq!(first.name, "Build café & 中");
        assert_eq!(first.command, "tool $(CURRENT_WORD)");
        assert!(
            state
                .save_named(None, "BUILD CAFÉ & 中", "duplicate")
                .is_err()
        );
        let edited = state
            .save_named(Some(&first.id), "Rename", "new $(FULL_CURRENT_PATH)")
            .unwrap();
        assert_eq!(edited.id, first.id);
        assert_eq!(state.saved, [edited.clone(), second.clone()]);
        state.remove_named(&edited.id).unwrap();
        assert_eq!(state.saved, [second]);
        assert!(
            state
                .save_named(Some(&edited.id), "Removed", "tool")
                .is_err()
        );
        assert!(state.remove_named(&edited.id).is_err());
    }

    #[test]
    fn saved_definitions_reject_unbounded_names_commands_and_counts() {
        let mut state = RunHistory::default();
        for name in [
            "".to_string(),
            " \t ".into(),
            "line\nnext".into(),
            "x".repeat(MAX_NAME_BYTES + 1),
        ] {
            assert!(state.save_named(None, &name, "tool").is_err());
        }
        for command in [
            "".to_string(),
            "\"\"".into(),
            "tool \0arg".into(),
            "tool\nnext".into(),
            "tool \"unfinished".into(),
            "x".repeat(MAX_COMMAND_BYTES + 1),
        ] {
            assert!(state.save_named(None, "Name", &command).is_err());
        }
        for index in 0..MAX_SAVED_COMMANDS {
            state
                .save_named(None, &format!("Command {index}"), "tool")
                .unwrap();
        }
        assert!(
            state
                .save_named(None, "One more", "tool")
                .unwrap_err()
                .contains("100")
        );
        let first = state.saved[0].id.clone();
        assert!(state.save_named(Some(&first), "Changed", "other").is_ok());
    }

    #[test]
    fn old_recent_only_toml_loads_and_failed_updates_preserve_bytes() {
        let directory = crate::persist::TempDir::new("saved-run-compatibility");
        let path = directory.join("run.toml");
        std::fs::write(&path, "commands = ['tool café']\n").unwrap();
        let old = RunHistory::load(&path).unwrap();
        assert_eq!(old.commands, ["tool café"]);
        assert!(old.saved.is_empty());
        let (saved, _) = update_at(&path, |state| state.save_named(None, "First", "tool")).unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(update_at(&path, |state| state.save_named(None, "FIRST", "other")).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let (_, state) = update_at(&path, |state| {
            state.record("later");
            Ok(())
        })
        .unwrap();
        assert_eq!(state.saved, [saved]);
        assert_eq!(state.commands, ["later", "tool café"]);
    }

    #[test]
    fn invalid_saved_state_is_reported_without_overwriting_or_quarantining_it() {
        let directory = crate::persist::TempDir::new("saved-run-invalid");
        let path = directory.join("run.toml");
        let invalid =
            "commands = ['tool']\n[[saved]]\nid = 'invalid'\nname = 'Broken'\ncommand = 'tool'\n";
        std::fs::write(&path, invalid).unwrap();
        assert!(RunHistory::load(&path).unwrap_err().contains("id"));
        assert!(
            update_at(&path, |state| {
                state.record("next");
                Ok(())
            })
            .is_err()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), invalid);
        assert!(!directory.join("run.toml.corrupt").exists());
        let command = SavedRunCommand {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Same".into(),
            command: "tool".into(),
        };
        assert!(validate_saved(&[command.clone(), command]).is_err());
    }

    #[test]
    fn oversized_serialization_leaves_existing_run_settings_intact() {
        let directory = crate::persist::TempDir::new("saved-run-size");
        let path = directory.join("run.toml");
        std::fs::write(&path, "commands = []\n").unwrap();
        let mut state = RunHistory::default();
        for index in 0..40 {
            state
                .save_named(
                    None,
                    &format!("Command {index}"),
                    &format!("tool {}", "x".repeat(MAX_COMMAND_BYTES - 5)),
                )
                .unwrap();
        }
        assert!(state.save(&path).unwrap_err().contains("1 MB"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "commands = []\n");
    }

    #[test]
    fn reopened_named_command_expands_current_context_and_preserves_native_argv() {
        let directory = crate::persist::TempDir::new("saved-run-native-argv");
        let path = directory.join("run.toml");
        let output = directory.join("argv.json");
        let executable = std::env::current_exe().unwrap();
        let platform = Platform::current();
        let source = format!(
            "{} --exact run_command::tests::process_fixture --nocapture -- {} {}",
            quote_path(executable.to_str().unwrap(), platform),
            quote_path("$(CURRENT_WORD)", platform),
            quote_path("$(FULL_CURRENT_PATH)", platform)
        );
        let mut state = RunHistory::default();
        state
            .save_named(None, "Fixture café & 中", &source)
            .unwrap();
        state.save(&path).unwrap();
        let reopened = RunHistory::load(&path).unwrap();
        let argument = "café 中 \" & | >";
        let mut context = sample_context(
            argument,
            Selection {
                anchor: 0,
                head: argument.len(),
                ..Default::default()
            },
        );
        let document = directory.join("current café & 中.txt");
        context.path = Some(document.clone());
        let prepared = prepare(&reopened.saved[0].command, &context, platform, |_| None).unwrap();
        let mut child = process(&prepared).unwrap();
        let status = child
            .env("NOTEPAD_RUN_FIXTURE_OUTPUT", &output)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(
            serde_json::from_slice::<Vec<String>>(&std::fs::read(output).unwrap()).unwrap(),
            [argument, document.to_str().unwrap()]
        );
    }

    // A subprocess of the unit-test executable records its literal argv into
    // a neutral temporary fixture. The ordinary full-suite invocation is inert.
    #[test]
    fn process_fixture() {
        let Some(path) = std::env::var_os("NOTEPAD_RUN_FIXTURE_OUTPUT") else {
            return;
        };
        let args: Vec<_> = std::env::args()
            .skip_while(|arg| arg != "--")
            .skip(1)
            .collect();
        std::fs::write(path, serde_json::to_vec(&args).unwrap()).unwrap();
    }

    #[test]
    fn native_process_spawn_preserves_literal_arguments_and_working_directory() {
        let directory =
            std::env::temp_dir().join(format!("notepad-studio-run-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let output = directory.join("argv.json");
        let arguments = [
            "",
            "café 中",
            "\" & | > $(CURRENT_WORD)",
            r#"C:\folder with spaces\"#,
        ];
        let prepared = PreparedCommand {
            program: std::env::current_exe()
                .unwrap()
                .to_str()
                .unwrap()
                .to_string(),
            arguments: [
                "--exact",
                "run_command::tests::process_fixture",
                "--nocapture",
                "--",
            ]
            .into_iter()
            .chain(arguments)
            .map(str::to_string)
            .collect(),
            directory: Some(directory.clone()),
        };
        let mut command = process(&prepared).unwrap();
        assert_eq!(command.get_current_dir(), Some(directory.as_path()));
        let status = command
            .env("NOTEPAD_RUN_FIXTURE_OUTPUT", &output)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(
            serde_json::from_slice::<Vec<String>>(&std::fs::read(&output).unwrap()).unwrap(),
            arguments
        );
        std::fs::remove_file(output).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn relative_executable_paths_resolve_from_the_document_directory() {
        let directory = std::env::temp_dir().join("notepad-studio-run-sample");
        let executable = Path::new("tools").join("program.exe");
        let prepared = PreparedCommand {
            program: executable.to_str().unwrap().into(),
            arguments: vec![],
            directory: Some(directory.clone()),
        };
        let command = process(&prepared).unwrap();
        assert_eq!(Path::new(command.get_program()), directory.join(executable));
        assert_eq!(command.get_current_dir(), Some(directory.as_path()));
    }
}
