//! Province placement for active naval combat map icons.
//!
//! `14127C6B0` first walks each province's ordered direct combat vector,
//! then the global combat registry. The second pass can add an engaged combat
//! to a sea province occupied by one of its transferred units. The two passes
//! are independent: an unresolved transfer chain cannot erase a direct icon.
//!
//! The recorded transfer units are live ones only (`1415B10A0` skips a stale
//! unit handle, and the recorder does too), so a recorded unit without a
//! location is unresolved, not stale.

use std::collections::{BTreeMap, BTreeSet};

use installed_content::map::ProvinceKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Combat {
    pub id: u64,
    pub home: u32,
    pub registry_ordinal: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Direct {
    pub province: u32,
    pub combats: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Transfers {
    pub combat: u64,
    pub transfers: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Units {
    pub transfer: u64,
    pub units: Vec<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Occurrence {
    /// Province whose native category-9 bucket owns and deduplicates the icon.
    pub province: u32,
    /// Direct entries keep combat H even when their source province is P != H.
    pub anchor_province: u32,
    pub combat: u64,
    /// Direct icons run home rotation/progress; global spillovers do not.
    pub home: bool,
    /// Last admitted call's per-source-province ordinal. `None` means a prior
    /// unknown admission or incomplete global pass makes placement uncertain.
    pub ordinal: Option<u32>,
}

pub(super) struct Sources<'a> {
    pub combats: &'a [Combat],
    pub combats_complete: bool,
    pub direct: &'a [Direct],
    pub direct_complete: bool,
    pub transfers: Option<&'a [Transfers]>,
    pub units: Option<&'a [Units]>,
    pub unit_locations: Option<&'a BTreeMap<u64, u32>>,
    pub province_kind: &'a dyn Fn(u32) -> Option<ProvinceKind>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Occurrences {
    pub by_province: BTreeMap<u32, Vec<Occurrence>>,
    /// Missing operands omit global spillovers and may make sea-bucket direct
    /// offsets unknown; their proven direct presence remains in by_province.
    pub spillover_gap: Option<&'static str>,
    /// Each direct call, as (province, combat), whose combat the census lacks.
    pub direct_gaps: Vec<(u32, u64)>,
    /// Each direct call, as (province, combat), whose admission is unknown.
    pub admission_gaps: Vec<(u32, u64)>,
}

#[derive(Default)]
struct Slot {
    next: u32,
    uncertain: bool,
}

fn apply_call(
    out: &mut Occurrences,
    indices: &mut BTreeMap<u32, BTreeMap<u64, usize>>,
    slots: &mut BTreeMap<u32, Slot>,
    province: u32,
    anchor_province: u32,
    combat: u64,
    home: bool,
) {
    let slot = slots.entry(province).or_default();
    let ordinal = if slot.uncertain {
        None
    } else {
        let ordinal = Some(slot.next);
        if let Some(next) = slot.next.checked_add(1) {
            slot.next = next;
        } else {
            slot.uncertain = true;
        }
        ordinal
    };
    let rows = out.by_province.entry(province).or_default();
    let indices = indices.entry(province).or_default();
    if let Some(&index) = indices.get(&combat) {
        // Native +0x98 is updated on every admitted call, even a dedup hit.
        rows[index].ordinal = ordinal;
    } else {
        indices.insert(combat, rows.len());
        rows.push(Occurrence {
            province,
            anchor_province,
            combat,
            home,
            ordinal,
        });
    }
}

fn unknown_call(
    out: &mut Occurrences,
    indices: &BTreeMap<u32, BTreeMap<u64, usize>>,
    slots: &mut BTreeMap<u32, Slot>,
    province: u32,
    combat: u64,
) {
    slots.entry(province).or_default().uncertain = true;
    if let Some(&index) = indices.get(&province).and_then(|rows| rows.get(&combat))
        && let Some(row) = out
            .by_province
            .get_mut(&province)
            .and_then(|rows| rows.get_mut(index))
    {
        row.ordinal = None;
    }
}

fn unknown_sea_offsets(out: &mut Occurrences, kind: &dyn Fn(u32) -> Option<ProvinceKind>) {
    for (&province, rows) in &mut out.by_province {
        if matches!(kind(province), Some(ProvinceKind::Sea) | None) {
            for row in rows {
                row.ordinal = None;
            }
        }
    }
}

/// `UpdateScreenOffset@1418E4CD0` multiplies the final native category-9
/// ordinal by the GUI object's integer height, with positive screen Y down.
pub(super) fn screen_y_offset(ordinal: u32, gui_height: f32) -> Option<f32> {
    if !gui_height.is_finite()
        || gui_height < 0.0
        || gui_height.fract() != 0.0
        || gui_height >= 2_147_483_648.0
    {
        return None;
    }
    let ordinal = i32::try_from(ordinal).ok()?;
    Some(ordinal.checked_mul(gui_height as i32)? as f32)
}

pub(super) fn derive(
    sources: Sources<'_>,
    admitted: &impl Fn(u64) -> Option<bool>,
    engaged: &impl Fn(u64) -> Option<bool>,
) -> Occurrences {
    let mut out = Occurrences::default();
    let combats: BTreeMap<_, _> = sources.combats.iter().map(|row| (row.id, row)).collect();
    let mut indices: BTreeMap<u32, BTreeMap<u64, usize>> = BTreeMap::new();
    let mut slots: BTreeMap<u32, Slot> = BTreeMap::new();
    for row in sources.direct {
        for &id in &row.combats {
            match admitted(id) {
                Some(false) => continue,
                None => {
                    out.admission_gaps.push((row.province, id));
                    unknown_call(&mut out, &indices, &mut slots, row.province, id);
                    continue;
                }
                Some(true) => {}
            }
            if let Some(combat) = combats.get(&id) {
                apply_call(
                    &mut out,
                    &mut indices,
                    &mut slots,
                    row.province,
                    combat.home,
                    id,
                    true,
                );
            } else {
                // The authenticated native call still consumed a slot. Its
                // missing combat row prevents us from placing the icon.
                out.direct_gaps.push((row.province, id));
                let slot = slots.entry(row.province).or_default();
                if let Some(next) = slot.next.checked_add(1) {
                    slot.next = next;
                } else {
                    slot.uncertain = true;
                }
            }
        }
    }
    if sources.combats_complete && sources.combats.is_empty() {
        return out;
    }

    // A missing direct census makes the relative order of a global entry and
    // a possible earlier direct entry unknowable. Proven direct entries above
    // are still safe to show.
    let gap = if !sources.direct_complete {
        Some("direct naval-combat province census is incomplete")
    } else if !sources.combats_complete {
        Some("active naval-combat registry is incomplete")
    } else if sources.transfers.is_none() {
        Some("naval-combat transfer census is incomplete")
    } else if sources.units.is_none() {
        Some("naval-transfer unit census is incomplete")
    } else if sources.unit_locations.is_none() {
        Some("naval-transfer unit locations are incomplete")
    } else {
        None
    };
    if let Some(reason) = gap {
        out.spillover_gap = Some(reason);
        unknown_sea_offsets(&mut out, sources.province_kind);
        return out;
    }

    let transfers: BTreeMap<_, _> = sources
        .transfers
        .unwrap_or_default()
        .iter()
        .map(|row| (row.combat, row.transfers.as_slice()))
        .collect();
    let units: BTreeMap<_, _> = sources
        .units
        .unwrap_or_default()
        .iter()
        .map(|row| (row.transfer, row.units.as_slice()))
        .collect();
    let empty_locations = BTreeMap::new();
    let locations = sources.unit_locations.unwrap_or(&empty_locations);
    let mut ordered = sources.combats.to_vec();
    ordered.sort_by_key(|row| row.registry_ordinal);
    if ordered
        .windows(2)
        .any(|pair| pair[0].registry_ordinal == pair[1].registry_ordinal)
    {
        out.spillover_gap = Some("active naval-combat registry order is ambiguous");
        unknown_sea_offsets(&mut out, sources.province_kind);
        return out;
    }
    let mut spillovers = Vec::new();
    for combat in ordered {
        let primary = admitted(combat.id);
        // `14127C6B0` tests the engagement before it walks any transfer, so a combat known to
        // be unengaged needs none of its units resolved.
        let seen = engaged(combat.id);
        if primary == Some(false) || seen == Some(false) {
            continue;
        }
        // A complete sparse table omits combats with no transfer slots.
        let Some(handles) = transfers.get(&combat.id) else {
            continue;
        };
        let mut provinces = BTreeSet::new();
        for &handle in *handles {
            if handle == 0 {
                continue;
            }
            let Some(unit_handles) = units.get(&handle) else {
                // A complete sparse transfer-unit table omits stale native
                // references after the source reader fails their live join.
                continue;
            };
            for &unit in *unit_handles {
                if unit == 0 {
                    continue;
                }
                let Some(&province) = locations.get(&unit) else {
                    out.spillover_gap = Some("a naval-transfer unit location is unresolved");
                    unknown_sea_offsets(&mut out, sources.province_kind);
                    return out;
                };
                if province == 0 || province == combat.home {
                    continue;
                }
                let Some(kind) = (sources.province_kind)(province) else {
                    out.spillover_gap =
                        Some("a naval-transfer province is absent from matching map content");
                    unknown_sea_offsets(&mut out, sources.province_kind);
                    return out;
                };
                if kind == ProvinceKind::Sea {
                    provinces.insert(province);
                }
            }
        }
        if provinces.is_empty() {
            continue;
        }
        if seen != Some(true) {
            out.spillover_gap = Some("naval-combat country engagement is unknown");
            unknown_sea_offsets(&mut out, sources.province_kind);
            return out;
        }
        if primary != Some(true) {
            out.spillover_gap = Some("naval-combat primary-side admission is unknown");
            unknown_sea_offsets(&mut out, sources.province_kind);
            return out;
        }
        for province in provinces {
            spillovers.push((province, combat.id));
        }
    }
    // Commit only after all chains are proven. An unresolved earlier registry
    // member could otherwise invalidate relative icon order in a province.
    for (province, combat) in spillovers {
        apply_call(
            &mut out,
            &mut indices,
            &mut slots,
            province,
            province,
            combat,
            false,
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds() -> BTreeMap<u32, ProvinceKind> {
        BTreeMap::from([
            (10, ProvinceKind::Sea),
            (11, ProvinceKind::Sea),
            (12, ProvinceKind::Land),
            (13, ProvinceKind::Lake),
        ])
    }

    #[test]
    fn direct_order_and_duplicates_are_separate_from_global_sea_spillover() {
        let combats = [
            Combat {
                id: 8,
                home: 10,
                registry_ordinal: 2,
            },
            Combat {
                id: 7,
                home: 12,
                registry_ordinal: 1,
            },
            Combat {
                id: 9,
                home: 11,
                registry_ordinal: 0,
            },
        ];
        let direct = [Direct {
            province: 10,
            combats: vec![8, 8, 7],
        }];
        let transfers = [
            Transfers {
                combat: 7,
                transfers: vec![100, 100, 0],
            },
            Transfers {
                combat: 9,
                transfers: vec![101],
            },
        ];
        let units = [
            Units {
                transfer: 100,
                units: vec![200, 201, 202, 203],
            },
            Units {
                transfer: 101,
                units: vec![200],
            },
        ];
        let locations = BTreeMap::from([(200, 10), (201, 11), (202, 12), (203, 13)]);
        let kinds = kinds();
        let kind = |province| kinds.get(&province).copied();
        let out = derive(
            Sources {
                combats: &combats,
                combats_complete: true,
                direct: &direct,
                direct_complete: true,
                transfers: Some(&transfers),
                units: Some(&units),
                unit_locations: Some(&locations),
                province_kind: &kind,
            },
            &|_| Some(true),
            &|_| Some(true),
        );
        assert_eq!(
            out.by_province[&10]
                .iter()
                .map(|row| row.combat)
                .collect::<Vec<_>>(),
            [8, 7, 9]
        );
        assert_eq!(
            out.by_province[&11]
                .iter()
                .map(|row| row.combat)
                .collect::<Vec<_>>(),
            [7]
        );
        assert_eq!(
            out.by_province[&10]
                .iter()
                .map(|row| row.home)
                .collect::<Vec<_>>(),
            [true, true, false]
        );
        assert_eq!(
            out.by_province[&10]
                .iter()
                .map(|row| (row.anchor_province, row.ordinal))
                .collect::<Vec<_>>(),
            [(10, Some(1)), (12, Some(4)), (10, Some(3))],
            "direct P!=H keeps H after a later global dedup hit, while every admitted call consumes a slot"
        );
        assert!(!out.by_province[&11][0].home);
        assert_eq!(out.by_province[&11][0].ordinal, Some(0));
        assert!(!out.by_province.contains_key(&12));
        assert!(!out.by_province.contains_key(&13));
        assert_eq!(out.spillover_gap, None);
    }

    #[test]
    fn absent_transfer_sources_keep_direct_presence_but_not_sea_offset() {
        let combats = [Combat {
            id: 8,
            home: 10,
            registry_ordinal: 0,
        }];
        let direct = [Direct {
            province: 10,
            combats: vec![8],
        }];
        let kinds = kinds();
        let kind = |province| kinds.get(&province).copied();
        let out = derive(
            Sources {
                combats: &combats,
                combats_complete: true,
                direct: &direct,
                direct_complete: true,
                transfers: None,
                units: None,
                unit_locations: None,
                province_kind: &kind,
            },
            &|_| Some(true),
            &|_| Some(true),
        );
        assert_eq!(
            out.by_province[&10],
            [Occurrence {
                province: 10,
                anchor_province: 10,
                combat: 8,
                home: true,
                ordinal: None,
            }]
        );
        assert_eq!(
            out.spillover_gap,
            Some("naval-combat transfer census is incomplete")
        );
    }

    #[test]
    fn stale_transfer_ref_is_skipped_from_a_complete_unit_table() {
        let combats = [Combat {
            id: 8,
            home: 10,
            registry_ordinal: 0,
        }];
        let direct = [Direct {
            province: 10,
            combats: vec![8],
        }];
        let transfers = [Transfers {
            combat: 8,
            transfers: vec![100],
        }];
        let kinds = kinds();
        let kind = |province| kinds.get(&province).copied();
        let out = derive(
            Sources {
                combats: &combats,
                combats_complete: true,
                direct: &direct,
                direct_complete: true,
                transfers: Some(&transfers),
                units: Some(&[]),
                unit_locations: Some(&BTreeMap::new()),
                province_kind: &kind,
            },
            &|_| Some(true),
            &|_| Some(true),
        );
        assert_eq!(out.by_province[&10].len(), 1);
        assert_eq!(out.by_province[&10][0].ordinal, Some(0));
        assert_eq!(out.spillover_gap, None);
    }

    #[test]
    fn unengaged_combat_needs_no_transfer_resolution() {
        let combats = [Combat {
            id: 8,
            home: 10,
            registry_ordinal: 0,
        }];
        let transfers = [Transfers {
            combat: 8,
            transfers: vec![100],
        }];
        let kinds = kinds();
        let kind = |province| kinds.get(&province).copied();
        let out = derive(
            Sources {
                combats: &combats,
                combats_complete: true,
                direct: &[],
                direct_complete: true,
                transfers: Some(&transfers),
                units: Some(&[]),
                unit_locations: Some(&BTreeMap::new()),
                province_kind: &kind,
            },
            &|_| Some(true),
            &|_| Some(false),
        );
        assert!(out.by_province.is_empty());
        assert_eq!(out.spillover_gap, None);
    }

    #[test]
    fn global_spillovers_follow_registry_order_and_unknown_engagement_keeps_direct() {
        let combats = [
            Combat {
                id: 8,
                home: 11,
                registry_ordinal: 9,
            },
            Combat {
                id: 7,
                home: 11,
                registry_ordinal: 2,
            },
        ];
        let direct = [Direct {
            province: 11,
            combats: vec![8],
        }];
        let transfers = [
            Transfers {
                combat: 8,
                transfers: vec![108],
            },
            Transfers {
                combat: 7,
                transfers: vec![107],
            },
        ];
        let units = [
            Units {
                transfer: 108,
                units: vec![208],
            },
            Units {
                transfer: 107,
                units: vec![207],
            },
        ];
        let locations = BTreeMap::from([(207, 10), (208, 10)]);
        let kinds = kinds();
        let kind = |province| kinds.get(&province).copied();
        let sources = || Sources {
            combats: &combats,
            combats_complete: true,
            direct: &direct,
            direct_complete: true,
            transfers: Some(&transfers),
            units: Some(&units),
            unit_locations: Some(&locations),
            province_kind: &kind,
        };
        let complete = derive(sources(), &|_| Some(true), &|_| Some(true));
        assert_eq!(
            complete.by_province[&10]
                .iter()
                .map(|row| row.combat)
                .collect::<Vec<_>>(),
            [7, 8]
        );
        let unknown = derive(sources(), &|_| Some(true), &|id| (id == 8).then_some(true));
        assert_eq!(
            unknown.by_province[&11],
            [Occurrence {
                province: 11,
                anchor_province: 11,
                combat: 8,
                home: true,
                ordinal: None,
            }]
        );
        assert!(!unknown.by_province.contains_key(&10));
        assert_eq!(
            unknown.spillover_gap,
            Some("naval-combat country engagement is unknown")
        );
    }

    #[test]
    fn missing_unit_location_is_not_a_stale_transfer_and_poison_sea_offsets() {
        let combats = [Combat {
            id: 8,
            home: 10,
            registry_ordinal: 0,
        }];
        let direct = [Direct {
            province: 10,
            combats: vec![8],
        }];
        let transfers = [Transfers {
            combat: 8,
            transfers: vec![100],
        }];
        let units = [Units {
            transfer: 100,
            units: vec![200],
        }];
        let kinds = kinds();
        let kind = |province| kinds.get(&province).copied();
        let out = derive(
            Sources {
                combats: &combats,
                combats_complete: true,
                direct: &direct,
                direct_complete: true,
                transfers: Some(&transfers),
                units: Some(&units),
                unit_locations: Some(&BTreeMap::new()),
                province_kind: &kind,
            },
            &|_| Some(true),
            &|_| Some(true),
        );
        assert_eq!(out.by_province[&10][0].ordinal, None);
        assert_eq!(
            out.spillover_gap,
            Some("a naval-transfer unit location is unresolved")
        );
    }

    #[test]
    fn unknown_engagement_without_sea_candidate_does_not_poison_other_spillovers() {
        let combats = [
            Combat {
                id: 7,
                home: 11,
                registry_ordinal: 0,
            },
            Combat {
                id: 8,
                home: 11,
                registry_ordinal: 1,
            },
        ];
        let transfers = [Transfers {
            combat: 8,
            transfers: vec![108],
        }];
        let units = [Units {
            transfer: 108,
            units: vec![208],
        }];
        let locations = BTreeMap::from([(208, 10)]);
        let kinds = kinds();
        let kind = |province| kinds.get(&province).copied();
        let out = derive(
            Sources {
                combats: &combats,
                combats_complete: true,
                direct: &[],
                direct_complete: true,
                transfers: Some(&transfers),
                units: Some(&units),
                unit_locations: Some(&locations),
                province_kind: &kind,
            },
            &|_| Some(true),
            &|id| (id == 8).then_some(true),
        );
        assert_eq!(out.spillover_gap, None);
        assert_eq!(out.by_province[&10][0].combat, 8);
        assert_eq!(out.by_province[&10][0].ordinal, Some(0));
    }

    #[test]
    fn unknown_direct_admission_makes_later_offsets_unknown_without_removing_earlier_icon() {
        let combats = [
            Combat {
                id: 6,
                home: 10,
                registry_ordinal: 0,
            },
            Combat {
                id: 7,
                home: 10,
                registry_ordinal: 1,
            },
            Combat {
                id: 8,
                home: 10,
                registry_ordinal: 2,
            },
        ];
        let direct = [Direct {
            province: 10,
            combats: vec![6, 7, 8],
        }];
        let kinds = kinds();
        let kind = |province| kinds.get(&province).copied();
        let locations = BTreeMap::new();
        let out = derive(
            Sources {
                combats: &combats,
                combats_complete: true,
                direct: &direct,
                direct_complete: true,
                transfers: Some(&[]),
                units: Some(&[]),
                unit_locations: Some(&locations),
                province_kind: &kind,
            },
            &|id| (id != 7).then_some(true),
            &|_| Some(true),
        );
        assert_eq!(out.spillover_gap, None);
        assert_eq!(out.admission_gaps, [(10, 7)]);
        assert_eq!(
            out.by_province[&10]
                .iter()
                .map(|row| row.combat)
                .collect::<Vec<_>>(),
            [6, 8]
        );
        assert_eq!(
            out.by_province[&10]
                .iter()
                .map(|row| row.ordinal)
                .collect::<Vec<_>>(),
            [Some(0), None]
        );
    }

    /// A combat known to be unengaged is skipped before its units are looked up, so its
    /// unresolved locations cannot unsettle an unrelated direct icon; a transfer whose units the
    /// recorder dropped as stale has an empty unit row and spills nowhere.
    #[test]
    fn an_unengaged_combat_with_unresolved_units_leaves_other_icons_placed() {
        let combats = [
            Combat {
                id: 7,
                home: 10,
                registry_ordinal: 0,
            },
            Combat {
                id: 8,
                home: 11,
                registry_ordinal: 1,
            },
        ];
        let direct = [Direct {
            province: 10,
            combats: vec![7],
        }];
        let transfers = [
            Transfers {
                combat: 7,
                transfers: vec![107],
            },
            Transfers {
                combat: 8,
                transfers: vec![108],
            },
        ];
        // Combat 7's transfer kept no live unit; combat 8's unit has no recorded location.
        let units = [
            Units {
                transfer: 107,
                units: vec![],
            },
            Units {
                transfer: 108,
                units: vec![208],
            },
        ];
        let kinds = kinds();
        let kind = |province| kinds.get(&province).copied();
        let locations = BTreeMap::new();
        let out = derive(
            Sources {
                combats: &combats,
                combats_complete: true,
                direct: &direct,
                direct_complete: true,
                transfers: Some(&transfers),
                units: Some(&units),
                unit_locations: Some(&locations),
                province_kind: &kind,
            },
            &|_| Some(true),
            &|id| Some(id == 7),
        );
        assert_eq!(out.spillover_gap, None);
        assert_eq!(out.by_province[&10][0].combat, 7);
        assert_eq!(out.by_province[&10][0].ordinal, Some(0));
        assert_eq!(out.by_province.len(), 1);
    }

    #[test]
    fn authored_integer_gui_height_sets_positive_screen_stack_offset() {
        assert_eq!(screen_y_offset(0, 32.0), Some(0.0));
        assert_eq!(screen_y_offset(2, 32.0), Some(64.0));
        assert_eq!(screen_y_offset(2, 32.5), None);
        assert_eq!(screen_y_offset(u32::MAX, 32.0), None);
    }
}
