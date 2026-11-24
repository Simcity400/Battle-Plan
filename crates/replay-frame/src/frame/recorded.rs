use super::{Frame, Metrics, TABLE_COUNT, TableUpdate};
use crate::{BranchId, Hour};
use engine_allocation as allocation;
use std::{io, ops::Index, sync::Arc};

#[derive(Clone, Debug)]
pub struct Updates {
    values: Arc<Vec<Arc<TableUpdate>>>,
    len: usize,
}
impl Updates {
    pub fn new(updates: Vec<TableUpdate>) -> io::Result<Self> {
        let len = updates.len();
        Ok(Self {
            values: Arc::new(allocation::collect_vec(updates.into_iter().map(Arc::new))?),
            len,
        })
    }
    pub fn repeat(update: TableUpdate, len: usize) -> io::Result<Self> {
        let value = Arc::new(update);
        Ok(Self {
            values: Arc::new(allocation::collect_vec(
                (0..len).map(|_| Arc::clone(&value)),
            )?),
            len,
        })
    }
    pub fn prefix(&self, len: usize) -> Self {
        Self {
            values: Arc::clone(&self.values),
            len,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn get(&self, at: usize) -> &Arc<TableUpdate> {
        &self.values[..self.len][at]
    }
    pub fn first(&self) -> Option<&TableUpdate> {
        self.values.first().map(AsRef::as_ref)
    }
    pub fn bytes(&self) -> usize {
        size_of::<Vec<Arc<TableUpdate>>>()
            + 2 * size_of::<usize>()
            + self.values.capacity() * size_of::<Arc<TableUpdate>>()
            + self
                .values
                .iter()
                .map(|value| cell_bytes(value))
                .sum::<usize>()
    }
}

#[derive(Clone, Debug)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RecordedFrame {
    pub branch: BranchId,
    pub hour: Hour,
    pub keyframe: bool,
    pub metrics: Metrics,
    pub tables: RecordedTables,
}
#[derive(Clone, Debug)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RecordedTables(Backing);
#[derive(Clone, Debug)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
enum Backing {
    Frame(Arc<Frame>),
    Tables {
        slots: Arc<[Option<usize>; TABLE_COUNT]>,
        updates: Vec<Arc<TableUpdate>>,
        unselected: Arc<TableUpdate>,
    },
}
impl Index<usize> for RecordedTables {
    type Output = TableUpdate;
    fn index(&self, index: usize) -> &Self::Output {
        match &self.0 {
            Backing::Frame(frame) => &frame.tables[index],
            Backing::Tables {
                slots,
                updates,
                unselected,
            } => slots[index].map_or(&**unselected, |slot| &*updates[slot]),
        }
    }
}
impl RecordedTables {
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &TableUpdate> {
        (0..TABLE_COUNT).map(|index| &self[index])
    }
    pub fn len(&self) -> usize {
        TABLE_COUNT
    }
    pub fn is_empty(&self) -> bool {
        TABLE_COUNT == 0
    }
}
impl RecordedFrame {
    #[cfg(any(test, feature = "test-support"))]
    pub fn inline_identity(&self) -> Option<std::sync::Weak<Frame>> {
        match &self.tables.0 {
            Backing::Frame(frame) => Some(Arc::downgrade(frame)),
            Backing::Tables { .. } => None,
        }
    }
    pub fn from_updates(
        branch: BranchId,
        hour: Hour,
        keyframe: bool,
        metrics: Metrics,
        slots: Arc<[Option<usize>; TABLE_COUNT]>,
        updates: Vec<Arc<TableUpdate>>,
        unselected: Arc<TableUpdate>,
    ) -> io::Result<Self> {
        let mut used = allocation::filled(updates.len(), false)?;
        for slot in slots.iter().flatten() {
            let present = used
                .get_mut(*slot)
                .ok_or_else(|| crate::codec::invalid("recorded table slot outside updates"))?;
            if std::mem::replace(present, true) {
                return Err(crate::codec::invalid("recorded table slot repeated"));
            }
        }
        if used.iter().any(|used| !used) {
            return Err(crate::codec::invalid("unmapped recorded table update"));
        }
        Ok(Self {
            branch,
            hour,
            keyframe,
            metrics,
            tables: RecordedTables(Backing::Tables {
                slots,
                updates,
                unselected,
            }),
        })
    }
    pub fn from_columns(
        branch: BranchId,
        hour: Hour,
        keyframe: bool,
        metrics: Metrics,
        slots: Arc<[Option<usize>; TABLE_COUNT]>,
        columns: &[Updates],
        position: usize,
        unselected: Arc<TableUpdate>,
    ) -> io::Result<Self> {
        Ok(Self {
            branch,
            hour,
            keyframe,
            metrics,
            tables: RecordedTables(Backing::Tables {
                slots,
                updates: allocation::collect_vec(
                    columns
                        .iter()
                        .map(|column| Arc::clone(column.get(position))),
                )?,
                unselected,
            }),
        })
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn to_owned_frame(&self) -> io::Result<Frame> {
        Ok(Frame {
            branch: self.branch,
            hour: self.hour,
            keyframe: self.keyframe,
            metrics: self.metrics.clone(),
            tables: copy_updates(self.tables.iter())?,
        })
    }
    pub fn retain_updates(&mut self, kept: &[bool; TABLE_COUNT]) -> io::Result<()> {
        if let Backing::Frame(frame) = &mut self.tables.0 {
            if kept.iter().all(|kept| *kept) {
                return Ok(());
            }
            if let Some(frame) = Arc::get_mut(frame) {
                for (update, &kept) in frame.tables.iter_mut().zip(kept) {
                    if !kept {
                        *update = TableUpdate::Unavailable {
                            reason: String::new(),
                        };
                    }
                }
            } else {
                let mut updates = allocation::vec(kept.iter().filter(|&&kept| kept).count())?;
                let mut slots = [None; TABLE_COUNT];
                for (index, &kept) in kept.iter().enumerate() {
                    if kept {
                        slots[index] = Some(updates.len());
                        updates.push(Arc::new(copy_update(&frame.tables[index])?));
                    }
                }
                self.tables.0 = Backing::Tables {
                    slots: Arc::new(slots),
                    updates,
                    unselected: Arc::new(TableUpdate::Unavailable {
                        reason: String::new(),
                    }),
                };
            }
        } else if let Backing::Tables { slots, updates, .. } = &mut self.tables.0 {
            if (0..TABLE_COUNT).all(|index| kept[index] || slots[index].is_none()) {
                return Ok(());
            }
            let mut next = 0;
            let filtered = std::array::from_fn(|index| {
                (kept[index] && slots[index].is_some()).then(|| {
                    let slot = next;
                    next += 1;
                    slot
                })
            });
            let mut position = 0;
            updates.retain(|_| {
                while slots[position].is_none() {
                    position += 1;
                }
                let keep = kept[position];
                position += 1;
                keep
            });
            *slots = Arc::new(filtered);
        }
        Ok(())
    }
    pub fn into_owned_frame(self) -> io::Result<Frame> {
        let Self {
            branch,
            hour,
            keyframe,
            metrics,
            tables,
        } = self;
        let values = match tables.0 {
            Backing::Frame(frame) => match Arc::try_unwrap(frame) {
                Ok(frame) => frame.tables,
                Err(frame) => copy_updates(frame.tables.iter())?,
            },
            Backing::Tables {
                slots,
                updates,
                unselected,
            } => {
                let mut cells = allocation::collect_vec(updates.into_iter().map(Some))?;
                let mut values = allocation::vec(TABLE_COUNT)?;
                for index in 0..TABLE_COUNT {
                    let cell = slots[index].map_or_else(
                        || Arc::clone(&unselected),
                        |slot| cells[slot].take().unwrap(),
                    );
                    values.push(match Arc::try_unwrap(cell) {
                        Ok(value) => value,
                        Err(value) => copy_update(&value)?,
                    });
                }
                values.try_into().unwrap()
            }
        };
        Ok(Frame {
            branch,
            hour,
            keyframe,
            metrics,
            tables: values,
        })
    }
    pub fn allocations(&self, mut visit: impl FnMut(*const (), usize)) {
        match &self.tables.0 {
            Backing::Frame(frame) => visit(
                Arc::as_ptr(frame).cast(),
                size_of::<Frame>()
                    + 2 * size_of::<usize>()
                    + frame.tables.iter().map(update_bytes).sum::<usize>(),
            ),
            Backing::Tables {
                slots,
                updates,
                unselected,
            } => {
                visit(
                    Arc::as_ptr(slots).cast(),
                    size_of_val(&**slots) + 2 * size_of::<usize>(),
                );
                visit(
                    updates.as_ptr().cast(),
                    updates.capacity() * size_of::<Arc<TableUpdate>>(),
                );
                for update in updates.iter().chain(std::iter::once(unselected)) {
                    visit(Arc::as_ptr(update).cast(), cell_bytes(update));
                }
            }
        }
    }
}
impl From<Arc<Frame>> for RecordedFrame {
    fn from(frame: Arc<Frame>) -> Self {
        Self {
            branch: frame.branch,
            hour: frame.hour,
            keyframe: frame.keyframe,
            metrics: frame.metrics.clone(),
            tables: RecordedTables(Backing::Frame(frame)),
        }
    }
}
impl From<Frame> for RecordedFrame {
    fn from(frame: Frame) -> Self {
        Arc::new(frame).into()
    }
}

pub trait IntoRecordedFrame {
    fn into_recorded(self) -> Arc<RecordedFrame>;
}
impl IntoRecordedFrame for Arc<RecordedFrame> {
    fn into_recorded(self) -> Arc<RecordedFrame> {
        self
    }
}
impl IntoRecordedFrame for RecordedFrame {
    fn into_recorded(self) -> Arc<RecordedFrame> {
        Arc::new(self)
    }
}
impl IntoRecordedFrame for Arc<Frame> {
    fn into_recorded(self) -> Arc<RecordedFrame> {
        Arc::new(self.into())
    }
}
impl IntoRecordedFrame for Frame {
    fn into_recorded(self) -> Arc<RecordedFrame> {
        Arc::new(self.into())
    }
}

pub trait FrameSource {
    fn branch(&self) -> BranchId;
    fn hour(&self) -> Hour;
    fn keyframe(&self) -> bool;
    fn metrics(&self) -> &Metrics;
    fn update(&self, index: usize) -> &TableUpdate;
    fn shared_update(&self, index: usize) -> Option<&Arc<TableUpdate>> {
        let _ = index;
        None
    }
}
macro_rules! source {
    ($name:ty) => {
        impl FrameSource for $name {
            fn branch(&self) -> BranchId {
                self.branch
            }
            fn hour(&self) -> Hour {
                self.hour
            }
            fn keyframe(&self) -> bool {
                self.keyframe
            }
            fn metrics(&self) -> &Metrics {
                &self.metrics
            }
            fn update(&self, index: usize) -> &TableUpdate {
                &self.tables[index]
            }
        }
    };
}
source!(Frame);
impl FrameSource for RecordedFrame {
    fn branch(&self) -> BranchId {
        self.branch
    }
    fn hour(&self) -> Hour {
        self.hour
    }
    fn keyframe(&self) -> bool {
        self.keyframe
    }
    fn metrics(&self) -> &Metrics {
        &self.metrics
    }
    fn update(&self, index: usize) -> &TableUpdate {
        &self.tables[index]
    }
    fn shared_update(&self, index: usize) -> Option<&Arc<TableUpdate>> {
        match &self.tables.0 {
            Backing::Frame(_) => None,
            Backing::Tables { slots, updates, .. } => slots[index].map(|slot| &updates[slot]),
        }
    }
}
macro_rules! forwarded_source {
    ($wrapper:ty) => {
        impl<T: FrameSource + ?Sized> FrameSource for $wrapper {
            fn branch(&self) -> BranchId {
                (**self).branch()
            }
            fn hour(&self) -> Hour {
                (**self).hour()
            }
            fn keyframe(&self) -> bool {
                (**self).keyframe()
            }
            fn metrics(&self) -> &Metrics {
                (**self).metrics()
            }
            fn update(&self, index: usize) -> &TableUpdate {
                (**self).update(index)
            }
            fn shared_update(&self, index: usize) -> Option<&Arc<TableUpdate>> {
                (**self).shared_update(index)
            }
        }
    };
}
forwarded_source!(Arc<T>);
forwarded_source!(Box<T>);
forwarded_source!(&T);
fn cell_bytes(value: &TableUpdate) -> usize {
    size_of::<TableUpdate>() + 2 * size_of::<usize>() + update_bytes(value)
}
pub fn update_bytes(update: &TableUpdate) -> usize {
    match update {
        TableUpdate::Snapshot(rows) | TableUpdate::Truncated { rows, .. } => rows.heap_bytes(),
        TableUpdate::Delta {
            changed, deleted, ..
        } => {
            changed.capacity() * size_of::<crate::rows::DeltaRow>()
                + changed
                    .iter()
                    .map(|row| row.values.capacity())
                    .sum::<usize>()
                + deleted.capacity() * size_of::<u128>()
        }
        TableUpdate::Unavailable { reason } | TableUpdate::Incompatible { reason, .. } => {
            reason.capacity()
        }
        TableUpdate::Unchanged { .. } | TableUpdate::NotRecorded { .. } => 0,
    }
}

fn copy_updates<'a>(
    updates: impl IntoIterator<Item = &'a TableUpdate>,
) -> io::Result<[TableUpdate; TABLE_COUNT]> {
    let mut copied = allocation::vec(TABLE_COUNT)?;
    for update in updates {
        copied.push(copy_update(update)?);
    }
    Ok(copied.try_into().unwrap())
}

pub(super) fn copy_update(update: &TableUpdate) -> io::Result<TableUpdate> {
    Ok(match update {
        TableUpdate::Snapshot(rows) => TableUpdate::Snapshot(rows.try_clone_validated()?),
        TableUpdate::Truncated { rows, missing } => TableUpdate::Truncated {
            rows: rows.try_clone_validated()?,
            missing: *missing,
        },
        TableUpdate::Delta {
            baseline,
            changed,
            deleted,
        } => {
            let mut rows = allocation::vec(changed.len())?;
            for row in changed {
                rows.push(crate::rows::DeltaRow {
                    format: row.format,
                    key: row.key,
                    mask: row.mask,
                    recorded_fields: row.recorded_fields,
                    values: allocation::copy_vec(&row.values)?,
                });
            }
            TableUpdate::Delta {
                baseline: *baseline,
                changed: rows,
                deleted: allocation::copy_vec(deleted)?,
            }
        }
        TableUpdate::Unchanged { baseline } => TableUpdate::Unchanged {
            baseline: *baseline,
        },
        TableUpdate::Unavailable { reason } => TableUpdate::Unavailable {
            reason: allocation::string(reason)?,
        },
        TableUpdate::NotRecorded { mode } => TableUpdate::NotRecorded { mode: *mode },
        TableUpdate::Incompatible {
            reason,
            source_format,
            recorded_bytes,
        } => TableUpdate::Incompatible {
            reason: allocation::string(reason)?,
            source_format: *source_format,
            recorded_bytes: *recorded_bytes,
        },
    })
}

impl PartialEq for RecordedFrame {
    fn eq(&self, other: &Self) -> bool {
        self.branch == other.branch
            && self.hour == other.hour
            && self.keyframe == other.keyframe
            && self.metrics == other.metrics
            && self.tables.iter().eq(other.tables.iter())
    }
}
impl Eq for RecordedFrame {}

#[cfg(test)]
mod tests;
