//! Route-independent publication of the exact active D49/D50
//! `CFEXMember* -> (combat_id, generation)` graph.
//!
//! This registry carries process-local pointers only. It is published from the
//! same fully validated walk that commits the D49/D50 delta baseline, and no
//! pointer is serialized into either compact domain. Convoy terminal routing
//! remains a separate, stricter publication which still requires its
//! authenticated H-CONVOY route evidence.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use crate::convoy_terminal_attribution::{CombatEpoch, ExactCombatEpochResolver};

pub const MAX_ACTIVE_MEMBER_BINDINGS: usize = 65_536;

static NEXT_PUBLICATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static MEMBER_EPOCH_STATE: OnceLock<RwLock<Arc<MemberEpochSnapshot>>> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemberEpochBinding {
    /// Process-local `CFEXMember*`; never serialized.
    pub member_address: usize,
    pub combat_id: u32,
    pub combat_generation: u64,
    /// Exact D50 `CFEXMember+0x120` save-persisted `unique_id` from the
    /// validated outer member. It is assigned from the combat-owned monotonic
    /// used as identity.
    pub battle_slot_index_raw: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemberEpochResolution {
    pub epoch: CombatEpoch,
    pub battle_slot_index_raw: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemberEpochPublicationReceipt {
    pub publication_sequence: u64,
    pub game_hour_raw: i32,
    pub member_bindings: u32,
}

#[derive(Clone, Debug, Default)]
pub struct MemberEpochSnapshot {
    receipt: MemberEpochPublicationReceipt,
    member_epochs: BTreeMap<usize, MemberEpochResolution>,
}

impl MemberEpochSnapshot {
    pub fn receipt(&self) -> MemberEpochPublicationReceipt {
        self.receipt
    }

    /// Exact route-independent A33 join. The slot is sourced from the outer
    /// D50 member, never from a `CSunkShipInfo` reference handle.
    pub fn resolve_member(
        &self,
        member_address: usize,
    ) -> Result<MemberEpochResolution, String> {
        self.member_epochs
            .get(&member_address)
            .copied()
            .ok_or_else(|| "CFEXMember is absent from the same-hour d49/d50 epoch snapshot".into())
    }
}

impl ExactCombatEpochResolver for MemberEpochSnapshot {
    fn resolve_member_epoch(&self, member_address: usize) -> Result<CombatEpoch, String> {
        Ok(self.resolve_member(member_address)?.epoch)
    }
}

fn state() -> &'static RwLock<Arc<MemberEpochSnapshot>> {
    MEMBER_EPOCH_STATE.get_or_init(|| RwLock::new(Arc::new(MemberEpochSnapshot::default())))
}

/// Shared validation used by both the route-independent A33 registry and the
/// convoy route resolver. It performs no publication and consults no convoy
/// evidence gate.
pub(crate) fn build_member_epoch_map(
    bindings: &[MemberEpochBinding],
) -> Result<BTreeMap<usize, MemberEpochResolution>, String> {
    let mut member_epochs = BTreeMap::new();
    for binding in bindings {
        if binding.member_address == 0
            || binding.combat_id == 0
            || binding.combat_generation == 0
            || binding.battle_slot_index_raw
                > crate::domain49_50_naval_combat_live::MAX_BATTLE_SLOT_INDEX
        {
            return Err("d49/d50 member pointer, combat epoch, or battle slot is invalid".into());
        }
        let epoch = CombatEpoch {
            combat_id: binding.combat_id,
            generation: binding.combat_generation,
        };
        let resolution = MemberEpochResolution {
            epoch,
            battle_slot_index_raw: binding.battle_slot_index_raw,
        };
        if member_epochs
            .insert(binding.member_address, resolution)
            .is_some()
        {
            return Err("CFEXMember pointer appears in more than one active epoch".into());
        }
        if member_epochs.len() > MAX_ACTIVE_MEMBER_BINDINGS {
            return Err("d49/d50 member binding cap exceeded".into());
        }
    }
    Ok(member_epochs)
}

/// Atomically replaces the active member graph after the collector has drained
/// terminal callbacks produced under the preceding graph.
pub fn publish_member_epoch_snapshot(
    bindings: &[MemberEpochBinding],
    game_hour_raw: i32,
) -> Result<MemberEpochPublicationReceipt, String> {
    let member_epochs = build_member_epoch_map(bindings)?;
    let publication_sequence = NEXT_PUBLICATION_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    if publication_sequence == 0 {
        return Err("member epoch publication sequence exhausted".into());
    }
    let receipt = MemberEpochPublicationReceipt {
        publication_sequence,
        game_hour_raw,
        member_bindings: u32::try_from(member_epochs.len())
            .map_err(|_| "member binding count exceeds u32".to_string())?,
    };
    let next = Arc::new(MemberEpochSnapshot {
        receipt,
        member_epochs,
    });
    *state()
        .write()
        .map_err(|_| "member epoch resolver state poisoned".to_string())? = next;
    Ok(receipt)
}

pub fn current_member_epoch_snapshot() -> Result<Arc<MemberEpochSnapshot>, String> {
    state()
        .read()
        .map(|snapshot| Arc::clone(&snapshot))
        .map_err(|_| "member epoch resolver state poisoned".to_string())
}

/// Callback-safe acquisition: contention or poison drops the observation
/// rather than blocking the game thread.
pub fn try_current_member_epoch_snapshot() -> Option<Arc<MemberEpochSnapshot>> {
    state()
        .try_read()
        .ok()
        .map(|snapshot| Arc::clone(&snapshot))
}

pub fn current_member_epoch_publication_receipt() -> Result<MemberEpochPublicationReceipt, String> {
    Ok(current_member_epoch_snapshot()?.receipt())
}

/// Clears all process-local pointers. Call after terminal-hook quiescence on
/// teardown, and whenever the D49/D50 source fails closed for a frame.
pub fn clear_member_epoch_snapshot() -> Result<(), String> {
    *state()
        .write()
        .map_err(|_| "member epoch resolver state poisoned".to_string())? =
        Arc::new(MemberEpochSnapshot::default());
    Ok(())
}
