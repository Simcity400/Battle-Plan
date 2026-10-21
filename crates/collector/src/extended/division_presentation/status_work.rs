//! The same reserved ranges acquire independent preparation leaves first,
//! then finish native status after ordered shared admission has published.
use super::*;

pub(crate) enum StatusWork {
    Preparation {
        store: Option<Arc<super::super::attack_stats::ArmyStatStore>>,
        part: super::super::attack_stats::PreparationPart,
    },
    Status {
        prepared: Arc<carmy::StatusPacket>,
        part: carmy::StatusPart,
        attack: super::super::attack_stats::AttackPart,
    },
}
impl StatusWork {
    pub(crate) fn capture(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        seeds: &[DivisionSeed],
        changes: &dyn crate::army_stat_changes::Changes,
    ) {
        match self {
            Self::Preparation { store, part } => part.capture(
                memory,
                context.base,
                context.game_state,
                seeds,
                store.as_deref(),
                changes,
            ),
            Self::Status {
                prepared,
                part,
                attack,
            } => {
                part.capture(memory, context.base, context.game_state, seeds, prepared);
                attack.capture(memory, context.base);
            }
        }
    }
    pub(crate) fn fail(&mut self, reason: &'static str) {
        if let Self::Status { part, .. } = self {
            part.fail(reason);
        }
    }
}
