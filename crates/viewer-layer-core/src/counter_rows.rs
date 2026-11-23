use crate::layers::Dirty;
use playhead::changes::Changes;

#[derive(Clone, Copy)]
pub enum Row {
    Gone(u128),
    At(u32),
}

#[derive(Debug, Default)]
pub struct Rows {
    slots: Dirty,
    gone: Vec<u128>,
    gone_since: Option<u64>,
}

impl Rows {
    pub fn take(&mut self, changes: &Changes, frame: u64, reads: impl Fn(usize) -> bool) {
        let gone = changes.deleted().keys().unwrap_or_default();
        if !gone.is_empty() {
            self.gone.extend_from_slice(gone);
            self.gone_since.get_or_insert(frame);
        }
        for &slot in changes.inserted().slots() {
            self.slots.mark(slot, frame);
        }
        for written in changes.columns().iter().filter(|written| reads(written.column())) {
            for &slot in written.slots() {
                self.slots.mark(slot, frame);
            }
        }
    }

    pub fn settle(&mut self, mut each: impl FnMut(Row)) {
        for key in std::mem::take(&mut self.gone) {
            each(Row::Gone(key));
        }
        self.gone_since = None;
        let _ = self.slots.settle(|slot| {
            each(Row::At(slot));
            Ok(())
        });
    }

    pub fn len(&self) -> usize {
        self.slots.len() + self.gone.len()
    }

    pub fn clear(&mut self) {
        self.slots.clear();
        self.gone.clear();
        self.gone_since = None;
    }

    pub fn bytes(&self) -> usize {
        self.slots.bytes() + self.gone.capacity() * 16
    }
}
