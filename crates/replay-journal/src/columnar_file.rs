use replay_columnar::state::{Column, Table};
use std::io;

pub struct RestartState {
    pub bytes: Vec<u8>,
    pub ids: Vec<u32>,
}

#[cfg(any(test, feature = "recording"))]
mod writer;
#[cfg(any(test, feature = "recording"))]
pub use writer::*;

pub fn referenced_ids(table: &Table, refusal: Option<u32>) -> io::Result<Vec<u32>> {
    let mut ids = Vec::new();
    if let Some(id) = refusal {
        engine_allocation::reserve(&mut ids, 1)?;
        ids.push(id);
    }
    for column in 0..table.kinds().count() {
        if let Some(Column::Bytes(texts)) = table.column(column) {
            for slot in table.live().ones() {
                if table.recorded(column, slot)
                    && let Some(&id) = texts.get(slot as usize)
                {
                    engine_allocation::reserve(&mut ids, 1)?;
                    ids.push(id);
                }
            }
        }
    }
    Ok(ids)
}

#[cfg(any(test, feature = "test-support"))]
pub use hours::HourWriter;

#[cfg(any(test, feature = "test-support"))]
mod hours {
    use super::{BlockEnd, FileWriter};
    use crate::summaries::Summaries;
    use recorder_derive_framework::framework::Writers;
    use replay_columnar::{
        envelope::UNIT_ORDINALS,
        lane::{Clock, ClockHour},
        manifest::Manifest,
        store::{Store, StringSource, journal::Journal},
        unit::{self, Encoder},
    };
    use std::{
        io::{self, Read, Seek, Write},
        sync::{Arc, Mutex, PoisonError},
    };

    #[derive(Clone, Default)]
    struct Appended(Arc<Mutex<Vec<Vec<u8>>>>);

    impl StringSource for Appended {
        fn text(&mut self, id: u32) -> io::Result<Option<Vec<u8>>> {
            Ok(self.0.lock().unwrap_or_else(PoisonError::into_inner).get(id as usize).cloned())
        }
    }

    struct OpenBlock {
        hours: u32,
        clock: Clock,
        count: u64,
        strings: Vec<u8>,
        first: u32,
    }

    pub struct HourWriter<W: Write + Seek + Read> {
        file: FileWriter<W>,
        store: Store,
        adapter: Journal,
        writers: Writers,
        columns: Vec<usize>,
        appended: Appended,
        running: bool,
        block: Option<OpenBlock>,
        summaries: Summaries,
        run: (i32, u32),
    }

    impl<W: Write + Seek + Read> HourWriter<W> {
        pub fn new(file: FileWriter<W>, level: i32, writers: Writers) -> io::Result<Self> {
            Self::with_unit(file, level, unit::Settings::at(level), writers)
        }

        pub fn with_unit(file: FileWriter<W>, level: i32, settings: unit::Settings, writers: Writers) -> io::Result<Self> {
            Self::with_schema(file, settings, replay_recording::schema::schema()?, writers, Summaries::new(level)?)
        }

        #[cfg(any(test, feature = "test-support"))]
        pub fn framed(file: FileWriter<W>, level: i32, settings: unit::Settings, columns: &[usize], writers: impl FnOnce(&[usize], &[usize]) -> Writers) -> io::Result<Self> {
            let mut kinds = replay_recording::schema::schema()?;
            let current: Vec<usize> = kinds.iter().map(Vec::len).collect();
            kinds.truncate(columns.len());
            for (kinds, &held) in kinds.iter_mut().zip(columns) {
                kinds.truncate(held);
            }
            let writers = writers(columns, &current);
            Self::with_schema(file, settings, kinds, writers, Summaries::new(level)?)
        }

        pub(crate) fn with_summaries(file: FileWriter<W>, level: i32, writers: Writers, summaries: Summaries) -> io::Result<Self> {
            Self::with_schema(file, unit::Settings::at(level), replay_recording::schema::schema()?, writers, summaries)
        }

