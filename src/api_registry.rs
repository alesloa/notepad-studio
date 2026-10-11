// PowerEditor/src/ScintillaComponent/AutoCompletion.cpp:1101-1233 and
// FunctionCallTip.cpp:306-379 — API XML schema and environment behavior.
// Original parser; no Notepad++ definitions or implementation are bundled.

use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
use std::path::Path;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

pub(crate) const MAX_API_BYTES: usize = 8 * 1024 * 1024;
const MAX_DEPTH: usize = 8;
const MAX_ENTRIES: usize = 100_000;
const MAX_OVERLOADS: usize = 64;
const MAX_PARAMETERS: usize = 128;
const MAX_ATTRIBUTE_BYTES: usize = 8 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ApiEnvironment {
    pub(crate) ignore_case: bool,
    pub(crate) start: char,
    pub(crate) stop: char,
    pub(crate) separator: char,
    pub(crate) terminal: char,
    pub(crate) additional_word_chars: String,
}

impl Default for ApiEnvironment {
    fn default() -> Self {
        Self { ignore_case: true, start: '(', stop: ')', separator: ',',
            terminal: ';', additional_word_chars: String::new() }
    }
}

impl ApiEnvironment {
    pub(crate) fn is_word_char(&self, c: char) -> bool {
        c == '_' || c.is_alphanumeric() || self.additional_word_chars.contains(c)
    }

    pub(crate) fn matches_prefix(&self, name: &str, prefix: &str) -> bool {
        name.get(..prefix.len()).is_some_and(|part| {
            if self.ignore_case { part.eq_ignore_ascii_case(prefix) } else { part == prefix }
        })
    }

    fn key(&self, name: &str) -> String {
        if self.ignore_case { name.to_ascii_lowercase() } else { name.to_owned() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ApiOverload {
    pub(crate) return_text: String,
    pub(crate) parameters: Vec<String>,
    pub(crate) description: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ApiEntry {
    pub(crate) name: String,
    pub(crate) is_function: bool,
    pub(crate) overloads: Vec<ApiOverload>,
    match_key: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ApiRegistry {
    pub(crate) environment: ApiEnvironment,
    entries: Vec<ApiEntry>,
}

#[derive(Debug)]
pub(crate) struct ApiError(String);

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) }
}

impl std::error::Error for ApiError {}

fn invalid(message: impl Into<String>) -> ApiError { ApiError(message.into()) }

impl ApiRegistry {
    pub(crate) fn parse(xml: &str) -> Result<Self, ApiError> {
        if xml.len() > MAX_API_BYTES { return Err(invalid("API definitions exceed the 8 MiB limit")); }
        let mut reader = Reader::from_str(xml.trim_start_matches('\u{feff}'));
        reader.config_mut().expand_empty_elements = true;
        reader.config_mut().check_comments = true;
        let mut parser = Parser::default();
        loop {
            let event = reader.read_event().map_err(|error| invalid(format!("Invalid API XML: {error}")))?;
            match event {
                Event::Start(tag) => parser.open(&tag)?,
                Event::End(_) => parser.close()?,
                Event::Text(text) => {
                    let text = text.unescape().map_err(|error| invalid(format!("Invalid API text: {error}")))?;
                    if !text.trim().is_empty() && parser.stack.last() != Some(&Frame::Ignored) {
                        return Err(invalid("API definitions store names and signatures in attributes"));
                    }
                }
                Event::CData(_) => return Err(invalid("CDATA is not supported in API definitions")),
                Event::DocType(_) => return Err(invalid("DTD and external entities are not supported in API definitions")),
                Event::Decl(declaration) => {
                    if parser.root_seen || parser.declaration_seen {
                        return Err(invalid("API XML has a misplaced or duplicate declaration"));
                    }
                    let version = declaration.version().map_err(|error| invalid(format!("Invalid XML declaration: {error}")))?;
                    if version.as_ref() != b"1.0" && version.as_ref() != b"1.1" {
                        return Err(invalid("API XML requires version 1.0 or 1.1"));
                    }
                    if let Some(encoding) = declaration.encoding() {
                        let encoding = encoding.map_err(|error| invalid(format!("Invalid XML encoding: {error}")))?;
                        if !encoding.eq_ignore_ascii_case(b"utf-8") && !encoding.eq_ignore_ascii_case(b"utf-16") {
                            return Err(invalid("API definitions must use UTF-8 or UTF-16"));
                        }
                    }
                    parser.declaration_seen = true;
                }
                Event::Eof => break,
                Event::Comment(_) | Event::PI(_) => {}
                Event::Empty(_) => return Err(invalid("Invalid empty API XML element")),
            }
        }
        if !parser.stack.is_empty() || !parser.root_seen || !parser.auto_seen {
            return Err(invalid("API XML requires a complete NotepadPlus/AutoComplete document"));
        }
        let environment = parser.environment;
        let mut sorted = BTreeMap::<String, ApiEntry>::new();
        for mut entry in parser.entries {
            let key = environment.key(&entry.name);
            entry.match_key = key.clone();
            if let Some(existing) = sorted.get_mut(&key) {
                if existing.is_function != entry.is_function {
                    return Err(invalid(format!("API name '{}' is both a keyword and a function", entry.name)));
                }
                if existing.overloads.len().saturating_add(entry.overloads.len()) > MAX_OVERLOADS {
                    return Err(invalid("An API function exceeds the 64 overload limit"));
                }
                existing.overloads.extend(entry.overloads);
            } else { sorted.insert(key, entry); }
        }
        Ok(Self { environment, entries: sorted.into_values().collect() })
    }

    /// Read on the background executor. A file growing during the read stays bounded.
    pub(crate) fn load(path: &Path) -> Result<Option<Self>, ApiError> {
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(invalid(format!("Cannot read {}: {error}", path.display()))),
        };
        let mut bytes = Vec::new();
        file.take((MAX_API_BYTES + 1) as u64).read_to_end(&mut bytes)
            .map_err(|error| invalid(format!("Cannot read {}: {error}", path.display())))?;
        if bytes.len() > MAX_API_BYTES { return Err(invalid("API definitions exceed the 8 MiB limit")); }
        let xml = if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
            let little_endian = bytes.starts_with(&[0xff, 0xfe]);
            let content = bytes.get(2..).ok_or_else(|| invalid("Invalid UTF-16 API definitions"))?;
            if content.len() % 2 != 0 { return Err(invalid("API definitions have an incomplete UTF-16 character")); }
            let units: Vec<_> = content.as_chunks::<2>().0.iter().map(|&pair| {
                if little_endian { u16::from_le_bytes(pair) } else { u16::from_be_bytes(pair) }
            }).collect();
            String::from_utf16(&units).map_err(|_| invalid("API definitions contain invalid UTF-16"))?
        } else {
            String::from_utf8(bytes).map_err(|_| invalid("API definitions contain invalid UTF-8"))?
        };
        Self::parse(&xml).map(Some)
    }

