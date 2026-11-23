use std::{collections::BTreeMap, sync::OnceLock};
use replay_derive::naval_math::dominance_value;

use content_reader_defines::defines::Defines;
use content_reader_map::map::{MapData, ProvinceKind};
use content_reader_key_names::terrain::TerrainDefinitions;
use content_reader_regions::regions::StrategicRegion;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;

use map_display_data::landscape::Attributes;
use replay_derive::directed_wars::Owner;
use viewer_foundation::visibility::Families;

pub type Fact<T> = Result<T, String>;

pub const BLOCKADE: u32 = 0x27f8;

#[derive(Clone)]
struct RegionContent {
    name: String,
    minimum: Fact<i64>,
    gradient: Option<u8>,
    has_sea: bool,
}

#[derive(Clone)]
pub struct Content {
    regions: BTreeMap<u32, RegionContent>,
    ratio: Fact<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fill {
    pub attributes: BTreeMap<u32, Attributes>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Supremacy {
    pub country: u32,
    pub current: i64,
    pub previous: i64,
}

pub trait Relations {
    fn links(&self) -> Result<&BTreeMap<u32, u32>, String>;
    fn family(&self, links: &BTreeMap<u32, u32>, source: u32, viewer: u32) -> Fact<bool>;
    fn war(&self, source: u32, viewer: u32) -> Fact<bool>;
    fn blockades(&self) -> Fact<&[(u32, u32)]>;
}

#[derive(Clone, Debug, Default)]
pub struct Sides(Vec<(u32, Fact<Side>)>);

impl Sides {
    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn reread(&mut self, relations: &dyn Relations, viewer: u32) -> Vec<u32> {
        let mut moved = Vec::new();
        for (raw, held) in &mut self.0 {
            let now = side(relations, *raw, viewer);
            if *held != now {
                *held = now;
                moved.push(*raw);
            }
        }
        moved
    }
}

impl Content {
    pub fn new(
        map: &MapData,
        regions: &BTreeMap<u32, StrategicRegion>,
        terrain: Result<&TerrainDefinitions, &str>,
        defines: &Defines,
    ) -> Self {
        Self {
            regions: regions.iter().map(|(&id, region)| {
                let minimum = terrain.map_err(str::to_owned).and_then(|terrain| {
                    let category = region.naval_terrain.as_ref().and_then(|name| terrain.category(name.as_bytes()))
                        .or_else(|| terrain.category(b"ocean"));
                    category.map(|index| terrain.categories[index].minimum_seazone_dominance)
                        .ok_or_else(|| format!("naval terrain {} and executable ocean fallback absent from recorded content", region.naval_terrain.as_deref().unwrap_or("ocean")))
                });
                let kinds = region.provinces.iter().filter_map(|id| map.definitions.get(id)).map(|p| (p.id, p.kind)).collect::<BTreeMap<_,_>>();
                let sea = kinds.values().filter(|&&kind| kind == ProvinceKind::Sea).count() as u64;
                let fraction = if kinds.is_empty() { 0 } else { sea * 100_000 / kinds.len() as u64 };
                (id, RegionContent { name:region.name_key.clone(), minimum, gradient:(fraction < 1_000).then_some(255), has_sea:sea != 0 })
            }).collect(),
            ratio: defines.fixed5("NNavy", "DOMINANCE_CONTROLLED_THRESHOLD_RATIO").map_err(|e| e.to_string()),
        }
    }

    pub fn read(&self, frame: &DecodedFrame, viewer: u32) -> Fill {
        let rows = frame
            .complete::<tables::NavalSupremacy>()
            .map_err(|e| e.to_string());
        let grouped = rows.as_ref().map(|rows| {
            let mut groups: BTreeMap<u32, Vec<Supremacy>> = BTreeMap::new();
            for row in *rows {
                groups.entry(row.region).or_default().push(Supremacy {
                    country: row.country,
                    current: row.current,
                    previous: row.previous,
                });
            }
            groups
        });
        let relations = FrameRelations {
            frame,
            links: frame
                .complete::<tables::CountryLinks>()
                .map(|rows| {
                    rows.iter()
                        .map(|r| (r.id, r.canonical_slot))
                        .collect::<BTreeMap<_, _>>()
                })
                .map_err(|e| e.to_string()),
            factions: OnceLock::new(),
            subjects: OnceLock::new(),
            blockades: OnceLock::new(),
        };
        let epoch = cache_epoch(
            frame.hour,
            frame.tables[replay_data::rows::TableId::NavalSupremacy.index()].sampled_at(),
        );
        let mut sides = Sides::default();
        let mut fill = Fill::default();
        let mut refused = Vec::new();
        for &id in self.regions.keys() {
            let values = grouped
                .as_ref()
                .map(|groups| groups.get(&id).map(Vec::as_slice).unwrap_or_default())
                .map_err(|reason| (*reason).clone());
            let (attributes, reason) =
                self.region(id, &epoch, values, &relations, viewer, &mut sides);
            refused.extend(reason);
            fill.attributes.insert(id, attributes);
        }
        fill.warnings.extend(withheld(&refused));
        fill
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_tests(regions: &[(u32, bool, i64)], ratio: i64) -> Self {
        Self {
            regions: regions
                .iter()
                .map(|&(id, has_sea, minimum)| {
                    let region = RegionContent { name: format!("region{id}"), minimum: Ok(minimum), gradient: None, has_sea };
                    (id, region)
                })
                .collect(),
            ratio: Ok(ratio),
        }
    }

    pub fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.regions.keys().copied()
    }

    pub fn sea(&self, id: u32) -> bool {
        self.regions.get(&id).is_some_and(|region| region.has_sea)
    }

    pub fn region(
        &self,
        id: u32,
        epoch: &Fact<()>,
        values: Result<&[Supremacy], String>,
        relations: &dyn Relations,
        viewer: u32,
        sides: &mut Sides,
    ) -> (Attributes, Option<String>) {
        let Some(RegionContent {
            name,
            minimum,
            gradient,
            has_sea,
        }) = self.regions.get(&id)
        else {
            return (Attributes::default(), None);
        };
        let result = (|| {
            if !has_sea {
                return Ok([0, 0]);
            }
            epoch.clone()?;
            let values = values?;
            classify(
                values,
                *minimum.as_ref().map_err(Clone::clone)?,
                *self.ratio.as_ref().map_err(Clone::clone)?,
                &mut |raw| match sides.0.binary_search_by_key(&raw, |held| held.0) {
                    Ok(at) => sides.0[at].1.clone(),
                    Err(at) => {
                        let value = side(relations, raw, viewer);
                        sides.0.insert(at, (raw, value.clone()));
                        value
                    }
                },
                &|raw| blockade(relations, raw, viewer),
            )
        })();
        let ([control, contested], reason) = match result {
            Ok(value) => (value, None),
            Err(reason) => ([0, 0], Some(format!("region {id} ({name}): {reason}"))),
        };
        (
            Attributes {
                control,
                contested,
                gradient: *gradient,
            },
            reason,
        )
    }
}

pub fn withheld(refused: &[String]) -> Option<String> {
    (!refused.is_empty()).then(|| {
        format!(
            "Naval dominance FX withheld for {} strategic regions: {}",
            refused.len(),
            refused.join("; ")
        )
    })
}

struct FrameRelations<'a> {
    frame: &'a DecodedFrame,
    links: Fact<BTreeMap<u32, u32>>,
    factions: OnceLock<Fact<Families>>,
    subjects: OnceLock<Fact<Families>>,
    blockades: OnceLock<Fact<Vec<(u32, u32)>>>,
}

impl Relations for FrameRelations<'_> {
    fn links(&self) -> Result<&BTreeMap<u32, u32>, String> {
        self.links.as_ref().map_err(Clone::clone)
    }

