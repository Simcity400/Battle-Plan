//! Combat-detail panel population (U1 wave — `interface/landcombat.gui`
//! `landcombatview`, the land-battle detail window).
//!
//! This is a *population* seam identical in spirit to [`crate::binding`]: it
//! maps `replay_core`'s typed [`CombatRow`] view onto a layout-only binding
//! the combat-detail chrome consumes. It parses no recording bytes (Constraint
//! 2) and fabricates no value (Constraint 3): a `Leaf` that is
//! `Absent`/`Unobtained` renders the fail-closed absent form (`None` / empty),
//! never a default like `0`.
//!
//! ## What is POPULATED
//!
//! `replay_core::WorldFrame::combats()` surfaces one [`CombatRow`] per active
//! combat with `combat_id`, `kind`, `location_province`, and the participant
//! division-id lists (`attackers`/`defenders`). Those map onto the panel's
//! identity + the attacker/defender **division counts**
//! (`attacker_committed_label` / `defender_committed_label` in the `.gui`).
//!
//! ## Producer-owned display detail and external residuals
//!
//! `CombatRow` also exposes producer-owned terrain, progress, combat width,
//! phase events, and both exact d55 side-detail rows. Only
//! bombardment/air-superiority selectors, leaders, effective modifier stacks,
//! and terminal result/loss joins remain external and fail closed.

use replay_core::{CombatRow, LandCombatSideDetail, Leaf, ViewStatus, WorldFrame};

/// One active-combat row bound from a [`CombatRow`]. Producer-owned typed
/// values are populated; only genuinely external selectors remain `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CombatPanelRow {
    /// Stable combat id (`combats[i].combat_id`), `None` if unmeasured.
    pub combat_id: Option<i64>,
    /// Combat kind token (`"land"`/`"naval"`/…), `None` if unmeasured.
    pub kind: Option<String>,
    /// Province the combat is fought in, `None` if unmeasured.
    pub location_province: Option<u32>,
    /// Attacker participant division ids (bare `div_id`s), `None` when the
    /// `attackers` leaf was not measured (fail-closed, never an empty-as-zero).
    pub attacker_divisions: Option<Vec<i64>>,
    /// Defender participant division ids, `None` when unmeasured.
    pub defender_divisions: Option<Vec<i64>>,
    pub terrain: Option<String>,
    pub phase_event: Option<String>,
    pub attacker: Option<LandCombatSideDetail>,
    pub defender: Option<LandCombatSideDetail>,

    // ---- Combat-display detail ----
    /// Producer-owned combat progress for `combat_progress`.
    pub progress: Option<f64>,
    /// Shore-bombardment active flag. A16 — never set.
    pub bombardment_active: Option<bool>,
    /// Air-superiority modifier state (0..9 strip frame). A16 — never set.
    pub air_superiority_state: Option<i64>,
    /// Producer-owned combat width value (`combat_width` textbox).
    pub combat_width: Option<f64>,
}

impl CombatPanelRow {
    /// Attacker division count for `attacker_committed_label`, `None` when the
    /// participant list was not measured (fail-closed).
    pub fn attacker_division_count(&self) -> Option<usize> {
        self.attacker_divisions.as_ref().map(Vec::len)
    }

    /// Defender division count for `defender_committed_label`, `None` when the
    /// participant list was not measured (fail-closed).
    pub fn defender_division_count(&self) -> Option<usize> {
        self.defender_divisions.as_ref().map(Vec::len)
    }
}

/// Read-only combat-detail panel binding for one world frame (U1). `available`
/// is `true` only when the combats collection is `Present` AND at least one
/// active combat exists this hour — i.e. the detail window would open.
///
/// Availability is owned by [`WorldFrame::combats_status`]: `Present` with an
/// empty `combats` is the MEASURED "no combats were running", while
/// `DomainAbsent`/`DomainUnavailable` mean the hour carried no readable
/// collection at all — the panel fails closed (no rows, `available: false`)
/// and the status says which fact that is, instead of deriving availability
/// from vector non-emptiness.
#[derive(Clone, Debug, PartialEq)]
pub struct CombatPanelBinding {
    /// `true` iff the collection is `Present` and at least one active combat
    /// row exists this hour.
    pub available: bool,
    /// Availability of the active-combat collection itself
    /// ([`WorldFrame::combats_status`]).
    pub combats_status: ViewStatus,
    /// One row per active combat. Empty under `Present` means measured
    /// no-active-combats; empty under any other status means fail-closed.
    pub combats: Vec<CombatPanelRow>,
}

