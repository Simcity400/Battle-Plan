use super::*;

#[allow(clippy::large_enum_variant)]
pub(crate) enum StatusWork {
    Preparation {
        store: Option<Arc<super::super::attack_stats::ArmyStatStore>>,
        part: super::super::attack_stats::PreparationPart,
    },
    Status {
        prepared: Arc<carmy::StatusPacket>,
        part: carmy::StatusPart,
        attack: super::super::attack_stats::AttackPart,
        naming: NamingPart,
        carry: engine_allocation::SharedBox<NamingCarry>,
        store: Option<Arc<super::super::attack_stats::ArmyStatStore>>,
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
                naming,
                carry,
                store,
            } => {
                part.capture_signaled(
                    memory,
                    context.base,
                    context.game_state,
                    seeds,
                    prepared,
                    &collector_observers::order_membership_changes::Observer,
                );
                carry.stage_attack(attack, seeds, store.as_deref(), prepared);
                attack.capture(memory, context.base);
                naming.capture(
                    memory,
                    context.base,
                    seeds,
                    carry.carry(),
                    store.as_deref(),
                    changes,
                );
            }
        }
    }
    pub(crate) fn fail(&mut self, reason: &'static str) {
        if let Self::Status { part, naming, .. } = self {
            part.fail(reason);
            naming.fail();
        }
    }
}
