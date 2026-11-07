//! A version 3 columnar recording, read whole into memory and walked a segment at a time:
//! each hour's manifest and sections as its writer wrote them.

use crate::v3::{self, Extent, IndexPage, Root, RunDirectory};
use replay::{
    columnar_blocks::note_availability,
    columnar_frames::{keeps_last, schema},
};
use replay_columnar::{
    leaf::Reader,
    manifest::{Manifest, Status},
    walk::State,
};
use std::{io, path::Path};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

pub struct Run {
    pub branch: u64,
    pub epoch: u8,
    pub first_hour: i32,
    pub ordinals: u32,
    pub pages: Vec<IndexPage>,
}

/// Hours a walk takes on its own: from its run's start or a restart, through the hour before
/// the run's next restart.
#[derive(Clone, Copy, Debug)]
pub struct Segment {
    pub run: usize,
    pub first: u32,
    pub last: u32,
}

pub struct Source {
    bytes: Vec<u8>,
    pub root: Root,
    pub runs: Vec<Run>,
    pub segments: Vec<Segment>,
}

impl Source {
    pub fn open(path: &Path) -> io::Result<Self> {
        let bytes = std::fs::read(path)?;
        let (root_extent, _) = v3::last_root(&bytes)?;
        let raw = Self::raw_of(&bytes, &root_extent)?;
        let root = Root::read(&raw)?;
        let tables = schema()?.len();
        let mut runs = Vec::new();
        let mut segments = Vec::new();
        for (index, extent) in root.runs.iter().enumerate() {
            let directory = RunDirectory::read(&Self::raw_of(&bytes, extent)?, extent, index as u32)?;
            let mut pages = Vec::new();
            for page in &directory.pages {
                pages.push(IndexPage::read(&Self::raw_of(&bytes, page)?, page, tables)?);
            }
            let mut start = (None::<usize>, 0_u32);
            for (at, page) in pages.iter().enumerate() {
                if let Some(restart) = &page.restart
                    && restart.after + 1 < directory.ordinals
                {
                    segments.push(Segment {
                        run: index,
                        first: start.1,
                        last: restart.after,
                    });
                    start = (Some(at), restart.after + 1);
                }
            }
            if start.1 < directory.ordinals {
                segments.push(Segment {
                    run: index,
                    first: start.1,
                    last: directory.ordinals - 1,
                });
            }
            runs.push(Run {
                branch: directory.branch,
                epoch: directory.epoch,
                first_hour: directory.first_hour,
                ordinals: directory.ordinals,
                pages,
            });
        }
        Ok(Self {
            bytes,
            root,
            runs,
            segments,
        })
    }

    fn raw_of(bytes: &[u8], extent: &Extent) -> io::Result<Vec<u8>> {
        let stored = extent.stored_in(bytes)?;
        // A root's envelope, from the footer, names no raw length.
        match extent.raw {
            0 => zstd::stream::decode_all(stored),
            raw => zstd::bulk::decompress(stored, raw as usize),
        }
    }

    pub fn raw(&self, extent: &Extent) -> io::Result<Vec<u8>> {
        Self::raw_of(&self.bytes, extent)
    }

