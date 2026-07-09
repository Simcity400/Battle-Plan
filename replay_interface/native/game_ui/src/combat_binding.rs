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
//! ## A16 combat-DISPLAY detail — recording-Absent, fail-closed
//!
//! The `landcombatview` chrome also shows a **combat progress** bar
//! (`combat_progress` / `land_battle_progress_frame` / `combat_heading`), a
//! **combat width** value, shore-**bombardment** and **air-superiority**
//! modifiers, and the two participant **leaders**. None of these are carried by
//! `replay_core::CombatRow` (which is scoped to id / kind / location /
//! participant-id lists), so every such field here is fail-closed `None`.
//!
//! This is tracked two ways:
//!   * **EXTRACTION_NEEDS A16** (already OPEN) — the recording does not yet
//!     carry per-combat `progress` / `bombardment_active` /
//!     `air_superiority_state` (land) nor the naval-combat result block. The
//!     C3 combat-indicator law (`COMBAT_INDICATOR_LAW.md`) filed the same A16
//!     dependency for the on-map battle bubbles.
//!   * **replay_core VIEW gap** — even the fixture scaffolding
//!     (`fx_division_movement` emits `combat_width`/`terrain`/`progress` on the
//!     combat object) is deliberately NOT surfaced by `CombatRow`, so no
//!     binding can read it without a `replay_core` change. This lane is
//!     READ-ONLY on `replay_core`; closing the gap is a `replay_core` change,
//!     not a binding change. See `LANE_REPORT.md`.

use replay_core::{CombatRow, Leaf, WorldFrame};

/// One active-combat row bound from a [`CombatRow`]. Identity + participant
/// division-id lists are populated; every A16 display-detail field is
/// fail-closed `None` (see the module header).
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

    // ---- A16 combat-display detail: recording-Absent → always None ----
    /// Combat progress fraction (0..1) for `combat_progress`. A16 — never set.
    pub progress: Option<f64>,
    /// Shore-bombardment active flag. A16 — never set.
    pub bombardment_active: Option<bool>,
    /// Air-superiority modifier state (0..9 strip frame). A16 — never set.
    pub air_superiority_state: Option<i64>,
    /// Combat width value (`combat_width` textbox). A16 — never set.
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
/// is `true` only when at least one active combat exists this hour — i.e. the
/// detail window would open.
///
/// Fail-closed honesty note: `replay_core`'s combats view carries **no**
/// `ViewStatus`, so a domain-absent recording is indistinguishable from "no
/// active combats" — both yield `available: false` and an empty `combats`
/// list. The combat-detail window only appears when a combat is active, so this
/// conflation is benign for this panel.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CombatPanelBinding {
    /// `true` iff at least one active combat row exists this hour.
    pub available: bool,
    /// One row per active combat (empty = no active combats / domain absent).
    pub combats: Vec<CombatPanelRow>,
}

fn combat_row(row: &CombatRow) -> CombatPanelRow {
    CombatPanelRow {
        combat_id: row.combat_id.measured().copied(),
        kind: row.kind.measured().cloned(),
        location_province: row.location_province.measured().copied(),
        attacker_divisions: measured_ids(&row.attackers),
        defender_divisions: measured_ids(&row.defenders),
        // A16 combat-display detail is not surfaced by replay_core (see header).
        progress: None,
        bombardment_active: None,
        air_superiority_state: None,
        combat_width: None,
    }
}

/// Clone a measured participant id list, or `None` (fail-closed) when the leaf
/// was `Absent`/`Unobtained`.
fn measured_ids(leaf: &Leaf<Vec<i64>>) -> Option<Vec<i64>> {
    leaf.measured().cloned()
}

/// Build the read-only combat-detail panel binding for one world frame (U1).
pub fn combat_panel_from_frame(frame: &WorldFrame) -> CombatPanelBinding {
    let combats: Vec<CombatPanelRow> = frame.combats().iter().map(combat_row).collect();
    CombatPanelBinding {
        available: !combats.is_empty(),
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
            kind,
            location_province: loc,
            attackers: att,
            defenders: def,
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
        // A16 display detail is always fail-closed None.
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
