//! Native regional control sides and complete cached supremacy snapshots.
use super::{directed_wars::Owner, visibility::Families};
use replay_data::{rows::AirCountryRelationRow, rows::NavalSupremacyRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};
type Fact<T> = Result<T, String>;
const NAVAL_BLOCKADE_RELATION_TYPE: u32 = 0x27f8;

pub struct Snapshot<'a> {
    pub rows: Vec<&'a NavalSupremacyRow>,
    pub friendly: i64,
    pub enemy: i64,
    pub ratio_friendly: i64,
    pub ratio_enemy: i64,
}
impl<'a> Snapshot<'a> {
    pub fn read(
        frame: &'a DecodedFrame,
        viewer: u32,
        region: u32,
        minimum: i64,
        links: &BTreeMap<u32, u32>,
    ) -> Fact<Self> {
        let rows = regional_rows(frame, region)?;
        let (friendly, enemy) = control_rows(frame, viewer, &rows, links)?;
        let (ratio_friendly, ratio_enemy) = share_rows(frame, viewer, &rows, minimum, links)?;
        Ok(Self {
            rows,
            friendly,
            enemy,
            ratio_friendly,
            ratio_enemy,
        })
    }
    pub fn control(&self, minimum: i64, ratio: i64) -> Fact<u8> {
        control(self.friendly, self.enemy, minimum, ratio)
    }
    pub fn share(&self) -> Fact<i64> {
        share(self.ratio_friendly, self.ratio_enemy)
    }
}

/// The explanatory supremacy share from its friendly and enemy operands: the
/// native fixed5 quotient of friendly over their wrapping sum, zero when the
/// sum is zero.
pub fn share(friendly: i64, enemy: i64) -> Fact<i64> {
    let denominator = friendly.wrapping_add(enemy);
    if denominator == 0 {
        Ok(0)
    } else {
        quotient(friendly, denominator)
    }
}

/// Cached rows admitted only inside their supplying midnight epoch.
pub fn regional_rows(frame: &DecodedFrame, region: u32) -> Fact<Vec<&NavalSupremacyRow>> {
    let sampled = frame.tables[replay_data::rows::TableId::NavalSupremacy.index()]
        .sampled_at()
        .ok_or("NavalSupremacy acquisition provenance is unrecorded")?;
    if sampled > frame.hour || sampled.div_euclid(24) != frame.hour.div_euclid(24) {
        return Err("NavalSupremacy is outside its native daily cache epoch".into());
    }
    let all = frame
        .complete::<tables::NavalSupremacy>()
        .map_err(|e| e.to_string())?;
    let first = all.partition_point(|row| row.region < region);
    let end = first + all[first..].partition_point(|row| row.region == region);
    let rows: Vec<_> = all[first..end].iter().collect();
    Ok(rows)
}

/// Control operands remain independent of explanatory-roster failures.
pub fn control_sums(
    frame: &DecodedFrame,
    viewer: u32,
    region: u32,
    links: &BTreeMap<u32, u32>,
) -> Fact<(i64, i64)> {
    control_rows(frame, viewer, &regional_rows(frame, region)?, links)
}
/// Admissions reused across regions of one immutable frame and viewer.
pub struct ControlContext<'a> {
    frame: &'a DecodedFrame,
    links: &'a BTreeMap<u32, u32>,
    viewer: u32,
    factions: OnceLock<Fact<Families>>,
    subjects: OnceLock<Fact<Families>>,
    owners: BTreeMap<u32, Fact<Owner<'a>>>,
}
impl<'a> ControlContext<'a> {
    pub fn new(frame: &'a DecodedFrame, viewer: u32, links: &'a BTreeMap<u32, u32>) -> Self {
        Self {
            frame,
            viewer,
            links,
            factions: OnceLock::new(),
            subjects: OnceLock::new(),
            owners: BTreeMap::new(),
        }
    }
    pub fn sums(&mut self, region: u32) -> Fact<(i64, i64)> {
        self.rows(&regional_rows(self.frame, region)?)
    }
    fn rows(&mut self, rows: &[&NavalSupremacyRow]) -> Fact<(i64, i64)> {
        let (mut friendly, mut enemy) = (0_i64, 0_i64);
        for row in rows {
            if row.current == 0 {
                continue;
            }
            let slot = canonical(self.links, row.country)?;
            if family(
                self.frame,
                self.links,
                slot,
                self.viewer,
                &self.factions,
                &self.subjects,
            )? {
                friendly = friendly.wrapping_add(row.current);
            } else {
                let owner = self
                    .owners
                    .entry(row.country)
                    .or_insert_with(|| Owner::new(self.frame, row.country))
                    .as_ref()
                    .map_err(Clone::clone)?;
                if owner.pointer_active_slot(self.viewer)? {
                    enemy = enemy.wrapping_add(row.current);
                }
            }
        }
        Ok((friendly, enemy))
    }
}
fn control_rows(
    frame: &DecodedFrame,
    viewer: u32,
    rows: &[&NavalSupremacyRow],
    links: &BTreeMap<u32, u32>,
) -> Fact<(i64, i64)> {
    ControlContext::new(frame, viewer, links).rows(rows)
}

