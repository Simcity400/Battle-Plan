use super::*;

#[expect(
    clippy::large_enum_variant,
    reason = "work moves through the executor by value so dispatch never allocates"
)]
pub enum StatusWork {
    Preparation {
        store: Option<Arc<collector_division_parts::attack_stats::ArmyStatStore>>,
        part: collector_division_parts::attack_stats::PreparationPart,
    },
    Status {
        prepared: Arc<carmy::StatusPacket>,
        part: carmy::StatusPart,
        attack: collector_division_parts::attack_stats::AttackPart,
        naming: NamingPart,
        carry: engine_allocation::SharedBox<NamingCarry>,
        store: Option<Arc<collector_division_parts::attack_stats::ArmyStatStore>>,
    },
}
impl StatusWork {
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        seeds: &[DivisionSeed],
        changes: &dyn collector_division_parts::army_stat_changes::Changes,
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
    pub fn fail(&mut self, reason: &'static str) {
        if let Self::Status { part, naming, .. } = self {
            part.fail(reason);
            naming.fail();
        }
    }
}