        fn with_schema(mut file: FileWriter<W>, settings: unit::Settings, kinds: Vec<Vec<replay_columnar::state::Kind>>, writers: Writers, summaries: Summaries) -> io::Result<Self> {
            let tables = kinds.len();
            file.record_derived_versions(tables, &writers.table_versions())?;
            let columns = kinds.iter().map(Vec::len).collect();
            let mut store = Store::new(kinds.clone(), replay_recording::schema::keeps_last()[..tables].to_vec());
            store.declare_logs(&replay_data::rows::logs()[..tables])?;
            store.attach(Encoder::new(&kinds, settings)?);
            let appended = Appended::default();
            store.read_strings_from(Box::new(appended.clone()));
            let adapter = Journal::leaving(&writers.tables(), replay_recording::schema::names());
            Ok(Self { file, store, adapter, writers, columns, appended, running: false, block: None, summaries, run: (0, 0) })
        }

        pub fn file(&mut self) -> &mut FileWriter<W> {
            &mut self.file
        }

        #[cfg(any(test, feature = "test-support"))]
        pub fn define(&mut self, definitions: recorder_derive_framework::framework::Definitions) {
            self.writers.define(definitions);
        }

        pub fn frame(&self, tables: &[replay_data::rows::TableId], (branch, hour): (u64, i32), mode: replay_data::scope::CaptureMode) -> io::Result<replay_frame::frame::DecodedFrame> {
            let format = replay_data::format::FormatVersion::CURRENT.version();
            let mut frame = replay_frame::frame::stored_frame(tables.iter().map(|&table| (table, self.store.view(table.index()))), (branch, hour), mode, format)?;
            for &table in tables {
                let view = self.store.view(table.index());
                if matches!(view.status(), replay_columnar::manifest::Status::Complete) {
                    let held = std::mem::replace(&mut frame.tables[table.index()], replay_frame::frame::not_selected());
                    frame.tables[table.index()] = held.with_sampled_at(view.sampled_at());
                }
            }
            Ok(frame)
        }

        pub fn derived_frame(&self, at: (u64, i32), mode: replay_data::scope::CaptureMode) -> io::Result<replay_frame::frame::DecodedFrame> {
            let tables: Vec<replay_data::rows::TableId> = self.writers.tables().into_iter().map(|index| replay_data::rows::TableId::ALL[index]).collect();
            let mut frame = self.frame(&tables, at, mode)?;
            for table in tables {
                if self.store.view(table.index()).sampled_at() == Some(at.1) {
                    frame.metrics.table_ns[table.index()] = replay_frame::frame::READ_TIME_NOT_RECORDED;
                }
            }
            Ok(frame)
        }

        pub fn start_run(&mut self, branch: u64, epoch: u8, first_hour: i32) -> io::Result<()> {
            self.end_run()?;
            self.file.start_run(branch, epoch, first_hour)?;
            self.store.begin_run(first_hour);
            self.writers.start_run();
            self.summaries.start_run();
            self.run = (first_hour, 0);
            self.appended.0.lock().unwrap_or_else(PoisonError::into_inner).clear();
            self.running = true;
            Ok(())
        }

        pub fn hour(&mut self, manifest: &[u8], sections: &[u8]) -> io::Result<()> {
            if !self.running {
                return Err(io::Error::other("An hour needs a run under way"));
            }
            let manifest = Manifest::read(manifest, &self.columns)?;
            let first = self.store.strings().next();
            if self.block.is_none() {
                self.summaries.open(&mut self.store, manifest.hour);
            }
            let block = self.block.get_or_insert_with(|| OpenBlock { hours: 0, clock: Clock::default(), count: 0, strings: Vec::new(), first });
            for text in &manifest.appends {
                unit::put_varint(&mut block.strings, text.len() as u64);
                block.strings.extend_from_slice(text);
                self.appended.0.lock().unwrap_or_else(PoisonError::into_inner).push(text.to_vec());
            }
            block.count += manifest.appends.len() as u64;
            block.clock.hour(&ClockHour { flags: manifest.flags, total_ns: manifest.total_ns, pool_bytes: manifest.pool_bytes, dropped_frames: manifest.dropped_frames });
            block.hours += 1;
            let ends = block.hours == UNIT_ORDINALS;
            self.adapter.begin_hour(&mut self.store, &manifest)?;
            let mut at = 0_usize;
            for entry in &manifest.sections {
                let end = at + entry.bytes as usize;
                let bytes = sections.get(at..end).ok_or_else(|| io::Error::other("An hour's sections end before its manifest's"))?;
                self.adapter.section(&mut self.store, entry, bytes)?;
                at = end;
            }
            self.adapter.applied(&mut self.store);
            self.writers.hour(manifest.branch, manifest.hour, self.run.1 == 0, &mut self.store)?;
            self.summaries.hour(&self.store, manifest.hour);
            self.store.end_hour(&mut self.file)?;
            self.adapter.moved(&mut self.store);
            self.run.1 += 1;
            if ends {
                self.end_block()?;
            }
            Ok(())
        }

