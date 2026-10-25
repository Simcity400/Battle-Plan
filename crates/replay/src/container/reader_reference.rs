use super::super::{SeekResult, recording_damage, validate_campaign};
use super::*;
use crate::frame::{FrameSource, advance_table, keyframe_update, not_selected};

// The pre-streaming serial orchestration is independent of FrameAdvance.
#[derive(Clone, Default)]
pub struct ReferenceDecoder {
    current: Option<DecodedFrame>,
    selected: Option<[bool; TABLE_COUNT]>,
}

impl ReferenceDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn for_tables(tables: &[TableId]) -> Self {
        let mut selected = [false; TABLE_COUNT];
        for table in tables {
            selected[table.index()] = true;
        }
        Self {
            current: None,
            selected: Some(selected),
        }
    }

    pub fn advance(&mut self, frame: &(impl FrameSource + ?Sized)) -> io::Result<&DecodedFrame> {
        if self
            .current
            .as_ref()
            .is_some_and(|current| current.branch == frame.branch() && current.hour >= frame.hour())
        {
            return Err(invalid("frame hour did not advance on its branch"));
        }
        if frame.keyframe() && !(0..TABLE_COUNT).all(|index| keyframe_update(frame.update(index))) {
            return Err(invalid("keyframe contains a dependent or partial table"));
        }
        let selected = |index: usize| {
            self.selected
                .as_ref()
                .is_none_or(|selected| selected[index])
        };
        let current = self.current.take();
        let continues = current
            .as_ref()
            .is_some_and(|current| !frame.keyframe() && current.branch == frame.branch());
        let (previous_hour, mut held) = match current {
            Some(current) => (current.hour, current.tables.map(Some)),
            None => (frame.hour(), std::array::from_fn(|_| None)),
        };
        if !continues {
            for (index, view) in held.iter_mut().enumerate() {
                if selected(index) {
                    *view = None;
                }
            }
        }
        let mut allocation_failure = None;
        let tables = std::array::from_fn(|index| {
            if !selected(index) {
                return held[index].take().unwrap_or_else(not_selected);
            }
            if allocation_failure.is_some() {
                return not_selected();
            }
            let table = TableId::ALL[index];
            let previous = held[index]
                .take()
                .filter(|view| matches!(**view, TableView::Complete(_)))
                .map(|view| (previous_hour, view));
            let advanced = advance_table(
                table,
                frame.update(index),
                previous,
                frame.hour(),
                frame.metrics().table_ns[index] != 0,
            );
            match advanced {
                Ok(view) => view,
                Err(error) => {
                    allocation_failure = Some(error);
                    not_selected()
                }
            }
        });
        if let Some(error) = allocation_failure {
            return Err(error);
        }
        self.current = Some(DecodedFrame {
            branch: frame.branch(),
            hour: frame.hour(),
            tables,
            metrics: frame.metrics().clone(),
        });
        Ok(self.current.as_ref().unwrap())
    }
}

impl ReferenceDecoder {
    pub fn current(&self) -> Option<&DecodedFrame> {
        self.current.as_ref()
    }
}

pub(super) fn seek(
    replay: &Replay,
    branch: BranchId,
    hour: Hour,
    tables: Option<&[TableId]>,
    check: &mut Check<'_>,
) -> io::Result<SeekResult> {
    check.run()?;
    let gap = |reason| SeekResult::Gap {
        branch,
        hour,
        reason,
    };
    if !replay.branches.iter().any(|entry| entry.id == branch) {
        return Ok(gap("unknown branch".into()));
    }
    let Some(index) = replay.index.iter().find(|index| {
        index.branch == branch && index.first_hour <= hour && hour <= index.last_hour
    }) else {
        return Ok(gap(replay.gap_reason(branch, hour)?));
    };
    let selected = tables
        .map(|tables| -> io::Result<_> {
            let include_campaign = !tables.contains(&TableId::Campaign);
            let capacity = engine_allocation::add(tables.len(), usize::from(include_campaign))?;
            let mut selected = engine_allocation::vec(capacity)?;
            selected.extend_from_slice(tables);
            if include_campaign {
                selected.push(TableId::Campaign);
            }
            Ok(selected)
        })
        .transpose()?;
    let mut decoder = selected
        .as_deref()
        .map_or_else(ReferenceDecoder::new, ReferenceDecoder::for_tables);
    let mut found = None;
    let read = replay
        .read_block_selected_through_checked(index, selected.as_deref(), hour, None, check)
        .and_then(|frames| {
            let mut visit = |frame: Frame| {
                check.run()?;
                if frame.hour > hour {
                    return Ok(false);
                }
                let decoded = match decoder.advance(&frame) {
                    Ok(decoded) => decoded,
                    Err(error) => {
                        if !recording_damage(&error) {
                            return Err(error);
                        }
                        found = Some(gap(crate::codec::try_format(format_args!(
                            "invalid frame baseline at hour {}: {}",
                            frame.hour,
                            crate::codec::diagnostic_text(&error.to_string())
                        ))?));
                        return Ok(false);
                    }
                };
                if let Err(error) = validate_campaign(decoded, &replay.campaign_id) {
                    found = Some(gap(engine_allocation::string(
                        crate::codec::diagnostic_text(&error.to_string()),
                    )?));
                    return Ok(false);
                }
                if frame.hour == hour {
                    found = Some(SeekResult::Frame(engine_allocation::boxed(
                        decoded.clone(),
                    )?));
                    return Ok(false);
                }
                Ok(true)
            };
            for frame in frames {
                if !visit(frame)? {
                    break;
                }
            }
            Ok(())
        });
    if let Err(error) = read {
        if !recording_damage(&error) {
            return Err(error);
        }
        return Ok(gap(crate::codec::try_format(format_args!(
            "unreadable block: {}",
            crate::codec::diagnostic_text(&error.to_string())
        ))?));
    }
    match found {
        Some(found) => Ok(found),
        None => Ok(gap(replay.gap_reason(branch, hour)?)),
    }
}