    fn family(&self, links: &BTreeMap<u32, u32>, source: u32, viewer: u32) -> Fact<bool> {
        family(self.frame, links, source, viewer, &self.factions, &self.subjects)
    }

    fn war(&self, source: u32, viewer: u32) -> Fact<bool> {
        Owner::new(self.frame, source)?.pointer_active_slot(viewer)
    }

    fn blockades(&self) -> Fact<&[(u32, u32)]> {
        self.blockades
            .get_or_init(|| {
                let rows = self
                    .frame
                    .complete::<tables::RelationTuples>()
                    .map_err(|e| e.to_string())?;
                Ok(rows
                    .iter()
                    .filter(|row| row.type_id == BLOCKADE)
                    .map(|row| (row.first_country, row.second_country))
                    .collect())
            })
            .as_deref()
            .map_err(Clone::clone)
    }
}

fn side(relations: &dyn Relations, raw: u32, viewer: u32) -> Fact<Side> {
    let links = relations.links()?;
    let slot = canonical(links, raw)?;
    if relations.family(links, slot, viewer)? {
        return Ok(Side::Friendly);
    }
    if relations.war(raw, viewer)? {
        Ok(Side::Enemy)
    } else {
        Ok(Side::Neutral)
    }
}

fn blockade(relations: &dyn Relations, raw: u32, viewer: u32) -> Fact<bool> {
    let links = relations.links()?;
    let source = canonical(links, raw)?;
    if source == 0 || viewer == 0 {
        return Err("naval blockade pointer for canonical Country0 is not recorded".into());
    }
    for &(first, second) in relations.blockades()? {
        if canonical(links, first)? == source && canonical(links, second)? == viewer {
            return Ok(true);
        }
    }
    Ok(false)
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
        return replay_derive::transport_subject_access::matches(
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
    replay_derive::transport_subject_access::matches(
        subject,
        packet
            .target
            .ok_or_else(|| "naval family target refused".into()),
        |raw| canonical(links, raw),
    )
}

pub fn cache_epoch(hour: replay_data::Hour, sampled: Option<replay_data::Hour>) -> Fact<()> {
    match sampled {
        Some(sample) if sample <= hour && sample.div_euclid(24) == hour.div_euclid(24) => Ok(()),
        Some(sample) => Err(format!(
            "NavalSupremacy sampled at hour {sample} cannot certify native midnight cache epoch at {hour}"
        )),
        None => Err("NavalSupremacy acquisition provenance is unrecorded".into()),
    }
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

pub fn subject(
    subject: &replay_data::rows::ZeroSubjectInputs,
    target: Fact<u32>,
    links: &BTreeMap<u32, u32>,
) -> Fact<bool> {
    replay_derive::transport_subject_access::matches(subject, target, |raw| canonical(links, raw))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Friendly,
    Enemy,
    Neutral,
}

fn quotient(numerator: i64, denominator: i64) -> Fact<i64> {
    if denominator == 0 {
        return Ok(100_000);
    }
    if (numerator as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
        return Ok(u32::MAX as i64);
    }
    numerator
        .wrapping_mul(100_000)
        .checked_div(denominator)
        .ok_or_else(|| "native naval dominance signed division would overflow".into())
}

fn classify(
    rows: &[Supremacy],
    minimum: i64,
    ratio: i64,
    side: &mut dyn FnMut(u32) -> Fact<Side>,
    blockade: &dyn Fn(u32) -> Fact<bool>,
) -> Fact<[u8; 2]> {
    let (mut friendly, mut enemy) = (0_i64, 0_i64);
    let mut highest = [None; 2];
    for row in rows {
        if highest[0].is_none_or(|value| row.current > value) {
            highest[1] = highest[0];
            highest[0] = Some(row.current);
        } else if highest[1].is_none_or(|value| row.current > value) {
            highest[1] = Some(row.current);
        }
        if row.current == 0 {
            continue;
        }
        match side(row.country)? {
            Side::Friendly => friendly = friendly.wrapping_add(row.current),
            Side::Enemy => enemy = enemy.wrapping_add(row.current),
            Side::Neutral => {}
        }
    }
    let threshold = dominance_value(minimum, ratio);
    let control = if (quotient(friendly, enemy)? < ratio && quotient(enemy, friendly)? < ratio)
        || (friendly < threshold && enemy < threshold)
    {
        0
    } else if friendly >= enemy {
        255
    } else {
        128
    };
    let rank = usize::from(control != 0);
    let Some(selected) = highest[rank] else {
        return Ok([control, 0]);
    };
    let mut contested = None;
    for row in rows.iter().filter(|r| r.current == selected) {
        let value = if row.current <= row.previous {
            0
        } else {
            match side(row.country)? {
                Side::Friendly => 255,
                Side::Enemy => 128,
                Side::Neutral => {
                    if blockade(row.country)? {
                        128
                    } else {
                        0
                    }
                }
            }
        };
        if contested.is_some_and(|old| old != value) {
            return Err(format!(
                "native sorted dominance rank {rank} has ambiguous equal-current countries"
            ));
        }
        contested = Some(value);
    }
    Ok([control, contested.unwrap_or(0)])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> DecodedFrame {
        DecodedFrame {
            branch: 1,
            hour: 7,
            metrics: Default::default(),
            tables: std::array::from_fn(|_| {
                replay_frame::frame::TableView::Unavailable {
                    reason: "unreached".into(),
                }
                .into()
            }),
        }
    }
    #[test]
    fn same_country_and_faction_do_not_demand_subject_or_war_capture() {
        use replay_data::{rows::FactionMemberRow, rows::TableId, rows::TableRows};
        use replay_frame::frame::TableView;
        let mut frame = frame();
        let links = [(11, 1), (22, 2)].into_iter().collect();
        assert_eq!(
            family(&frame, &links, 1, 1, &OnceLock::new(), &OnceLock::new()),
            Ok(true)
        );
        frame.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(vec![
                FactionMemberRow {
                    country: 11,
                    faction: 88,
                    ordinal: 0,
                },
                FactionMemberRow {
                    country: 22,
                    faction: 88,
                    ordinal: 1,
                },
            ]))
            .into();
        assert_eq!(
            family(&frame, &links, 1, 2, &OnceLock::new(), &OnceLock::new()),
            Ok(true)
        );
    }
    #[test]
    fn country_zero_subject_uses_recorded_pointer_and_preserves_refusal() {
        use replay_data::{
            rows::Recorded, rows::ReservedCountryNamingRow, rows::ReservedReverseVisibilityInputs,
            rows::ReverseZeroVisibilityInputs, rows::TableId, rows::TableRows,
            rows::ZeroSubjectInputs,
        };
        use replay_frame::frame::TableView;
        let mut frame = frame();
        let links = [(11, 1)].into_iter().collect();
        frame.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(Vec::new())).into();
        let naming = |subject| {
            TableView::Complete(TableRows::ReservedCountryNaming(vec![
                ReservedCountryNamingRow {
                    reverse_visibility: Recorded::Value(Some(ReservedReverseVisibilityInputs {
                        sample_hour: replay_data::rows::PacketHour::Stamped(7),
                        cells: vec![
                            None,
                            Some(ReverseZeroVisibilityInputs {
                                target: Some(11),
                                subject,
                                ..Default::default()
                            }),
                        ],
                    })),
                    ..Default::default()
                },
            ]))
            .into()
        };
        frame.tables[TableId::ReservedCountryNaming.index()] =
            naming(Some(ZeroSubjectInputs::default()));
        assert_eq!(
            family(&frame, &links, 0, 1, &OnceLock::new(), &OnceLock::new()),
            Ok(false)
        );
        frame.tables[TableId::ReservedCountryNaming.index()] = naming(Some(ZeroSubjectInputs {
            relation_type: Some(0x30d1),
            parties: Some([Some(0), Some(11)]),
        }));
        assert_eq!(
            family(&frame, &links, 0, 1, &OnceLock::new(), &OnceLock::new()),
            Ok(true)
        );
        frame.tables[TableId::ReservedCountryNaming.index()] = naming(None);
        assert!(
            family(&frame, &links, 0, 1, &OnceLock::new(), &OnceLock::new())
                .unwrap_err()
                .contains("subject pointer not recorded")
        );
    }

    #[test]
    fn country_zero_target_uses_cached_faction_without_requiring_own_membership() {
        use replay_data::{
            rows::FactionMemberRow, rows::Recorded, rows::ReservedCountryNamingRow,
            rows::ReservedReverseVisibilityInputs, rows::ReverseZeroVisibilityInputs,
            rows::TableId, rows::TableRows, rows::ZeroSubjectInputs,
        };
        use replay_frame::frame::TableView;
        let mut frame = frame();
        let links = [(11, 1)].into_iter().collect();
        let naming = |faction| {
            TableView::Complete(TableRows::ReservedCountryNaming(vec![
                ReservedCountryNamingRow {
                    faction,
                    reverse_visibility: Recorded::Value(Some(ReservedReverseVisibilityInputs {
                        sample_hour: replay_data::rows::PacketHour::Stamped(frame.hour),
                        cells: vec![Some(ReverseZeroVisibilityInputs {
                            target: Some(0),
                            subject: Some(ZeroSubjectInputs::default()),
                            ..Default::default()
                        })],
                    })),
                    ..Default::default()
                },
            ]))
            .into()
        };
        frame.tables[TableId::ReservedCountryNaming.index()] = naming(Recorded::Value(Some(88)));
        frame.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(vec![FactionMemberRow {
                country: 11,
                faction: 88,
                ordinal: 0,
            }]))
            .into();
        assert_eq!(
            family(&frame, &links, 1, 0, &OnceLock::new(), &OnceLock::new()),
            Ok(true)
        );
        assert_eq!(
            family(&frame, &links, 0, 0, &OnceLock::new(), &OnceLock::new()),
            Ok(false)
        );
        for refused in [Recorded::NotRecorded, Recorded::Value(None)] {
            frame.tables[TableId::ReservedCountryNaming.index()] = naming(refused);
            assert!(family(&frame, &links, 0, 0, &OnceLock::new(), &OnceLock::new()).is_err());
        }
    }

    #[test]
    fn country_zero_null_faction_skips_unavailable_roster() {
        use replay_data::{
            rows::CountryCounterInputRow, rows::CountryRow, rows::Recorded,
            rows::ReservedCountryNamingRow, rows::ReverseZeroVisibilityInputs, rows::TableId,
            rows::TableRows, rows::ZeroSubjectInputs,
        };
        use replay_frame::frame::TableView;
        let mut frame = frame();
        let links = [(11, 1)].into_iter().collect();
        frame.tables[TableId::ReservedCountryNaming.index()] =
            TableView::Complete(TableRows::ReservedCountryNaming(vec![
                ReservedCountryNamingRow {
                    faction: Recorded::Value(Some(0)),
                    ..Default::default()
                },
            ]))
            .into();
        assert_eq!(
            family(&frame, &links, 0, 0, &OnceLock::new(), &OnceLock::new()),
            Ok(true)
        );
        frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                id: 11,
                canonical_slot: 1,
                ..Default::default()
            }]))
            .into();
        frame.tables[TableId::CountryCounterInputs.index()] =
            TableView::Complete(TableRows::CountryCounterInputs(vec![
                CountryCounterInputRow {
                    country: 11,
                    forward_visibility: Recorded::Value(Some(ReverseZeroVisibilityInputs {
                        subject: Some(ZeroSubjectInputs::default()),
                        ..Default::default()
                    })),
                    ..Default::default()
                },
            ]))
            .into();
        assert_eq!(
            family(&frame, &links, 1, 0, &OnceLock::new(), &OnceLock::new()),
            Ok(false)
        );
    }

    #[test]
    fn country_zero_forward_subject_uses_packet_beyond_positive_target_certificate() {
        use replay_data::{
            rows::CountryCounterInputRow, rows::CountryRow, rows::Recorded, rows::RelationLinkRow,
            rows::ReservedCountryNamingRow, rows::ReverseZeroVisibilityInputs, rows::TableId,
            rows::TableRows, rows::TransportSubjectAccess, rows::ZeroSubjectInputs,
        };
        use replay_frame::frame::TableView;
        let mut frame = frame();
        let links = [(11, 1)].into_iter().collect();
        frame.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(Vec::new())).into();
        frame.tables[TableId::ReservedCountryNaming.index()] =
            TableView::Complete(TableRows::ReservedCountryNaming(vec![
                ReservedCountryNamingRow {
                    faction: Recorded::Value(Some(0)),
                    ..Default::default()
                },
            ]))
            .into();
        frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                id: 11,
                canonical_slot: 1,
                ..Default::default()
            }]))
            .into();
        frame.tables[TableId::RelationLinks.index()] =
            TableView::Complete(TableRows::RelationLinks(vec![RelationLinkRow {
                country: 11,
                target_index: 1,
                transport_subject: Recorded::Value(Some(TransportSubjectAccess {
                    sample_hour: replay_data::rows::PacketHour::Stamped(frame.hour),
                    complete_owner: true,
                    ..Default::default()
                })),
                ..Default::default()
            }]))
            .into();
        let packets = |packet| {
            TableView::Complete(TableRows::CountryCounterInputs(vec![
                CountryCounterInputRow {
                    country: 11,
                    forward_visibility: packet,
                    ..Default::default()
                },
            ]))
            .into()
        };
        frame.tables[TableId::CountryCounterInputs.index()] =
            packets(Recorded::Value(Some(ReverseZeroVisibilityInputs {
                target: Some(0),
                subject: Some(ZeroSubjectInputs {
                    relation_type: Some(engine_executable::native_values::subject_relation::TYPE_VALUE),
                    parties: Some([Some(11), Some(0)]),
                }),
                ..Default::default()
            })));
        assert_eq!(
            family(&frame, &links, 1, 0, &OnceLock::new(), &OnceLock::new()),
            Ok(true)
        );
        frame.tables[TableId::CountryCounterInputs.index()] =
            packets(Recorded::Value(Some(ReverseZeroVisibilityInputs {
                subject: Some(ZeroSubjectInputs::default()),
                ..Default::default()
            })));
        assert_eq!(
            family(&frame, &links, 1, 0, &OnceLock::new(), &OnceLock::new()),
            Ok(false)
        );
        for refused in [
            Recorded::NotRecorded,
            Recorded::Value(None),
            Recorded::Value(Some(ReverseZeroVisibilityInputs::default())),
        ] {
            frame.tables[TableId::CountryCounterInputs.index()] = packets(refused);
            assert!(family(&frame, &links, 1, 0, &OnceLock::new(), &OnceLock::new()).is_err());
        }
    }

    fn row(country: u32, current: i64, previous: i64) -> Supremacy {
        Supremacy {
            country,
            current,
            previous,
        }
    }
    #[test]
    fn control_and_gaining_side_use_independent_native_inputs() {
        let rows = [row(1, 900_000, 900_000), row(2, 100_000, 50_000)];
        let side = &mut |raw| {
            Ok(if raw == 1 {
                Side::Friendly
            } else {
                Side::Enemy
            })
        };
        assert_eq!(
            classify(
                &rows,
                100_000,
                60_000,
                side,
                &|_| Ok(false)
            )
            .unwrap(),
            [255, 128]
        );
        let rows = [row(1, 0, -10), row(2, 0, 0)];
        assert!(
            classify(
                &rows,
                100_000,
                60_000,
                side,
                &|_| Ok(false)
            )
            .is_err()
        );
    }
    #[test]
    fn faction_members_use_lazy_native_family_admission() {
        use replay_data::{
            rows::CountryLinkRow, rows::FactionMemberRow, rows::NavalSupremacyRow, rows::TableId,
            rows::TableRows,
        };
        use replay_frame::{frame::FrameDecoder, frame::FrameEncoder, frame::TableView};
        let mut source = frame();
        let countries = 1..=12;
        source.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(
                countries
                    .clone()
                    .map(|id| CountryLinkRow {
                        id,
                        canonical_slot: id,
                    })
                    .collect(),
            ))
            .into();
        source.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(
                countries
                    .clone()
                    .map(|id| FactionMemberRow {
                        country: id,
                        faction: 99,
                        ordinal: id - 1,
                    })
                    .collect(),
            ))
            .into();
        source.tables[TableId::NavalSupremacy.index()] =
            TableView::Complete(TableRows::NavalSupremacy(
                countries
                    .map(|country| NavalSupremacyRow {
                        region: 19,
                        region_slot: 1,
                        country,
                        current: 100_000,
                        previous: 100_000,
                        ..Default::default()
                    })
                    .collect(),
            ))
            .into();
        source.metrics.table_ns[TableId::NavalSupremacy.index()] = 1;
        let wire = FrameEncoder::new().encode(source, false).unwrap();
        let frame = FrameDecoder::new().apply(&wire).unwrap();
        let content = Content {
            ratio: Ok(60_000),
            regions: [(
                19,
                RegionContent {
                    name: "synthetic".into(),
                    minimum: Ok(100_000),
                    gradient: None,
                    has_sea: true,
                },
            )]
            .into_iter()
            .collect(),
        };
        let fill = content.read(&frame, 1);
        assert!(fill.warnings.is_empty(), "{:?}", fill.warnings);
        assert_eq!(
            fill.attributes[&19],
            Attributes {
                control: 255,
                contested: 0,
                gradient: None
            }
        );
    }
    #[test]
    fn a_neutral_gaining_country_reaches_only_its_blockade_operand() {
        let rows = [row(1, 100_000, 0)];
        assert_eq!(
            classify(
                &rows,
                100_000,
                60_000,
                &mut |_| Ok(Side::Neutral),
                &|_| Ok(true)
            )
            .unwrap(),
            [0, 128]
        );
        assert!(
            classify(
                &rows,
                100_000,
                60_000,
                &mut |_| Ok(Side::Neutral),
                &|_| Err("missing blockade".into())
            )
            .unwrap_err()
            .contains("missing blockade")
        );
    }

    #[test]
    fn retained_values_are_valid_within_the_native_cache_epoch() {
        assert!(cache_epoch(47, Some(24)).is_ok());
        assert!(cache_epoch(48, Some(47)).is_err());
        assert!(cache_epoch(47, Some(48)).is_err());
        assert!(cache_epoch(47, None).is_err());
    }

    #[test]
    fn the_scheduled_read_certifies_every_hour_of_its_day() {
        use replay_data::rows::TableId;
        for hour in 24..24 * 4 {
            let sampled = (0..=hour)
                .rev()
                .find(|&h| replay_data::cadence::is_refresh(TableId::NavalSupremacy, h));
            assert!(
                cache_epoch(hour, sampled).is_ok(),
                "hour {hour} holds the read of hour {sampled:?}"
            );
        }
    }

    #[test]
    fn mod_terrain_and_control_ratio_supply_the_minimum() {
        content_reader_script::test_game::enter();
        let terrain = content_reader_key_names::terrain::parse_definitions(b"categories={ ocean={minimum_seazone_dominance=9} crystal_sea={minimum_seazone_dominance=17} }").unwrap();
        let defines = Defines::new().unwrap();
        defines
            .load(
                "base.lua",
                b"NDefines={NNavy={DOMINANCE_CONTROLLED_THRESHOLD_RATIO=0.81}}",
            )
            .unwrap();
        defines
            .load(
                "mod.lua",
                b"NDefines.NNavy.DOMINANCE_CONTROLLED_THRESHOLD_RATIO=0.35",
            )
            .unwrap();
        let map = MapData {
            width: 2,
            height: 1,
            province_ids: vec![1, 2],
            definitions: (1..=2)
                .map(|id| {
                    (
                        id,
                        content_reader_map::map::ProvinceDefinition {
                            id,
                            rgb: [0; 3],
                            kind: ProvinceKind::Sea,
                            continent: 0,
                        },
                    )
                })
                .collect(),
            country_colors: Default::default(),
        };
        let regions = [(
            19,
            StrategicRegion {
                id: 19,
                name_key: "custom".into(),
                provinces: vec![1, 2],
                naval_terrain: Some("CRYSTAL_SEA".into()),
                static_modifiers: false,
                static_modifier_names: Vec::new(),
                ..Default::default()
            },
        )]
        .into_iter()
        .collect();
        let content = Content::new(&map, &regions, Ok(&terrain), &defines);
        assert_eq!(content.regions[&19].minimum, Ok(1_700_000));
        assert_eq!(content.ratio, Ok(35_000));
        let rows = [row(1, 1_000_000, 1_000_000)];
        assert_eq!(
            classify(
                &rows,
                content.regions[&19].minimum.clone().unwrap(),
                content.ratio.clone().unwrap(),
                &mut |_| Ok(Side::Friendly),
                &|_| Ok(false)
            )
            .unwrap(),
            [255, 0]
        );
    }
}
