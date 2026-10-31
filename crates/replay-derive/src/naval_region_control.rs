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
        let mut context = ControlContext::new(frame, viewer, links);
        let (friendly, enemy) = context.rows(&rows)?;
        let (ratio_friendly, ratio_enemy) = context.share_rows(&rows, minimum)?;
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
#[cfg(any(test, feature = "test-support"))]
pub fn control_sums(
    frame: &DecodedFrame,
    viewer: u32,
    region: u32,
    links: &BTreeMap<u32, u32>,
) -> Fact<(i64, i64)> {
    control_rows(frame, viewer, &regional_rows(frame, region)?, links)
}
/// Admissions reused across regions of one immutable frame and viewer: the control's directed
/// families and owners, and the explanatory share's group, enemy roster and blockaders.
pub struct ControlContext<'a> {
    frame: &'a DecodedFrame,
    links: &'a BTreeMap<u32, u32>,
    viewer: u32,
    factions: OnceLock<Fact<Families>>,
    subjects: OnceLock<Fact<Families>>,
    owners: BTreeMap<u32, Fact<Owner<'a>>>,
    group: OnceLock<Fact<BTreeSet<u32>>>,
    enemies: OnceLock<Fact<Vec<u32>>>,
    blockaders: OnceLock<Fact<BTreeSet<u32>>>,
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
            group: OnceLock::new(),
            enemies: OnceLock::new(),
            blockaders: OnceLock::new(),
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
    /// Explanatory ratio operands retain ordered enemy occurrences and their floor.
    pub fn share(&self, region: u32, minimum: i64) -> Fact<(i64, i64)> {
        self.share_rows(&regional_rows(self.frame, region)?, minimum)
    }
    fn share_rows(&self, rows: &[&NavalSupremacyRow], minimum: i64) -> Fact<(i64, i64)> {
        if rows.iter().all(|row| row.current == 0) {
            return Ok((0, 0_i64.max(minimum)));
        }
        let (frame, links, viewer) = (self.frame, self.links, self.viewer);
        // The explanatory ratio uses the country group's unique membership,
        // independently of the control's directed family predicate.
        let group = self
            .group
            .get_or_init(|| {
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
                Ok(group)
            })
            .as_ref()
            .map_err(Clone::clone)?;
        let mut ratio_friendly = 0_i64;
        let mut first_current = BTreeMap::new();
        for row in rows {
            let slot = canonical(links, row.country)?;
            first_current.entry(slot).or_insert(row.current);
            if group.contains(&slot) {
                ratio_friendly = ratio_friendly.wrapping_add(row.current);
            }
        }
        let enemies = self
            .enemies
            .get_or_init(|| {
                raw_roster(frame, viewer, AirCountryRelationRow::ENEMY)?
                    .into_iter()
                    .map(|raw| canonical(links, raw))
                    .collect()
            })
            .as_ref()
            .map_err(Clone::clone)?;
        let mut ratio_enemy = 0_i64;
        for slot in enemies {
            ratio_enemy = ratio_enemy.wrapping_add(first_current.get(slot).copied().unwrap_or(0));
        }
        let blockaders = self
            .blockaders
            .get_or_init(|| {
                Ok(frame
                    .complete::<tables::RelationTuples>()
                    .map_err(|e| e.to_string())?
                    .iter()
                    .filter(|tuple| {
                        tuple.type_id == NAVAL_BLOCKADE_RELATION_TYPE
                            && canonical(links, tuple.second_country) == Ok(viewer)
                    })
                    .filter_map(|tuple| canonical(links, tuple.first_country).ok())
                    .collect())
            })
            .as_ref()
            .map_err(Clone::clone)?;
        for row in rows {
            if canonical(links, row.country).is_ok_and(|slot| blockaders.contains(&slot)) {
                ratio_enemy = ratio_enemy.wrapping_add(row.current);
            }
        }
        Ok((ratio_friendly, ratio_enemy.max(minimum)))
    }
}
#[cfg(any(test, feature = "test-support"))]
fn control_rows(
    frame: &DecodedFrame,
    viewer: u32,
    rows: &[&NavalSupremacyRow],
    links: &BTreeMap<u32, u32>,
) -> Fact<(i64, i64)> {
    ControlContext::new(frame, viewer, links).rows(rows)
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
    #[test]
    fn one_context_shares_every_region_as_a_fresh_one_does() {
        use replay_data::rows::{RelationTupleRow, TableId, TableRows};
        use replay_frame::frame::{SharedView, TableView};
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
        let current = |region, country, current| NavalSupremacyRow {
            region,
            country,
            current,
            ..Default::default()
        };
        // Region 7 holds the viewer, its ally, its enemy and a blockader; 8 an enemy alone; 9 nothing.
        frame.tables[TableId::NavalSupremacy.index()] =
            SharedView::from(TableView::Complete(TableRows::NavalSupremacy(vec![
                current(7, 101, 10),
                current(7, 102, 5),
                current(7, 103, 7),
                current(7, 104, 3),
                current(8, 102, 0),
                current(8, 103, 4),
                current(9, 103, 0),
            ])))
            .with_sampled_at(Some(24));
        let links = BTreeMap::from([(101, 1), (102, 2), (103, 3), (104, 4)]);
        let regions = [7, 8, 9];
        let shares = |frame: &DecodedFrame| {
            let shared = ControlContext::new(frame, 1, &links);
            let once: Vec<_> = regions.iter().map(|&r| shared.share(r, 2)).collect();
            let fresh: Vec<_> = regions
                .iter()
                .map(|&r| ControlContext::new(frame, 1, &links).share(r, 2))
                .collect();
            assert_eq!(once, fresh);
            once
        };
        // Without the diplomacy tables, a region holding currents refuses and an empty one does not.
        let refused = shares(&frame);
        assert!(refused[0].is_err() && refused[1].is_err());
        assert_eq!(refused[2], Ok((0, 2)));
        frame.tables[TableId::AirCountryRelations.index()] =
            TableView::Complete(TableRows::AirCountryRelations(vec![
                AirCountryRelationRow {
                    country: 1,
                    kind: AirCountryRelationRow::ALLY,
                    ordinal: 0,
                    other: 102,
                },
                AirCountryRelationRow {
                    country: 1,
                    kind: AirCountryRelationRow::ENEMY,
                    ordinal: 0,
                    other: 103,
                },
            ]))
            .into();
        frame.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(vec![])).into();
        frame.tables[TableId::Subjects.index()] =
            TableView::Complete(TableRows::Subjects(vec![])).into();
        frame.tables[TableId::RelationTuples.index()] =
            TableView::Complete(TableRows::RelationTuples(vec![RelationTupleRow {
                type_id: NAVAL_BLOCKADE_RELATION_TYPE,
                first_country: 104,
                second_country: 101,
                ..Default::default()
            }]))
            .into();
        // Friendly: the viewer and its ally; enemy: the roster's enemy and the blockader, floored.
        assert_eq!(shares(&frame), [Ok((15, 10)), Ok((0, 4)), Ok((0, 2))]);
    }
}
