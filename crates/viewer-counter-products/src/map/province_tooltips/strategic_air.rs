mod selection;
pub use selection::assignment_checks;
use super::{Arg, Context, Tooltip, line};
use replay_data::rows::tables;
use std::{cell::OnceCell, collections::BTreeMap};
use viewer_unit_reads::{air_detection as detection, air_range::FrameSources};
use viewer_foundation::visibility::Relations;

const UNRESOLVED_BASE_REGION: u32 = 0xaa;

fn keyed<T, K: Ord>(rows: &[T], key: K, field: impl FnMut(&T) -> K) -> Option<&T> {
    rows.binary_search_by_key(&key, field)
        .ok()
        .map(|index| &rows[index])
}

fn fraction(value: i64, count: usize, empty: i64) -> i64 {
    if count == 0 {
        empty
    } else if !viewer_unit_reads::air_range::finite(value) {
        0xffff_ffff
    } else {
        value
            .wrapping_mul(100_000)
            .wrapping_div((count as i64).wrapping_mul(100_000))
    }
}

fn radar_contribution(covered: i64, share: i64) -> i64 {
    covered.wrapping_mul(share).wrapping_mul(100_000) / 100_000
}

pub(super) fn append(context: &Context<'_>, selected: &[u64], out: &mut Tooltip) {
    let Some(&region) = context.content.region_of.get(&context.province.id) else {
        return;
    };
    let text = regional_detection(
        context.content,
        context.frame,
        context.vision.ours(),
        region,
        &|province| {
            context
                .reads
                .updated_clock(province, context.frame.hour)
                .map_err(str::to_owned)
        },
    )
    .map(|(_, text)| text);
    line(out, "regional Air detection", 0, text);
    if !selected.is_empty() {
        line(
            out,
            "selected Air regional assignment",
            0,
            selection::text(context, region, selected),
        );
    }
    super::strategic_navy::merge_sections(
        context,
        out,
        super::naval_mines::build(context.content, context.frame, context.vision, region, true),
    );
}

pub(crate) fn pool_region(
    content: &super::counter_content::Content,
    frame: &replay_frame::frame::DecodedFrame,
    registry: &viewer_unit_reads::air_registry_sources::AirRegistrySources,
    base: u64,
) -> Result<(u32, bool), String> {
    let provinces = frame.complete::<tables::Provinces>().map_err(|e| e.to_string())?;
    let movement = content
        .movement
        .as_ref()
        .ok_or("Air occupation needs the content province terrain flags")?;
    let mut base_region = UNRESOLVED_BASE_REGION;
    let mut carrier_mission = false;
    if let Some(base) = registry.bases.get(&base) {
        let province = if let Some(state) = base.state {
            match base.province {
                Some(province) => Some(province),
                None => content
                    .air_base_first_provinces
                    .get(&(state, base.kind as usize))
                    .copied()
                    .transpose()
                    .map_err(str::to_owned)?,
            }
        } else {
            let ships = frame.complete::<tables::Ships>().map_err(|e| e.to_string())?;
            let force = match keyed(ships, base.host, |row| row.id) {
                Some(ship) => {
                    let forces = frame.complete::<tables::TaskForces>().map_err(|e| e.to_string())?;
                    keyed(forces, ship.task_force, |row| row.id)
                }
                None => None,
            };
            if let Some(force) = force {
                carrier_mission = force.mission != 0;
                let province = keyed(provinces, force.province, |row| row.id)
                    .ok_or("Air carrier province is unavailable")?;
                let terrain = movement
                    .map
                    .terrain(province.id)
                    .ok_or("Air carrier terrain flags are unavailable")?;
                if terrain.flags & 1 != 0 {
                    u32::try_from(province.port_sea).ok().filter(|&id| id != 0)
                } else {
                    Some(province.id)
                }
            } else {
                None
            }
        };
        if let Some(province) = province {
            base_region = *content
                .region_of
                .get(&province)
                .ok_or("Air pool-base strategic region is unavailable")?;
        }
    }
    Ok((base_region, carrier_mission))
}

pub struct RegionalDetection<'a> {
    content: &'a super::counter_content::Content,
    frame: &'a replay_frame::frame::DecodedFrame,
    region: u32,
    links: BTreeMap<u32, u32>,
    updated: &'a dyn Fn(u32) -> Result<replay_data::day_night::Clock, String>,
    countries: OnceCell<Result<detection::Countries<'a>, String>>,
    registry:
        OnceCell<Result<viewer_unit_reads::air_registry_sources::AirRegistrySources, String>>,
    sources: FrameSources<'a>,
    registered:
        OnceCell<Result<BTreeMap<u32, Vec<(u32, &'a replay_data::rows::AirWingRow)>>, String>>,
    radar_countries:
        OnceCell<Result<BTreeMap<u32, &'a replay_data::rows::CountryCounterInputRow>, String>>,
    relations: OnceCell<Result<Relations, String>>,
}

