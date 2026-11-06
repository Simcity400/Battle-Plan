//! A family emit's live edges: each key's open run and open evidence. An edge is appended to
//! the writer when it changes, and at a cut through the last hour when it began since the cut
//! before; one carried out of that cut is held, so a segment refers to it where it began.

use crate::{
    codec::Decode,
    invalid,
    types::{Evidence, KeyHandle, Knownness, Span},
    writer::Writer,
};
use std::io;

/// What a key's open evidence names besides its extent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Read {
    pub holder: Option<u64>,
    pub read: Option<i32>,
    pub identity_first: Option<i32>,
    /// Presented as an event of the key where it starts ([`Writer::evidence`]).
    pub presented: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Run {
    first: i32,
    value: i128,
    break_before: bool,
    /// A read supplied it anew at `first`: no equal run before it merges into it.
    supplying: bool,
    /// The last hour a cut appended of it.
    through: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Held {
    first: i32,
    read: Read,
    through: Option<i32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Edge {
    run: Option<Run>,
    evidence: Option<Held>,
}

/// Every key's open edges, by key handle.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Carry {
    edges: Vec<Edge>,
    /// Keys with an open run or open evidence, unordered.
    open: Vec<KeyHandle>,
    listed: Vec<bool>,
}

fn index(key: KeyHandle) -> io::Result<usize> {
    usize::try_from(key).map_err(|_| invalid("series key outside the carry"))
}

/// The piece of `first..=last` a cut has not appended yet.
fn piece(first: i32, through: Option<i32>, last: i32) -> Option<Span> {
    let first = match through {
        Some(through) => through.checked_add(1)?,
        None => first,
    };
    (first <= last).then_some(Span { first, last })
}

impl Carry {
    fn edge(&mut self, key: KeyHandle) -> io::Result<&mut Edge> {
        let at = index(key)?;
        if at >= self.edges.len() {
            engine_allocation::resize(&mut self.edges, at + 1, Edge::default())?;
            engine_allocation::resize(&mut self.listed, at + 1, false)?;
        }
        if !self.listed[at] {
            engine_allocation::reserve(&mut self.open, 1)?;
            self.listed[at] = true;
            self.open.push(key);
        }
        Ok(&mut self.edges[at])
    }

    /// The value of `key` from `hour` on: None ends its open run before `hour`. Whether a run
    /// starts at `hour`; an equal value continuing without a break changes nothing.
    pub fn value(
        &mut self,
        writer: &mut Writer,
        key: KeyHandle,
        hour: i32,
        value: Option<i128>,
        break_before: bool,
    ) -> io::Result<bool> {
        let edge = *self.edge(key)?;
        if let (Some(run), Some(value)) = (edge.run, value)
            && run.value == value
            && !break_before
        {
            return Ok(false);
        }
        end_run(writer, key, edge.run, hour)?;
        self.edges[index(key)?].run = value.map(|value| Run {
            first: hour,
            value,
            break_before,
            supplying: false,
            through: None,
        });
        Ok(value.is_some())
    }

    /// The value of `key` from `hour` on, as a read supplied it there: a run starts at `hour`
    /// even where the run before holds the same value, apart from it by a supplying boundary.
    pub fn supplied(
        &mut self,
        writer: &mut Writer,
        key: KeyHandle,
        hour: i32,
        value: i128,
        break_before: bool,
    ) -> io::Result<()> {
        let edge = *self.edge(key)?;
        end_run(writer, key, edge.run, hour)?;
        self.edges[index(key)?].run = Some(Run {
            first: hour,
            value,
            break_before,
            supplying: true,
            through: None,
        });
        Ok(())
    }

    /// The value `key` holds now.
    pub fn held(&self, key: KeyHandle) -> Option<i128> {
        let at = usize::try_from(key).ok()?;
        self.edges.get(at)?.run.map(|run| run.value)
    }

    /// The evidence of `key` from `hour` on: None ends it before `hour`. An equal read continues.
    pub fn evidence(
        &mut self,
        writer: &mut Writer,
        key: KeyHandle,
        hour: i32,
        read: Option<Read>,
    ) -> io::Result<()> {
        let edge = *self.edge(key)?;
        if let (Some(held), Some(read)) = (edge.evidence, read)
            && (Read {
                presented: held.read.presented,
                ..read
            }) == held.read
        {
            return Ok(());
        }
        if let Some(held) = edge.evidence
            && let Some(span) = hour
                .checked_sub(1)
                .and_then(|last| piece(held.first, held.through, last))
        {
            append(writer, key, held, span)?;
        }
        self.edges[index(key)?].evidence = read.map(|read| Held {
            first: hour,
            read,
            through: None,
        });
        Ok(())
    }

    /// Ends every open run and evidence after `last`: the hour after it is not the next one
    /// read.
    pub fn close(&mut self, writer: &mut Writer, last: i32) -> io::Result<()> {
        self.append(writer, last)?;
        for &key in &self.open {
            let at = key as usize;
            self.edges[at] = Edge::default();
            self.listed[at] = false;
        }
        self.open.clear();
        Ok(())
    }

    /// Takes every open run and evidence through `last`, the last hour of the segment a cut
    /// closes; they stay open. One that began since the last cut is appended; one carried out
    /// of it is held, so the segment refers to it where it began.
    pub fn cut(&mut self, writer: &mut Writer, last: i32) -> io::Result<()> {
        let mut kept = 0;
        for at in 0..self.open.len() {
            let key = self.open[at];
            let edge = &mut self.edges[key as usize];
            if let Some(run) = &mut edge.run {
                match run.through {
                    None => {
                        if let Some(span) = piece(run.first, None, last) {
                            write_run(writer, key, *run, span)?;
                        }
                    }
                    Some(through) if through < last => writer.hold(key, last)?,
                    Some(_) => {}
                }
                run.through = Some(last);
            }
            if let Some(held) = &mut edge.evidence {
                match held.through {
                    None => {
                        if let Some(span) = piece(held.first, None, last) {
                            append(writer, key, *held, span)?;
                        }
                    }
                    Some(through) if through < last => writer.hold_evidence(key, last)?,
                    Some(_) => {}
                }
                held.through = Some(last);
            }
            if edge.run.is_some() || edge.evidence.is_some() {
                self.open[kept] = key;
                kept += 1;
            } else {
                self.listed[key as usize] = false;
            }
        }
        self.open.truncate(kept);
        Ok(())
    }

    fn append(&self, writer: &mut Writer, last: i32) -> io::Result<()> {
        for &key in &self.open {
            let edge = self.edges[key as usize];
            if let Some(run) = edge.run
                && let Some(span) = piece(run.first, run.through, last)
            {
                write_run(writer, key, run, span)?;
            }
            if let Some(held) = edge.evidence
                && let Some(span) = piece(held.first, held.through, last)
            {
                append(writer, key, held, span)?;
            }
        }
        Ok(())
    }

    /// Keys with an open run or evidence.
    pub fn len(&self) -> usize {
        self.open.len()
    }

    pub fn is_empty(&self) -> bool {
        self.open.is_empty()
    }

    /// The open edges, for the emit state a family goes on with.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let mut keys = self.open.clone();
        keys.sort_unstable();
        engine_allocation::reserve(out, 8 + keys.len() * 64)?;
        out.extend_from_slice(&(keys.len() as u64).to_le_bytes());
        for key in keys {
            let edge = self.edges[key as usize];
            out.extend_from_slice(&key.to_le_bytes());
            match edge.run {
                Some(run) => {
                    out.push(1 | u8::from(run.break_before) << 1 | u8::from(run.supplying) << 2);
                    out.extend_from_slice(&run.first.to_le_bytes());
                    out.extend_from_slice(&run.value.to_le_bytes());
                    put_hour(out, run.through);
                }
                None => out.push(0),
            }
            match edge.evidence {
                Some(held) => {
                    out.push(1 | u8::from(held.read.presented) << 1);
                    out.extend_from_slice(&held.first.to_le_bytes());
                    match held.read.holder {
                        Some(holder) => {
                            out.push(1);
                            out.extend_from_slice(&holder.to_le_bytes());
                        }
                        None => out.push(0),
                    }
                    put_hour(out, held.read.read);
                    put_hour(out, held.read.identity_first);
                    put_hour(out, held.through);
                }
                None => out.push(0),
            }
        }
        Ok(())
    }

    /// Edges [`Self::save`] wrote.
    pub fn load(bytes: &[u8]) -> io::Result<Self> {
        let mut d = Decode::new(bytes);
        let count = d.u64()?;
        let mut carry = Self::default();
        for _ in 0..count {
            let key = d.u64()?;
            let flags = d.u8()?;
            let run = match flags {
                0 => None,
                1 | 3 | 5 | 7 => Some(Run {
                    first: d.i32()?,
                    value: d.i128()?,
                    break_before: flags & 2 != 0,
                    supplying: flags & 4 != 0,
                    through: hour(&mut d)?,
                }),
                _ => return Err(invalid("carried run flags")),
            };
            let flags = d.u8()?;
            let evidence = match flags {
                0 => None,
                1 | 3 => {
                    let first = d.i32()?;
                    let holder = match d.u8()? {
                        0 => None,
                        1 => Some(d.u64()?),
                        _ => return Err(invalid("carried holder flag")),
                    };
                    Some(Held {
                        first,
                        read: Read {
                            holder,
                            read: hour(&mut d)?,
                            identity_first: hour(&mut d)?,
                            presented: flags & 2 != 0,
                        },
                        through: hour(&mut d)?,
                    })
                }
                _ => return Err(invalid("carried evidence flags")),
            };
            if run.is_none() && evidence.is_none() {
                return Err(invalid("carried edge holds nothing"));
            }
            let edge = carry.edge(key)?;
            if edge.run.is_some() || edge.evidence.is_some() {
                return Err(invalid("carried key twice"));
            }
            *edge = Edge { run, evidence };
        }
        d.end()?;
        Ok(carry)
    }
}