/// Explanatory ratio operands retain ordered enemy occurrences and their floor.
pub fn share_sums(
    frame: &DecodedFrame,
    viewer: u32,
    region: u32,
    minimum: i64,
    links: &BTreeMap<u32, u32>,
) -> Fact<(i64, i64)> {
    share_rows(
        frame,
        viewer,
        &regional_rows(frame, region)?,
        minimum,
        links,
    )
}
fn share_rows(
    frame: &DecodedFrame,
    viewer: u32,
    rows: &[&NavalSupremacyRow],
    minimum: i64,
    links: &BTreeMap<u32, u32>,
) -> Fact<(i64, i64)> {
    if rows.iter().all(|row| row.current == 0) {
        return Ok((0, 0_i64.max(minimum)));
    }
    // The explanatory ratio uses the country group's unique membership,
    // independently of the control's directed family predicate.
    let mut group = BTreeSet::from([viewer]);
    for row in frame
        .complete::<tables::AirCountryRelations>()
        .map_err(|e| e.to_string())?
    {
        if row.country == viewer
            && row.kind == AirCountryRelationRow::ALLY
            && (row.other as i32) > 0
        {
            group.insert(canonical(links, row.other)?);
        }
    }
    let mut family_group = Families::default();
    family_group
        .read_factions(frame, links)
        .map_err(str::to_owned)?;
    family_group
        .read_subjects(frame, links)
        .map_err(str::to_owned)?;
    for &slot in links.values() {
        if family_group.family(slot, viewer) {
            group.insert(slot);
        }
    }
    let mut ratio_friendly = 0_i64;
    let mut first_current = BTreeMap::new();
    for row in rows {
        let slot = canonical(links, row.country)?;
        first_current.entry(slot).or_insert(row.current);
        if group.contains(&slot) {
            ratio_friendly = ratio_friendly.wrapping_add(row.current);
        }
    }
    let mut ratio_enemy = 0_i64;
    let value = |raw| -> Fact<i64> {
        Ok(first_current
            .get(&canonical(links, raw)?)
            .copied()
            .unwrap_or(0))
    };
    for raw in raw_roster(frame, viewer, AirCountryRelationRow::ENEMY)? {
        ratio_enemy = ratio_enemy.wrapping_add(value(raw)?);
    }
    let tuples = frame
        .complete::<tables::RelationTuples>()
        .map_err(|e| e.to_string())?;
    let blockades: BTreeSet<_> = tuples
        .iter()
        .filter(|tuple| {
            tuple.type_id == NAVAL_BLOCKADE_RELATION_TYPE
                && canonical(links, tuple.second_country) == Ok(viewer)
        })
        .filter_map(|tuple| canonical(links, tuple.first_country).ok())
        .collect();
    for row in rows {
        if canonical(links, row.country).is_ok_and(|slot| blockades.contains(&slot)) {
            ratio_enemy = ratio_enemy.wrapping_add(row.current);
        }
    }
    Ok((ratio_friendly, ratio_enemy.max(minimum)))
}

