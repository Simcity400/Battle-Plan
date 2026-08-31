use std::sync::atomic::{fence, AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CharacterWitness {
    pub generation: u64,
    pub quiet: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum WalkReason {
    None = 0,
    HooksNotInstalled = 1,
    BaselinePending = 2,
    Mutated = 3,
    WriterInFlight = 4,
    SignalChangedDuringRead = 5,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ProvenMutatorSite {
    AdvisorEraseEffect,
    AdvisorReplaceEffect,
    RoleTreeDestructor,
    AdvisorBulkCopy,
    RoleTreeMerge,
    AdvisorInsert,
    CharacterDefinitionBuild,
    CharacterRoleParser,
}

pub struct CharacterChangeGate {
    generation: AtomicU64,
    in_flight: AtomicU64,
    signals: AtomicU64,
    windows: AtomicU64,
    skips: AtomicU64,
}

impl CharacterChangeGate {
    pub const fn new() -> Self {
        Self {
            generation: AtomicU64::new(0),
            in_flight: AtomicU64::new(0),
            signals: AtomicU64::new(0),
            windows: AtomicU64::new(0),
            skips: AtomicU64::new(0),
        }
    }

    #[inline]
    pub fn mark_enter(&self) {
        self.in_flight.fetch_add(1, Ordering::AcqRel);
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.signals.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn mark_proven_mutator_enter(&self, _site: ProvenMutatorSite) {
        self.mark_enter();
    }

    #[inline]
    pub fn mark_exit(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.in_flight.fetch_sub(1, Ordering::AcqRel);
    }

    pub fn witness(&self) -> CharacterWitness {
        for _ in 0..8 {
            fence(Ordering::SeqCst);
            let opening = self.generation.load(Ordering::SeqCst);
            let quiet = self.in_flight.load(Ordering::SeqCst) == 0;
            let closing = self.generation.load(Ordering::SeqCst);
            fence(Ordering::SeqCst);
            if opening == closing {
                return CharacterWitness {
                    generation: opening,
                    quiet,
                };
            }
        }
        CharacterWitness {
            generation: self.generation.load(Ordering::SeqCst),
            quiet: false,
        }
    }

    pub fn open_hour(
        &self,
        hooks_installed: bool,
        baseline: Option<CharacterWitness>,
    ) -> Result<CharacterWitness, WalkReason> {
        self.windows.fetch_add(1, Ordering::Relaxed);
        if !hooks_installed {
            return Err(WalkReason::HooksNotInstalled);
        }
        let Some(baseline) = baseline else {
            return Err(WalkReason::BaselinePending);
        };
        let current = self.witness();
        if !current.quiet {
            return Err(WalkReason::WriterInFlight);
        }
        if !baseline.quiet || current.generation != baseline.generation {
            return Err(WalkReason::Mutated);
        }
        self.skips.fetch_add(1, Ordering::Relaxed);
        Ok(current)
    }

    pub fn still_current(&self, hooks_installed: bool, opening: CharacterWitness) -> bool {
        hooks_installed && opening.quiet && self.witness() == opening
    }

    pub fn read_was_quiet(
        &self,
        hooks_installed: bool,
        before: CharacterWitness,
        after: CharacterWitness,
    ) -> bool {
        hooks_installed && before.quiet && after.quiet && before.generation == after.generation
    }

    pub fn active_writers(&self) -> u64 {
        self.in_flight.load(Ordering::Acquire)
    }

    pub fn census(&self) -> [u64; 3] {
        [
            self.signals.load(Ordering::Relaxed),
            self.windows.load(Ordering::Relaxed),
            self.skips.load(Ordering::Relaxed),
        ]
    }
}

impl Default for CharacterChangeGate {
    fn default() -> Self {
        Self::new()
    }
}

static GATE: CharacterChangeGate = CharacterChangeGate::new();

#[inline]
pub fn mark_enter() {
    GATE.mark_enter();
}

#[inline]
pub fn mark_proven_mutator_enter(_site: ProvenMutatorSite) {
    GATE.mark_proven_mutator_enter(_site);
}

#[inline]
pub fn mark_exit() {
    GATE.mark_exit();
}

pub fn witness() -> CharacterWitness {
    GATE.witness()
}

pub fn open_hour(
    hooks_installed: bool,
    baseline: Option<CharacterWitness>,
) -> Result<CharacterWitness, WalkReason> {
    GATE.open_hour(hooks_installed, baseline)
}

pub fn still_current(hooks_installed: bool, opening: CharacterWitness) -> bool {
    GATE.still_current(hooks_installed, opening)
}

pub fn read_was_quiet(
    hooks_installed: bool,
    before: CharacterWitness,
    after: CharacterWitness,
) -> bool {
    GATE.read_was_quiet(hooks_installed, before, after)
}

pub fn active_writers() -> u64 {
    GATE.active_writers()
}

pub fn gate_census() -> [u64; 3] {
    GATE.census()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_proven_mutator_signal_forces_the_next_read_to_walk() {
        let gate = CharacterChangeGate::new();
        let baseline = gate.witness();
        assert_eq!(gate.open_hour(true, Some(baseline)), Ok(baseline));
        gate.mark_proven_mutator_enter(ProvenMutatorSite::AdvisorReplaceEffect);
        gate.mark_exit();
        assert_eq!(
            gate.open_hour(true, Some(baseline)),
            Err(WalkReason::Mutated)
        );
        let before = gate.witness();
        let after = gate.witness();
        assert!(gate.read_was_quiet(true, before, after));
        assert_eq!(gate.open_hour(true, Some(after)), Ok(after));
    }

    #[test]
    fn an_in_flight_or_uninstalled_tap_never_licenses_a_skip() {
        let gate = CharacterChangeGate::new();
        let baseline = gate.witness();
        assert_eq!(
            gate.open_hour(false, Some(baseline)),
            Err(WalkReason::HooksNotInstalled)
        );
        gate.mark_enter();
        let active = gate.witness();
        assert!(!active.quiet);
        assert_eq!(
            gate.open_hour(true, Some(active)),
            Err(WalkReason::WriterInFlight)
        );
        gate.mark_exit();
    }
}