impl Default for CombatPanelBinding {
    fn default() -> Self {
        CombatPanelBinding {
            available: false,
            combats_status: ViewStatus::DomainAbsent,
            combats: Vec::new(),
        }
    }
}

fn combat_row(row: &CombatRow) -> CombatPanelRow {
    let detail = row.detail.measured();
    CombatPanelRow {
        combat_id: row.combat_id.measured().copied(),
        kind: row.kind.measured().cloned(),
        location_province: row.location_province.measured().copied(),
        attacker_divisions: measured_ids(&row.attackers),
        defender_divisions: measured_ids(&row.defenders),
        terrain: row.terrain.measured().cloned(),
        phase_event: detail.and_then(|value| value.phase_event.measured().cloned()),
        attacker: detail.and_then(|value| value.attacker.measured().cloned()),
        defender: detail.and_then(|value| value.defender.measured().cloned()),
        progress: row.progress.measured().copied(),
        bombardment_active: None,
        air_superiority_state: None,
        combat_width: row.combat_width.measured().copied(),
    }
}

/// Clone a measured participant id list, or `None` (fail-closed) when the leaf
/// was `Absent`/`Unobtained`.
fn measured_ids(leaf: &Leaf<Vec<i64>>) -> Option<Vec<i64>> {
    leaf.measured().cloned()
}

/// Build the read-only combat-detail panel binding for one world frame (U1).
/// Availability comes from [`WorldFrame::combats_status`], never from vector
/// non-emptiness: a non-`Present` collection binds no rows even if the frame
/// retained state.
pub fn combat_panel_from_frame(frame: &WorldFrame) -> CombatPanelBinding {
    let combats_status = frame.combats_status();
    let combats: Vec<CombatPanelRow> = if combats_status == ViewStatus::Present {
        frame.combats().iter().map(combat_row).collect()
    } else {
        Vec::new()
    };
    CombatPanelBinding {
        available: combats_status == ViewStatus::Present && !combats.is_empty(),
        combats_status,
        combats,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        id: Leaf<i64>,
        kind: Leaf<String>,
        loc: Leaf<u32>,
        att: Leaf<Vec<i64>>,
        def: Leaf<Vec<i64>>,
    ) -> CombatRow {
        CombatRow {
            combat_id: id,
            generation: Leaf::Measured(0),
            kind,
            location_province: loc,
            attackers: att,
            defenders: def,
            combat_width: Leaf::Unobtained,
            terrain: Leaf::Unobtained,
            progress: Leaf::Unobtained,
            detail: Leaf::Unobtained,
        }
    }

    #[test]
    fn measured_row_populates_identity_and_counts() {
        let r = combat_row(&row(
            Leaf::Measured(9001),
            Leaf::Measured("land".to_string()),
            Leaf::Measured(2000),
            Leaf::Measured(vec![101, 102]),
            Leaf::Measured(vec![201]),
        ));
        assert_eq!(r.combat_id, Some(9001));
        assert_eq!(r.kind.as_deref(), Some("land"));
        assert_eq!(r.location_province, Some(2000));
        assert_eq!(r.attacker_division_count(), Some(2));
        assert_eq!(r.defender_division_count(), Some(1));
        assert_eq!(r.terrain, None);
        assert_eq!(r.phase_event, None);
        assert_eq!(r.attacker, None);
        assert_eq!(r.defender, None);
        // External selectors remain fail-closed.
        assert_eq!(r.progress, None);
        assert_eq!(r.bombardment_active, None);
        assert_eq!(r.air_superiority_state, None);
        assert_eq!(r.combat_width, None);
    }

    #[test]
    fn absent_participant_list_is_none_not_zero() {
        let r = combat_row(&row(
            Leaf::Absent,
            Leaf::Unobtained,
            Leaf::Absent,
            Leaf::Absent,
            Leaf::Unobtained,
        ));
        assert_eq!(r.combat_id, None);
        assert_eq!(r.kind, None);
        assert_eq!(r.location_province, None);
        assert_eq!(r.attacker_divisions, None);
        assert_eq!(r.attacker_division_count(), None);
        assert_eq!(r.defender_division_count(), None);
    }
}