    /// The capture the root's control records hold, as version 3 wrote it: a version 2
    /// capture record, then each later revision's added control records.
    pub fn capture(&self) -> io::Result<Option<replay::columnar_file::Capture>> {
        let Some((first, rest)) = self.root.controls.split_first() else {
            return Ok(None);
        };
        let bytes = self.raw(first)?;
        let mut reader = Reader::new(&bytes);
        if reader.varint()? != 2 {
            return Err(invalid("Not a version 2 capture record"));
        }
        let owned = |reader: &mut Reader<'_>| -> io::Result<Vec<u8>> {
            let length = reader.count()?;
            Ok(reader.raw(length)?.to_vec())
        };
        let campaign_id = owned(&mut reader)?;
        let content = owned(&mut reader)?;
        let capture_mode = reader.varint()? as u8;
        let mut sessions = Vec::new();
        for _ in 0..reader.count()? {
            sessions.push(owned(&mut reader)?);
        }
        let mut run_sessions = Vec::new();
        for _ in 0..reader.count()? {
            run_sessions.push(reader.varint_u32()?);
        }
        let stop = match reader.varint()? {
            0 => None,
            _ => {
                let branch = reader.varint()?;
                let last_hour = match reader.varint()? {
                    0 => None,
                    hour => Some((hour - 1) as u32 as i32),
                };
                let reason = String::from_utf8(owned(&mut reader)?).map_err(|_| invalid("A stop's reason is not UTF-8"))?;
                Some(replay::columnar_file::Stop { branch, last_hour, reason })
            }
        };
        let format = reader.varint()? as u16;
        let mut controls = Vec::new();
        let mut read_controls = |reader: &mut Reader<'_>| -> io::Result<()> {
            for _ in 0..reader.count()? {
                controls.push(owned(reader)?);
            }
            Ok(())
        };
        read_controls(&mut reader)?;
        for extent in rest {
            let bytes = self.raw(extent)?;
            read_controls(&mut Reader::new(&bytes))?;
        }
        Ok(Some(replay::columnar_file::Capture {
            campaign_id,
            content,
            capture_mode,
            sessions,
            run_sessions,
            stop,
            format,
            journaled: 0,
            controls,
        }))
    }

    /// Every table's stored bytes in the units and in the restarts, and the manifests'.
    pub fn stored(&self, tables: usize) -> (Vec<u64>, Vec<u64>, u64) {
        let (mut units, mut restarts, mut manifests) = (vec![0; tables], vec![0; tables], 0);
        for run in &self.runs {
            for page in &run.pages {
                for unit in &page.units {
                    match unit.stream {
                        0 => manifests += unit.stored,
                        stream => units[stream as usize - 1] += unit.stored,
                    }
                }
                if let Some(restart) = &page.restart {
                    for table in &restart.tables {
                        restarts[table.table as usize] += table.extent.stored;
                    }
                }
            }
        }
        (units, restarts, manifests)
    }

    /// Walks run `run` from its start, handing `each` every segment's start: the walker's tables
    /// there, as restarts hold them.
    pub fn starts(&self, run: usize, mut each: impl FnMut(Segment, Snapshot) -> io::Result<()>) -> io::Result<()> {
        let mut state = State::new(schema()?)?;
        state.keep_last(&keeps_last())?;
        let columns: Vec<usize> = schema()?.iter().map(Vec::len).collect();
        let (mut statuses, mut available_from) = (vec![Status::NotRecorded; columns.len()], vec![None; columns.len()]);
        let segments: Vec<Segment> = self.segments.iter().copied().filter(|segment| segment.run == run).collect();
        let Some(&last) = segments.last() else {
            return Ok(());
        };
        let mut next = 0;
        let whole = Segment {
            run,
            first: 0,
            last: last.last,
        };
        if segments[0].first == 0 {
            each(segments[0], Snapshot::of(&state, &available_from)?)?;
            next = 1;
        }
        self.hours(&whole, |ordinal, manifest, sections| {
            let manifest = Manifest::read(manifest, &columns)?;
            let mut at = 0usize;
            let mut spans = Vec::with_capacity(manifest.sections.len());
            for section in &manifest.sections {
                spans.push(at..at + section.bytes as usize);
                at += section.bytes as usize;
            }
            state.apply(&manifest, |index| Ok(&sections[spans[index].clone()]))?;
            note_availability(&mut available_from, &statuses, &manifest.statuses, manifest.hour);
            statuses.copy_from_slice(&manifest.statuses);
            if let Some(&segment) = segments.get(next)
                && segment.first == ordinal + 1
            {
                each(segment, Snapshot::of(&state, &available_from)?)?;
                next += 1;
            }
            Ok(true)
        })
    }

    /// Each hour of `segment`, as its writer wrote it: the hour's manifest's bytes and its
    /// sections', together, in the manifest's order.
    pub fn hours(&self, segment: &Segment, mut visit: impl FnMut(u32, &[u8], &[u8]) -> io::Result<bool>) -> io::Result<()> {
        let columns: Vec<usize> = schema()?.iter().map(Vec::len).collect();
        let run = &self.runs[segment.run];
        let mut sections = Vec::new();
        for page in &run.pages {
            let end = page.first + page.hours;
            if end <= segment.first || page.first > segment.last {
                continue;
            }
            let mut units: Vec<Vec<u8>> = vec![Vec::new(); columns.len() + 1];
            for unit in &page.units {
                units[unit.stream as usize].extend_from_slice(&self.raw(unit)?);
            }
            let mut manifests = Reader::new(&units[0]);
            let mut tables: Vec<Reader<'_>> = units[1..].iter().map(|unit| Reader::new(unit)).collect();
            for ordinal in page.first..end {
                let length = manifests.count()?;
                let bytes = manifests.raw(length)?;
                let manifest = Manifest::read(bytes, &columns)?;
                sections.clear();
                for section in &manifest.sections {
                    let reader = &mut tables[section.table];
                    reader.align(8)?;
                    sections.extend_from_slice(reader.raw(section.bytes as usize)?);
                }
                if ordinal < segment.first {
                    continue;
                }
                if !visit(ordinal, bytes, &sections)? {
                    return Ok(());
                }
                if ordinal == segment.last {
                    return Ok(());
                }
            }
        }
        Ok(())
    }
}

/// A walker's tables, as restarts hold them, and its dictionary.
pub struct Snapshot {
    tables: Vec<Vec<u8>>,
    dictionary: Vec<u8>,
}

impl Snapshot {
    /// A copy, for the gate.
    pub fn clone_of(snapshot: &Self) -> Self {
        Self {
            tables: snapshot.tables.clone(),
            dictionary: snapshot.dictionary.clone(),
        }
    }

    fn of(state: &State, available_from: &[Option<i32>]) -> io::Result<Self> {
        use replay_columnar::restart;
        let mut tables = Vec::new();
        for table in 0..state.columns().len() {
            let status = state.status(table).unwrap_or(Status::NotRecorded);
            tables.push(restart::write_with(
                state.table(table).expect("schema table"),
                status,
                &restart::Acquisition {
                    last_read: state.sampled_at(table),
                    available_from: available_from[table],
                    refusal: match status {
                        Status::Unavailable { reason } => Some(reason),
                        _ => None,
                    },
                },
                state.truncated_view(table).unwrap_or(&[]),
                state.last(table),
            )?);
        }
        Ok(Self {
            tables,
            dictionary: state.dictionary().chunk(0)?,
        })
    }

    /// The walker's tables it holds.
    /// The walker's tables it holds, and each table's first hour of its current availability.
    pub fn state(&self) -> io::Result<(State, Vec<Option<i32>>)> {
        let mut state = State::new(schema()?)?;
        state.keep_last(&keeps_last())?;
        state.restore_dictionary([self.dictionary.as_slice()])?;
        let mut available_from = Vec::with_capacity(self.tables.len());
        for (table, bytes) in self.tables.iter().enumerate() {
            available_from.push(state.restore(table, bytes)?.available_from);
        }
        Ok((state, available_from))
    }
}
