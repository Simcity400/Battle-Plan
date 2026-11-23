use super::*;
use replay_frame::{branches::{BranchReason, Parentage}, frame::{Metrics, TableUpdate}};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

pub fn session() -> Record {
    use replay_data::identity::{Identity, Verification};
    Record::SessionStart(SessionStart {
        id: 3,
        process_id: 12,
        process_start_time: 34,
        qpc_frequency: 10_000_000,
        sample_boundary: replay_data::identity::SampleBoundary::EndOfHour,
        capture_mode: replay_data::identity::CaptureMode::Full,
        identity: Identity {
            game_version: "test".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: Vec::new(),
            mods: Vec::new(),
            coverage_policy: 1,
            language: "l_english".into(),
            recorder_version: "test".into(),
            collector_hash: [3; 32],
            verification: Verification::Unchecked,
        },
    })
}

pub fn branch() -> Record {
    Record::BranchStart(BranchStart {
        id: 7,
        session: 3,
        loaded_hour: Some(9),
        first_hour: 10,
        parent: Parentage::Unknown,
        reason: BranchReason::SessionStart,
    })
}

pub fn frame(hour: i32, keyframe: bool) -> Frame {
    Frame {
        branch: 7,
        hour,
        keyframe,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|index| {
            match replay_frame::frame::current_absence(replay_data::rows::TableId::ALL[index]) {
                Some(replay_frame::frame::TableView::NotRecorded { mode }) => {
                    TableUpdate::NotRecorded { mode }
                }
                Some(replay_frame::frame::TableView::Unavailable { reason }) => {
                    TableUpdate::Unavailable { reason }
                }
                None => TableUpdate::Unavailable { reason: "x".into() },
                _ => unreachable!("absence is an omission or a named refusal"),
            }
        }),
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Fault {
    WriteAfter(usize),
    Flush,
    Sync,
    Create,
    StartSyncOom,
    StopDuringWrite,
    StopDuringFlush,
}

thread_local! {
    pub(in crate::spool::writer) static CAPTURE_STOPPED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[derive(Default)]
pub struct Memory {
    pub files: BTreeMap<u32, Vec<u8>>,
    pub fault: Option<Fault>,
    pub(in crate::spool::writer) flushes: usize,
    pub(in crate::spool::writer) syncs: usize,
    pub(in crate::spool::writer) background: bool,
    pub(in crate::spool::writer) sync_pending: bool,
    pub(in crate::spool::writer) sync_ready: bool,
    pub(in crate::spool::writer) sync_requests: usize,
}

#[derive(Clone, Default)]
pub struct MemoryBackend(pub Rc<RefCell<Memory>>);

pub struct MemorySink {
    shared: Rc<RefCell<Memory>>,
    index: u32,
}

fn injected() -> io::Error {
    io::Error::other("injected spool I/O failure")
}

impl SegmentBackend for MemoryBackend {
    type Sink = MemorySink;
    fn create(&mut self, index: u32) -> io::Result<MemorySink> {
        let mut memory = self.0.borrow_mut();
        if matches!(memory.fault, Some(Fault::Create)) {
            return Err(injected());
        }
        if memory.files.insert(index, Vec::new()).is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "segment reused",
            ));
        }
        Ok(MemorySink {
            shared: self.0.clone(),
            index,
        })
    }
}

impl Write for MemorySink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut memory = self.shared.borrow_mut();
        let length = memory.files[&self.index].len();
        let count = match memory.fault {
            Some(Fault::WriteAfter(limit)) => {
                if length >= limit {
                    return Err(injected());
                }
                bytes.len().min(limit - length)
            }
            _ => bytes.len(),
        };
        memory
            .files
            .get_mut(&self.index)
            .unwrap()
            .extend_from_slice(&bytes[..count]);
        if matches!(memory.fault, Some(Fault::StopDuringWrite)) {
            CAPTURE_STOPPED.set(true);
        }
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        let mut memory = self.shared.borrow_mut();
        memory.flushes += 1;
        if matches!(memory.fault, Some(Fault::StopDuringFlush)) {
            CAPTURE_STOPPED.set(true);
        }
        if matches!(memory.fault, Some(Fault::Flush)) {
            Err(injected())
        } else {
            Ok(())
        }
    }
}

impl SegmentSink for MemorySink {
    fn sync_all(&mut self) -> io::Result<()> {
        let mut memory = self.shared.borrow_mut();
        memory.syncs += 1;
        memory.sync_pending = false;
        if matches!(memory.fault, Some(Fault::Sync)) {
            Err(injected())
        } else {
            Ok(())
        }
    }

    fn start_sync(&mut self) -> io::Result<()> {
        if matches!(self.shared.borrow().fault, Some(Fault::StartSyncOom)) {
            return Err(io::Error::from(io::ErrorKind::OutOfMemory));
        }
        if !self.shared.borrow().background {
            return self.sync_all();
        }
        let mut memory = self.shared.borrow_mut();
        assert!(!memory.sync_pending);
        memory.sync_pending = true;
        memory.sync_ready = false;
        memory.sync_requests += 1;
        Ok(())
    }

    fn poll_sync(&mut self) -> io::Result<bool> {
        let mut memory = self.shared.borrow_mut();
        if memory.sync_pending && memory.sync_ready {
            memory.sync_pending = false;
            memory.syncs += 1;
            if matches!(memory.fault, Some(Fault::Sync)) {
                return Err(injected());
            }
        }
        Ok(!memory.sync_pending)
    }
}
