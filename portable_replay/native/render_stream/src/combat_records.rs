//! One definition of each combat record the recorder writes and the replay
//! reads: the active land row (`combats[]`), the ended land row
//! (`ended_combats[]`), the D55 detail under both, and the naval row
//! (`naval_combats` object) with its D50 members.
//!
//! # Why one struct, and why here
//!
//! The recorder's fold and the replay's parse used to be two hand-written
//! copies of each shape, and they drifted three times without any guard
//! noticing: the active land row was written without `generation` while the
//! replay read it; the naval POV read `attacker`/`defender` strings the fold
//! never wrote; and each guard was green because each was built from its own
//! side's fixtures. The two trees share no path dependency by design -- the
//! `.battleplan` file is their only runtime interface -- so no test could run
//! one through the other. What they DO share is this crate.
//!
//! So each record is one struct. The fold constructs it and serializes it;
//! the replay deserializes it and reads it. A field one side has and the other
//! does not is now a compile error, a key outside the record is refused on
//! read (`deny_unknown_fields`), and a leaf's three states travel as the same
//! three spellings both ways ([`Leaf`]): an omitted key, a `null`, a value.
//!
//! # The field lists are the structs
//!
//! Every record also states `WIRE_FIELDS`, the keys it can carry, generated
//! from the same tokens as the struct so it cannot be a second copy. Tests on
//! both sides use it to say "this hour carried every key" as a checked count
//! rather than a reviewed claim.

use crate::leaf::{nullable, Leaf};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

/// The root keys the three collections travel under.
pub const LAND_COMBATS_WIRE_KEY: &str = "combats";
pub const ENDED_LAND_COMBATS_WIRE_KEY: &str = "ended_combats";
pub const NAVAL_COMBATS_WIRE_KEY: &str = "naval_combats";

/// The `kind` every land row states, and the one every naval row states.
pub const LAND_COMBAT_KIND: &str = "land";
pub const NAVAL_COMBAT_KIND: &str = "naval";
/// The `lifecycle` every ended row states.
pub const ENDED_LIFECYCLE: &str = "ended";
/// The two `side` spellings of a naval member and a land participant event.
pub const ATTACKER_SIDE: &str = "attacker";
pub const DEFENDER_SIDE: &str = "defender";

/// The one spelling of a naval combat's key in the `naval_combats` object.
///
/// `serde_json::Map` is a `BTreeMap`, so the replay iterates the object in
/// LEXICOGRAPHIC key order while the fold inserts in numeric order; consumers
/// join by identity for that reason, and the reader checks that each key IS
/// its row's identity through this function.
pub fn naval_combat_key(combat_id: i64, generation: i64) -> String {
    format!("{combat_id}:{generation}")
}

/// The `naval_combats` object as the wire carries it: every record under the
/// [`naval_combat_key`] minted from its OWN measured identity.
///
/// This is the writer's half of the rule [`NavalCombatRecord::validate_wire_entry`]
/// checks on the reader's, so a key and its row cannot disagree by
/// construction. A record without a measured identity has no key and is
/// refused rather than filed under a spelling the reader would reject; so is
/// a record whose two id spellings disagree, and a second record claiming an
/// identity already filed.
pub fn naval_combats_wire_object<'a>(
    rows: impl IntoIterator<Item = &'a NavalCombatRecord>,
) -> Result<Map<String, Value>, String> {
    let mut object = Map::new();
    for row in rows {
        let (Leaf::Measured(combat_id), Leaf::Measured(generation)) =
            (&row.combat_id, &row.generation)
        else {
            return Err("a naval combat record without a measured identity has no wire key".into());
        };
        let key = naval_combat_key(*combat_id, *generation);
        row.validate_wire_entry(&key)
            .map_err(|error| format!("naval combat {key} is not admissible: {error}"))?;
        let value = serde_json::to_value(row).map_err(|error| format!("{key}: {error}"))?;
        if object.insert(key.clone(), value).is_some() {
            return Err(format!("naval combat {key} is stated twice"));
        }
    }
    Ok(object)
}

/// A seam the recorder spells on the wire and has never filled.
///
/// An uninhabited type: a leaf of it can be `Absent` or `Unobtained` and
/// nothing else, so the fold can write the `null` a consumer reads as
/// `DomainUnavailable` ("this producer has no usable view here") without a
/// shape being declared for a value nobody has measured. A producer that fills
/// the seam replaces this with the shape it fills, which is the moment the
/// shape gets declared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnfilledSeam {}

macro_rules! wire_name {
    ($field:ident) => {
        stringify!($field)
    };
    ($field:ident, $wire:literal) => {
        $wire
    };
}