    pub(crate) fn entries(&self) -> &[ApiEntry] { &self.entries }

    pub(crate) fn find(&self, name: &str) -> Option<&ApiEntry> {
        let key = self.environment.key(name);
        let index = self.entries.binary_search_by(|entry| entry.match_key.cmp(&key)).ok()?;
        self.entries.get(index)
    }

    pub(crate) fn matching(&self, prefix: &str) -> impl Iterator<Item = &ApiEntry> {
        let key = self.environment.key(prefix);
        let start = self.entries.partition_point(|entry| entry.match_key < key);
        self.entries.get(start..).unwrap_or_default().iter()
            .take_while(move |entry| entry.match_key.starts_with(&key))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Frame { Root, AutoComplete, Environment, Keyword, Overload, Param, Ignored }

#[derive(Default)]
struct Parser {
    stack: Vec<Frame>,
    root_seen: bool,
    auto_seen: bool,
    declaration_seen: bool,
    environment_seen: bool,
    environment: ApiEnvironment,
    entries: Vec<ApiEntry>,
    entry: Option<ApiEntry>,
    overload: Option<ApiOverload>,
}

impl Parser {
    fn open(&mut self, tag: &BytesStart<'_>) -> Result<(), ApiError> {
        if self.stack.len() >= MAX_DEPTH { return Err(invalid("API XML exceeds the nesting limit")); }
        let attrs = attributes(tag)?;
        let parent = self.stack.last().copied();
        let frame = match (parent, tag.name().as_ref()) {
            (None, b"NotepadPlus") if !self.root_seen => { self.root_seen = true; Frame::Root }
            (Some(Frame::Root), b"AutoComplete") if !self.auto_seen => { self.auto_seen = true; Frame::AutoComplete }
            (Some(Frame::AutoComplete), b"Environment") if !self.environment_seen => {
                self.environment_seen = true;
                self.environment = environment(&attrs)?;
                Frame::Environment
            }
            (Some(Frame::AutoComplete), b"KeyWord") => {
                if self.entries.len() >= MAX_ENTRIES { return Err(invalid("API definitions exceed the 100,000 name limit")); }
                let name = required(&attrs, "name")?;
                if name.is_empty() || name.chars().any(char::is_control) {
                    return Err(invalid("An API name is empty or contains control characters"));
                }
                let is_function = yes_no(&attrs, "func", false)?;
                self.entry = Some(ApiEntry { name, is_function, overloads: Vec::new(), match_key: String::new() });
                Frame::Keyword
            }
            (Some(Frame::Keyword), b"Overload") => {
                let entry = self.entry.as_ref().ok_or_else(|| invalid("API overload has no function"))?;
                if !entry.is_function { return Err(invalid("A keyword cannot contain function overloads")); }
                if entry.overloads.len() >= MAX_OVERLOADS { return Err(invalid("An API function exceeds the 64 overload limit")); }
                self.overload = Some(ApiOverload { return_text: required(&attrs, "retVal")?,
                    parameters: Vec::new(), description: attrs.get("descr").cloned().unwrap_or_default() });
                Frame::Overload
            }
            (Some(Frame::Overload), b"Param") => {
                let overload = self.overload.as_mut().ok_or_else(|| invalid("API parameter has no overload"))?;
                if overload.parameters.len() >= MAX_PARAMETERS { return Err(invalid("An API overload exceeds the 128 parameter limit")); }
                overload.parameters.push(required(&attrs, "name")?);
                Frame::Param
            }
            (Some(Frame::Ignored), _) => Frame::Ignored,
            (None, _) => return Err(invalid("API XML has an invalid or duplicate root")),
            (_, b"NotepadPlus" | b"AutoComplete" | b"Environment" | b"KeyWord" | b"Overload" | b"Param") => {
                return Err(invalid("API XML has a misplaced or duplicate definition element"));
            }
            _ => Frame::Ignored,
        };
        self.stack.push(frame);
        Ok(())
    }

    fn close(&mut self) -> Result<(), ApiError> {
        match self.stack.pop().ok_or_else(|| invalid("API XML has an unmatched closing element"))? {
            Frame::Overload => {
                let overload = self.overload.take().ok_or_else(|| invalid("API overload is incomplete"))?;
                self.entry.as_mut().ok_or_else(|| invalid("API overload has no function"))?.overloads.push(overload);
            }
            Frame::Keyword => self.entries.push(self.entry.take().ok_or_else(|| invalid("API name is incomplete"))?),
            _ => {}
        }
        Ok(())
    }
}

fn attributes(tag: &BytesStart<'_>) -> Result<BTreeMap<String, String>, ApiError> {
    let mut attrs = BTreeMap::new();
    for attr in tag.attributes() {
        let attr = attr.map_err(|error| invalid(format!("Invalid API attribute: {error}")))?;
        if attrs.len() >= 32 || attr.key.as_ref().len() > 128 || attr.value.len() > MAX_ATTRIBUTE_BYTES {
            return Err(invalid("API attributes exceed their size or count limit"));
        }
        let key = std::str::from_utf8(attr.key.as_ref()).map_err(|_| invalid("Invalid API attribute name"))?;
        let value = attr.unescape_value().map_err(|error| invalid(format!("Invalid API attribute: {error}")))?;
        if value.chars().any(|c| c == '\0' || (c.is_control() && !matches!(c, '\n' | '\r' | '\t'))) {
            return Err(invalid("An API attribute contains invalid control characters"));
        }
        attrs.insert(key.to_owned(), value.into_owned());
    }
    Ok(attrs)
}

fn required(attrs: &BTreeMap<String, String>, key: &str) -> Result<String, ApiError> {
    attrs.get(key).cloned().ok_or_else(|| invalid(format!("API definition is missing '{key}'")))
}

fn yes_no(attrs: &BTreeMap<String, String>, key: &str, default: bool) -> Result<bool, ApiError> {
    match attrs.get(key).map(String::as_str) {
        None => Ok(default), Some("yes") => Ok(true), Some("no") => Ok(false),
        _ => Err(invalid(format!("API '{key}' must be 'yes' or 'no'"))),
    }
}

fn environment(attrs: &BTreeMap<String, String>) -> Result<ApiEnvironment, ApiError> {
    let mut env = ApiEnvironment { ignore_case: yes_no(attrs, "ignoreCase", true)?, ..Default::default() };
    for (key, target) in [("startFunc", &mut env.start), ("stopFunc", &mut env.stop),
        ("paramSeparator", &mut env.separator), ("terminal", &mut env.terminal)] {
        if let Some(value) = attrs.get(key) {
            let mut chars = value.chars();
            let character = chars.next().ok_or_else(|| invalid(format!("API '{key}' cannot be empty")))?;
            if chars.next().is_some() || character.is_control() || character.is_whitespace() || character.is_alphanumeric() {
                return Err(invalid(format!("API '{key}' must be one punctuation character")));
            }
            *target = character;
        }
    }
    let delimiters = [env.start, env.stop, env.separator, env.terminal];
    for (index, delimiter) in delimiters.iter().enumerate() {
        if delimiters.get(..index).is_some_and(|previous| previous.contains(delimiter)) {
            return Err(invalid("API call delimiters must be distinct"));
        }
    }
    if let Some(chars) = attrs.get("additionalWordChar") {
        if chars.len() > 128 || chars.chars().any(|c| c.is_control() || c.is_whitespace() || delimiters.contains(&c)) {
            return Err(invalid("API extra word characters are invalid or exceed 128 bytes"));
        }
        env.additional_word_chars = chars.clone();
    }
    Ok(env)
}

#[cfg(test)]
#[path = "api_registry_tests.rs"]
mod tests;