        fn end_block(&mut self) -> io::Result<()> {
            let Some(block) = self.block.take() else { return Ok(()) };
            let mut strings = Vec::new();
            unit::put_varint(&mut strings, block.count);
            strings.extend_from_slice(&block.strings);
            let clock = block.clock.finish();
            let (first_hour, hours) = self.run;
            let (summaries, span) = self.summaries.finish(first_hour + hours as i32 - 1, (hours - block.hours) / UNIT_ORDINALS, first_hour, &mut self.file)?;
            let (_, span) = self.file.end_block(BlockEnd { clock: &clock, strings: (&strings, block.first), summaries, span })?;
            self.summaries.written(&span)?;
            self.store.end_block();
            Ok(())
        }

        pub fn end_run(&mut self) -> io::Result<()> {
            if !std::mem::take(&mut self.running) {
                return Ok(());
            }
            self.store.end_run(&mut self.file)?;
            self.end_block()?;
            let tail = self.summaries.tail(self.run.0, self.run.1, &mut self.file)?;
            self.file.tail(&tail)?;
            self.file.finish_run()
        }

        pub fn finish(mut self) -> io::Result<FileWriter<W>> {
            self.end_run()?;
            Ok(self.file)
        }
    }
}

pub struct FileStrings {
    file: std::fs::File,
    blocks: std::sync::Arc<std::sync::Mutex<Vec<(u32, replay_columnar::unit::FrameEntry)>>>,
    held: std::collections::HashMap<usize, (Vec<u8>, Vec<(u32, u32)>)>,
}

impl FileStrings {
    pub fn new(file: std::fs::File, blocks: std::sync::Arc<std::sync::Mutex<Vec<(u32, replay_columnar::unit::FrameEntry)>>>) -> Self {
        Self { file, blocks, held: std::collections::HashMap::new() }
    }
}

impl replay_columnar::store::StringSource for FileStrings {
    fn text(&mut self, id: u32) -> io::Result<Option<Vec<u8>>> {
        let (at, frame, first) = {
            let blocks = self.blocks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let at = blocks.partition_point(|&(first, _)| first <= id);
            let Some(at) = at.checked_sub(1) else { return Ok(None) };
            let (first, frame) = blocks[at].clone();
            (at, frame, first)
        };
        if !self.held.contains_key(&at) {
            let _label = engine_allocation::labels::enter(engine_allocation::labels::owner("strings: chunks read again"), engine_allocation::labels::Kind::Operator);
            let decoded = replay_recording::schema::strings_frame(&self.file, &frame)?;
            self.held.insert(at, decoded);
        }
        let (raw, places) = &self.held[&at];
        Ok(places.get((id - first) as usize).map(|&(at, length)| raw[at as usize..(at + length) as usize].to_vec()))
    }

    fn release(&mut self) {
        self.held = std::collections::HashMap::new();
    }
}

#[cfg(any(test, feature = "recording"))]
fn invalid(message: &'static str) -> io::Error {
    if engine_allocation::capture_active() {
        io::Error::from(io::ErrorKind::InvalidData)
    } else {
        io::Error::new(io::ErrorKind::InvalidData, message)
    }
}
