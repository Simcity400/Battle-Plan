//! Political mode's naval control and gaining-side gradient attributes.
use std::{collections::BTreeMap, sync::OnceLock};
use viewer_foundation::naval_math::dominance_value;

use installed_content_core::{
    defines::Defines, map::MapData, map::ProvinceKind, terrain::TerrainDefinitions,
};
use installed_content_regions::regions::StrategicRegion;
use replay_data::{rows::NavalSupremacyRow, rows::tables};
use replay_frame::frame::DecodedFrame;

use {
    super::gradient::Attributes,
    viewer_foundation::map::{directed_wars::Owner, visibility::Families},
};

type Fact<T> = Result<T, String>;

struct RegionContent {
    name: String,
    minimum: Fact<i64>,
    gradient: Option<u8>,
    has_sea: bool,
}

pub struct Content {
    regions: BTreeMap<u32, RegionContent>,
    ratio: Fact<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fill {
    pub attributes: BTreeMap<u32, Attributes>,
    pub warnings: Vec<String>,
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
                // Resolve the region's naval terrain category, falling back to the content's ocean category.
                let minimum = terrain.map_err(str::to_owned).and_then(|terrain| {
                    let category = region.naval_terrain.as_ref().and_then(|name| terrain.category(name.as_bytes()))
                        .or_else(|| terrain.category(b"ocean"));
                    category.map(|index| terrain.categories[index].minimum_seazone_dominance)
                        .ok_or_else(|| format!("naval terrain {} and executable ocean fallback absent from recorded content", region.naval_terrain.as_deref().unwrap_or("ocean")))
                });
                // Count unique valid provinces; below one percent sea, use constant 255.
                let kinds = region.provinces.iter().filter_map(|id| map.definitions.get(id)).map(|p| (p.id, p.kind)).collect::<BTreeMap<_,_>>();
                let sea = kinds.values().filter(|&&kind| kind == ProvinceKind::Sea).count() as u64;
                let fraction = if kinds.is_empty() { 0 } else { sea * 100_000 / kinds.len() as u64 };
                (id, RegionContent { name:region.name_key.clone(), minimum, gradient:(fraction < 1_000).then_some(255), has_sea:sea != 0 })
            }).collect(),
            ratio: defines.fixed5("NNavy", "DOMINANCE_CONTROLLED_THRESHOLD_RATIO").map_err(|e| e.to_string()),
        }
    }

    pub fn read(&self, frame: &DecodedFrame, viewer: u32) -> Fill {
        let mut fill = Fill::default();
        let rows = frame
            .complete::<tables::NavalSupremacy>()
            .map_err(|e| e.to_string());
        let grouped = rows.as_ref().map(|rows| {
            let mut groups: BTreeMap<u32, Vec<&NavalSupremacyRow>> = BTreeMap::new();
            for row in *rows {
                groups.entry(row.region).or_default().push(row);
            }
            groups
        });
        let links = frame
            .complete::<tables::CountryLinks>()
            .map(|rows| {
                rows.iter()
                    .map(|r| (r.id, r.canonical_slot))
                    .collect::<BTreeMap<_, _>>()
            })
            .map_err(|e| e.to_string());
        let factions = OnceLock::new();
        let subjects = OnceLock::new();
        let mut sides: BTreeMap<u32, Fact<Side>> = BTreeMap::new();
        let mut side = |raw: u32| -> Fact<Side> {
            if let Some(value) = sides.get(&raw) {
                return value.clone();
            }
            let value = (|| {
                let links = links.as_ref().map_err(Clone::clone)?;
                let slot = canonical(links, raw)?;
                // Test the source's family relation toward the viewer before its directed war pointer.
                if family(frame, links, slot, viewer, &factions, &subjects)? {
                    return Ok(Side::Friendly);
                }
                if Owner::new(frame, raw)?.pointer_active_slot(viewer)? {
                    Ok(Side::Enemy)
                } else {
                    Ok(Side::Neutral)
                }
            })();
            sides.insert(raw, value.clone());
            value
        };
        let blockade = |raw| -> Fact<bool> {
            let links = links.as_ref().map_err(Clone::clone)?;
            let source = canonical(links, raw)?;
            if source == 0 || viewer == 0 {
                return Err("naval blockade pointer for canonical Country0 is not recorded".into());
            }
            // Installed naval-blockade tuples retain source-to-viewer direction and membership. Retired,
            // date and cancellation flags do not add another gate here.
            for row in frame
                .complete::<tables::RelationTuples>()
                .map_err(|e| e.to_string())?
            {
                if row.type_id == 0x27f8
                    && canonical(links, row.first_country)? == source
                    && canonical(links, row.second_country)? == viewer
                {
                    return Ok(true);
                }
            }
            Ok(false)
        };
        let mut refused = Vec::new();
        for (&id, region) in &self.regions {
            let RegionContent {
                name,
                minimum,
                gradient,
                has_sea,
            } = region;
            let result = (|| {
                // Pure land regions have no province in this gradient layer.
                if !has_sea {
                    return Ok([0, 0]);
                }
                cache_epoch(
                    frame.hour,
                    frame.tables[replay_data::rows::TableId::NavalSupremacy.index()].sampled_at(),
                )?;
                let groups = grouped.as_ref().map_err(|reason| (*reason).clone())?;
                let values = groups.get(&id).map(Vec::as_slice).unwrap_or_default();
                classify(
                    values,
                    *minimum.as_ref().map_err(Clone::clone)?,
                    *self.ratio.as_ref().map_err(Clone::clone)?,
                    &mut side,
                    &blockade,
                )
            })();
            let [control, contested] = match result {
                Ok(value) => value,
                Err(reason) => {
                    refused.push(format!("region {id} ({name}): {reason}"));
                    [0, 0]
                }
            };
            fill.attributes.insert(
                id,
                Attributes {
                    control,
                    contested,
                    gradient: *gradient,
                },
            );
        }
        if !refused.is_empty() {
            fill.warnings.push(format!(
                "Naval dominance FX withheld for {} strategic regions: {}",
                refused.len(),
                refused.join("; ")
            ));
        }
        fill
    }
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
    replay_derive::transport_subject_access::matches(
        subject,
        packet
            .target
            .ok_or_else(|| "naval family target refused".into()),
        |raw| canonical(links, raw),
    )
}