/// A record whose every field is a [`Leaf`], with the wire form of all three
/// states written on every field once, here, rather than per field by hand.
///
/// `pub name @ "wire_key": Leaf<T>` declares a field whose Rust name and wire
/// key differ.
macro_rules! wire_record {
    (
        $(#[$meta:meta])*
        pub struct $name:ident {
            $(
                $(#[$fmeta:meta])*
                pub $field:ident $(@ $wire:literal)? : $ty:ty
            ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            $(
                $(#[$fmeta])*
                $(#[serde(rename = $wire)])?
                #[serde(default, skip_serializing_if = "Leaf::is_absent")]
                pub $field: $ty,
            )*
        }

        impl $name {
            /// The keys this record can carry on the wire, in declaration
            /// order. Generated from the same tokens as the fields.
            pub const WIRE_FIELDS: &'static [&'static str] =
                &[$(wire_name!($field $(, $wire)?)),*];

            /// The same keys with each field's TYPE beside it, as the compiler
            /// tokenised it. `stringify!` normalises the tokens, so a type
            /// written across three lines and one written on a single line
            /// produce the same string -- which is why the replay reader's
            /// carrier-census scan reads this instead of the source text: a
            /// declaration cannot be SPELLED past the check when the spelling
            /// never reaches it (Codex rounds 5 and 6 of the XC landing
            /// review, where an escaped key, a field split across lines and a
            /// type split across lines each defeated a line-oriented read).
            pub const WIRE_FIELD_TYPES: &'static [(&'static str, &'static str)] =
                &[$((wire_name!($field $(, $wire)?), stringify!($ty))),*];

            /// The record that says nothing: every leaf `Absent`, which is
            /// what `{}` reads as and what serializes to `{}`. Spelled as a
            /// constructor rather than `Default` so a writer cannot fill a
            /// record with `..Default::default()` and leave a field unstated
            /// by accident.
            pub fn all_absent() -> Self {
                Self { $($field: Leaf::Absent,)* }
            }
        }
    };
}

wire_record! {
    /// One ACTIVE land combat, an element of `combats[]`.
    ///
    /// The identity is `(combat_id, generation)`, and both halves are stated
    /// by the fold from D21. `combat_width`, `terrain`, `progress` and
    /// `modifiers` are seams the recorder spells as `null` on every row: no
    /// domain measures them, and an explicit `null` reads as
    /// `DomainUnavailable` where an omitted key would read as "never heard of
    /// it". `detail` is the D55 enrichment for this hour, carried only when
    /// the hour has one.
    pub struct LandCombatRecord {
        pub combat_id: Leaf<i64>,
        pub generation: Leaf<i64>,
        pub kind: Leaf<String>,
        pub location_province: Leaf<u32>,
        pub attackers: Leaf<Vec<i64>>,
        pub defenders: Leaf<Vec<i64>>,
        /// XMAP14-PLACE-1's operand: the attacker whose `CUnit+0x298` map
        /// entity the engine anchors the icon on. `0` = measured, none live.
        pub first_live_attacker: Leaf<i64>,
        pub combat_width: Leaf<f64>,
        pub terrain: Leaf<String>,
        pub progress: Leaf<f64>,
        pub modifiers: Leaf<UnfilledSeam>,
        pub detail @ "land_combat_detail_events_live": Leaf<LandCombatDetail>,
    }
}

wire_record! {
    /// One ENDED land combat, an element of `ended_combats[]`: a D55 terminal
    /// frame whose combat D21 no longer lists as active.
    ///
    /// A separate collection from `combats[]` so a consumer never mistakes an
    /// ended battle for a live one. `attackers` and `defenders` are `null`
    /// here: the combat is gone from the active set, so the D21 participants
    /// are unknown rather than empty. `side_detail_availability` says whether
    /// the per-side D55 detail survived into this hour (`present`) or only the
    /// terminal phase row did (`unavailable`).
    pub struct EndedLandCombatRecord {
        pub combat_id: Leaf<i64>,
        pub generation: Leaf<i64>,
        pub kind: Leaf<String>,
        pub lifecycle: Leaf<String>,
        pub location_province: Leaf<u32>,
        pub attackers: Leaf<Vec<i64>>,
        pub defenders: Leaf<Vec<i64>>,
        pub combat_width: Leaf<f64>,
        pub terrain: Leaf<String>,
        pub progress: Leaf<f64>,
        pub modifiers: Leaf<UnfilledSeam>,
        pub side_detail_availability: Leaf<String>,
        pub detail @ "land_combat_detail_events_live": Leaf<LandCombatDetail>,
    }
}

wire_record! {
    /// The D55 enrichment of one land combat for one hour.
    ///
    /// `terminal_record_status` is `exact` beside a terminal record,
    /// `unavailable` on an ended phase without one, and absent otherwise.
    /// `side_detail_completeness` says why a side is or is not here: `both`,
    /// `both_counts_incomplete`, or the one side that was read and therefore
    /// SUPPRESSED (`attacker_only_suppressed` / `defender_only_suppressed`),
    /// because per-side detail is all-or-nothing per combat. A suppressed side
    /// may still carry a `damage_attribution`-only object.
    pub struct LandCombatDetail {
        pub phase_event: Leaf<String>,
        pub generation: Leaf<i64>,
        pub location_province: Leaf<u32>,
        pub age_days: Leaf<u32>,
        pub duration_days: Leaf<u32>,
        pub terminal_record: Leaf<LandCombatTerminalRecord>,
        pub terminal_record_status: Leaf<String>,
        pub participant_events: Leaf<Vec<CombatParticipantEvent>>,
        pub side_detail_completeness: Leaf<String>,
        pub attacker: Leaf<LandCombatSideDetail>,
        pub defender: Leaf<LandCombatSideDetail>,
    }
}

wire_record! {
    /// The engine's terminal combat record, as the D55 writer hook saw it.
    pub struct LandCombatTerminalRecord {
        pub record_index: Leaf<i64>,
        pub writer: Leaf<String>,
        pub attacker_manpower_loss_raw: Leaf<i64>,
        pub defender_manpower_loss_raw: Leaf<i64>,
        pub end_date_raw: Leaf<i64>,
        pub location_province: Leaf<u32>,
        pub winner_side_raw: Leaf<i64>,
        pub winter: Leaf<bool>,
        pub result_selector_raw: Leaf<i64>,
        pub layout_side_raw: Leaf<i64>,
        pub source: Leaf<String>,
    }
}

wire_record! {
    /// One D55 participant event. `semantic` is stated only on a
    /// `participant_left_unclassified` event.
    pub struct CombatParticipantEvent {
        pub event: Leaf<String>,
        pub generation: Leaf<i64>,
        pub side: Leaf<String>,
        pub division_id: Leaf<i64>,
        pub old_side_count: Leaf<i64>,
        pub new_side_count: Leaf<i64>,
        pub semantic: Leaf<String>,
    }
}

wire_record! {
    /// One side's D55 current-state detail.
    ///
    /// The seven `*_raw` scalars are the fixed-point words the scaled values
    /// beside them are divided from. `tactic` and `leader_id` are nullable
    /// leaves: their `null` is the engine's own "none", and the `*_status`
    /// word beside each says whether it was read at all.
    pub struct LandCombatSideDetail {
        pub units: Leaf<i64>,
        pub committed: Leaf<i64>,
        pub reserve: Leaf<i64>,
        pub air_kills: Leaf<i64>,
        pub org_loss_summary_index: Leaf<i64>,
        pub num_org_losses: Leaf<i64>,
        pub anti_air_attack: Leaf<f64>,
        pub air_damage_str: Leaf<f64>,
        pub air_damage_org: Leaf<f64>,
        pub ground_damage_str: Leaf<f64>,
        pub ground_damage_org: Leaf<f64>,
        pub prevented_damage_str: Leaf<f64>,
        pub prevented_damage_org: Leaf<f64>,
        pub anti_air_attack_raw: Leaf<i64>,
        pub air_damage_str_raw: Leaf<i64>,
        pub air_damage_org_raw: Leaf<i64>,
        pub ground_damage_str_raw: Leaf<i64>,
        pub ground_damage_org_raw: Leaf<i64>,
        pub prevented_damage_str_raw: Leaf<i64>,
        pub prevented_damage_org_raw: Leaf<i64>,
        pub progress: Leaf<f64>,
        pub progress_raw: Leaf<i64>,
        #[serde(with = "nullable")]
        pub tactic: Leaf<Option<LandCombatTacticIdentity>>,
        pub tactic_status: Leaf<String>,
        pub losses_status: Leaf<String>,
        pub width_status: Leaf<String>,
        pub width_observations: Leaf<Vec<LandCombatWidthObservation>>,
        pub current_loss_observations: Leaf<Vec<LandCombatCurrentLossObservation>>,
        #[serde(with = "nullable")]
        pub leader_id: Leaf<Option<i64>>,
        pub leader_status: Leaf<String>,
        pub effective_displayed_modifiers: Leaf<Vec<String>>,
        pub effective_displayed_modifiers_status: Leaf<String>,
        pub panel_capture_sequence: Leaf<u64>,
        pub panel_observation_source: Leaf<String>,
        pub counts_incomplete: Leaf<bool>,
        pub reserve_division_ids: Leaf<Vec<i64>>,
        pub reserve_members_status: Leaf<String>,
        pub damage_attribution: Leaf<Vec<LandDamageAttributionRow>>,
    }
}

wire_record! {
    pub struct LandCombatTacticIdentity {
        pub id: Leaf<i64>,
        pub token: Leaf<String>,
        pub name: Leaf<String>,
    }
}

wire_record! {
    pub struct LandCombatWidthObservation {
        pub capture_sequence: Leaf<u64>,
        pub final_width_raw: Leaf<i64>,
        pub wrappers: Leaf<Vec<LandCombatWidthWrapper>>,
        pub source: Leaf<String>,
    }
}

wire_record! {
    pub struct LandCombatWidthWrapper {
        pub ordinal: Leaf<i64>,
        pub division_id: Leaf<i64>,
        pub width_raw: Leaf<i64>,
    }
}

wire_record! {
    pub struct LandCombatCurrentLossObservation {
        pub capture_sequence: Leaf<u64>,
        pub manpower_before_raw: Leaf<i64>,
        pub manpower_after_raw: Leaf<i64>,
        pub equipment: Leaf<Vec<LandCombatEquipmentLossDelta>>,
        pub source: Leaf<String>,
    }
}

wire_record! {
    pub struct LandCombatEquipmentLossDelta {
        pub ordinal: Leaf<i64>,
        pub equipment_id: Leaf<i64>,
        pub before_raw: Leaf<i64>,
        pub after_raw: Leaf<i64>,
    }
}

wire_record! {
    pub struct LandDamageAttributionRow {
        pub dealer_tag_db_id: Leaf<i64>,
        pub victim_tag_db_id: Leaf<i64>,
        pub damage: Leaf<f64>,
        pub damage_raw: Leaf<u64>,
    }
}

wire_record! {
    /// One active naval combat, the value under its [`naval_combat_key`] in
    /// the `naval_combats` object.
    ///
    /// `naval_combat_id` restates `combat_id`; the reader refuses a row whose
    /// two spellings disagree. The four task-force vectors and `members` are
    /// stated together, and only on an hour whose D50 member census was
    /// known: the two `*_ids` / `*_id_spaces` pairs are POSITIONALLY PAIRED
    /// references, because two clausewitz registries mint task forces whose
    /// numbers collide.
    ///
    /// The per-side OWNER sets XMAP14 attributes a viewer with are not on the
    /// wire: they are derived from `members[]` by
    /// [`NavalCombatRecord::side_owner_country_ids`], on both sides of the
    /// file, from this one definition.
    pub struct NavalCombatRecord {
        pub combat_id: Leaf<i64>,
        pub naval_combat_id: Leaf<i64>,
        pub generation: Leaf<i64>,
        pub kind: Leaf<String>,
        pub location_province: Leaf<u32>,
        pub region_id: Leaf<u32>,
        pub attacker_member_count: Leaf<u32>,
        pub defender_member_count: Leaf<u32>,
        pub attacker_positioning_raw: Leaf<i64>,
        pub attacker_positioning: Leaf<f64>,
        pub defender_positioning_raw: Leaf<i64>,
        pub defender_positioning: Leaf<f64>,
        pub attacker_anti_air_raw: Leaf<i64>,
        pub attacker_anti_air: Leaf<f64>,
        pub defender_anti_air_raw: Leaf<i64>,
        pub defender_anti_air: Leaf<f64>,
        pub attacker_task_force_ids: Leaf<Vec<i64>>,
        pub attacker_task_force_id_spaces: Leaf<Vec<i64>>,
        pub defender_task_force_ids: Leaf<Vec<i64>>,
        pub defender_task_force_id_spaces: Leaf<Vec<i64>>,
        pub members: Leaf<Vec<NavalCombatMemberRow>>,
    }
}

wire_record! {
    /// One D50 member of a naval combat.
    ///
    /// A convoy member carries no CShip-derived field -- no `task_force_id`,
    /// no `ship_instance_id` -- and DOES carry `owner_tag_db_id`. `state_raw`
    /// restates `state_bitmask`.
    pub struct NavalCombatMemberRow {
        pub side: Leaf<String>,
        pub group_index: Leaf<i64>,
        pub member_index: Leaf<i64>,
        pub is_convoy: Leaf<bool>,
        pub ship_instance_id: Leaf<i64>,
        pub cship_join_validated: Leaf<bool>,
        pub owner_tag_db_id: Leaf<i64>,
        pub task_force_id: Leaf<i64>,
        pub task_force_id_space: Leaf<i64>,
        pub fleet_id: Leaf<i64>,
        pub battle_slot_index: Leaf<i64>,
        pub state_bitmask: Leaf<i64>,
        pub state_raw: Leaf<i64>,
        pub state_name: Leaf<String>,
        pub strength: Leaf<f64>,
        pub organisation: Leaf<f64>,
        pub sunk_convoys: Leaf<i64>,
    }
}

/// A measured, non-negative generation: the wire spells it signed, the engine
/// counts from zero, and a row without one has no identity.
fn measured_generation(generation: &Leaf<i64>) -> Result<i64, String> {
    match generation {
        Leaf::Measured(generation) if *generation >= 0 => Ok(*generation),
        Leaf::Measured(generation) => Err(format!("generation {generation} is negative")),
        _ => Err("generation is not measured".into()),
    }
}

/// A measured, positive engine id: the fold refuses a zero combat id at the
/// domain boundary, so one here is a row no recorder produced.
fn measured_combat_id(combat_id: &Leaf<i64>) -> Result<i64, String> {
    match combat_id {
        Leaf::Measured(combat_id) if *combat_id > 0 => Ok(*combat_id),
        Leaf::Measured(combat_id) => {
            Err(format!("combat_id {combat_id} is not a positive engine id"))
        }
        _ => Err("combat_id is not measured".into()),
    }
}

fn measured_word(leaf: &Leaf<String>, field: &str, expected: &str) -> Result<(), String> {
    match leaf {
        Leaf::Measured(word) if word == expected => Ok(()),
        Leaf::Measured(word) => Err(format!("{field} is {word:?}, not exactly {expected:?}")),
        _ => Err(format!("{field} is not measured")),
    }
}

impl LandCombatRecord {
    /// What an ACTIVE land row has to state before it is one member of the
    /// hour's census: a measured identity pair and the land kind. A row that
    /// fails is refused by name at the reader, so the census it belongs to is
    /// unavailable rather than a row the XMAP14 join cannot identify -- which
    /// was a plain error out of `prepare` and cost the whole frame.
    pub fn validate_active(&self) -> Result<(i64, i64), String> {
        let combat_id = measured_combat_id(&self.combat_id)?;
        let generation = measured_generation(&self.generation)?;
        measured_word(&self.kind, "kind", LAND_COMBAT_KIND)?;
        Ok((combat_id, generation))
    }
}

impl EndedLandCombatRecord {
    /// What an ENDED row has to state: a positive identity pair -- the
    /// terminal record's own rule, older than this crate -- and the ended
    /// lifecycle, so a row nothing has ended is never exposed as terminal.
    pub fn validate_ended(&self) -> Result<(i64, i64), String> {
        let Leaf::Measured(combat_id) = self.combat_id else {
            return Err("missing or non-integer combat_id".into());
        };
        let Leaf::Measured(generation) = self.generation else {
            return Err("missing or non-integer generation".into());
        };
        if combat_id <= 0 || generation <= 0 {
            return Err("non-positive terminal identity".into());
        }
        if self.lifecycle.measured().map(String::as_str) != Some(ENDED_LIFECYCLE) {
            return Err("lifecycle is not exactly 'ended'".into());
        }
        Ok((combat_id, generation))
    }
}

impl NavalCombatRecord {
    /// The `owner_tag_db_id`s D50 recorded for the members of one side,
    /// sorted and deduplicated -- exactly, or not at all.
    ///
    /// `Absent` / `Unobtained` follow `members`. A member whose side is
    /// unmeasured or unrecognised makes BOTH sides unobtained, because it
    /// cannot be placed on either; a member on this side whose owner is
    /// unmeasured or zero makes this side unobtained. A side with no member
    /// at all is `Measured` EMPTY: that is D50 recording no member there,
    /// which a consumer that needs a country to attribute treats as unstated.
    ///
    /// This is the one derivation. The XMAP14 producer builds its carrier's
    /// owner sets from it over the very record it is about to write, and the
    /// replay's join re-derives them from the record it read, so the two can
    /// only disagree if the file did.
    pub fn side_owner_country_ids(&self, side: &str) -> Leaf<Vec<i64>> {
        let rows = match &self.members {
            Leaf::Absent => return Leaf::Absent,
            Leaf::Unobtained => return Leaf::Unobtained,
            Leaf::Measured(rows) => rows,
        };
        let mut owners = BTreeSet::new();
        for row in rows {
            let member_side = match &row.side {
                Leaf::Measured(member_side)
                    if member_side == ATTACKER_SIDE || member_side == DEFENDER_SIDE =>
                {
                    member_side.as_str()
                }
                _ => return Leaf::Unobtained,
            };
            if member_side != side {
                continue;
            }
            match &row.owner_tag_db_id {
                Leaf::Measured(owner) if *owner > 0 => {
                    owners.insert(*owner);
                }
                _ => return Leaf::Unobtained,
            }
        }
        Leaf::Measured(owners.into_iter().collect())
    }

    pub fn attacker_country_tag_db_ids(&self) -> Leaf<Vec<i64>> {
        self.side_owner_country_ids(ATTACKER_SIDE)
    }

    pub fn defender_country_tag_db_ids(&self) -> Leaf<Vec<i64>> {
        self.side_owner_country_ids(DEFENDER_SIDE)
    }

    /// One side's task-force references as `(id_space, task_force_id)` pairs,
    /// read off the two positionally paired vectors the fold emits.
    ///
    /// `Unobtained` when either vector is unmeasured or the two differ in
    /// length: half a reference is not a reference.
    pub fn side_task_force_references(&self, side: &str) -> Leaf<Vec<(i64, i64)>> {
        let (ids, spaces) = match side {
            ATTACKER_SIDE => (
                &self.attacker_task_force_ids,
                &self.attacker_task_force_id_spaces,
            ),
            DEFENDER_SIDE => (
                &self.defender_task_force_ids,
                &self.defender_task_force_id_spaces,
            ),
            _ => return Leaf::Unobtained,
        };
        match (ids, spaces) {
            (Leaf::Absent, Leaf::Absent) => Leaf::Absent,
            (Leaf::Measured(ids), Leaf::Measured(spaces)) if ids.len() == spaces.len() => {
                Leaf::Measured(spaces.iter().copied().zip(ids.iter().copied()).collect())
            }
            _ => Leaf::Unobtained,
        }
    }

    /// The checks a row under `key` in the `naval_combats` object has to pass
    /// before it is one naval combat: a measured identity pair and the naval
    /// kind, its key spelling that identity, and its two id spellings
    /// agreeing. The writer applies it before filing a row and the reader
    /// before admitting one, so an entry the census cannot identify is refused
    /// by name on both sides rather than joined against later.
    pub fn validate_wire_entry(&self, key: &str) -> Result<(i64, i64), String> {
        let combat_id = measured_combat_id(&self.combat_id)?;
        let generation = measured_generation(&self.generation)?;
        measured_word(&self.kind, "kind", NAVAL_COMBAT_KIND)?;
        if let Leaf::Measured(naval_combat_id) = &self.naval_combat_id {
            if *naval_combat_id != combat_id {
                return Err(format!(
                    "combat_id {combat_id} and naval_combat_id {naval_combat_id} disagree"
                ));
            }
        }
        let expected = naval_combat_key(combat_id, generation);
        if key != expected {
            return Err(format!(
                "key {key:?} does not spell the row's identity {expected:?}"
            ));
        }
        Ok((combat_id, generation))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Every record type, driven through the same three checks by name.
    macro_rules! for_each_record {
        ($f:ident) => {
            $f::<LandCombatRecord>("LandCombatRecord", LandCombatRecord::WIRE_FIELDS);
            $f::<EndedLandCombatRecord>(
                "EndedLandCombatRecord",
                EndedLandCombatRecord::WIRE_FIELDS,
            );
            $f::<LandCombatDetail>("LandCombatDetail", LandCombatDetail::WIRE_FIELDS);
            $f::<LandCombatTerminalRecord>(
                "LandCombatTerminalRecord",
                LandCombatTerminalRecord::WIRE_FIELDS,
            );
            $f::<CombatParticipantEvent>(
                "CombatParticipantEvent",
                CombatParticipantEvent::WIRE_FIELDS,
            );
            $f::<LandCombatSideDetail>("LandCombatSideDetail", LandCombatSideDetail::WIRE_FIELDS);
            $f::<LandCombatTacticIdentity>(
                "LandCombatTacticIdentity",
                LandCombatTacticIdentity::WIRE_FIELDS,
            );
            $f::<LandCombatWidthObservation>(
                "LandCombatWidthObservation",
                LandCombatWidthObservation::WIRE_FIELDS,
            );
            $f::<LandCombatWidthWrapper>(
                "LandCombatWidthWrapper",
                LandCombatWidthWrapper::WIRE_FIELDS,
            );
            $f::<LandCombatCurrentLossObservation>(
                "LandCombatCurrentLossObservation",
                LandCombatCurrentLossObservation::WIRE_FIELDS,
            );
            $f::<LandCombatEquipmentLossDelta>(
                "LandCombatEquipmentLossDelta",
                LandCombatEquipmentLossDelta::WIRE_FIELDS,
            );
            $f::<LandDamageAttributionRow>(
                "LandDamageAttributionRow",
                LandDamageAttributionRow::WIRE_FIELDS,
            );
            $f::<NavalCombatRecord>("NavalCombatRecord", NavalCombatRecord::WIRE_FIELDS);
            $f::<NavalCombatMemberRow>("NavalCombatMemberRow", NavalCombatMemberRow::WIRE_FIELDS);
        };
    }

    fn all_null(fields: &[&str]) -> Value {
        Value::Object(
            fields
                .iter()
                .map(|field| (field.to_string(), Value::Null))
                .collect::<Map<_, _>>(),
        )
    }

    /// kills: a field declared without the wire form (an Absent that fails to
    /// serialize, a missing key that fails to read), and a `WIRE_FIELDS` entry
    /// that is not a key the record accepts.
    fn absent_and_null_round_trip<T>(name: &str, fields: &[&str])
    where
        T: Serialize + for<'de> Deserialize<'de> + std::fmt::Debug,
    {
        let empty: T = serde_json::from_value(json!({}))
            .unwrap_or_else(|error| panic!("{name}: {{}} must read as all-Absent: {error}"));
        assert_eq!(
            serde_json::to_value(&empty).unwrap(),
            json!({}),
            "{name}: all-Absent must serialize to an empty object"
        );
        let nulls = all_null(fields);
        let unobtained: T = serde_json::from_value(nulls.clone())
            .unwrap_or_else(|error| panic!("{name}: every wire field must accept null: {error}"));
        assert_eq!(
            serde_json::to_value(&unobtained).unwrap(),
            nulls,
            "{name}: a null on every field must come back as a null on every field"
        );
        assert!(!fields.is_empty(), "{name}: no wire fields");
        let mut seen = BTreeSet::new();
        for field in fields {
            assert!(seen.insert(*field), "{name}: wire field {field} is declared twice");
        }
    }

    #[test]
    fn every_record_reads_and_writes_all_three_leaf_states() {
        for_each_record!(absent_and_null_round_trip);
    }

    /// kills: a reader that walks past a key the record does not declare --
    /// the shape the naval POV's `attacker`/`defender` read had, and the shape
    /// an operand outside a closed vocabulary has.
    fn a_key_outside_the_record_is_refused<T>(name: &str, fields: &[&str])
    where
        T: Serialize + for<'de> Deserialize<'de> + std::fmt::Debug,
    {
        let mut object = all_null(fields);
        object
            .as_object_mut()
            .unwrap()
            .insert("result_type".into(), json!("naval_battle"));
        let error = serde_json::from_value::<T>(object)
            .err()
            .unwrap_or_else(|| panic!("{name}: an undeclared key must be refused"));
        assert!(error.to_string().contains("result_type"), "{name}: {error}");
    }

    #[test]
    fn a_key_no_record_declares_is_refused_on_read() {
        for_each_record!(a_key_outside_the_record_is_refused);
    }

    /// kills: the second of the three shape mismatches by name. The naval POV
    /// read `attacker` and `defender` country strings off the typed row, and
    /// an earlier reader read `result_type` and `progress`; `build_naval_combats`
    /// never wrote any of the four. They are not fields of the record, so a
    /// reader cannot spell them, and a writer that put them on the wire is
    /// refused rather than read around.
    #[test]
    fn the_keys_the_naval_pov_once_read_are_not_on_the_record_and_are_refused() {
        for key in ["attacker", "defender", "result_type", "progress"] {
            assert!(
                !NavalCombatRecord::WIRE_FIELDS.contains(&key),
                "{key} is a key the recorder never wrote and the record must not declare"
            );
            let error = serde_json::from_value::<NavalCombatRecord>(json!({
                "combat_id": 9, "generation": 2, key: "ENG"
            }))
            .expect_err("a key outside the naval record is refused");
            assert!(error.to_string().contains(key), "{error}");
        }
    }

    /// kills: the first of the three shape mismatches by name. The identity is
    /// `(combat_id, generation)` and the active land row went out without its
    /// second half; the record declares it, the fold cannot construct a row
    /// without it, and a document that omits it reads as `Absent` for a
    /// consumer to refuse by name rather than as a row with an identity.
    #[test]
    fn the_active_land_record_spells_both_halves_of_its_identity() {
        assert!(LandCombatRecord::WIRE_FIELDS.contains(&"combat_id"));
        assert!(LandCombatRecord::WIRE_FIELDS.contains(&"generation"));
        let row: LandCombatRecord = serde_json::from_value(json!({"combat_id": 7})).unwrap();
        assert_eq!(row.combat_id, Leaf::Measured(7));
        assert_eq!(row.generation, Leaf::Absent);
        let written = serde_json::to_value(LandCombatRecord {
            combat_id: Leaf::Measured(7),
            generation: Leaf::Measured(3),
            ..LandCombatRecord::all_absent()
        })
        .unwrap();
        assert_eq!(written, json!({"combat_id": 7, "generation": 3}));
    }

    #[test]
    fn a_wrong_typed_leaf_is_refused_with_the_value_named() {
        let error = serde_json::from_value::<LandCombatRecord>(json!({
            "combat_id": 900, "attackers": [6, "bad"]
        }))
        .unwrap_err();
        assert!(error.to_string().contains("bad"), "{error}");
        let error = serde_json::from_value::<LandCombatRecord>(json!({"location_province": -1}))
            .unwrap_err();
        assert!(error.to_string().contains("-1"), "{error}");
    }

    #[test]
    fn the_detail_key_is_the_long_spelling_and_the_field_is_the_short_one() {
        let row: LandCombatRecord = serde_json::from_value(json!({
            "combat_id": 7, "generation": 3,
            "land_combat_detail_events_live": {"phase_event": "updated", "generation": 3}
        }))
        .unwrap();
        let detail = row.detail.measured().expect("the detail is read under its wire key");
        assert_eq!(detail.phase_event, Leaf::Measured("updated".into()));
        assert!(LandCombatRecord::WIRE_FIELDS.contains(&"land_combat_detail_events_live"));
        assert!(!LandCombatRecord::WIRE_FIELDS.contains(&"detail"));
        assert_eq!(
            serde_json::to_value(&row).unwrap(),
            json!({
                "combat_id": 7, "generation": 3,
                "land_combat_detail_events_live": {"phase_event": "updated", "generation": 3}
            })
        );
    }

    #[test]
    fn the_unfilled_seam_has_two_states_and_no_value() {
        let row: LandCombatRecord = serde_json::from_value(json!({"modifiers": null})).unwrap();
        assert_eq!(row.modifiers, Leaf::Unobtained);
        for filled in [json!([]), json!({}), json!("x"), json!(1)] {
            serde_json::from_value::<LandCombatRecord>(json!({"modifiers": filled}))
                .expect_err("the seam has no filled form");
        }
    }

    #[test]
    fn nullable_detail_leaves_read_null_as_measured_none() {
        let side: LandCombatSideDetail =
            serde_json::from_value(json!({"tactic": null, "leader_id": null})).unwrap();
        assert_eq!(side.tactic, Leaf::Measured(None));
        assert_eq!(side.leader_id, Leaf::Measured(None));
        let side: LandCombatSideDetail = serde_json::from_value(json!({
            "tactic": {"id": 7, "token": "tactic_basic_attack", "name": "TACTIC_BASIC_ATTACK"},
            "leader_id": 41
        }))
        .unwrap();
        assert_eq!(
            side.tactic.measured().unwrap().as_ref().unwrap().token,
            Leaf::Measured("tactic_basic_attack".into())
        );
        assert_eq!(side.leader_id, Leaf::Measured(Some(41)));
    }

    fn member(side: Value, owner: Value) -> Value {
        json!({"side": side, "owner_tag_db_id": owner})
    }

    fn naval(members: Value) -> NavalCombatRecord {
        serde_json::from_value(json!({"combat_id": 9, "generation": 2, "members": members}))
            .unwrap()
    }

    /// kills: an owner set a carrier could state that the members do not.
    #[test]
    fn naval_owner_sets_are_derived_from_members_exactly_or_not_at_all() {
        let row = naval(json!([
            member(json!("attacker"), json!(3)),
            member(json!("attacker"), json!(1)),
            member(json!("attacker"), json!(3)),
            member(json!("defender"), json!(2)),
        ]));
        assert_eq!(row.attacker_country_tag_db_ids(), Leaf::Measured(vec![1, 3]));
        assert_eq!(row.defender_country_tag_db_ids(), Leaf::Measured(vec![2]));

        // A member with no owner costs ITS side and only its side.
        let row = naval(json!([
            member(json!("attacker"), json!(1)),
            member(json!("attacker"), Value::Null),
            member(json!("defender"), json!(2)),
        ]));
        assert_eq!(row.attacker_country_tag_db_ids(), Leaf::Unobtained);
        assert_eq!(row.defender_country_tag_db_ids(), Leaf::Measured(vec![2]));
        // Owner zero is not an owner.
        let row = naval(json!([member(json!("defender"), json!(0))]));
        assert_eq!(row.defender_country_tag_db_ids(), Leaf::Unobtained);

        // A member that cannot be placed on a side costs both sides.
        for side in [Value::Null, json!("observer")] {
            let row = naval(json!([
                member(json!("attacker"), json!(1)),
                member(side, json!(2)),
            ]));
            assert_eq!(row.attacker_country_tag_db_ids(), Leaf::Unobtained);
            assert_eq!(row.defender_country_tag_db_ids(), Leaf::Unobtained);
        }

        // No member on a side is a MEASURED empty set, not an unmeasured one.
        let row = naval(json!([member(json!("attacker"), json!(1))]));
        assert_eq!(row.defender_country_tag_db_ids(), Leaf::Measured(vec![]));

        // And the members' own three states pass through.
        assert_eq!(naval(Value::Null).attacker_country_tag_db_ids(), Leaf::Unobtained);
        let row: NavalCombatRecord =
            serde_json::from_value(json!({"combat_id": 9, "generation": 2})).unwrap();
        assert_eq!(row.attacker_country_tag_db_ids(), Leaf::Absent);
    }

    #[test]
    fn task_force_references_pair_the_two_vectors_positionally() {
        let row: NavalCombatRecord = serde_json::from_value(json!({
            "combat_id": 9, "generation": 2,
            "attacker_task_force_ids": [51, 490], "attacker_task_force_id_spaces": [61, 61],
            "defender_task_force_ids": [7], "defender_task_force_id_spaces": [61, 62]
        }))
        .unwrap();
        assert_eq!(
            row.side_task_force_references(ATTACKER_SIDE),
            Leaf::Measured(vec![(61, 51), (61, 490)])
        );
        assert_eq!(row.side_task_force_references(DEFENDER_SIDE), Leaf::Unobtained);
        let bare: NavalCombatRecord =
            serde_json::from_value(json!({"combat_id": 9, "generation": 2})).unwrap();
        assert_eq!(bare.side_task_force_references(ATTACKER_SIDE), Leaf::Absent);
    }

    /// kills: a writer keying the object by anything but the row's own
    /// identity, and a writer filing a row the reader's key check would
    /// refuse. The object comes back through the reader's own checks entry by
    /// entry, in the reader's (lexicographic) order, which is not the order
    /// the rows were given in.
    #[test]
    fn the_wire_object_keys_each_naval_row_by_its_own_identity() {
        let row = |combat_id: i64, generation: i64| {
            let mut row = NavalCombatRecord::all_absent();
            row.combat_id = Leaf::Measured(combat_id);
            row.naval_combat_id = Leaf::Measured(combat_id);
            row.generation = Leaf::Measured(generation);
            row.kind = Leaf::Measured(NAVAL_COMBAT_KIND.into());
            row
        };
        let rows = [row(9, 1), row(240_580, 1), row(9, 2)];
        let object = naval_combats_wire_object(&rows).unwrap();
        assert_eq!(
            object.keys().cloned().collect::<Vec<_>>(),
            vec!["240580:1", "9:1", "9:2"]
        );
        for (key, value) in &object {
            let read: NavalCombatRecord = serde_json::from_value(value.clone()).unwrap();
            read.validate_wire_entry(key).unwrap();
        }
        assert_eq!(
            serde_json::from_value::<NavalCombatRecord>(object["240580:1"].clone()).unwrap(),
            rows[1]
        );

        let unidentified = NavalCombatRecord::all_absent();
        assert!(naval_combats_wire_object([&unidentified])
            .unwrap_err()
            .contains("measured identity"));
        let mut disagreeing = row(9, 1);
        disagreeing.naval_combat_id = Leaf::Measured(10);
        assert!(naval_combats_wire_object([&disagreeing])
            .unwrap_err()
            .contains("disagree"));
        assert!(naval_combats_wire_object([&row(9, 1), &row(9, 1)])
            .unwrap_err()
            .contains("twice"));
        let mut unkinded = row(9, 1);
        unkinded.kind = Leaf::Absent;
        assert!(naval_combats_wire_object([&unkinded])
            .unwrap_err()
            .contains("kind"));
        assert_eq!(naval_combats_wire_object([]).unwrap(), Map::new());
    }

    /// kills: a naval entry the census cannot identify being admitted as one
    /// of its members -- `{}` under any key, a row without its kind, a key
    /// that is not the row's identity -- for the XMAP14 join to fail on with a
    /// plain error later.
    #[test]
    fn a_naval_wire_entry_must_spell_its_own_identity_once() {
        let row: NavalCombatRecord = serde_json::from_value(json!({
            "combat_id": 9, "naval_combat_id": 9, "generation": 2, "kind": "naval"
        }))
        .unwrap();
        assert_eq!(row.validate_wire_entry("9:2").unwrap(), (9, 2));
        assert!(row.validate_wire_entry("2:9").unwrap_err().contains("identity"));
        let disagreeing: NavalCombatRecord = serde_json::from_value(json!({
            "combat_id": 9, "naval_combat_id": 10, "generation": 2, "kind": "naval"
        }))
        .unwrap();
        assert!(disagreeing.validate_wire_entry("9:2").unwrap_err().contains("disagree"));
        for (label, entry) in [
            ("empty", json!({})),
            ("no generation", json!({"combat_id": 9, "kind": "naval"})),
            ("no kind", json!({"combat_id": 9, "generation": 2})),
            ("land kind", json!({"combat_id": 9, "generation": 2, "kind": "land"})),
            ("zero id", json!({"combat_id": 0, "generation": 2, "kind": "naval"})),
            ("negative generation", json!({"combat_id": 9, "generation": -1, "kind": "naval"})),
        ] {
            let row: NavalCombatRecord = serde_json::from_value(entry).unwrap();
            assert!(row.validate_wire_entry("9:2").is_err(), "{label}");
        }
        // Generation zero is an identity the engine counts from.
        let first: NavalCombatRecord = serde_json::from_value(json!({
            "combat_id": 9, "generation": 0, "kind": "naval"
        }))
        .unwrap();
        assert_eq!(first.validate_wire_entry("9:0").unwrap(), (9, 0));
        assert_eq!(naval_combat_key(240_580, 1), "240580:1");
    }

    /// kills: an active land row without its identity pair or its kind being
    /// admitted into the census, and an ended row without the terminal rule.
    #[test]
    fn active_and_ended_land_rows_state_their_identity_before_they_are_admitted() {
        let active = |value: serde_json::Value| -> Result<(i64, i64), String> {
            serde_json::from_value::<LandCombatRecord>(value)
                .unwrap()
                .validate_active()
        };
        assert_eq!(
            active(json!({"combat_id": 7, "generation": 0, "kind": "land"})).unwrap(),
            (7, 0)
        );
        assert!(active(json!({"combat_id": 7, "kind": "land"}))
            .unwrap_err()
            .contains("generation"));
        assert!(active(json!({"generation": 1, "kind": "land"}))
            .unwrap_err()
            .contains("combat_id"));
        assert!(active(json!({"combat_id": 7, "generation": 1}))
            .unwrap_err()
            .contains("kind"));
        assert!(active(json!({"combat_id": 7, "generation": 1, "kind": "naval"}))
            .unwrap_err()
            .contains("kind"));
        assert!(active(json!({"combat_id": 0, "generation": 1, "kind": "land"}))
            .unwrap_err()
            .contains("combat_id"));
        assert!(active(json!({"combat_id": 7, "generation": -3, "kind": "land"}))
            .unwrap_err()
            .contains("negative"));

        let ended = |value: serde_json::Value| -> Result<(i64, i64), String> {
            serde_json::from_value::<EndedLandCombatRecord>(value)
                .unwrap()
                .validate_ended()
        };
        assert_eq!(
            ended(json!({"combat_id": 42, "generation": 7, "lifecycle": "ended"})).unwrap(),
            (42, 7)
        );
        assert!(ended(json!({"combat_id": 42, "generation": 7, "lifecycle": "active"}))
            .unwrap_err()
            .contains("lifecycle"));
        assert!(ended(json!({"combat_id": 42, "generation": 0, "lifecycle": "ended"}))
            .unwrap_err()
            .contains("non-positive"));
        assert!(ended(json!({"generation": 7, "lifecycle": "ended"}))
            .unwrap_err()
            .contains("combat_id"));
    }
}
