use super::{directed_wars::Owner, visibility::Families};
use replay_data::{rows::AirCountryRelationRow, rows::NavalSupremacyRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};
type Fact<T> = Result<T, String>;
pub const NAVAL_BLOCKADE_RELATION_TYPE: u32 = 0x27f8;

pub struct Snapshot<'a> {
    pub rows: Vec<&'a NavalSupremacyRow>,
    pub friendly: i64,
    pub enemy: i64,
    pub ratio_friendly: i64,
    pub ratio_enemy: i64,
    pub raw_enemy: i64,
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
        let context = ControlContext::new(frame, viewer, links);
        let (friendly, enemy) = context.rows(&rows)?;
        let (ratio_friendly, raw_enemy) = context.share_raw(&rows)?;
        Ok(Self {
            rows,
            friendly,
            enemy,
            ratio_friendly,
            ratio_enemy: raw_enemy.max(minimum),
            raw_enemy,
        })
    }
    pub fn control(&self, minimum: i64, ratio: i64) -> Fact<u8> {
        control(self.friendly, self.enemy, minimum, ratio)
    }
    pub fn share(&self) -> Fact<i64> {
        share(self.ratio_friendly, self.ratio_enemy)
    }
}

pub fn share(friendly: i64, enemy: i64) -> Fact<i64> {
    let denominator = friendly.wrapping_add(enemy);
    if denominator == 0 {
        Ok(0)
    } else {
        quotient(friendly, denominator)
    }
}

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

#[cfg(any(test, feature = "test-support"))]
pub fn control_sums(
    frame: &DecodedFrame,
    viewer: u32,
    region: u32,
    links: &BTreeMap<u32, u32>,
) -> Fact<(i64, i64)> {
    control_rows(frame, viewer, &regional_rows(frame, region)?, links)
}
pub struct World<'a> {
    frame: &'a DecodedFrame,
    links: &'a BTreeMap<u32, u32>,
    factions: OnceLock<Fact<Families>>,
    subjects: OnceLock<Fact<Families>>,
    owners: BTreeMap<u32, OnceLock<Fact<Owner<'a>>>>,
    group: OnceLock<Fact<Group>>,
    blockaders: OnceLock<Fact<BTreeMap<u32, BTreeSet<u32>>>>,
}

struct Group {
    families: Families,
    live: BTreeSet<u32>,
    members: BTreeMap<u64, BTreeSet<u32>>,
}

impl<'a> World<'a> {
    pub fn new(frame: &'a DecodedFrame, links: &'a BTreeMap<u32, u32>) -> Self {
        let owners = frame
            .complete::<tables::NavalSupremacy>()
            .map(|rows| rows.iter().map(|row| (row.country, OnceLock::new())).collect())
            .unwrap_or_default();
        Self {
            frame,
            links,
            factions: OnceLock::new(),
            subjects: OnceLock::new(),
            owners,
            group: OnceLock::new(),
            blockaders: OnceLock::new(),
        }
    }

    fn owner(&self, raw: u32) -> Fact<&Owner<'a>> {
        self.owners
            .get(&raw)
            .ok_or_else(|| format!("naval dominance owner {raw} holds no NavalSupremacy row"))?
            .get_or_init(|| Owner::new(self.frame, raw))
            .as_ref()
            .map_err(Clone::clone)
    }

    fn group(&self, viewer: u32) -> Fact<BTreeSet<u32>> {
        let mut group = BTreeSet::from([viewer]);
        for row in rows_of(self.frame, viewer, AirCountryRelationRow::ALLY)? {
            if (row.other as i32) > 0 {
                group.insert(canonical(self.links, row.other)?);
            }
        }
        let shared = self
            .group
            .get_or_init(|| {
                let mut families = Families::default();
                families
                    .read_factions(self.frame, self.links)
                    .map_err(str::to_owned)?;
                families
                    .read_subjects(self.frame, self.links)
                    .map_err(str::to_owned)?;
                let live: BTreeSet<u32> = self.links.values().copied().collect();
                let mut members: BTreeMap<u64, BTreeSet<u32>> = BTreeMap::new();
                for (slot, faction) in families.members() {
                    if live.contains(&slot) {
                        members.entry(faction).or_default().insert(slot);
                    }
                }
                Ok(Group {
                    families,
                    live,
                    members,
                })
            })
            .as_ref()
            .map_err(Clone::clone)?;
        group.extend(
            shared
                .families
                .partners(viewer)
                .filter(|slot| shared.live.contains(slot)),
        );
        if let Some(faction) = shared.families.faction(viewer) {
            group.extend(shared.members.get(&faction).into_iter().flatten());
        }
        Ok(group)
    }

    fn blockaders(&self, viewer: u32) -> Fact<Option<&BTreeSet<u32>>> {
        let by = self
            .blockaders
            .get_or_init(|| {
                let mut by: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
                for tuple in self
                    .frame
                    .complete::<tables::RelationTuples>()
                    .map_err(|e| e.to_string())?
                {
                    if tuple.type_id != NAVAL_BLOCKADE_RELATION_TYPE {
                        continue;
                    }
                    if let (Ok(blockaded), Ok(blockader)) = (
                        canonical(self.links, tuple.second_country),
                        canonical(self.links, tuple.first_country),
                    ) {
                        by.entry(blockaded).or_default().insert(blockader);
                    }
                }
                Ok(by)
            })
            .as_ref()
            .map_err(Clone::clone)?;
        Ok(by.get(&viewer))
    }
}