fn cache_epoch(hour: replay_data::Hour, sampled: Option<replay_data::Hour>) -> Fact<()> {
    // The sorted supremacy cache rebuilds at midnight. Daily acquisition can straddle that boundary;
    // retain certified values within their cache epoch, without interpolating updates.
    match sampled {
        Some(sample) if sample <= hour && sample.div_euclid(24) == hour.div_euclid(24) => Ok(()),
        Some(sample) => Err(format!(
            "NavalSupremacy sampled at hour {sample} cannot certify native midnight cache epoch at {hour}"
        )),
        None => Err("NavalSupremacy acquisition provenance is unrecorded".into()),
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Friendly,
    Enemy,
    Neutral,
}

fn quotient(numerator: i64, denominator: i64) -> Fact<i64> {
    //fixed5 signed guard, wrapping multiplication, truncation.
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
    rows: &[&NavalSupremacyRow],
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
        // Zero contributes no bits to either wrapping sum. The independent
        //top/second gaining predicate below still admits a zero-current row.
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
    // Select the top candidate when control is zero, otherwise the second candidate. A gaining side
    // requires current supremacy above previous supremacy. Sorting uses only current supremacy, so
    // ties are usable only when every possible selected candidate writes alike.
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
                    relation_type: Some(engine_native::csubject_relation::TYPE_VALUE),
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

    fn row(country: u32, current: i64, previous: i64) -> NavalSupremacyRow {
        NavalSupremacyRow {
            country,
            current,
            previous,
            ..Default::default()
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
                &rows.iter().collect::<Vec<_>>(),
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
                &rows.iter().collect::<Vec<_>>(),
                100_000,
                60_000,
                side,
                &|_| Ok(false)
            )
            .is_err()
        );
    }
    #[test]
    fn five_thousand_countries_use_lazy_native_family_admission() {
        use replay_data::{
            rows::CountryLinkRow, rows::FactionMemberRow, rows::TableId, rows::TableRows,
        };
        use replay_frame::{frame::FrameDecoder, frame::FrameEncoder, frame::TableView};
        let mut source = frame();
        let countries = 1..=5_000;
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
                &[&rows[0]],
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
                &[&rows[0]],
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
        // The sorted cache is rebuilt inside the update entered at hour 23,
        // so the latest scheduled read must belong to the hour's own day.
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let terrain = installed_content_core::terrain::parse_definitions(b"categories={ ocean={minimum_seazone_dominance=9} crystal_sea={minimum_seazone_dominance=17} }").unwrap();
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
                        installed_content_core::map::ProvinceDefinition {
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
                &[&rows[0]],
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
