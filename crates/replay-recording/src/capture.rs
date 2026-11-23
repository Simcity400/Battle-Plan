use replay_columnar::{envelope::{self, Extent}, leaf::{Reader, Writer as Bytes}};
use std::{ffi::{OsStr, OsString}, io::{self, Read, Seek, SeekFrom, Write}, path::{Path, PathBuf}};

pub const CAPTURE_NAME_BYTES: usize = "columnar.4294967295.h4col.partial".len();

pub fn capture_name(n: u32, partial: bool, out: &mut [u8; CAPTURE_NAME_BYTES]) -> &str {
    let mut cursor = io::Cursor::new(&mut out[..]);
    let suffix = if partial { ".partial" } else { "" };
    write!(cursor, "columnar.{n}.h4col{suffix}").expect("the longest name fits");
    let length = cursor.position() as usize;
    std::str::from_utf8(&out[..length]).expect("ASCII")
}

pub fn capture_of(name: &OsStr) -> Option<(u32, bool)> {
    let name = name.to_str()?.strip_prefix("columnar.")?;
    let (number, partial) = match name.strip_suffix(".h4col.partial") {
        Some(number) => (number, true),
        None => (name.strip_suffix(".h4col")?, false),
    };
    Some((counted(number)?, partial))
}

pub fn campaign_folder(replay_dir: &Path, campaign: &[u8]) -> PathBuf {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut name = String::with_capacity(2 * campaign.len());
    for byte in campaign.iter().map(u8::to_ascii_lowercase) {
        name.push(char::from(HEX[usize::from(byte >> 4)]));
        name.push(char::from(HEX[usize::from(byte & 15)]));
    }
    replay_dir.join(name)
}

pub fn published_name(session: &OsStr, n: u32) -> OsString {
    let mut name = session.to_os_string();
    name.push(format!(".{n}.h4col"));
    name
}

pub fn published_of(name: &OsStr) -> Option<(&str, u32)> {
    let (session, number) = name.to_str()?.strip_suffix(".h4col")?.rsplit_once('.')?;
    Some((session, counted(number)?)).filter(|(session, _)| !session.is_empty())
}

pub fn same_recording(a: &Path, b: &Path) -> io::Result<bool> {
    let end = |path: &Path| -> io::Result<(u64, [u8; envelope::FOOTER_BYTES])> {
        let mut file = std::fs::File::open(path)?;
        let length = file.seek(SeekFrom::End(0))?;
        let mut footer = [0; envelope::FOOTER_BYTES];
        let held = length.min(envelope::FOOTER_BYTES as u64) as usize;
        file.seek(SeekFrom::Start(length - held as u64))?;
        file.read_exact(&mut footer[..held])?;
        Ok((length, footer))
    };
    Ok(end(a)? == end(b)?)
}

