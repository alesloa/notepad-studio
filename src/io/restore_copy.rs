// Restore prior destination bytes after a failed in-place save, retaining its handle.
use std::{fs::File, io::{self, Read, Seek, SeekFrom, Write}, path::Path};

const SCRATCH_BYTES: usize = 64 * 1024;

pub(super) fn restore(target: &mut File, copy: &Path) -> io::Result<()> {
    #[cfg(test)]
    tests::check(copy, tests::Phase::Open)?;
    let mut original = File::open(copy)?;
    #[cfg(test)]
    tests::check(copy, tests::Phase::Metadata)?;
    let length = original.metadata()?.len();
    #[cfg(test)]
    tests::check(copy, tests::Phase::Read)?;
    #[cfg(test)]
    tests::check(copy, tests::Phase::Seek)?;
    target.seek(SeekFrom::Start(0))?;
    let mut scratch = [0u8; SCRATCH_BYTES];
    #[cfg(test)]
    tests::observe(copy, scratch.len());
    #[cfg(test)]
    tests::check(copy, tests::Phase::Write)?;
    pump(&mut original, target, length, &mut scratch)?;
    #[cfg(test)]
    tests::check(copy, tests::Phase::SetLen)?;
    target.set_len(length)?;
    #[cfg(test)]
    tests::check(copy, tests::Phase::Sync)?;
    target.sync_all()
}

fn pump(reader: &mut (impl Read + ?Sized), writer: &mut (impl Write + ?Sized),
    expected: u64, scratch: &mut [u8]) -> io::Result<()> {
    let capacity = scratch.len().min(SCRATCH_BYTES);
    if capacity == 0 { return Err(io::ErrorKind::InvalidInput.into()); }
    let mut copied = 0u64;
    while copied < expected {
        let remaining = expected.checked_sub(copied).ok_or(io::ErrorKind::InvalidData)?;
        let amount = usize::try_from(remaining.min(capacity as u64)).map_err(|_| io::ErrorKind::InvalidData)?;
        let buffer = scratch.get_mut(..amount).ok_or(io::ErrorKind::InvalidInput)?;
        let count = read_checked(reader, buffer)?;
        if count == 0 { return Err(io::ErrorKind::UnexpectedEof.into()); }
        let bytes = buffer.get(..count).ok_or(io::ErrorKind::InvalidData)?;
        write_checked(writer, bytes)?;
        copied = copied.checked_add(u64::try_from(count).map_err(|_| io::ErrorKind::InvalidData)?)
            .filter(|copied| *copied <= expected).ok_or(io::ErrorKind::InvalidData)?;
    }
    if read_checked(reader, &mut [0u8; 1])? != 0 { return Err(io::ErrorKind::InvalidData.into()); }
    Ok(())
}

fn read_checked(reader: &mut (impl Read + ?Sized), buffer: &mut [u8]) -> io::Result<usize> {
    loop {
        match reader.read(buffer) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Ok(count) if count > buffer.len() => return Err(io::ErrorKind::InvalidData.into()),
            result => return result,
        }
    }
}

fn write_checked(writer: &mut (impl Write + ?Sized), mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        match writer.write(bytes) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(count) => bytes = bytes.get(count..).ok_or(io::ErrorKind::InvalidData)?,
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "restore_copy_tests.rs"]
mod tests;
