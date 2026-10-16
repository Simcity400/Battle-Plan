//! Province placement for active naval combat map icons.
//!
//! first walks each province's ordered direct combat vector,
//! then the global combat registry. The second pass can add an engaged combat
//! to a sea province occupied by one of its transferred units. The two passes
//! are independent: an unresolved transfer chain cannot erase a direct icon.
//!
//! The recorded transfer units are live ones only ( skips a stale
//! unit handle, and the recorder does too), so a recorded unit without a
//! location is unresolved, not stale.

use std::collections::{BTreeMap, BTreeSet};

use installed_content::map::ProvinceKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Combat {
    pub id: u64,
    pub home: u32,
    pub registry_ordinal: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Direct {
    pub province: u32,
    pub combats: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transfers {
    pub combat: u64,
    pub transfers: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Units {
    pub transfer: u64,
    pub units: Vec<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Occurrence {
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

pub struct Sources<'a> {
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
pub struct Occurrences {
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
        // Advance the retained walk cursor on every admitted call, including deduplication hits.
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

/// Multiply the final category-nine ordinal by integer GUI height, with positive screen Y downward.
pub fn screen_y_offset(ordinal: u32, gui_height: f32) -> Option<f32> {
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

pub fn derive(
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
        // Test the engagement before it walks any transfer, so a combat known to
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
mod tests;