/// The raw recorded diplomacy vector, preserving aliases and occurrences.
pub fn raw_roster(frame: &DecodedFrame, viewer: u32, kind: u8) -> Fact<Vec<u32>> {
    let mut rows: Vec<_> = frame
        .complete::<tables::AirCountryRelations>()
        .map_err(|e| e.to_string())?
        .iter()
        .filter(|row| row.country == viewer && row.kind == kind)
        .collect();
    rows.sort_by_key(|row| row.ordinal);
    if rows
        .iter()
        .enumerate()
        .any(|(index, row)| row.ordinal as usize != index)
    {
        return Err("country diplomacy roster ordinal is incomplete".into());
    }
    Ok(rows.into_iter().map(|row| row.other).collect())
}

pub fn quotient(numerator: i64, denominator: i64) -> Fact<i64> {
    if denominator == 0 {
        return Ok(100_000);
    }
    if (numerator as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
        return Ok(0xffff_ffff);
    }
    numerator
        .wrapping_mul(100_000)
        .checked_div(denominator)
        .ok_or_else(|| "native regional dominance division overflows".into())
}

pub fn control(friendly: i64, enemy: i64, minimum: i64, ratio: i64) -> Fact<u8> {
    let threshold = crate::naval_math::dominance_value(minimum, ratio);
    Ok(
        if (quotient(friendly, enemy)? < ratio && quotient(enemy, friendly)? < ratio)
            || (friendly < threshold && enemy < threshold)
        {
            0
        } else if friendly >= enemy {
            1
        } else {
            2
        },
    )
}

fn family(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    source: u32,
    viewer: u32,
    factions: &OnceLock<Fact<Families>>,
    subjects: &OnceLock<Fact<Families>>,
) -> Fact<bool> {
    // Positive faction membership is authenticated in both directions;
    // Country0's cached faction may omit itself from the roster.
    if source == viewer && source != 0 {
        return Ok(true);
    }
    // Test the target faction roster before the cached subject pointer.
    let zero_faction = if viewer == 0 {
        let [row] = frame
            .complete::<tables::ReservedCountryNaming>()
            .map_err(|e| e.to_string())?
        else {
            return Err("Country0 naval family naming singleton missing or duplicated".into());
        };
        Some(
            row.faction
                .require("ReservedCountryNaming.faction")?
                .ok_or("Country0 naval family faction capture refused")?,
        )
    } else {
        None
    };
    let roster_member = if zero_faction == Some(0) {
        // Native null faction yields a singleton target, without a roster read.
        source == 0
    } else {
        let faction = factions
            .get_or_init(|| {
                let mut family = Families::default();
                family.read_factions(frame, links).map_err(str::to_owned)?;
                Ok(family)
            })
            .as_ref()
            .map_err(Clone::clone)?;
        if let Some(handle) = zero_faction {
            faction.member_of_faction(source, handle)
        } else {
            faction.same_faction(source, viewer)
        }
    };
    if roster_member {
        return Ok(true);
    }
    if source == 0 || viewer == 0 {
        return zero_subject(frame, links, source, viewer);
    }
    let family = subjects
        .get_or_init(|| {
            let mut family = Families::default();
            family.read_subjects(frame, links).map_err(str::to_owned)?;
            Ok(family)
        })
        .as_ref()
        .map_err(Clone::clone)?;
    Ok(family.family(source, viewer))
}

