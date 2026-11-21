use super::PanelData;
use replay_data::Hour;
use std::{collections::BTreeMap, io};

pub const MAGIC: &[u8; 4] = b"PNL2";
const VERSION: u16 = 5;
const HEAD: usize = 24;
const PANEL_HEAD: usize = 16;
pub const REFUSED: u8 = 0;
pub const DATA: u8 = 1;
pub const STANDS: u8 = 3;
pub const COMPLETED: u8 = 4;

pub struct ProductBatch {
    bytes: Vec<u8>,
    count: u32,
}

impl ProductBatch {
    pub fn new((clock, route, seek): (i32, u32, u32)) -> Self {
        Self { bytes: head(0, clock, (route, seek)), count: 0 }
    }

    pub fn changed(&mut self, id: u32, hour: Hour, kind: u8, body: &[u8]) {
        let offset = self.bytes.len();
        record(&mut self.bytes, id, hour, kind, body);
        self.bytes[offset + 9] |= 1;
        self.count += 1;
    }

    pub fn finish(mut self, through: Option<Hour>) -> Vec<u8> {
        if let Some(hour) = through {
            record(&mut self.bytes, 0, hour, COMPLETED, &[]);
            self.count += 1;
        }
        self.bytes[8..12].copy_from_slice(&self.count.to_le_bytes());
        self.bytes
    }
}

pub enum Update<'a> {
    Data(u32, Hour, &'a PanelData),
    Stands(u32, Hour),
}

pub fn encoded(clock: i32, route: u32, seek: u32, id: u32, hour: Hour, kind: u8, body: &[u8]) -> Vec<u8> {
    let mut out = head(1, clock, (route, seek));
    record(&mut out, id, hour, kind, body);
    out
}

pub fn message(clock: i32, route: u32, seek: u32, updates: &[Update<'_>]) -> Vec<u8> {
    let mut out = head(updates.len(), clock, (route, seek));
    for update in updates {
        match *update {
            Update::Data(id, hour, data) => reading(&mut out, id, hour, data),
            Update::Stands(id, hour) => record(&mut out, id, hour, STANDS, &[]),
        }
    }
    out
}

fn reading(out: &mut Vec<u8>, id: u32, hour: Hour, data: &PanelData) {
    match data {
        Ok(value) => record(out, id, hour, DATA, &rmp_serde::to_vec(value).expect("a panel value")),
        Err(reason) => record(out, id, hour, REFUSED, reason.as_bytes()),
    }
}

fn record(out: &mut Vec<u8>, id: u32, hour: Hour, kind: u8, body: &[u8]) {
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&hour.to_le_bytes());
    out.extend_from_slice(&[kind, 0, 0, 0]);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    out.resize(out.len().next_multiple_of(8), 0);
}

fn head(count: usize, clock: i32, (route, seek): (u32, u32)) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&[0; 2]);
    out.extend_from_slice(&(count as u32).to_le_bytes());
    out.extend_from_slice(&clock.to_le_bytes());
    out.extend_from_slice(&route.to_le_bytes());
    out.extend_from_slice(&seek.to_le_bytes());
    out
}

pub fn clock(bytes: &[u8]) -> i32 {
    bytes.get(12..16).and_then(|clock| clock.try_into().ok()).map_or(0, i32::from_le_bytes)
}

pub fn stamp(bytes: &[u8]) -> Option<(u32, u32)> {
    (bytes.len() >= HEAD && bytes[..4] == MAGIC[..] && bytes[4..6] == VERSION.to_le_bytes())
        .then(|| (u32_at(bytes, 16), u32_at(bytes, 20)))
}

#[derive(Clone, Copy, Debug)]
pub struct Record<'a> {
    pub id: u32,
    pub hour: Hour,
    pub kind: u8,
    pub body: &'a [u8],
    bytes: &'a [u8],
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"))
}

pub fn records(bytes: &[u8]) -> io::Result<Vec<Record<'_>>> {
    if bytes.len() < HEAD || &bytes[..4] != MAGIC {
        return Err(crate::invalid("A panel message does not start with PNL2"));
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != VERSION {
        return Err(crate::invalid(format!("A panel message is version {version}, not {VERSION}")));
    }
    let count = u32_at(bytes, 8) as usize;
    if count > (bytes.len() - HEAD) / PANEL_HEAD { return Err(crate::invalid("A panel message ends early")); }
    let mut out = Vec::with_capacity(count);
    let mut at = HEAD;
    for _ in 0..count {
        if at + PANEL_HEAD > bytes.len() {
            return Err(crate::invalid("A panel message ends early"));
        }
        let length = u32_at(bytes, at + 12) as usize;
        let end = (at + PANEL_HEAD + length).next_multiple_of(8);
        if end > bytes.len() {
            return Err(crate::invalid("A panel in the message runs past its end"));
        }
        out.push(Record {
            id: u32_at(bytes, at),
            hour: u32_at(bytes, at + 4) as Hour,
            kind: bytes[at + 8],
            body: &bytes[at + PANEL_HEAD..at + PANEL_HEAD + length],
            bytes: &bytes[at..end],
        });
        at = end;
    }
    if at != bytes.len() {
        return Err(crate::invalid("A panel message holds bytes past its panels"));
    }
    Ok(out)
}

pub fn merge(older: &[u8], newer: &[u8]) -> io::Result<(Vec<u8>, u32)> {
    let stamp = stamp(older).filter(|s| Some(*s) == self::stamp(newer) && clock(older) == clock(newer))
        .ok_or_else(|| crate::invalid("Panel messages belong to different routes or seeks"))?;
    let mut readings = BTreeMap::new();
    let mut stands = BTreeMap::new();
    let mut completed = None;
    let mut count = 0;
    for r in records(older)?.into_iter().chain(records(newer)?) {
        count += 1;
        match r.kind {
            DATA | REFUSED => { stands.remove(&r.id); readings.insert(r.id, r); }
            STANDS => { stands.insert(r.id, r); }
            COMPLETED => completed = Some(r),
            _ => return Err(crate::invalid("Unknown dated panel kind")),
        }
    }
    let kept = readings.len() + stands.len() + usize::from(completed.is_some());
    let mut out = head(kept, clock(newer), stamp);
    for r in readings.into_values().chain(stands.into_values()).chain(completed) { out.extend_from_slice(r.bytes); }
    Ok((out, (count - kept) as u32))
}
