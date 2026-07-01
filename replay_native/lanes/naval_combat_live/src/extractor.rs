use std::collections::{BTreeMap, BTreeSet};

use crate::domain49_50_naval_combat_live::{
    compact_record, encode_lifecycle_payload, encode_member_payload, CombatSide, LifecycleRow,
    MemberRow, COMPACT_RECORD_TYPE_DOMAIN_DATA, DOMAIN_NAVAL_COMBAT_LIFECYCLE,
    DOMAIN_NAVAL_COMBAT_MEMBERS, STATUS_DELTA_DATA,
};

pub const CURRENT_870EBF_OFFSETS: ExtractorOffsets = ExtractorOffsets {
    naval_combat_body_vtable_rva: 0x29B6660,
    naval_combat_side_vtable_rva: 0x29BC908,
    naval_combat_registry_vtable_rva: 0x29B6588,
    naval_combat_member_vtable_rva: 0x29F9608,
    body_handle_off: 0x08,
    body_attacker_side_off: 0x18,
    body_defender_side_off: 0x20,
    body_front_off: 0x28,
    side_registry_off: 0x18,
    side_group_data_off: 0xC8,
    side_group_size_off: 0xD4,
    side_positioning_off: 0x288,
    registry_handle_off: 0x18,
    registry_attacker_side_off: 0x28,
    registry_defender_side_off: 0x30,
    registry_front_off: 0x38,
    front_location_off: 0xA4,
    group_members_data_off: 0x08,
    group_members_size_off: 0x14,
    member_handle_off: 0x08,
    member_owner_tag_off: 0x20,
    member_max_strength_raw_off: 0x30,
    member_state_off: 0xE0,
    member_weight_off: 0xF8,
    member_slot_off: 0x120,
};

