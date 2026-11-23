use replay_data::rows::{NavalMissionLedgerRow, NavalPatrolMissionInputRow};
use super::{BTreeMap, Fact, Hour, Prepared, Value, data, derive, io, keys, name, tables};
#[cfg(test)]
use super::{NativeInputs, Op};
use viewer_unit_reads::counter_order;
use replay_derive::{directed_wars, naval_region_control};
use viewer_foundation::visibility::Relations;

const ONE: i64 = 100_000;
const REFUSED: i64 = 0xffff_ffff;

#[derive(Clone, Copy, Debug, Default)]
pub(in crate) struct Defines {
    pub base: i64,
    pub radar: i64,
    pub navy_factor: i64,
    pub navy: i64,
    pub air: i64,
    pub dominance: i64,
    pub control_ratio: i64,
}
impl Defines {
    pub(in crate) fn read(
        defines: &content_reader_core::defines::Defines,
    ) -> io::Result<Self> {
        Ok(Self {
            base: defines.fixed5("NNavy", "BASE_SPOTTING")?,
            radar: defines.fixed5("NNavy", "BASE_SPOTTING_FROM_RADAR")?,
            navy_factor: defines.fixed5("NNavy", "NAVY_SPOTTER_DETECTION_FACTOR")?,
            navy: defines.fixed5("NNavy", "BASE_SPOTTING_FROM_NAVY")?,
            air: defines.fixed5("NNavy", "BASE_SPOTTING_FROM_AIR")?,
            dominance: defines.fixed5("NNavy", "NAVAL_DOMINANCE_SPOTTING_BONUS")?,
            control_ratio: defines.fixed5("NNavy", "DOMINANCE_CONTROLLED_THRESHOLD_RATIO")?,
        })
    }
}

fn mul(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / ONE
}
fn divide(value: i64, denominator: i64) -> Fact<i64> {
    game_model::fixed5::fraction(value, denominator).map_err(str::to_owned)
}
fn printed(value: i64) -> i64 {
    game_model::fixed5::fraction(value, 10_000_000).unwrap_or(REFUSED)
}
fn saturate(density: i64, maximum: i64) -> Fact<i64> {
    let fraction = if density >= 1_000_000 {
        ONE
    } else {
        ONE.wrapping_sub(divide(
            ONE,
            derive::power(200_000, density).map_err(str::to_owned)?,
        )?)
    };
    Ok(mul(fraction, maximum))
}

pub(in crate) fn packet<'a>(
    hour: &Hour<'a>,
    handle: u64,
) -> Fact<&'a NavalPatrolMissionInputRow> {
    let rows = data::<tables::NavalPatrolMissionInputs>(hour.frame)?;
    rows.binary_search_by_key(&handle, |row| row.task_force)
        .ok()
        .map(|at| &rows[at])
        .ok_or_else(|| format!("regional mission {handle:#x} has no patrol operands"))
}

pub fn combined_detection(hour: &Hour<'_>, handle: u64) -> Fact<i64> {
    let force = hour.force(handle)?;
    let packet = packet(hour, handle)?;
    let known = hour.input(handle).ok();
    let (leader, headquarters) = if let Some(row) = known {
        (row.leader, row.headquarters.as_slice())
    } else {
        if !packet.headquarters_known {
            return Err("patrol headquarters references are unavailable".into());
        }
        (packet.leader, packet.headquarters.as_slice())
    };
    let location = if force.province == 0 {
        None
    } else {
        let (local, weather) = if let Some(row) = known {
            (row.local_detection, row.weather_detection)
        } else {
            if !packet.local_detection_known || !packet.weather_detection_known {
                return Err(
                    "patrol local or regional weather detection entry is unavailable".into(),
                );
            }
            (packet.local_detection, packet.weather_detection)
        };
        Some(derive::Location {
            terrain: hour.terrain_value(force, keys::NAVAL_DETECTION)?,
            weather: hour.effective(keys::NAVAL_DETECTION, weather)?,
            weather_reduction: derive::weather_reduction(
                hour.country_value(force.roster_owner, keys::NAVY_WEATHER_PENALTY)?,
            ),
            local: hour.effective(keys::NAVAL_DETECTION, local)?,
        })
    };
    let country = hour.country_id(force.roster_owner)?;
    let mut sums = Vec::with_capacity(headquarters.len());
    for &reference in headquarters {
        let mut terms = [0; 4];
        for (source, value) in terms.iter_mut().enumerate() {
            *value =
                hour.commander_value(country, reference, source as u8 + 1, keys::NAVAL_DETECTION)?;
        }
        sums.push(derive::headquarters_sum(terms));
    }
    Ok(derive::combined(
        hour.country_value(force.roster_owner, keys::NAVAL_DETECTION)?,
        location,
        hour.commander_value(country, leader, 0, keys::NAVAL_DETECTION)?,
        sums,
    ))
}

