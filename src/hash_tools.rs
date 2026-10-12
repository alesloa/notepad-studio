// NppCommands.cpp:3577-3729; MISC/md5/md5Dlgs.cpp:359-510.
// Independent streamed hashing of raw files, dialog text and editor bytes.
use std::{io::{self, Read}, ops::Range, sync::atomic::{AtomicBool, AtomicU64, Ordering}};
use ropey::Rope;
use sha2::Digest;
use crate::text::{encoding::Encoding, line_ending::LineEnding};

pub(crate) const CHUNK: usize = 64 * 1024;
pub(crate) const MAX_TEXT_INPUT: usize = 32 * 1024 * 1024;
pub(crate) const MAX_TEXT_OUTPUT: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Algorithm { Md5, Sha1, Sha256, Sha512 }
impl Algorithm {
    pub(crate) fn label(self) -> &'static str {
        match self { Self::Md5 => "MD5", Self::Sha1 => "SHA-1", Self::Sha256 => "SHA-256", Self::Sha512 => "SHA-512" }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextMode { Whole, PerLine }
#[derive(Clone, Copy)]
pub(crate) struct EditorFormat { pub(crate) encoding: Encoding, pub(crate) ending: LineEnding }

#[derive(Debug, thiserror::Error)]
pub(crate) enum HashError {
    #[error("Hash cancelled; previous results retained")]
    Cancelled,
    #[error("Hash input positions are no longer valid; select the text again")]
    Input,
    #[error("Hash input exceeds the temporary 32 MiB limit; use a smaller input or hash a file")]
    InputLimit,
    #[error("Hash output exceeds the temporary 16 MiB limit; reduce the input or turn off per-line mode")]
    OutputLimit,
    #[error("Not enough memory to prepare the hash; previous results retained")]
    Memory,
    #[error("U+{code:04X} cannot be hashed as Windows-1252; choose UTF-8 in the Encoding menu", code = *ch as u32)]
    Unmappable { ch: char },
    #[error("Could not read the file: {0}")]
    Io(#[from] io::Error),
}
fn check(cancel: &AtomicBool) -> Result<(), HashError> {
    if cancel.load(Ordering::Relaxed) { Err(HashError::Cancelled) } else { Ok(()) }
}
enum Engine { Md5(md5::Md5), Sha1(sha1::Sha1), Sha256(sha2::Sha256), Sha512(sha2::Sha512) }
impl Engine {
    fn new(algorithm: Algorithm) -> Self {
        match algorithm { Algorithm::Md5 => Self::Md5(md5::Md5::new()), Algorithm::Sha1 => Self::Sha1(sha1::Sha1::new()),
            Algorithm::Sha256 => Self::Sha256(sha2::Sha256::new()), Algorithm::Sha512 => Self::Sha512(sha2::Sha512::new()) }
    }
    fn update(&mut self, bytes: &[u8]) {
        match self { Self::Md5(d) => d.update(bytes), Self::Sha1(d) => d.update(bytes),
            Self::Sha256(d) => d.update(bytes), Self::Sha512(d) => d.update(bytes) }
    }
    fn finish(self) -> Result<String, HashError> {
        match self { Self::Md5(d) => hex(d.finalize().as_ref()), Self::Sha1(d) => hex(d.finalize().as_ref()),
            Self::Sha256(d) => hex(d.finalize().as_ref()), Self::Sha512(d) => hex(d.finalize().as_ref()) }
    }
}
fn digit(value: u8) -> char {
    char::from(if value < 10 { b'0'.saturating_add(value) } else { b'a'.saturating_add(value.saturating_sub(10)) })
}
fn hex(bytes: &[u8]) -> Result<String, HashError> {
    let length = bytes.len().checked_mul(2).ok_or(HashError::Memory)?;
    let mut result = String::new(); result.try_reserve_exact(length).map_err(|_| HashError::Memory)?;
    for &byte in bytes { result.push(digit(byte >> 4)); result.push(digit(byte & 15)); }
    Ok(result)
}
fn progress_add(progress: &AtomicU64, amount: usize) {
    let current = progress.load(Ordering::Relaxed);
    progress.store(current.saturating_add(amount as u64), Ordering::Relaxed);
}

pub(crate) fn digest_reader(algorithm: Algorithm, reader: &mut impl Read, cancel: &AtomicBool, progress: &AtomicU64) -> Result<String, HashError> {
    let mut engine = Engine::new(algorithm); let mut bytes = [0u8; CHUNK];
    loop {
        check(cancel)?;
        let length = match reader.read(&mut bytes) { Ok(length) => length,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue, Err(error) => return Err(error.into()) };
        if length == 0 { break; }
        engine.update(bytes.get(..length).ok_or(HashError::Input)?); progress_add(progress, length);
    }
    check(cancel)?; engine.finish()
}

fn append(result: &mut String, text: &str) -> Result<(), HashError> {
    let length = result.len().checked_add(text.len()).ok_or(HashError::OutputLimit)?;
    if length > MAX_TEXT_OUTPUT { return Err(HashError::OutputLimit); }
    if length > result.capacity() {
        let capacity = result.capacity().max(128).saturating_mul(2).max(length).min(MAX_TEXT_OUTPUT);
        result.try_reserve_exact(capacity.saturating_sub(result.len())).map_err(|_| HashError::Memory)?;
    }
    result.push_str(text); Ok(())
}
fn finish_row(result: &mut String, engine: Engine, nonempty: bool) -> Result<(), HashError> {
    if nonempty { append(result, &engine.finish()?)?; } append(result, "\r\n")
}
pub(crate) fn text_result(algorithm: Algorithm, input: &Rope, mode: TextMode, cancel: &AtomicBool, progress: &AtomicU64) -> Result<String, HashError> {
    check(cancel)?; if input.len_bytes() > MAX_TEXT_INPUT { return Err(HashError::InputLimit); }
    if mode == TextMode::Whole {
        let mut engine = Engine::new(algorithm); let mut nonempty = false;
        for chunk in input.chunks() {
            check(cancel)?;
            let bytes = chunk.as_bytes(); let nul = bytes.iter().position(|&byte| byte == 0);
            let prefix = bytes.get(..nul.unwrap_or(bytes.len())).ok_or(HashError::Input)?;
            engine.update(prefix); nonempty |= !prefix.is_empty(); progress_add(progress, prefix.len());
            if nul.is_some() { break; }
        }
        check(cancel)?; return if nonempty { engine.finish() } else { Ok(String::new()) };
    }
    let mut result = String::new(); let mut engine = Engine::new(algorithm);
    let (mut present, mut nonempty, mut pending_cr) = (false, false, false);
    'input: for chunk in input.chunks() {
        check(cancel)?;
        for ch in chunk.chars() {
            if ch == '\0' { break 'input; }
            progress_add(progress, ch.len_utf8());
            if ch == '\n' {
                finish_row(&mut result, engine, nonempty)?; engine = Engine::new(algorithm);
                present = false; nonempty = false; pending_cr = false; continue;
            }
            present = true;
            if pending_cr { engine.update(b"\r"); nonempty = true; }
            pending_cr = ch == '\r';
            if !pending_cr { let mut encoded = [0u8; 4]; engine.update(ch.encode_utf8(&mut encoded).as_bytes()); nonempty = true; }
        }
    }
    if present { finish_row(&mut result, engine, nonempty)?; }
    check(cancel)?; Ok(result)
}

struct Sink { engine: Engine, bytes: [u8; CHUNK], used: usize }
impl Sink {
    fn new(algorithm: Algorithm) -> Self { Self { engine: Engine::new(algorithm), bytes: [0; CHUNK], used: 0 } }
    fn write(&mut self, bytes: &[u8]) -> Result<(), HashError> {
        let mut offset = 0usize;
        while offset < bytes.len() {
            if self.used == self.bytes.len() { self.flush()?; }
            let length = bytes.len().saturating_sub(offset).min(self.bytes.len().saturating_sub(self.used));
            let end = self.used.checked_add(length).ok_or(HashError::Input)?;
            let source_end = offset.checked_add(length).ok_or(HashError::Input)?;
            self.bytes.get_mut(self.used..end).ok_or(HashError::Input)?.copy_from_slice(bytes.get(offset..source_end).ok_or(HashError::Input)?);
            self.used = end; offset = source_end;
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<(), HashError> { self.engine.update(self.bytes.get(..self.used).ok_or(HashError::Input)?); self.used = 0; Ok(()) }
    fn finish(mut self) -> Result<String, HashError> { self.flush()?; self.engine.finish() }
}
pub(crate) fn selection_digest(algorithm: Algorithm, source: &Rope, range: Range<usize>, format: EditorFormat, cancel: &AtomicBool, progress: &AtomicU64) -> Result<Option<String>, HashError> {
    check(cancel)?; let range = crate::edit_ops::util::clamp_byte_range(source, range);
    if range.is_empty() { return Ok(None); }
    let slice = source.get_byte_slice(range.clone()).ok_or(HashError::Input)?;
    if format.encoding != Encoding::Latin1 && format.ending == LineEnding::Lf {
        let mut engine = Engine::new(algorithm);
        for chunk in slice.chunks() {
            check(cancel)?; let bytes = chunk.as_bytes(); let nul = bytes.iter().position(|&byte| byte == 0);
            let prefix = bytes.get(..nul.unwrap_or(bytes.len())).ok_or(HashError::Input)?;
            engine.update(prefix); progress_add(progress, prefix.len()); if nul.is_some() { break; }
        }
        check(cancel)?; return engine.finish().map(Some);
    }
    let mut sink = Sink::new(algorithm); let mut encoder = encoding_rs::WINDOWS_1252.new_encoder();
    let mut previous_cr = range.start.checked_sub(1).and_then(|at| source.get_byte(at)) == Some(b'\r');
    'input: for chunk in slice.chunks() {
        check(cancel)?;
        for ch in chunk.chars() {
            if ch == '\0' { break 'input; }
            progress_add(progress, ch.len_utf8());
            if ch == '\n' {
                match format.ending {
                    LineEnding::Lf => sink.write(b"\n")?,
                    LineEnding::Crlf => { if !previous_cr { sink.write(b"\r")?; } sink.write(b"\n")?; },
                    LineEnding::Cr => { if !previous_cr { sink.write(b"\r")?; } },
                }
            } else {
                let mut utf8 = [0u8; 4]; let text = ch.encode_utf8(&mut utf8);
                if format.encoding == Encoding::Latin1 {
                    let mut encoded = [0u8; 4];
                    let (status, read, written) = encoder.encode_from_utf8_without_replacement(text, &mut encoded, false);
                    if let encoding_rs::EncoderResult::Unmappable(ch) = status { return Err(HashError::Unmappable { ch }); }
                    if read != text.len() { return Err(HashError::Input); }
                    sink.write(encoded.get(..written).ok_or(HashError::Input)?)?;
                } else { sink.write(text.as_bytes())?; }
            }
            previous_cr = ch == '\r';
        }
    }
    check(cancel)?; sink.finish().map(Some)
}

#[cfg(test)]
#[path = "hash_tools_tests.rs"]
mod tests;