pub const CURRENT_870EBF_CSHIP_ROUTE: CshipRoute = CshipRoute {
    cship_vtable_rva: 0x2941748,
    handle_off: 0x08,
    alias_instance_id_off: 0x0C,
    sunk_convoys_off: 0x3C,
    category_off: 0xE0,
    strength_off: 0x6F8,
    organisation_off: 0x700,
    experience_off: 0x708,
    ref100_sentinel_off: 0x710,
    ref100_sentinel_raw: 10_000_000,
    type_tag: 51,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtractorOffsets {
    pub naval_combat_body_vtable_rva: u64,
    pub naval_combat_side_vtable_rva: u64,
    pub naval_combat_registry_vtable_rva: u64,
    pub naval_combat_member_vtable_rva: u64,
    pub body_handle_off: u64,
    pub body_attacker_side_off: u64,
    pub body_defender_side_off: u64,
    pub body_front_off: u64,
    pub side_registry_off: u64,
    pub side_group_data_off: u64,
    pub side_group_size_off: u64,
    pub side_positioning_off: u64,
    pub registry_handle_off: u64,
    pub registry_attacker_side_off: u64,
    pub registry_defender_side_off: u64,
    pub registry_front_off: u64,
    pub front_location_off: u64,
    pub group_members_data_off: u64,
    pub group_members_size_off: u64,
    pub member_handle_off: u64,
    pub member_owner_tag_off: u64,
    pub member_max_strength_raw_off: u64,
    pub member_state_off: u64,
    pub member_weight_off: u64,
    pub member_slot_off: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CshipRoute {
    pub cship_vtable_rva: u64,
    pub handle_off: u64,
    pub alias_instance_id_off: u64,
    pub sunk_convoys_off: u64,
    pub category_off: u64,
    pub strength_off: u64,
    pub organisation_off: u64,
    pub experience_off: u64,
    pub ref100_sentinel_off: u64,
    pub ref100_sentinel_raw: u32,
    pub type_tag: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavalCombatBodyBridgeRow {
    pub combat_id: u32,
    pub generation: u64,
    pub body_ptr: u64,
    pub location_id: Option<u32>,
    pub attacker_member_count: u32,
    pub defender_member_count: u32,
    pub attacker_positioning_milli: Option<i32>,
    pub defender_positioning_milli: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberBridgeRow {
    pub combat_id: u32,
    pub generation: u64,
    pub side: CombatSide,
    pub group_index: u32,
    pub member_index: u32,
    pub member_ptr: u64,
    pub unit_type_tag: u32,
    pub ship_instance_id: Option<u32>,
    pub is_convoy: bool,
    pub owner_tag_db_id: Option<u32>,
    pub designed_max_strength_raw: Option<u32>,
    pub battle_slot_index_raw: u32,
    pub state_bitmask: Option<u32>,
    pub engagement_weight_raw: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CshipSnapshotRow {
    pub instance_id: u32,
    pub body_ptr: u64,
    pub vtable_rva: u64,
    pub handle_type_tag: u32,
    pub alias_instance_id: u32,
    pub ref100_sentinel_raw: u32,
    pub strength_raw: u32,
    pub organisation_raw: u32,
    pub experience_raw: u32,
    pub sunk_convoys: u32,
    pub category_enum: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedCombatSnapshot {
    pub lifecycle: NavalCombatBodyBridgeRow,
    pub members: Vec<MemberBridgeRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureFrame {
    pub compact_stream: Vec<u8>,
    pub lifecycle_rows: Vec<LifecycleRow>,
    pub member_rows: Vec<MemberRow>,
    pub status_flags: u32,
}

pub fn build_capture_frame(
    snapshots: &[ValidatedCombatSnapshot],
    cships: &[CshipSnapshotRow],
    sequence_start: u64,
    game_hour_raw: i32,
    elapsed_wall_ms: u64,
) -> Result<CaptureFrame, Vec<String>> {
    let mut errors = Vec::new();
    if snapshots.is_empty() {
        errors.push(
            "no validated active naval combat snapshots; active two-sided fixture is required"
                .to_string(),
        );
        return Err(errors);
    }

    let cship_index = index_cships(cships, &mut errors);
    let mut lifecycle_rows = Vec::new();
    let mut raw_member_rows = Vec::new();
    for snapshot in snapshots {
        if snapshot.lifecycle.attacker_member_count == 0
            || snapshot.lifecycle.defender_member_count == 0
        {
            errors.push(format!(
                "combat {}:{} is not two-sided",
                snapshot.lifecycle.combat_id, snapshot.lifecycle.generation
            ));
        }
        lifecycle_rows.push(LifecycleRow {
            combat_id: snapshot.lifecycle.combat_id,
            generation: snapshot.lifecycle.generation,
            body_ptr: snapshot.lifecycle.body_ptr,
            deleted: false,
            location_id: snapshot.lifecycle.location_id,
            attacker_member_count: Some(snapshot.lifecycle.attacker_member_count),
            defender_member_count: Some(snapshot.lifecycle.defender_member_count),
            attacker_positioning_milli: snapshot.lifecycle.attacker_positioning_milli,
            defender_positioning_milli: snapshot.lifecycle.defender_positioning_milli,
        });
        raw_member_rows.extend(snapshot.members.iter().cloned());
    }

    let member_rows = match validate_member_cship_joins(&raw_member_rows, &cship_index) {
        Ok(rows) => rows,
        Err(mut join_errors) => {
            errors.append(&mut join_errors);
            Vec::new()
        }
    };
    if !errors.is_empty() {
        return Err(errors);
    }

    let lifecycle_payload =
        encode_lifecycle_payload(elapsed_wall_ms, STATUS_DELTA_DATA, &lifecycle_rows);
    let member_payload = encode_member_payload(elapsed_wall_ms, STATUS_DELTA_DATA, &member_rows);
    let mut compact_stream = Vec::new();
    compact_stream.extend_from_slice(&compact_record(
        sequence_start,
        game_hour_raw,
        DOMAIN_NAVAL_COMBAT_LIFECYCLE,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &lifecycle_payload,
    ));
    compact_stream.extend_from_slice(&compact_record(
        sequence_start.saturating_add(1),
        game_hour_raw,
        DOMAIN_NAVAL_COMBAT_MEMBERS,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &member_payload,
    ));

    Ok(CaptureFrame {
        compact_stream,
        lifecycle_rows,
        member_rows,
        status_flags: STATUS_DELTA_DATA,
    })
}

pub fn validate_member_cship_joins(
    members: &[MemberBridgeRow],
    cships: &BTreeMap<u32, CshipSnapshotRow>,
) -> Result<Vec<MemberRow>, Vec<String>> {
    let mut errors = Vec::new();
    let mut rows = Vec::with_capacity(members.len());
    let mut non_convoy_count = 0usize;
    let mut cship_join_count = 0usize;
    let mut seen = BTreeSet::new();

    for member in members {
        let key = (
            member.combat_id,
            member.generation,
            member.side,
            member.group_index,
            member.member_index,
        );
        if !seen.insert(key) {
            errors.push(format!(
                "duplicate member row combat={} generation={} side={} group={} member={}",
                member.combat_id,
                member.generation,
                member.side.name(),
                member.group_index,
                member.member_index
            ));
            continue;
        }
        let mut cship_join_validated = false;
        if member.is_convoy {
            if member.ship_instance_id.is_some() {
                errors.push(format!(
                    "convoy member combat={} side={} has unexpected ship_instance_id",
                    member.combat_id,
                    member.side.name()
                ));
            }
        } else {
            non_convoy_count = non_convoy_count.saturating_add(1);
            if member.unit_type_tag != CURRENT_870EBF_CSHIP_ROUTE.type_tag {
                errors.push(format!(
                    "non-convoy member combat={} side={} has unit_type_tag {}, expected {}",
                    member.combat_id,
                    member.side.name(),
                    member.unit_type_tag,
                    CURRENT_870EBF_CSHIP_ROUTE.type_tag
                ));
            }
            let Some(ship_id) = member.ship_instance_id else {
                errors.push(format!(
                    "non-convoy member combat={} side={} missing ship_instance_id",
                    member.combat_id,
                    member.side.name()
                ));
                continue;
            };
            match cships.get(&ship_id) {
                Some(cship) if cship_row_matches_current_route(cship) => {
                    cship_join_validated = true;
                    cship_join_count = cship_join_count.saturating_add(1);
                }
                Some(cship) => errors.push(format!(
                    "ship_instance_id {ship_id} failed current CShip route validation: vtable_rva=0x{:X}, handle_type={}, alias={}, ref100={}",
                    cship.vtable_rva,
                    cship.handle_type_tag,
                    cship.alias_instance_id,
                    cship.ref100_sentinel_raw
                )),
                None => errors.push(format!(
                    "ship_instance_id {ship_id} missing from current CShip route snapshot"
                )),
            }
        }
        rows.push(MemberRow {
            combat_id: member.combat_id,
            generation: member.generation,
            side: member.side,
            group_index: member.group_index,
            member_index: member.member_index,
            unit_type_tag: member.unit_type_tag,
            ship_instance_id: member.ship_instance_id,
            cship_join_validated,
            is_convoy: member.is_convoy,
            owner_tag_db_id: member.owner_tag_db_id,
            designed_max_strength_raw: member.designed_max_strength_raw,
            battle_slot_index_raw: member.battle_slot_index_raw,
            state_bitmask: member.state_bitmask,
            engagement_weight_raw: member.engagement_weight_raw,
        });
    }

    if non_convoy_count == 0 {
        errors.push(
            "fixture has no non-convoy CFEXMember rows to validate against CShip".to_string(),
        );
    }
    if cship_join_count == 0 {
        errors.push(
            "no CFEXMember ship_instance_id resolved through current CShip route".to_string(),
        );
    }

    if errors.is_empty() {
        Ok(rows)
    } else {
        Err(errors)
    }
}

fn index_cships(
    cships: &[CshipSnapshotRow],
    errors: &mut Vec<String>,
) -> BTreeMap<u32, CshipSnapshotRow> {
    let mut index = BTreeMap::new();
    for cship in cships {
        if !cship_row_matches_current_route(cship) {
            errors.push(format!(
                "CShip instance {} failed current route validation",
                cship.instance_id
            ));
            continue;
        }
        if index.insert(cship.instance_id, cship.clone()).is_some() {
            errors.push(format!("duplicate CShip instance {}", cship.instance_id));
        }
    }
    index
}

fn cship_row_matches_current_route(row: &CshipSnapshotRow) -> bool {
    row.instance_id != 0
        && row.vtable_rva == CURRENT_870EBF_CSHIP_ROUTE.cship_vtable_rva
        && row.handle_type_tag == CURRENT_870EBF_CSHIP_ROUTE.type_tag
        && row.alias_instance_id == row.instance_id
        && row.ref100_sentinel_raw == CURRENT_870EBF_CSHIP_ROUTE.ref100_sentinel_raw
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain49_50_naval_combat_live::gate_stream;

    fn cship(id: u32) -> CshipSnapshotRow {
        CshipSnapshotRow {
            instance_id: id,
            body_ptr: 0x22aa_0100_000,
            vtable_rva: CURRENT_870EBF_CSHIP_ROUTE.cship_vtable_rva,
            handle_type_tag: CURRENT_870EBF_CSHIP_ROUTE.type_tag,
            alias_instance_id: id,
            ref100_sentinel_raw: CURRENT_870EBF_CSHIP_ROUTE.ref100_sentinel_raw,
            strength_raw: 10_000_000,
            organisation_raw: 9_000_000,
            experience_raw: 100_000,
            sunk_convoys: 0,
            category_enum: 3,
        }
    }

    fn fixture() -> ValidatedCombatSnapshot {
        ValidatedCombatSnapshot {
            lifecycle: NavalCombatBodyBridgeRow {
                combat_id: 240_580,
                generation: 1,
                body_ptr: 0x22aa_0004_000,
                location_id: Some(2799),
                attacker_member_count: 1,
                defender_member_count: 1,
                attacker_positioning_milli: Some(812),
                defender_positioning_milli: Some(765),
            },
            members: vec![
                MemberBridgeRow {
                    combat_id: 240_580,
                    generation: 1,
                    side: CombatSide::Attacker,
                    group_index: 0,
                    member_index: 0,
                    member_ptr: 0x22aa_0020_000,
                    unit_type_tag: CURRENT_870EBF_CSHIP_ROUTE.type_tag,
                    ship_instance_id: Some(1001),
                    is_convoy: false,
                    owner_tag_db_id: Some(17),
                    designed_max_strength_raw: Some(10_000_000),
                    battle_slot_index_raw: 0,
                    state_bitmask: Some(1),
                    engagement_weight_raw: Some(1_000),
                },
                MemberBridgeRow {
                    combat_id: 240_580,
                    generation: 1,
                    side: CombatSide::Defender,
                    group_index: 0,
                    member_index: 0,
                    member_ptr: 0x22aa_0021_000,
                    unit_type_tag: 0,
                    ship_instance_id: None,
                    is_convoy: true,
                    owner_tag_db_id: Some(18),
                    designed_max_strength_raw: Some(10_000_000),
                    battle_slot_index_raw: 1,
                    state_bitmask: Some(1),
                    engagement_weight_raw: Some(500),
                },
            ],
        }
    }

    #[test]
    fn builds_compact_frame_from_validated_member_cship_join() {
        let frame = build_capture_frame(&[fixture()], &[cship(1001)], 10, 512, 5).unwrap();
        assert_eq!(frame.lifecycle_rows.len(), 1);
        assert_eq!(frame.member_rows.len(), 2);
        assert!(frame.member_rows.iter().any(|row| row.cship_join_validated));
        let gate = gate_stream(&frame.compact_stream);
        assert!(gate.ok, "{:?}", gate.errors);
        assert_eq!(gate.summary.two_sided_combat_count, 1);
    }

    #[test]
    fn rejects_member_without_current_cship_row() {
        let errors = build_capture_frame(&[fixture()], &[], 10, 512, 5).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.contains("missing from current CShip route")));
    }

    #[test]
    fn rejects_empty_fixture_as_lifecycle_blocker() {
        let errors = build_capture_frame(&[], &[cship(1001)], 10, 512, 5).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.contains("active two-sided fixture is required")));
    }
}