pub fn carrier_fraction(hour: &Hour<'_>, handle: u64) -> Fact<i64> {
    let models = data::<tables::ShipModels>(hour.frame)?;
    let mut sum = 0_i64;
    for ship in hour.ships(handle)? {
        let model = models
            .binary_search_by_key(&ship.id, |row| row.ship)
            .ok()
            .map(|at| &models[at])
            .ok_or("patrol carrier classification is unavailable")?;
        if !counter_order::is_carrier(model)
            .ok_or("patrol carrier classification inputs are unavailable")?
        {
            continue;
        }
        let base = model
            .counter
            .as_ref()
            .ok_or("patrol carrier base reference is unavailable")?
            .carrier_air_base;
        if base == 0 {
            continue;
        }
        let bases = data::<tables::AirBases>(hour.frame)?;
        let base = bases
            .binary_search_by_key(&base, |row| row.id)
            .ok()
            .map(|at| &bases[at])
            .ok_or("patrol carrier air base is unavailable")?;
        let countries = base
            .carrier
            .require("AirBases.carrier")?
            .as_ref()
            .and_then(|packet| packet.registered_countries.as_ref())
            .ok_or("patrol carrier dense country order is unavailable")?;
        if base.capacity == 0 {
            continue;
        }
        let registrations = data::<tables::AirBaseRegistrations>(hour.frame)?;
        let wings = data::<tables::AirWings>(hour.frame)?;
        let mut aircraft = 0_i32;
        for country in countries {
            let registration = registrations
                .binary_search_by_key(&(base.id, country.slot), |row| (row.base, row.slot))
                .ok()
                .map(|at| &registrations[at])
                .ok_or("patrol carrier country registration is unavailable")?;
            for &wing in &registration.wings {
                let wing = wings
                    .binary_search_by_key(&wing, |row| row.id)
                    .ok()
                    .map(|at| &wings[at])
                    .ok_or("patrol carrier registered wing is unavailable")?;
                aircraft = aircraft.wrapping_add(wing.planes);
            }
        }
        let fraction = divide(
            i64::from(aircraft).wrapping_mul(ONE),
            i64::from(base.capacity).wrapping_mul(ONE),
        )?
        .clamp(0, ONE);
        sum = sum.wrapping_add(fraction);
    }
    Ok(sum)
}

fn navy_source(hour: &Hour<'_>, handle: u64, index: usize) -> Fact<i64> {
    let row = packet(hour, handle)?;
    let (base, carrier) = if index == 0 {
        (
            row.surface_detection
                .ok_or("patrol surface detection statistic is unavailable")?,
            row.carrier_detection
                .ok_or("patrol carrier detection statistic is unavailable")?,
        )
    } else {
        (
            row.submarine_detection
                .ok_or("patrol submarine detection statistic is unavailable")?,
            row.carrier_submarine_detection
                .ok_or("patrol carrier submarine detection statistic is unavailable")?,
        )
    };
    let fraction = carrier_fraction(hour, handle)?;
    let modifier = combined_detection(hour, handle)?.wrapping_add(ONE);
    Ok(mul(base.wrapping_add(mul(fraction, carrier)), modifier).max(0))
}
fn navy_density(
    hour: &mut Hour<'_>,
    ledger: &NavalMissionLedgerRow,
    visibility: [i64; 2],
    surface_only: bool,
) -> Fact<i64> {
    let mut detection = [0_i64; 2];
    for &handle in &ledger.task_forces {
        for (index, total) in
            detection
                .iter_mut()
                .enumerate()
                .take(if surface_only { 1 } else { 2 })
        {
            let key = (handle, index);
            let source = if let Some(value) = hour.patrol_navy.get(&key) {
                value.clone()
            } else {
                let value = navy_source(hour, handle, index);
                hour.patrol_navy.insert(key, value.clone());
                value
            }?;
            *total = total.wrapping_add(source);
        }
    }
    Ok(mul(
        mul(visibility[0], detection[0]).wrapping_add(mul(visibility[1], detection[1])),
        hour.content.spotting.navy_factor,
    ))
}