enum Held<'w, 'a> {
    Own(Box<World<'a>>),
    Shared(&'w World<'a>),
}

impl<'a> std::ops::Deref for Held<'_, 'a> {
    type Target = World<'a>;
    fn deref(&self) -> &World<'a> {
        match self {
            Self::Own(world) => world,
            Self::Shared(world) => world,
        }
    }
}

pub struct ControlContext<'w, 'a> {
    world: Held<'w, 'a>,
    viewer: u32,
    group: OnceLock<Fact<BTreeSet<u32>>>,
    enemies: OnceLock<Fact<Vec<u32>>>,
}
impl<'a> ControlContext<'a, 'a> {
    pub fn new(frame: &'a DecodedFrame, viewer: u32, links: &'a BTreeMap<u32, u32>) -> Self {
        Self::held(Held::Own(Box::new(World::new(frame, links))), viewer)
    }
}
impl<'w, 'a> ControlContext<'w, 'a> {
    pub fn shared(world: &'w World<'a>, viewer: u32) -> Self {
        Self::held(Held::Shared(world), viewer)
    }

    fn held(world: Held<'w, 'a>, viewer: u32) -> Self {
        Self {
            world,
            viewer,
            group: OnceLock::new(),
            enemies: OnceLock::new(),
        }
    }

    pub fn sums(&mut self, region: u32) -> Fact<(i64, i64)> {
        self.rows(&regional_rows(self.world.frame, region)?)
    }
    fn rows(&self, rows: &[&NavalSupremacyRow]) -> Fact<(i64, i64)> {
        let world = &*self.world;
        let (mut friendly, mut enemy) = (0_i64, 0_i64);
        for row in rows {
            if row.current == 0 {
                continue;
            }
            let slot = canonical(world.links, row.country)?;
            if family(
                world.frame,
                world.links,
                slot,
                self.viewer,
                &world.factions,
                &world.subjects,
            )? {
                friendly = friendly.wrapping_add(row.current);
            } else if world.owner(row.country)?.pointer_active_slot(self.viewer)? {
                enemy = enemy.wrapping_add(row.current);
            }
        }
        Ok((friendly, enemy))
    }
    pub fn share(&self, region: u32, minimum: i64) -> Fact<(i64, i64)> {
        self.share_rows(&regional_rows(self.world.frame, region)?, minimum)
    }
    fn share_rows(&self, rows: &[&NavalSupremacyRow], minimum: i64) -> Fact<(i64, i64)> {
        let (friendly, enemy) = self.share_raw(rows)?;
        Ok((friendly, enemy.max(minimum)))
    }
    fn share_raw(&self, rows: &[&NavalSupremacyRow]) -> Fact<(i64, i64)> {
        if rows.iter().all(|row| row.current == 0) {
            return Ok((0, 0));
        }
        let (world, viewer) = (&*self.world, self.viewer);
        let links = world.links;
        let group = self
            .group
            .get_or_init(|| world.group(viewer))
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
                raw_roster(world.frame, viewer, AirCountryRelationRow::ENEMY)?
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
        if let Some(blockaders) = world.blockaders(viewer)? {
            for row in rows {
                if canonical(links, row.country).is_ok_and(|slot| blockaders.contains(&slot)) {
                    ratio_enemy = ratio_enemy.wrapping_add(row.current);
                }
            }
        }
        Ok((ratio_friendly, ratio_enemy))
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

fn rows_of(frame: &DecodedFrame, country: u32, kind: u8) -> Fact<&[AirCountryRelationRow]> {
    let rows = frame
        .complete::<tables::AirCountryRelations>()
        .map_err(|e| e.to_string())?;
    let first = rows.partition_point(|row| (row.country, row.kind) < (country, kind));
    let end = first + rows[first..].partition_point(|row| (row.country, row.kind) == (country, kind));
    Ok(&rows[first..end])
}

pub fn raw_roster(frame: &DecodedFrame, viewer: u32, kind: u8) -> Fact<Vec<u32>> {
    let mut rows: Vec<_> = rows_of(frame, viewer, kind)?.iter().collect();
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
    if source == viewer && source != 0 {
        return Ok(true);
    }
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

pub fn canonical(links: &BTreeMap<u32, u32>, raw: u32) -> Fact<u32> {
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
        use replay_data::rows::{FactionMemberRow, RelationTupleRow, TableId, TableRows};
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
        assert_eq!(shares(&frame), [Ok((15, 10)), Ok((0, 4)), Ok((0, 2))]);
        frame.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(vec![
                FactionMemberRow { country: 103, faction: 9, ordinal: 0 },
                FactionMemberRow { country: 104, faction: 9, ordinal: 1 },
            ]))
            .into();
        let world = World::new(&frame, &links);
        for viewer in [1, 2, 3, 4, 5] {
            let mut own = ControlContext::new(&frame, viewer, &links);
            let mut shared = ControlContext::shared(&world, viewer);
            for region in regions {
                assert_eq!(shared.sums(region), own.sums(region), "viewer {viewer} region {region}");
                assert_eq!(shared.share(region, 2), own.share(region, 2), "viewer {viewer} region {region}");
            }
        }
        assert_eq!(ControlContext::shared(&world, 3).share(7, 2), Ok((10, 2)));
    }
}