impl<'a> RegionalDetection<'a> {
    pub fn new(
        content: &'a super::counter_content::Content,
        frame: &'a replay_frame::frame::DecodedFrame,
        region: u32,
        updated: &'a dyn Fn(u32) -> Result<replay_data::day_night::Clock, String>,
    ) -> Result<Self, String> {
        let links = frame
            .complete::<tables::CountryLinks>()
            .map_err(|e| e.to_string())?
            .iter()
            .map(|row| (row.id, row.canonical_slot))
            .collect();
        Ok(Self {
            content,
            frame,
            region,
            links,
            updated,
            countries: OnceCell::new(),
            registry: OnceCell::new(),
            sources: FrameSources::new(frame),
            registered: OnceCell::new(),
            radar_countries: OnceCell::new(),
            relations: OnceCell::new(),
        })
    }

    pub fn country(&self, ours: u32) -> Result<(i64, String), String> {
        calculate(self, ours)
    }
}

struct DetectionContext<'r, 'a> {
    content: &'a super::counter_content::Content,
    frame: &'a replay_frame::frame::DecodedFrame,
    ours: u32,
    links: &'r BTreeMap<u32, u32>,
    updated: &'a dyn Fn(u32) -> Result<replay_data::day_night::Clock, String>,
    reads: &'r RegionalDetection<'a>,
}
impl DetectionContext<'_, '_> {
    fn slot(&self, raw: u32) -> Result<u32, String> {
        if raw == 0 {
            return Ok(0);
        }
        self.links
            .get(&raw)
            .copied()
            .ok_or_else(|| format!("CountryLinks lacks {raw}"))
    }
    fn loc(&self, key: &str, args: &[(&str, Arg<'_>)]) -> Result<String, String> {
        super::localise(&self.content.localisation, key, args)
    }
}

pub fn regional_detection(
    content: &super::counter_content::Content,
    frame: &replay_frame::frame::DecodedFrame,
    ours: u32,
    region: u32,
    updated: &dyn Fn(u32) -> Result<replay_data::day_night::Clock, String>,
) -> Result<(i64, String), String> {
    RegionalDetection::new(content, frame, region, updated)?.country(ours)
}

fn calculate(reads: &RegionalDetection<'_>, ours: u32) -> Result<(i64, String), String> {
    let region = reads.region;
    let context = &DetectionContext {
        content: reads.content,
        frame: reads.frame,
        ours,
        links: &reads.links,
        updated: reads.updated,
        reads,
    };
    if let Some(raw) = detection::console_override(context.frame, context.ours)? {
        if !engine_executable::executable_tables::is_initialized() {
            return Err("Air detection console prefix needs verified executable metadata".into());
        }
        let prefix = engine_executable::executable_tables::auxiliary_part(|tables| {
            &tables.air_detection_override_prefix
        })
        .map_err(|withheld| withheld.to_string())?;
        let amount = content_reader_parser::fixed5::fixed5_to_string(raw.wrapping_mul(100), 0);
        return Ok((raw, format!("{prefix}{amount}%\n")));
    }
    replay_derive::fixed_layout::validate_recording(
        context.frame,
        replay_derive::fixed_layout::AIR_MAP,
    )?;
    let content = &context.content.province_hover;
    let defines = *content.air_detection.as_ref().map_err(Clone::clone)?;
    let range = *content.air_range.as_ref().map_err(Clone::clone)?;
    let region_definition = context
        .content
        .strategic_regions
        .get(&region)
        .ok_or("strategic region content is missing")?;
    let members = &region_definition.provinces;
    let provinces = context
        .frame
        .complete::<tables::Provinces>()
        .map_err(|e| e.to_string())?;
    let ours = context.ours;
    let movement = context
        .content
        .movement
        .as_ref()
        .ok_or("Air occupation needs the content province terrain flags")?;
    let (mut land, mut owned) = (0_usize, 0_i64);
    for &id in members {
        if movement
            .map
            .terrain(id)
            .ok_or("Air region province terrain is unavailable")?
            .flags
            & 1
            == 0
        {
            continue;
        }
        land += 1;
        let province =
            keyed(provinces, id, |row| row.id).ok_or("Air region controller is unavailable")?;
        owned += i64::from(context.slot(province.controller)? == ours);
    }
    let occupation = fraction(owned.wrapping_mul(100_000), land, 0);
    let countries = reads
        .countries
        .get_or_init(|| detection::Countries::read(context.frame))
        .as_ref()
        .map_err(Clone::clone)?;
    let radar = radar(context, region, members.len(), provinces)?;
    let weather_rows = context
        .frame
        .complete::<tables::WeatherProvinces>()
        .map_err(|e| e.to_string())?;
    let mut night = 0_i64;
    for &id in members {
        let row = keyed(weather_rows, id, |row| row.id)
            .ok_or("Air region province night record is unavailable")?;
        let clock = super::clock::resolve(
            row,
            |province| (context.updated)(province),
            || {
                context
                    .frame
                    .complete::<tables::SimulationFeatures>()
                    .map_err(|e| e.to_string())?
                    .first()
                    .ok_or("simulation features absent")?
                    .weather_suspend
                    .require("SimulationFeatures.weather_suspend")
                    .copied()
            },
        )?;
        night = night.wrapping_add(clock.night);
    }
    let mean_night = fraction(night, members.len(), 0xffff_ffff);
    let weather = context
        .frame
        .complete::<tables::WeatherRegions>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.id == region)
        .map(|row| {
            let raw = row
                .air_detection
                .require("WeatherRegions.air_detection")?
                .as_ref()
                .ok_or("regional Air detection accumulator was unreadable")?;
            effective(context.frame, 1, *raw)
        })
        .transpose()?;
    let registered = reads
        .registered
        .get_or_init(|| {
            let wings = context
                .frame
                .complete::<tables::AirWings>()
                .map_err(|e| e.to_string())?;
            let mut registered = BTreeMap::<u32, Vec<(u32, &replay_data::rows::AirWingRow)>>::new();
            for row in context
                .frame
                .complete::<tables::AirRegionalMissions>()
                .map_err(|e| e.to_string())?
            {
                let wing = keyed(wings, row.wing, |row| row.id)
                    .ok_or("registered Air mission wing is unavailable")?;
                if wing.mission_region == Some(region) {
                    registered
                        .entry(context.slot(wing.owner)?)
                        .or_default()
                        .push((row.ordinal, wing));
                }
            }
            for wings in registered.values_mut() {
                wings.sort_unstable_by_key(|(ordinal, _)| *ordinal);
            }
            Ok::<_, String>(registered)
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let registry = &reads.registry;
    let sources = &reads.sources;
    let mut missions = Vec::new();
    for &(_, wing) in registered.get(&ours).into_iter().flatten() {
        let inputs = context
            .frame
            .complete::<tables::AirMissionInputs>()
            .map_err(|e| e.to_string())?;
        let input = keyed(inputs, wing.id, |row| row.wing)
            .ok_or("registered Air mission inputs are unavailable")?;
        let Some(execution) = detection::Execution::read(input)? else {
            continue;
        };
        if execution.aircraft == 0 {
            continue;
        }
        let registry = registry
            .get_or_init(|| {
                viewer_unit_reads::air_registry_sources::AirRegistrySources::read(
                    context.frame,
                )
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "Air detection base registry is incomplete".to_owned())
            })
            .as_ref()
            .map_err(Clone::clone)?;
        let (base_region, carrier_mission) = pool_region(context.content, context.frame, registry, wing.air_base)?;
        let home = base_region == region;
        let normalized = input
            .normalization
            .require("AirMissionInputs.normalization")?
            .as_ref()
            .ok_or("executing Air mission normalization packet is unavailable")?;
        let (night, weather) = detection::wing_normalization(
            wing,
            normalized,
            carrier_mission,
            range,
            countries,
            sources,
        )?;
        missions.push(detection::Mission {
            execution,
            home,
            night,
            weather,
        });
    }
    let result = detection::derive(
        defines,
        countries.detection(ours)?,
        occupation,
        radar,
        mean_night,
        weather,
        missions,
    );
    let text = context.loc(
        "AIR_DETECTION_IN_REGION_DETAILS",
        &[
            ("TOT", Arg::Fixed5(result.total)),
            ("BASE", Arg::Fixed5(result.occupation)),
            ("RAD", Arg::Fixed5(result.radar)),
            ("AIR", Arg::Fixed5(result.aircraft)),
            ("DETECT_RED", Arg::Fixed5(result.environmental_factor)),
            ("NIGHT", Arg::Fixed5(result.night)),
            ("WEA", Arg::Fixed5(result.weather)),
        ],
    )?;
    Ok((result.total, text))
}

fn effective(
    frame: &replay_frame::frame::DecodedFrame,
    key: u32,
    raw: Option<i64>,
) -> Result<i64, String> {
    if raw.is_none() {
        return Ok(0);
    }
    let definition = frame
        .complete::<tables::ModifierDefinitions>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.index == key)
        .ok_or("regional Air modifier metadata is unavailable")?;
    let features = match frame.complete::<tables::SimulationFeatures>() {
        Ok([row]) => Some(row.dlc_mask),
        _ => None,
    };
    replay_derive::naval_mission_derive::effective(definition, features, raw).map_err(str::to_owned)
}

fn radar(
    context: &DetectionContext<'_, '_>,
    region: u32,
    count: usize,
    provinces: &[replay_data::rows::ProvinceRow],
) -> Result<i64, String> {
    let relations = &context.reads.relations;
    let countries = context
        .reads
        .radar_countries
        .get_or_init(|| {
            context
                .frame
                .complete::<tables::CountryCounterInputs>()
                .map_err(|e| e.to_string())?
                .iter()
                .map(|row| context.slot(row.country).map(|slot| (slot, row)))
                .collect::<Result<BTreeMap<_, _>, _>>()
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let mut strength = 0_i64;
    for group in context
        .frame
        .complete::<tables::RadarCoverage>()
        .map_err(|e| e.to_string())?
    {
        let covered = group
            .provinces
            .iter()
            .filter(|province| context.content.region_of.get(province) == Some(&region))
            .count() as i64;
        if covered == 0 {
            continue;
        }
        let owner = context.slot(group.country)?;
        if owner != context.ours {
            if owner == 0
                || countries
                    .get(&owner)
                    .ok_or("radar owner's controlled-state count is unavailable")?
                    .controlled_state_count
                    <= 0
            {
                continue;
            }
            let relations = relations
                .get_or_init(|| {
                    Relations::read(context.frame, &context.links)
                        .and_then(|relations| {
                            relations.with_attaches(context.frame, &context.links)
                        })
                        .map_err(str::to_owned)
                })
                .as_ref()
                .map_err(Clone::clone)?;
            if !relations.allied_for_fog(context.ours, owner) {
                continue;
            }
        }
        let Some(state) = group.state else {
            return Err("radar coverage has no host-state controller-share witness".into());
        };
        let members = context
            .content
            .province_hover
            .members
            .get(&state)
            .ok_or("radar host state's provinces are unavailable")?;
        let mut controlled = 0_i64;
        for province in members {
            controlled += i64::from(
                context.slot(
                    keyed(provinces, *province, |row| row.id)
                        .ok_or("radar host controller is unavailable")?
                        .controller,
                )? == owner,
            );
        }
        let share = fraction(controlled.wrapping_mul(100_000), members.len(), 0xffff_ffff);
        strength = strength.wrapping_add(radar_contribution(covered, share));
    }
    Ok(fraction(strength, count, 100_000).clamp(0, 100_000))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_regions_and_guarded_sums_keep_each_native_fraction_default() {
        assert_eq!(fraction(0, 0, 0), 0);
        assert_eq!(fraction(0, 0, 100_000), 100_000);
        assert_eq!(fraction(0, 0, 0xffff_ffff), 0xffff_ffff);
        assert_eq!(fraction(100_001, 3, 0), 33_333);
        assert_eq!(fraction(-100_001, 3, 0), -33_333);
        for invalid in [i64::MIN, i64::MIN + 1, i64::MAX] {
            assert_eq!(fraction(invalid, 3, 0), 0xffff_ffff);
        }
        assert_eq!(radar_contribution(30_000, 0xffff_ffff), -55_618_421_887_095);
    }

    #[test]
    fn absent_weather_key_needs_no_metadata_but_present_disabled_value_needs_its_gate() {
        use replay_data::rows::*;
        use replay_frame::{frame::Metrics, frame::TableView};
        let mut frame = replay_frame::frame::DecodedFrame {
            branch: 1,
            hour: 1,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "unreached".into(),
                }
                .into()
            }),
        };
        assert_eq!(effective(&frame, 1, None).unwrap(), 0);
        assert!(effective(&frame, 1, Some(-50_000)).is_err());
        let rows = TableRows::ModifierDefinitions(vec![ModifierDefinitionRow {
            index: 1,
            feature_selector: 4,
            eligibility: 1,
            ..Default::default()
        }]);
        let index = rows.table().index();
        frame.tables[index] = TableView::Complete(rows).into();
        assert!(effective(&frame, 1, Some(-50_000)).is_err());
        let rows = TableRows::SimulationFeatures(vec![SimulationFeatureRow {
            dlc_mask: 0,
            ..Default::default()
        }]);
        let index = rows.table().index();
        frame.tables[index] = TableView::Complete(rows).into();
        assert_eq!(effective(&frame, 1, Some(-50_000)).unwrap(), 0);
        let rows = TableRows::SimulationFeatures(vec![SimulationFeatureRow {
            dlc_mask: 4,
            ..Default::default()
        }]);
        let index = rows.table().index();
        frame.tables[index] = TableView::Complete(rows).into();
        assert_eq!(effective(&frame, 1, Some(-50_000)).unwrap(), -50_000);
    }
}