/// Appends what the open `run` holds before `hour`, which a cut has not appended.
fn end_run(writer: &mut Writer, key: KeyHandle, run: Option<Run>, hour: i32) -> io::Result<()> {
    if let Some(run) = run
        && let Some(span) = hour
            .checked_sub(1)
            .and_then(|last| piece(run.first, run.through, last))
    {
        write_run(writer, key, run, span)?;
    }
    Ok(())
}

/// `run`'s piece `span`: only its first piece starts it, with its break and boundary.
fn write_run(writer: &mut Writer, key: KeyHandle, run: Run, span: Span) -> io::Result<()> {
    let first = run.through.is_none();
    writer.run(
        key,
        span,
        run.value,
        run.break_before && first,
        run.supplying && first,
    )
}

fn append(writer: &mut Writer, key: KeyHandle, held: Held, span: Span) -> io::Result<()> {
    let presentation = (held.read.presented && held.through.is_none())
        .then(|| writer.sequence())
        .transpose()?;
    writer.evidence(
        key,
        Evidence {
            span,
            known: Knownness::Known,
            supplying_read: held.read.read,
            holder: held.read.holder,
            identity_first: held.read.identity_first,
        },
        presentation,
    )
}

fn put_hour(out: &mut Vec<u8>, hour: Option<i32>) {
    match hour {
        Some(hour) => {
            out.push(1);
            out.extend_from_slice(&hour.to_le_bytes());
        }
        None => out.push(0),
    }
}

fn hour(d: &mut Decode<'_>) -> io::Result<Option<i32>> {
    match d.u8()? {
        0 => Ok(None),
        1 => Ok(Some(d.i32()?)),
        _ => Err(invalid("carried hour flag")),
    }
}