fn radar_counts(hour: &Hour<'_>) -> Fact<BTreeMap<u32, i32>> {
    let mut counts = BTreeMap::<u32, i32>::new();
    for group in data::<tables::RadarCoverage>(hour.frame)? {
        let owner = hour.links.get(&group.country).copied().ok_or_else(|| {
            format!(
                "patrol radar country {} has no canonical country link",
                group.country
            )
        })?;
        if owner != hour.ours {
            continue;
        }
        for province in &group.provinces {
            let region = *hour
                .anchors
                .region_of
                .get(province)
                .ok_or("patrol radar province strategic region is unavailable")?;
            let value = counts.entry(region).or_default();
            *value = value.wrapping_add(1);
        }
    }
    Ok(counts)
}
fn radar(hour: &Hour<'_>, region: u32) -> Fact<i64> {
    let count = hour
        .patrol_radar
        .get_or_init(|| radar_counts(hour))
        .as_ref()
        .map_err(Clone::clone)?
        .get(&region)
        .copied()
        .unwrap_or(0);
    let members = hour.anchors.region_provinces(region).len() as i32;
    let fraction = divide(
        i64::from(count).wrapping_mul(ONE),
        i64::from(members).wrapping_mul(ONE),
    )?;
    Ok(mul(fraction, hour.content.spotting.radar))
}