fn zero_subject(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    source: u32,
    viewer: u32,
) -> Fact<bool> {
    // The positive Subjects census excludes canonical Country0. Exact hourly visibility packets
    // supply its cached subject endpoints.
    if source == 0 {
        let rows = frame
            .complete::<tables::ReservedCountryNaming>()
            .map_err(|e| e.to_string())?;
        let [row] = rows else {
            return Err("Country0 naval family naming singleton missing or duplicated".into());
        };
        let packet = row
            .reverse_visibility
            .require("ReservedCountryNaming.reverse_visibility")?
            .as_ref()
            .ok_or("Country0 naval family reverse visibility refused")?;
        if !packet.sample_hour.fresh_at(frame.hour) {
            return Err("Country0 naval family reverse visibility stale".into());
        }
        let cell = packet
            .cells
            .get(viewer as usize)
            .and_then(Option::as_ref)
            .ok_or("Country0 naval family target cell missing")?;
        let subject = cell.subject.as_ref().ok_or(
            "Country0 naval family cached subject pointer not recorded at the reached branch",
        )?;
        return crate::transport_subject_access::matches(
            subject,
            cell.target
                .ok_or_else(|| "Country0 naval family target refused".into()),
            |raw| canonical(links, raw),
        );
    }
    let country = frame
        .complete::<tables::Countries>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|r| r.canonical_slot == source)
        .ok_or("naval family source country missing")?;
    // RelationLinks' complete-owner certificate excludes target Country0. The source's hourly forward
    // visibility packet supplies the subject pointer for this branch.
    let row = frame
        .complete::<tables::CountryCounterInputs>()
        .map_err(|e| e.to_string())?;
    let row = row
        .iter()
        .find(|row| row.country == country.id)
        .ok_or("Country0 naval family forward source missing")?;
    let packet = row
        .forward_visibility
        .require("CountryCounterInputs.forward_visibility")?
        .as_ref()
        .ok_or("Country0 naval family forward visibility refused")?;
    let subject = packet
        .subject
        .as_ref()
        .ok_or("naval family cached subject pointer not recorded at the reached branch")?;
    crate::transport_subject_access::matches(
        subject,
        packet
            .target
            .ok_or_else(|| "naval family target refused".into()),
        |raw| canonical(links, raw),
    )
}

fn canonical(links: &BTreeMap<u32, u32>, raw: u32) -> Fact<u32> {
    if raw == 0 {
        Ok(0)
    } else {
        links
            .get(&raw)
            .copied()
            .ok_or_else(|| format!("naval dominance raw country link {raw} is missing"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn control_needs_both_ratio_and_minimum_while_zero_denominator_keeps_native_default() {
        assert_eq!(control(100, 0, 100_000, 60_000).unwrap(), 0);
        assert_eq!(control(300_000, 100_000, 100_000, 60_000).unwrap(), 1);
        assert_eq!(control(100_000, 300_000, 100_000, 60_000).unwrap(), 2);
        assert_eq!(quotient(3, 0).unwrap(), 100_000);
        assert_eq!(quotient(i64::MIN, 1).unwrap(), 0xffff_ffff);
    }
    #[test]
    fn an_empty_cached_region_needs_no_unreached_diplomacy_tables() {
        use replay_data::{rows::TableId, rows::TableRows};
        use replay_frame::frame::TableView;
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 25,
            metrics: Default::default(),
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "unreached".into(),
                }
                .into()
            }),
        };
        frame.tables[TableId::NavalSupremacy.index()] = replay_frame::frame::SharedView::from(
            TableView::Complete(TableRows::NavalSupremacy(vec![])),
        )
        .with_sampled_at(Some(24));
        let links = BTreeMap::new();
        let empty = Snapshot::read(&frame, 1, 7, 100_000, &links).unwrap();
        assert_eq!(empty.control(100_000, 60_000).unwrap(), 0);
        assert_eq!(empty.share().unwrap(), 0);
        frame.hour = 48;
        assert!(Snapshot::read(&frame, 1, 7, 100_000, &links).is_err());
    }
    #[test]
    fn raw_rosters_use_ordinals_and_keep_alias_occurrences() {
        use replay_data::{rows::TableId, rows::TableRows};
        use replay_frame::frame::TableView;
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            metrics: Default::default(),
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "unreached".into(),
                }
                .into()
            }),
        };
        frame.tables[TableId::AirCountryRelations.index()] =
            TableView::Complete(TableRows::AirCountryRelations(vec![
                AirCountryRelationRow {
                    country: 1,
                    kind: 2,
                    ordinal: 1,
                    other: 22,
                },
                AirCountryRelationRow {
                    country: 1,
                    kind: 2,
                    ordinal: 0,
                    other: 22,
                },
            ]))
            .into();
        assert_eq!(raw_roster(&frame, 1, 2).unwrap(), [22, 22]);
        assert!(raw_roster(&frame, 1, 1).unwrap().is_empty());
    }
}
