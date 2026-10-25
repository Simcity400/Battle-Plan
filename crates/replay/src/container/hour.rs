use super::*;
use engine_allocation as allocation;
use std::sync::Arc;

pub enum PointResult<'a> {
    Frame(&'a DecodedFrame),
    Gap {
        branch: BranchId,
        hour: Hour,
        reason: &'a str,
    },
}

#[derive(Default)]
pub struct HourReader {
    core: Option<reader::Core>,
    gap: String,
    source: Option<Arc<Replay>>,
    selection: Option<ReaderSelection>,
    forward: bool,
}

impl HourReader {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_hour<T>(
        &mut self,
        replay: &Arc<Replay>,
        branch: BranchId,
        hour: Hour,
        selection: ReaderSelection,
        mut cancel: impl FnMut() -> io::Result<()>,
        inspect: impl FnOnce(PointResult<'_>) -> T,
    ) -> io::Result<T> {
        let mut check = Check::new(&mut cancel);
        let same = !replay.is_file_backed()
            && self
                .source
                .as_ref()
                .is_some_and(|source| Arc::ptr_eq(source, replay))
            && self.selection.as_ref() == Some(&selection);
        let mut held = self.core.take();
        self.source = None;
        check.run()?;
        let exact = same
            && held
                .as_ref()
                .and_then(reader::Core::current)
                .is_some_and(|frame| frame.branch == branch && frame.hour == hour);
        let forward = same
            && self.forward
            && held
                .as_ref()
                .is_some_and(|core| core.can_continue_point(branch, hour, &selection));
        if exact {
            self.core = held;
        } else if forward {
            let mut core = held.take().unwrap();
            let result = (|| {
                while core
                    .next_hour(replay, &mut check)?
                    .is_some_and(|next| next <= hour)
                {
                    core.advance(replay, &mut check)?;
                }
                let found = core.current().is_some_and(|frame| frame.hour == hour);
                if found {
                    core.finish_point(replay, &mut check)?;
                }
                Ok(found)
            })();
            self.store_result(replay, branch, hour, core, result, &check)?;
        } else {
            drop(held);
            self.read(replay, branch, hour, selection.clone(), false, &mut check)?;
        }
        self.source = Some(Arc::clone(replay));
        self.selection = Some(selection);
        if let Err(error) = check.run() {
            self.core = None;
            self.source = None;
            return Err(error);
        }
        Ok(inspect(
            match self.core.as_ref().and_then(reader::Core::current) {
                Some(frame) => PointResult::Frame(frame),
                None => PointResult::Gap {
                    branch,
                    hour,
                    reason: &self.gap,
                },
            },
        ))
    }

    pub fn census(&self) -> ReaderCensus {
        self.core
            .as_ref()
            .map_or_else(ReaderCensus::default, reader::Core::census)
    }

    pub(super) fn seek(
        &mut self,
        replay: &Replay,
        branch: BranchId,
        hour: Hour,
        tables: Option<&[TableId]>,
        check: &mut Check<'_>,
    ) -> io::Result<SeekResult> {
        let selection = ReaderSelection::new(tables.unwrap_or(&TableId::ALL))?;
        self.read(replay, branch, hour, selection, true, check)?;
        match self.core.as_mut().and_then(reader::Core::take_frame) {
            Some(frame) => {
                self.core = None;
                Ok(SeekResult::Frame(allocation::boxed(frame)?))
            }
            None => Ok(SeekResult::Gap {
                branch,
                hour,
                reason: std::mem::take(&mut self.gap),
            }),
        }
    }

    fn read(
        &mut self,
        replay: &Replay,
        branch: BranchId,
        hour: Hour,
        selection: ReaderSelection,
        compatibility: bool,
        check: &mut Check<'_>,
    ) -> io::Result<()> {
        self.core = None;
        self.source = None;
        self.selection = None;
        self.gap.clear();
        check.run()?;
        if !replay.branches.iter().any(|entry| entry.id == branch) {
            self.gap = allocation::string("unknown branch")?;
            return Ok(());
        }
        let Some((position, index)) = replay.index.iter().enumerate().find(|(_, index)| {
            index.branch == branch && index.first_hour <= hour && hour <= index.last_hour
        }) else {
            self.gap = replay.gap_reason(branch, hour)?;
            return Ok(());
        };
        let mut core = reader::Core::local(selection, 1)?;
        self.forward = hour != index.first_hour || index.first_hour == index.last_hour;
        let read = (|| -> io::Result<bool> {
            core.open(replay, position, self.forward, None, check)?;
            loop {
                if core.done() {
                    core.finish(replay, check)?;
                    return Ok(false);
                }
                if core
                    .next_hour(replay, check)?
                    .is_some_and(|next| next > hour)
                {
                    if compatibility {
                        core.validate_point_envelopes(replay, check)?;
                    }
                    core.finish_point(replay, check)?;
                    return Ok(false);
                }
                core.advance(replay, check)?;
                let reached = core.current().unwrap().hour;
                if reached == hour {
                    if compatibility {
                        core.validate_point_envelopes(replay, check)?;
                    }
                    core.finish_point(replay, check)?;
                    return Ok(true);
                }
            }
        })();
        self.store_result(replay, branch, hour, core, read, check)
    }

    fn store_result(
        &mut self,
        replay: &Replay,
        branch: BranchId,
        hour: Hour,
        core: reader::Core,
        read: io::Result<bool>,
        check: &Check<'_>,
    ) -> io::Result<()> {
        match read {
            Ok(true) => {
                self.core = Some(core);
                Ok(())
            }
            Ok(false) => {
                self.gap = replay.gap_reason(branch, hour)?;
                Ok(())
            }
            Err(error) if check.failed() || !recording_damage(&error) => Err(error),
            Err(error)
                if error
                    .get_ref()
                    .is_some_and(|error| error.is::<reader::FrameFailure>()) =>
            {
                self.gap = allocation::string(&error.to_string())?;
                Ok(())
            }
            Err(error) => {
                self.gap =
                    if error.to_string() == "frame campaign identity differs from replay header" {
                        allocation::string("frame campaign identity differs from replay header")?
                    } else {
                        crate::codec::try_format(format_args!(
                            "unreadable block: {}",
                            crate::codec::diagnostic_text(&error.to_string())
                        ))?
                    };
                Ok(())
            }
        }
    }
}