#[derive(Clone, Copy, Debug, Default)]
pub(in crate) struct Breakdown {
    pub total: i64,
    pub base: i64,
    pub navy_density: i64,
    pub navy: i64,
    pub air_density: i64,
    pub air: i64,
    pub radar: i64,
    pub decryption: i64,
    pub dominance: bool,
    pub dominance_bonus: i64,
    pub weather: i64,
}
impl Breakdown {
    pub(in crate) fn read(
        hour: &mut Hour<'_>,
        ledger: &NavalMissionLedgerRow,
        visibility: [i64; 2],
        decryption: i64,
    ) -> Fact<Self> {
        Self::read_inner(hour, ledger, visibility, decryption, false)
    }
    pub(in crate) fn read_convoy(
        hour: &mut Hour<'_>,
        ledger: &NavalMissionLedgerRow,
        visibility: [i64; 2],
        decryption: i64,
    ) -> Fact<Self> {
        Self::read_inner(hour, ledger, visibility, decryption, true)
    }
    fn read_inner(
        hour: &mut Hour<'_>,
        ledger: &NavalMissionLedgerRow,
        visibility: [i64; 2],
        decryption: i64,
        convoy: bool,
    ) -> Fact<Self> {
        let defines = hour.content.spotting;
        let dominance = if convoy {
            false
        } else {
            let minimum = hour
                .content
                .terrain(Some(ledger.region))?
                .minimum_seazone_dominance;
            naval_region_control::Snapshot::read(
                hour.frame,
                hour.ours,
                ledger.region,
                minimum,
                hour.links,
            )?
            .control(minimum, defines.control_ratio)?
                == 1
        };
        let mut navy_density = navy_density(hour, ledger, visibility, convoy)?;
        let mut base = defines.base;
        if dominance {
            navy_density = mul(navy_density, defines.dominance.wrapping_add(ONE));
            base = base.wrapping_add(mul(defines.dominance, 10_000_000));
        }
        let navy = saturate(navy_density, defines.navy)?;
        let air_density =
            super::super::patrol_air::density(hour, ledger.region, visibility)?.unwrap_or(0);
        let air = saturate(air_density, defines.air)?;
        let radar = radar(hour, ledger.region)?;
        let weather = if let Some(row) = data::<tables::WeatherRegions>(hour.frame)?
            .iter()
            .find(|row| row.id == ledger.region)
        {
            if row.map_modifiers_known != replay_data::rows::Recorded::Value(true) {
                return Err("patrol regional weather modifier census is unavailable".into());
            }
            hour.effective(
                keys::NAVAL_DETECTION,
                *row.naval_detection.require("naval_detection")?,
            )?
            .wrapping_add(ONE)
            .max(0)
        } else {
            ONE
        };
        let total = mul(
            base.wrapping_add(navy)
                .wrapping_add(air)
                .wrapping_add(radar)
                .wrapping_add(decryption),
            weather,
        )
        .max(0);
        Ok(Self {
            total,
            base: defines.base,
            navy_density,
            navy,
            air_density,
            air,
            radar,
            decryption,
            dominance,
            dominance_bonus: defines.dominance,
            weather,
        })
    }
    pub(in crate) fn append(&self, out: &mut Prepared, body: bool) {
        out.line(
            body,
            0,
            "SPOTTING_BASE_CHANCE_TOTAL",
            vec![("VAL", Value::Fixed(printed(self.total)))],
        );
        out.line(
            body,
            0,
            "SPOTTING_BASE_CHANCE",
            vec![("VAL", Value::Fixed(printed(self.base)))],
        );
        if self.navy_density != 0 {
            out.line(
                body,
                0,
                "SPOTTING_BASE_CHANCE_FROM_NAVY",
                vec![("VAL", Value::Fixed(printed(self.navy)))],
            );
        }
        if self.dominance {
            out.line(
                body,
                0,
                "NAVAL_DOMINANCE_SPOTTING_BASE_CHANCE",
                vec![("VAL", Value::Fixed(self.dominance_bonus))],
            );
        }
        if self.air_density != 0 {
            out.line(
                body,
                0,
                "SPOTTING_BASE_CHANCE_FROM_AIR",
                vec![("VAL", Value::Fixed(printed(self.air)))],
            );
        }
        if self.decryption != 0 {
            out.line(
                body,
                0,
                "SPOTTING_BASE_CHANCE_FROM_DECRYPTION",
                vec![("VAL", Value::Fixed(printed(self.decryption)))],
            );
        }
        if self.radar != 0 {
            out.line(
                body,
                0,
                "SPOTTING_BASE_CHANCE_FROM_RADAR",
                vec![("VAL", Value::Fixed(printed(self.radar)))],
            );
        }
        if self.weather != ONE {
            let text =
                content_reader_core::fixed5::fixed5_percent(self.weather.wrapping_sub(ONE), 1, true);
            let value = format!(
                "\u{11}{}{text}\u{11}!",
                if self.weather < ONE { 'R' } else { 'Y' }
            );
            out.newline(body);
            out.line(
                body,
                0,
                "NAVAL_DETECTION_CHANCE_DETAILS_WEATHER",
                vec![("VAL", Value::Text(value))],
            );
        }
    }
}

fn hidden(hour: &Hour<'_>, region: u32) -> Fact<i32> {
    let order = data::<tables::ProvinceUnitOrder>(hour.frame)?;
    let intel = data::<tables::UnitIntel>(hour.frame)?;
    let relations = Relations::read(hour.frame, hour.links).map_err(str::to_owned)?;
    let mut count = 0_i32;
    for &province in hour.anchors.region_provinces(region) {
        if hour.anchors.kinds.get(&province) != Some(&content_reader_core::map::ProvinceKind::Sea) {
            continue;
        }
        let roster = order
            .binary_search_by_key(&province, |row| row.province)
            .ok()
            .map(|at| &order[at])
            .ok_or("patrol sea-province naval occurrence order is unavailable")?;
        for &handle in &roster.naval {
            let force = hour.force(handle)?;
            if !directed_wars::Owner::new(hour.frame, force.roster_owner)?
                .pointer_active_slot(hour.ours)?
            {
                continue;
            }
            let owner = hour
                .links
                .get(&force.roster_owner)
                .copied()
                .ok_or_else(|| {
                    format!(
                        "patrol task-force country {} has no canonical country link",
                        force.roster_owner
                    )
                })?;
            if relations.shared(owner, hour.ours) {
                continue;
            }
            let level = intel
                .iter()
                .filter(|row| row.unit == handle && row.viewer == hour.ours)
                .min_by_key(|row| row.ordinal)
                .map_or(0, |row| row.level);
            if level == 0 {
                count = count.wrapping_add(1);
            }
        }
    }
    Ok(count)
}

