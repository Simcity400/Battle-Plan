
use replay_frame::branches::{BranchStart, Gap};
use replay_data::{codec::{Decoder, invalid, put_bytes, put_string}, format::FormatVersion, identity::{IdentityCodec, SessionStart}};
use crate::control::ControlRecord;
use std::io;

pub const MAGIC: &[u8; 4] = b"H4CR";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryNote {
    pub session: Option<u64>,
    pub segment: u32,
    pub valid_bytes: u64,
    pub reason: String,
}

impl RecoveryNote {
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        if self.reason.trim().is_empty() {
            return Err(invalid("empty recovery reason"));
        }
        out.push(u8::from(self.session.is_some()));
        if let Some(session) = self.session {
            out.extend_from_slice(&session.to_le_bytes());
        }
        out.extend_from_slice(&self.segment.to_le_bytes());
        out.extend_from_slice(&self.valid_bytes.to_le_bytes());
        put_string(out, &self.reason)
    }

    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let session = if input.bool()? {
            Some(input.u64()?)
        } else {
            None
        };
        let note = Self {
            session,
            segment: input.u32()?,
            valid_bytes: input.u64()?,
            reason: input.string()?,
        };
        if note.reason.trim().is_empty() {
            return Err(invalid("empty recovery reason"));
        }
        Ok(note)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub source: String,
    pub capture: u32,
    pub session: u64,
    pub segments: u32,
    pub finished: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureStop {
    pub spool: String,
    pub capture: u32,
    pub branch: u64,
    pub last_hour: Option<i32>,
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Records {
    pub sessions: Vec<SessionStart>,
    pub branches: Vec<BranchStart>,
    pub gaps: Vec<Gap>,
    pub controls: Vec<ControlRecord>,
    pub notes: Vec<RecoveryNote>,
    pub cursors: Vec<Cursor>,
    pub stops: Vec<CaptureStop>,
}

fn put_list<T>(
    out: &mut Vec<u8>,
    items: &[T],
    mut encode: impl FnMut(&T, &mut Vec<u8>) -> io::Result<()>,
) -> io::Result<()> {
    let count = u32::try_from(items.len()).map_err(|_| invalid("a record list passes 2^32"))?;
    out.extend_from_slice(&count.to_le_bytes());
    let mut item = Vec::new();
    for value in items {
        item.clear();
        encode(value, &mut item)?;
        put_bytes(out, &item)?;
    }
    Ok(())
}

fn read_list<T>(
    input: &mut Decoder<'_>,
    format: u16,
    into: &mut Vec<T>,
    mut decode: impl FnMut(&mut Decoder<'_>) -> io::Result<T>,
) -> io::Result<()> {
    let count = input.count(4)?;
    engine_allocation::reserve(into, count)?;
    for _ in 0..count {
        let bytes = input.bytes()?;
        let mut item = Decoder::new(&bytes);
        item.format = format;
        into.push(decode(&mut item)?);
        item.finish()?;
    }
    Ok(())
}

fn put_option_hour(out: &mut Vec<u8>, hour: Option<i32>) {
    match hour {
        None => out.push(0),
        Some(hour) => {
            out.push(1);
            out.extend_from_slice(&hour.to_le_bytes());
        }
    }
}

fn read_option_hour(input: &mut Decoder<'_>) -> io::Result<Option<i32>> {
    Ok(if input.bool()? { Some(input.i32()?) } else { None })
}

impl Records {
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
            && self.branches.is_empty()
            && self.gaps.is_empty()
            && self.controls.is_empty()
            && self.notes.is_empty()
            && self.cursors.is_empty()
            && self.stops.is_empty()
    }

    pub fn encode(&self, format: FormatVersion) -> io::Result<Vec<u8>> {
        for control in &self.controls {
            format.require(control.kind())?;
        }
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&format.version().to_le_bytes());
        put_list(&mut out, &self.sessions, |session, out| session.encode(out))?;
        put_list(&mut out, &self.branches, |branch, out| branch.encode(out))?;
        put_list(&mut out, &self.gaps, |gap, out| gap.encode(out))?;
        put_list(&mut out, &self.controls, |control, out| control.encode(out))?;
        put_list(&mut out, &self.notes, |note, out| note.encode(out))?;
        put_list(&mut out, &self.cursors, |cursor, out| {
            put_string(out, &cursor.source)?;
            out.extend_from_slice(&cursor.capture.to_le_bytes());
            out.extend_from_slice(&cursor.session.to_le_bytes());
            out.extend_from_slice(&cursor.segments.to_le_bytes());
            out.push(u8::from(cursor.finished));
            Ok(())
        })?;
        put_list(&mut out, &self.stops, |stop, out| {
            put_string(out, &stop.spool)?;
            out.extend_from_slice(&stop.capture.to_le_bytes());
            out.extend_from_slice(&stop.branch.to_le_bytes());
            put_option_hour(out, stop.last_hour);
            put_string(out, &stop.reason)
        })?;
        Ok(out)
    }

    pub fn format_of(bytes: &[u8]) -> Option<u16> {
        let rest = bytes.strip_prefix(MAGIC)?;
        Some(u16::from_le_bytes(rest.get(..2)?.try_into().ok()?))
    }

    pub fn decode_into(&mut self, bytes: &[u8]) -> io::Result<()> {
        let format = Self::format_of(bytes)
            .ok_or_else(|| invalid("A campaign records extent holds no campaign records"))?;
        let version = FormatVersion::read(format)?;
        let mut input = Decoder::new(&bytes[MAGIC.len() + 2..]);
        input.format = format;
        read_list(&mut input, format, &mut self.sessions, SessionStart::decode)?;
        read_list(&mut input, format, &mut self.branches, BranchStart::decode)?;
        read_list(&mut input, format, &mut self.gaps, Gap::decode)?;
        read_list(&mut input, format, &mut self.controls, |input| {
            ControlRecord::decode_version(input, version)
        })?;
        read_list(&mut input, format, &mut self.notes, RecoveryNote::decode)?;
        read_list(&mut input, format, &mut self.cursors, |input| {
            Ok(Cursor {
                source: input.string()?,
                capture: input.u32()?,
                session: input.u64()?,
                segments: input.u32()?,
                finished: input.bool()?,
            })
        })?;
        read_list(&mut input, format, &mut self.stops, |input| {
            Ok(CaptureStop {
                spool: input.string()?,
                capture: input.u32()?,
                branch: input.u64()?,
                last_hour: read_option_hour(input)?,
                reason: input.string()?,
            })
        })?;
        input.finish()
    }

    pub fn append(&mut self, other: &mut Self) -> io::Result<()> {
        fn move_all<T>(into: &mut Vec<T>, from: &mut Vec<T>) -> io::Result<()> {
            engine_allocation::reserve(into, from.len())?;
            into.append(from);
            Ok(())
        }
        move_all(&mut self.sessions, &mut other.sessions)?;
        move_all(&mut self.branches, &mut other.branches)?;
        move_all(&mut self.gaps, &mut other.gaps)?;
        move_all(&mut self.controls, &mut other.controls)?;
        move_all(&mut self.notes, &mut other.notes)?;
        move_all(&mut self.cursors, &mut other.cursors)?;
        move_all(&mut self.stops, &mut other.stops)
    }
}
