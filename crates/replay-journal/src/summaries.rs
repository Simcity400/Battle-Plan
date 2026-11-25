use crate::columnar_file::FileWriter;
use replay_columnar::{
    envelope::{self, SPAN_BLOCKS, UNIT_ORDINALS},
    leaf::{Reader, Writer as Bytes},
    store::Store,
    summary::{self, FANOUT, Shape},
};
use std::{io::{self, Seek}, time::Instant};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

#[derive(Default)]
struct Ladder {
    upper: Vec<Vec<PieceAt>>,
}

#[derive(Clone, Copy)]
struct PieceAt {
    offset: u64,
    stored: u64,
    digest: [u8; 16],
}

pub struct Summaries {
    registry: Vec<(usize, Shape)>,
    owners: Vec<engine_allocation::labels::Owner>,
    builders: Vec<summary::Builder>,
    ladders: Vec<Ladder>,
    level: i32,
    pub ns: u64,
    peaks: Vec<usize>,
    last_peaks: Vec<usize>,
}

impl Summaries {
    pub fn new(level: i32) -> io::Result<Self> {
        Ok(Self::with_registry(level, replay_recording::summaries::registry()?))
    }

    pub fn within(level: i32, tables: usize) -> io::Result<Self> {
        let mut registry = replay_recording::summaries::registry()?;
        registry.retain(|&(table, _)| table < tables);
        Ok(Self::with_registry(level, registry))
    }

    pub(crate) fn with_registry(level: i32, registry: Vec<(usize, Shape)>) -> Self {
        use engine_allocation::labels;
        let owners = registry
            .iter()
            .map(|&(table, _)| match labels::ENABLED {
                true => labels::owner(&format!("summaries: {:?}", replay_data::rows::TableId::ALL[table])),
                false => labels::Owner::RUN,
            })
            .collect();
        let ladders = registry.iter().map(|_| Ladder::default()).collect();
        let peaks = vec![0; registry.len()];
        Self { last_peaks: peaks.clone(), peaks, registry, owners, builders: Vec::new(), ladders, level, ns: 0 }
    }

    fn label(&self, at: usize) -> engine_allocation::labels::Scope {
        engine_allocation::labels::enter(self.owners[at], engine_allocation::labels::Kind::Operator)
    }

    pub fn start_run(&mut self) {
        self.builders.clear();
        self.ladders = self.registry.iter().map(|_| Ladder::default()).collect();
    }

    pub fn open(&mut self, store: &mut Store, hour: i32) {
        let clock = Instant::now();
        for &(table, _) in &self.registry {
            store.index(table);
        }
        self.builders = (0..self.registry.len())
            .map(|at| {
                let _label = self.label(at);
                let (table, shape) = &self.registry[at];
                let view = store.view(*table);
                summary::Builder::open(shape, &view, view.status(), hour)
            })
            .collect();
        self.ns += clock.elapsed().as_nanos() as u64;
    }

    pub fn hour(&mut self, store: &Store, hour: i32) {
        let clock = Instant::now();
        for ((builder, (table, _)), &owner) in self.builders.iter_mut().zip(&self.registry).zip(&self.owners) {
            let _label = engine_allocation::labels::enter(owner, engine_allocation::labels::Kind::Operator);
            let view = store.view(*table);
            builder.hour(hour, &view, &store.log().tables[*table], view.status(), view.view());
        }
        self.ns += clock.elapsed().as_nanos() as u64;
        if engine_allocation::labels::ENABLED && hour.rem_euclid(24) == 0 {
            self.note_peaks();
        }
    }

    fn note_peaks(&mut self) {
        for (peak, builder) in self.peaks.iter_mut().zip(&self.builders) {
            *peak = (*peak).max(builder.held_bytes());
        }
    }

    pub fn table_bytes(&self) -> Vec<(usize, usize, usize, usize)> {
        self.registry
            .iter()
            .enumerate()
            .map(|(at, (table, _))| {
                let ladder = self.ladders[at].upper.iter().map(|level| level.capacity() * size_of::<PieceAt>()).sum();
                (*table, ladder, self.builders.get(at).map_or(0, summary::Builder::held_bytes), self.last_peaks[at])
            })
            .collect()
    }

    pub fn finish<W: io::Write + Seek + io::Read>(&mut self, last_hour: i32, block: u32, first_hour: i32, file: &mut FileWriter<W>) -> io::Result<(Vec<(u32, Vec<u8>)>, Vec<(u32, u8, Vec<u8>)>)> {
        let clock = Instant::now();
        let closes = (block + 1) % SPAN_BLOCKS == 0;
        if engine_allocation::labels::ENABLED {
            self.note_peaks();
            self.last_peaks = std::mem::replace(&mut self.peaks, vec![0; self.registry.len()]);
        }
        let mut pieces = Vec::new();
        let mut above = Vec::new();
        let builders = std::mem::take(&mut self.builders);
        for (at, builder) in builders.into_iter().enumerate() {
            let _label = self.label(at);
            let table = self.registry[at].0 as u32;
            let piece = builder.finish(last_hour + 1)?.map(|raw| raw.assemble(self.level)).transpose()?;
            if !closes {
                pieces.extend(piece.map(|piece| (table, piece)));
                continue;
            }
            let mut held = file.span_summaries(table)?;
            held.extend(piece.iter().cloned());
            pieces.extend(piece.map(|piece| (table, piece)));
            let mut level = 0usize;
            let mut blocks = u64::from(SPAN_BLOCKS);
            while u64::from(block + 1) % blocks == 0 {
                let start = i64::from(first_hour) + (u64::from(block + 1) - blocks) as i64 * i64::from(UNIT_ORDINALS);
                let hours = (i64::from(last_hour) + 1 - start) as u32;
                let refs: Vec<&[u8]> = held.iter().map(Vec::as_slice).collect();
                let folded = match refs.is_empty() {
                    true => None,
                    false => summary::fold(&refs, start, hours)?.map(|raw| raw.assemble(self.level)).transpose()?,
                };
                level += 1;
                blocks *= FANOUT as u64;
                let ladder = &mut self.ladders[at];
                if ladder.upper.len() < level {
                    ladder.upper.push(Vec::new());
                }
                if u64::from(block + 1) % blocks == 0 {
                    held = std::mem::take(&mut ladder.upper[level - 1]).iter().map(|piece| file.summary(piece.offset, piece.stored, piece.digest)).collect::<io::Result<_>>()?;
                    held.extend(folded.iter().cloned());
                }
                above.extend(folded.map(|folded| (table, level as u8, folded)));
            }
        }
        self.ns += clock.elapsed().as_nanos() as u64;
        Ok((pieces, above))
    }