pub(crate) fn prepare(hour: &mut Hour<'_>, ledger: &NavalMissionLedgerRow, out: &mut Prepared) {
    out.newline(false);
    match Breakdown::read(hour, ledger, [200_000; 2], 0) {
        Ok(values) => values.append(out, false),
        Err(reason) => out.refuse("regional patrol spotting", reason),
    }
    match hidden(hour, ledger.region) {
        Ok(count) if count > 0 => out.line(
            false,
            1,
            "NAVAL_SPOTTING_STATE",
            vec![("NUM", Value::Int(count))],
        ),
        Ok(_) => {}
        Err(reason) => out.refuse("undetected regional naval occurrences", reason),
    }
    for &handle in &ledger.task_forces {
        let result = (|| {
            let row = packet(hour, handle)?;
            let target = row
                .target
                .ok_or("patrol retained target reference is unavailable")?;
            if target == 0 {
                return Ok(None);
            }
            if row
                .target_live
                .ok_or("patrol retained target lifetime is unavailable")?
                == 0
            {
                return Ok(None);
            }
            let force = hour.force(target)?;
            if hour.anchors.region_of.get(&force.province) != Some(&ledger.region) {
                return Ok(None);
            }
            Ok(Some((
                name(hour, handle)?,
                name(hour, target)?,
                row.progress
                    .ok_or("patrol retained progress is unavailable")?,
                row.chance
                    .ok_or("patrol retained detection chance is unavailable")?,
            )))
        })();
        match result {
            Ok(Some((name, target, progress, chance))) => out.line(
                false,
                1,
                "SPOTTING_DETECTION_PROCESS_PER_TASK_FORCE",
                vec![
                    ("NAME", name),
                    ("VALUE", Value::Fixed(printed(progress))),
                    ("CHANCE", Value::Fixed(printed(chance))),
                    ("TARGET", target),
                ],
            ),
            Ok(None) => {}
            Err(reason) => out.refuse("ordered patrol target progress", reason),
        }
    }
}

