use std::sync::atomic::{AtomicU64, AtomicU8, AtomicUsize, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum RelayCustody {
    NeverCreated = 0,
    Retained = 1,
    Live = 2,
}

impl RelayCustody {
    pub(super) const fn as_u8(self) -> u8 {
        self as u8
    }

    pub(super) fn from_u8(raw: u8) -> Self {
        match raw {
            1 => Self::Retained,
            2 => Self::Live,
            _ => Self::NeverCreated,
        }
    }
}

pub(super) struct RelaySlot {
    label: &'static str,
    trampoline_cell: AtomicUsize,
    custody: AtomicU8,
    engine_original_skipped: AtomicU64,
}

impl RelaySlot {
    pub(super) const fn new(label: &'static str) -> Self {
        Self {
            label,
            trampoline_cell: AtomicUsize::new(0),
            custody: AtomicU8::new(RelayCustody::NeverCreated.as_u8()),
            engine_original_skipped: AtomicU64::new(0),
        }
    }

    pub(super) fn custody(&self) -> RelayCustody {
        RelayCustody::from_u8(self.custody.load(Ordering::Acquire))
    }

    pub(super) fn is_live(&self) -> bool {
        self.custody() == RelayCustody::Live
    }

    /// out of the release assembly (`cargo rustc --release -- --emit asm`) for
    ///   `movq` either way -- an acquire load is a plain `mov` on x86-64 and the
    #[inline]
    pub(super) fn original_for_detour(&self) -> Option<usize> {
        match self.trampoline_cell.load(Ordering::Acquire) {
            0 => {
                self.engine_original_skipped.fetch_add(1, Ordering::AcqRel);
                None
            }
            original => Some(original),
        }
    }

    pub(super) fn record_created(&self, trampoline: usize) -> Result<usize, String> {
        if trampoline == 0 {
            return Err(format!(
                "{} hook creation returned a null trampoline",
                self.label
            ));
        }
        match self.trampoline_cell.compare_exchange(
            0,
            trampoline,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {}
            Err(existing) if existing == trampoline => {}
            Err(existing) => {
                return Err(format!(
                    "{} hook creation returned trampoline 0x{trampoline:X} but this process \
                     already recorded 0x{existing:X}",
                    self.label
                ))
            }
        }
        self.custody
            .store(RelayCustody::Live.as_u8(), Ordering::Release);
        Ok(trampoline)
    }

    pub(super) fn adopt_retained(&self) -> Result<usize, String> {
        match self.trampoline_cell.load(Ordering::Acquire) {
            0 => Err(format!(
                "{} hook is already created but this process retained no trampoline for it",
                self.label
            )),
            trampoline => {
                self.custody
                    .store(RelayCustody::Live.as_u8(), Ordering::Release);
                Ok(trampoline)
            }
        }
    }

    pub(super) fn retain_at_teardown(&self) {
        let _ = self.custody.compare_exchange(
            RelayCustody::Live.as_u8(),
            RelayCustody::Retained.as_u8(),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    pub(super) fn label(&self) -> &'static str {
        self.label
    }

    pub(super) fn engine_original_skipped(&self) -> u64 {
        self.engine_original_skipped.load(Ordering::Acquire)
    }

    pub(super) fn recorded_original(&self) -> usize {
        self.trampoline_cell.load(Ordering::Acquire)
    }
}