    pub fn written(&mut self, span: &[(u32, u8, envelope::Extent)]) -> io::Result<()> {
        if span.iter().any(|&(_, level, _)| level == 0) {
            return Err(invalid("A span summary needs a level above zero"));
        }
        for (at, ladder) in self.ladders.iter_mut().enumerate() {
            let table = self.registry[at].0 as u32;
            let Some(&(_, level, extent)) = span.iter().filter(|(held, _, _)| *held == table).max_by_key(|(_, level, _)| *level) else { continue };
            let level = usize::from(level);
            while ladder.upper.len() < level {
                ladder.upper.push(Vec::new());
            }
            ladder.upper[level - 1].push(PieceAt { offset: extent.offset, stored: extent.stored, digest: extent.digest });
        }
        Ok(())
    }

    pub fn tail<W: io::Write + Seek + io::Read>(&mut self, first_hour: i32, hours: u32, file: &mut FileWriter<W>) -> io::Result<Vec<(u32, Vec<u8>)>> {
        let clock = Instant::now();
        let blocks = hours.div_ceil(UNIT_ORDINALS);
        let start = i64::from(first_hour) + i64::from(blocks - blocks % SPAN_BLOCKS) * i64::from(UNIT_ORDINALS);
        let span_hours = (i64::from(first_hour) + i64::from(hours) - start) as u32;
        let mut tail = Vec::new();
        for at in 0..self.registry.len() {
            let _label = self.label(at);
            let table = self.registry[at].0 as u32;
            let held = file.span_summaries(table)?;
            if held.is_empty() {
                continue;
            }
            let refs: Vec<&[u8]> = held.iter().map(Vec::as_slice).collect();
            if let Some(folded) = summary::fold(&refs, start, span_hours)? {
                tail.push((table, folded.assemble(self.level)?));
            }
        }
        self.ns += clock.elapsed().as_nanos() as u64;
        Ok(tail)
    }

    pub fn write(&self, out: &mut Bytes) -> io::Result<()> {
        out.varint(self.ladders.len() as u64)?;
        for ladder in &self.ladders {
            out.varint(ladder.upper.len() as u64)?;
            for level in &ladder.upper {
                out.varint(level.len() as u64)?;
                for piece in level {
                    out.varint(piece.offset)?;
                    out.varint(piece.stored)?;
                    out.raw(&piece.digest)?;
                }
            }
        }
        Ok(())
    }

    pub fn read(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut reader = Reader::new(bytes);
        if reader.count()? != self.ladders.len() {
            return Err(invalid("A resume state's summaries are not this build's"));
        }
        for ladder in &mut self.ladders {
            let levels = reader.count()?;
            if levels > 16 {
                return Err(invalid("A resume state's summaries are malformed"));
            }
            ladder.upper = Vec::with_capacity(levels);
            for _ in 0..levels {
                let count = reader.count()?;
                if count > FANOUT as usize {
                    return Err(invalid("A resume state's summaries are malformed"));
                }
                let mut held = Vec::with_capacity(count);
                for _ in 0..count {
                    let offset = reader.varint()?;
                    let stored = reader.varint()?;
                    let digest = reader.raw(16)?.try_into().expect("sixteen bytes");
                    held.push(PieceAt { offset, stored, digest });
                }
                ladder.upper.push(held);
            }
        }
        reader.finish()
    }

    pub fn held_bytes(&self) -> (usize, usize) {
        let builders = self.builders.iter().map(summary::Builder::held_bytes).sum();
        let ladders = self.ladders.iter().map(|ladder| ladder.upper.iter().map(|level| level.capacity() * size_of::<PieceAt>()).sum::<usize>()).sum();
        (builders, ladders)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_span_level_refuses_the_entire_write() {
        let mut summaries = Summaries::new(1).unwrap();
        let first = summaries.registry[0].0 as u32;
        let second = summaries.registry[1].0 as u32;
        let extent = |stream, offset| envelope::Extent {
            kind: envelope::Kind::Summary,
            run: 0,
            stream,
            first: 0,
            hours: SPAN_BLOCKS * UNIT_ORDINALS,
            offset,
            stored: 64,
            raw: 64,
            digest: [0; 16],
        };
        summaries.written(&[(first, 1, extent(first, 128))]).unwrap();
        let mut before = Bytes::new();
        summaries.write(&mut before).unwrap();

        let error = summaries.written(&[(first, 2, extent(first, 192)), (second, 0, extent(second, 256))]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);

        let mut after = Bytes::new();
        summaries.write(&mut after).unwrap();
        assert_eq!(after.into_bytes(), before.into_bytes());
    }
}