#[cfg(test)]
mod tests {
    use replay_data::rows::{NavalMissionLedgerRow, NavalPatrolMissionInputRow, RadarCoverageRow, Recorded, TableId, TableRows, TaskForceCounterInputRow, TaskForceRow};
    use super::{Breakdown, NativeInputs, ONE, Op, Prepared, REFUSED, Value, prepare, printed, radar_counts, saturate};
    #[test]
    fn progress_display_preserves_native_wrapping_and_guard() {
        assert_eq!(printed(100_000), 1000);
        assert_eq!(printed(-100_000), -1000);
        for raw in [i64::MIN, i64::MIN + 1, i64::MAX] {
            assert_eq!(printed(raw), REFUSED);
        }
    }
    #[test]
    fn logistic_zero_and_native_saturation_are_distinct() {
        assert_eq!(saturate(0, 777).unwrap(), 0);
        assert_eq!(saturate(1_000_000, 777).unwrap(), 777);
        assert_eq!(saturate(i64::MAX, 777).unwrap(), 777);
    }
    #[test]
    fn conditional_terms_keep_native_order_and_unclamped_progress() {
        let mut out = Prepared::default();
        Breakdown {
            total: 100_000,
            base: 100_000,
            navy_density: 1,
            navy: 2,
            air_density: 1,
            air: 3,
            radar: 4,
            decryption: 5,
            dominance: true,
            dominance_bonus: 6,
            weather: ONE,
        }
        .append(&mut out, false);
        let keys: Vec<_> = out
            .0
            .iter()
            .filter_map(|op| match op {
                Op::Line(_, _, key, _) => Some(*key),
                _ => None,
            })
            .collect();
        assert_eq!(
            keys,
            [
                "SPOTTING_BASE_CHANCE_TOTAL",
                "SPOTTING_BASE_CHANCE",
                "SPOTTING_BASE_CHANCE_FROM_NAVY",
                "NAVAL_DOMINANCE_SPOTTING_BASE_CHANCE",
                "SPOTTING_BASE_CHANCE_FROM_AIR",
                "SPOTTING_BASE_CHANCE_FROM_DECRYPTION",
                "SPOTTING_BASE_CHANCE_FROM_RADAR"
            ]
        );
    }
    #[test]
    fn navy_radar_counts_own_occurrences_without_air_controller_weighting() {
        use crate::naval_mission_inputs::tests as fixture;
        let mut frame = fixture::frame();
        fixture::set(
            &mut frame,
            TableRows::RadarCoverage(vec![
                RadarCoverageRow {
                    country: 1,
                    provinces: vec![1, 1, 2],
                    ..Default::default()
                },
                RadarCoverageRow {
                    country: 2,
                    provinces: vec![1],
                    ..Default::default()
                },
            ]),
        );
        *frame.tables[TableId::Provinces.index()] = replay_frame::frame::TableView::Unavailable {
            reason: "not used by Navy radar".into(),
        };
        *frame.tables[TableId::CountryCounterInputs.index()] =
            replay_frame::frame::TableView::Unavailable {
                reason: "not used by Navy radar".into(),
            };
        let mut anchors = fixture::anchors();
        anchors.mission_content = Ok(fixture::content(b"categories={ocean={}}"));
        let native = NativeInputs::default();
        let links = [(1, 1), (2, 2)].into();
        let ids = [(1, 1), (2, 2)].into();
        let hour = fixture::hour(&frame, &anchors, &native, (&links, &ids));
        assert_eq!(radar_counts(&hour).unwrap(), [(1, 2), (2, 1)].into());
    }
    #[test]
    fn progress_preserves_ledger_repeats_when_an_unrelated_breakdown_refuses() {
        use crate::naval_mission_inputs::tests as fixture;
        let mut frame = fixture::frame();
        fixture::set(
            &mut frame,
            TableRows::TaskForces(vec![
                TaskForceRow {
                    id: 11,
                    roster_owner: 1,
                    mission: 1,
                    ..Default::default()
                },
                TaskForceRow {
                    id: 22,
                    roster_owner: 2,
                    province: 1,
                    ..Default::default()
                },
            ]),
        );
        fixture::set(
            &mut frame,
            TableRows::TaskForceCounterInputs(vec![
                TaskForceCounterInputRow {
                    task_force: 11,
                    name: Recorded::Value(Some(b"Source".to_vec())),
                    ..Default::default()
                },
                TaskForceCounterInputRow {
                    task_force: 22,
                    name: Recorded::Value(Some(b"Target".to_vec())),
                    ..Default::default()
                },
            ]),
        );
        fixture::set(
            &mut frame,
            TableRows::NavalPatrolMissionInputs(vec![NavalPatrolMissionInputRow {
                task_force: 11,
                target: Some(22),
                target_live: Some(1),
                progress: Some(125_000),
                chance: Some(-25_000),
                ..Default::default()
            }]),
        );
        let mut anchors = fixture::anchors();
        anchors.mission_content = Ok(fixture::content(b"categories={ocean={}}"));
        let native = NativeInputs::read(&frame).unwrap();
        let links = [(1, 1), (2, 2)].into();
        let ids = [(1, 1), (2, 2)].into();
        let mut hour = fixture::hour(&frame, &anchors, &native, (&links, &ids));
        let mut prepared = Prepared::default();
        prepare(
            &mut hour,
            &NavalMissionLedgerRow {
                owner: 1,
                region: 1,
                task_forces: vec![11, 11],
                ..Default::default()
            },
            &mut prepared,
        );
        let entries: Vec<_> = prepared
            .0
            .iter()
            .filter_map(|op| match op {
                Op::Line(_, _, "SPOTTING_DETECTION_PROCESS_PER_TASK_FORCE", args) => Some(args),
                _ => None,
            })
            .collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], entries[1]);
        assert!(entries[0].contains(&("VALUE", Value::Fixed(1250))));
        assert!(entries[0].contains(&("CHANCE", Value::Fixed(-250))));
        assert!(
            prepared
                .0
                .iter()
                .any(|op| matches!(op, Op::Refused("regional patrol spotting", _)))
        );
    }
}