fn counted(number: &str) -> Option<u32> {
    let valid = number.bytes().all(|byte| byte.is_ascii_digit())
        && (number == "0" || !number.starts_with('0'));
    number.parse().ok().filter(|_| valid)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capture {
    pub campaign_id: Vec<u8>,
    pub content: Vec<u8>,
    pub capture_mode: u8,
    pub sessions: Vec<Vec<u8>>,
    pub run_sessions: Vec<u32>,
    pub stop: Option<Stop>,
    pub format: u16,
    pub journaled: u32,
    pub controls: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stop {
    pub branch: u64,
    pub last_hour: Option<i32>,
    pub reason: String,
}

impl Capture {
    const VERSION: u64 = 3;

    pub fn write(&self) -> io::Result<Vec<u8>> {
        self.write_with(&self.controls)
    }

    pub fn write_with(&self, controls: &[Vec<u8>]) -> io::Result<Vec<u8>> {
        let mut out = Bytes::new();
        out.varint(Self::VERSION)?;
        for bytes in [&self.campaign_id, &self.content] {
            out.varint(bytes.len() as u64)?;
            out.raw(bytes)?;
        }
        out.varint(u64::from(self.capture_mode))?;
        out.varint(self.sessions.len() as u64)?;
        for session in &self.sessions {
            out.varint(session.len() as u64)?;
            out.raw(session)?;
        }
        out.varint(self.run_sessions.len() as u64)?;
        for &session in &self.run_sessions {
            out.varint(u64::from(session))?;
        }
        match &self.stop {
            None => out.varint(0)?,
            Some(stop) => {
                out.varint(1)?;
                out.varint(stop.branch)?;
                out.varint(stop.last_hour.map_or(0, |hour| (hour as u32 as u64) + 1))?;
                out.varint(stop.reason.len() as u64)?;
                out.raw(stop.reason.as_bytes())?;
            }
        }
        out.varint(u64::from(self.format))?;
        out.varint(u64::from(self.journaled))?;
        write_controls(&mut out, controls)?;
        Ok(out.into_bytes())
    }

    pub fn read(bytes: &[u8]) -> io::Result<Self> {
        let mut reader = Reader::new(bytes);
        if reader.varint()? != Self::VERSION {
            return Err(invalid("Unknown capture record version"));
        }
        let owned = |reader: &mut Reader<'_>| -> io::Result<Vec<u8>> {
            let length = usize::try_from(reader.varint()?)
                .map_err(|_| invalid("A capture field passes the address space"))?;
            engine_allocation::copy_vec(reader.raw(length)?)
        };
        let campaign_id = owned(&mut reader)?;
        let content = owned(&mut reader)?;
        let capture_mode = u8::try_from(reader.varint()?)
            .map_err(|_| invalid("A capture mode passes a byte"))?;
        let count = |reader: &mut Reader<'_>| -> io::Result<usize> {
            let count = usize::try_from(reader.varint()?)
                .map_err(|_| invalid("A capture count passes the address space"))?;
            if count > bytes.len() {
                return Err(invalid("A capture count passes its record"));
            }
            Ok(count)
        };
        let sessions_count = count(&mut reader)?;
        let mut sessions = engine_allocation::vec(sessions_count)?;
        for _ in 0..sessions_count {
            sessions.push(owned(&mut reader)?);
        }
        let runs_count = count(&mut reader)?;
        let mut run_sessions = engine_allocation::vec(runs_count)?;
        for _ in 0..runs_count {
            let session = u32::try_from(reader.varint()?)
                .map_err(|_| invalid("A run's session passes 2^32"))?;
            if session as usize >= sessions.len() {
                return Err(invalid("A run names no session of the capture"));
            }
            run_sessions.push(session);
        }
        let stop = match reader.varint()? {
            0 => None,
            1 => {
                let branch = reader.varint()?;
                let last_hour = match reader.varint()? {
                    0 => None,
                    hour => Some(
                        u32::try_from(hour - 1)
                            .map_err(|_| invalid("A stop's hour passes 2^32"))? as i32,
                    ),
                };
                let reason = String::from_utf8(owned(&mut reader)?)
                    .map_err(|_| invalid("A stop's reason is not UTF-8"))?;
                Some(Stop {
                    branch,
                    last_hour,
                    reason,
                })
            }
            _ => return Err(invalid("Unknown capture stop marker")),
        };
        let format = u16::try_from(reader.varint()?)
            .map_err(|_| invalid("A capture's format passes 2^16"))?;
        let journaled = u32::try_from(reader.varint()?)
            .map_err(|_| invalid("A capture's journal passes 2^32 segments"))?;
        let mut controls = Vec::new();
        read_controls(&mut reader, bytes.len(), &mut controls)?;
        reader.finish()?;
        Ok(Self {
            campaign_id,
            content,
            capture_mode,
            sessions,
            run_sessions,
            stop,
            format,
            journaled,
            controls,
        })
    }
}

fn write_controls(out: &mut Bytes, controls: &[Vec<u8>]) -> io::Result<()> {
    out.varint(controls.len() as u64)?;
    for control in controls {
        out.varint(control.len() as u64)?;
        out.raw(control)?;
    }
    Ok(())
}

fn read_controls(reader: &mut Reader<'_>, limit: usize, into: &mut Vec<Vec<u8>>) -> io::Result<()> {
    let count = usize::try_from(reader.varint()?)
        .ok()
        .filter(|&count| count <= limit)
        .ok_or_else(|| invalid("A control count passes its record"))?;
    engine_allocation::reserve(into, count)?;
    for _ in 0..count {
        let length = usize::try_from(reader.varint()?)
            .map_err(|_| invalid("A control record passes the address space"))?;
        into.push(engine_allocation::copy_vec(reader.raw(length)?)?);
    }
    Ok(())
}

pub fn read_capture(
    extents: &[Extent],
    mut raw: impl FnMut(&Extent) -> io::Result<Vec<u8>>,
) -> io::Result<Option<Capture>> {
    let Some((last, chunks)) = extents.split_last() else {
        return Ok(None);
    };
    let mut capture = Capture::read(&raw(last)?)?;
    for extent in chunks {
        let bytes = raw(extent)?;
        let mut reader = Reader::new(&bytes);
        read_controls(&mut reader, bytes.len(), &mut capture.controls)?;
        reader.finish()?;
    }
    Ok(Some(capture))
}

fn invalid(message: &'static str) -> io::Error {
    if engine_allocation::capture_active() {
        io::Error::from(io::ErrorKind::InvalidData)
    } else {
        io::Error::new(io::ErrorKind::InvalidData, message)
    }
}
