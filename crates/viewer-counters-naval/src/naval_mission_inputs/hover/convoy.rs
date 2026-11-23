use replay_data::rows::{ClockState, NavalMissionLedgerRow, NavalMissionTaskForceRow};
use super::{Fact, HashMap, Hour, Prepared, TokenError, Value, data, derive, io, keys, patrol, tables};
#[cfg(test)]
use super::{CountryIntelRow, Localisation, NativeInputs};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Defines {
    base: [i64; 3],
    multiplier: [i64; 3],
    surface: i64,
    sub: i64,
    count_power: i64,
    visibility: i64,
    detection: i64,
    repair_visibility: i64,
    enemy_running: i64,
    running: i64,
    catching: i64,
    coordination: i64,
    naval_speed: i64,
    radar: i64,
    air_superiority: i64,
    decryption_min: i64,
    decryption_max: i64,
    decryption_bonus: i64,
    out_of_fuel: i64,
    day_night: replay_data::day_night::Defines,
}
impl Defines {
    pub(crate) fn read(defines: &content_reader_defines::defines::Defines) -> io::Result<Self> {
        let get = |key| defines.fixed5("NNavy", key);
        Ok(Self {
            base: [
                get("BASE_CONVOY_SPOTTING_SPEED")?,
                get("BASE_UNIT_TRANSFER_SPOTTING_SPEED")?,
                get("BASE_NAVAL_INVASION_SPOTTING_SPEED")?,
            ],
            multiplier: [
                get("CONVOY_SPOTTING_SPEED_MULT")?,
                get("UNIT_TRANSFER_SPOTTING_SPEED_MULT")?,
                get("NAVAL_INVASION_SPOTTING_SPEED_MULT")?,
            ],
            surface: get("SPOTTING_MULTIPLIER_FOR_SURFACE")?,
            sub: get("SPOTTING_MULTIPLIER_FOR_SUB")?,
            count_power: get("SPOTTING_MOD_FOR_CONVOY_COUNT")?,
            visibility: get("VISIBILITY_MULTIPLIER_FOR_SPOTTING")?,
            detection: get("DETECTION_CHANCE_MULT_BASE")?,
            repair_visibility: get("NAVY_VISIBILITY_BONUS_ON_RETURN_FOR_REPAIR")?,
            enemy_running: get("SPOTTING_ENEMY_SPOTTING_MULTIPLIER_FOR_RUNNING_AWAY")?,
            running: get("SPOTTING_SPEED_MULT_FOR_RUNNING_AWAY")?,
            catching: get("SPOTTING_SPEED_MULT_FOR_CATCHING_UP")?,
            coordination: get("COORDINATION_EFFECT_ON_PATROL_SPOTTING")?,
            naval_speed: get("NAVAL_SPEED_MODIFIER")?,
            radar: get("DETECTION_CHANCE_MULT_RADAR_BONUS")?,
            air_superiority: get("DETECTION_CHANCE_MULT_AIR_SUPERIORITY_BONUS")?,
            decryption_min: defines.fixed5(
                "NIntel",
                "NAVY_INTEL_BASE_SPOTTING_BONUS_MIN_INTEL_FOR_BONUS",
            )?,
            decryption_max: defines.fixed5(
                "NIntel",
                "NAVY_INTEL_BASE_SPOTTING_BONUS_MAX_INTEL_FOR_BONUS",
            )?,
            decryption_bonus: get("BASE_SPOTTING_FROM_DECRYPTION")?,
            out_of_fuel: get("OUT_OF_FUEL_SPEED_FACTOR")?,
            day_night: replay_data::day_night::Defines::read(
                |section, key| defines.int32(section, key),
                |section, key| defines.fixed5(section, key),
            )?,
        })
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Inputs {
    pub kind: usize,
    pub convoys: i32,
    pub design: Option<[i64; 4]>,
    pub detection: i64,
    pub radar: i64,
    pub air_superiority: i64,
    pub supply: Option<i64>,
    pub visibility: [i64; 2],
    pub visibility_modifier: i64,
    pub repair: bool,
    pub speed: i64,
    pub coordination: i64,
    pub spotting_modifier: i64,
    pub navy_spotting_modifier: i64,
    pub night: bool,
    pub night_modifier: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Speed {
    value: i64,
    base: i64,
    surface: i64,
    our_detection: i64,
    their_detection: i64,
    sub: i64,
    their_sub_detection: i64,
    delta: i64,
    coordination: i64,
    spotting: i64,
    night: Option<i64>,
}

fn mul(left: i64, right: i64) -> i64 {
    left.wrapping_mul(right) / 100_000
}

pub(crate) fn speed(input: Inputs, d: Defines) -> Fact<Speed> {
    let Some(design) = input.design else {
        return Ok(Speed {
            base: d.base[input.kind],
            ..Default::default()
        });
    };
    let weight = mul(d.multiplier[input.kind], d.surface);
    let count = derive::power(
        i64::from(input.convoys).wrapping_mul(100_000),
        d.count_power,
    )
    .map_err(str::to_owned)?;
    let ours = mul(
        mul(mul(design[0], d.visibility), count).max(0),
        mission_detection(input, d),
    )
    .max(0);
    let return_factor = if input.repair {
        d.repair_visibility
    } else {
        100_000
    };
    let visibility = input.visibility.map(|raw| {
        mul(
            mul(mul(raw, d.visibility), return_factor),
            input.visibility_modifier.wrapping_add(100_000),
        )
        .max(0)
    });
    let theirs = mul(
        mul(
            visibility[0],
            design[1].wrapping_mul(d.detection).wrapping_mul(1000) / 100_000,
        )
        .max(0),
        d.enemy_running,
    );
    let sub_theirs = mul(
        mul(
            visibility[1],
            design[2].wrapping_mul(d.detection).wrapping_mul(1000) / 100_000,
        )
        .max(0),
        d.enemy_running,
    );
    let surface = mul(ours.wrapping_sub(theirs), weight);
    let sub = mul(sub_theirs.wrapping_neg(), d.sub);
    let mut value =
        d.base[input.kind].wrapping_add(surface.wrapping_add(sub).wrapping_mul(100) / 100_000);
    let delta = input.speed.wrapping_sub(mul(design[3], d.naval_speed));
    let delta = mul(
        mul(delta, if delta < 0 { d.running } else { d.catching }),
        weight,
    );
    value = value.wrapping_add(delta);
    let coordination = mul(input.coordination, d.coordination).wrapping_add(100_000);
    value = mul(coordination, value);
    let mut out = Speed {
        value,
        base: d.base[input.kind],
        surface,
        our_detection: ours,
        their_detection: theirs,
        sub,
        their_sub_detection: sub_theirs,
        delta,
        coordination,
        spotting: 100_000,
        night: None,
    };
    if value > 0 {
        out.spotting = input
            .spotting_modifier
            .wrapping_add(input.navy_spotting_modifier)
            .wrapping_add(100_000);
        out.value = mul(out.spotting, out.value);
        if input.night {
            out.value = mul(input.night_modifier.wrapping_add(100_000), out.value);
            out.night = Some(input.night_modifier);
        }
    }
    Ok(out)
}

pub(crate) fn mission_detection(input: Inputs, d: Defines) -> i64 {
    let raw = d
        .detection
        .wrapping_add(mul(input.radar, d.radar))
        .wrapping_add(mul(input.air_superiority, d.air_superiority))
        .wrapping_mul(1000);
    let factor = if let Some(supply) = input.supply.filter(|&value| value < 100_000) {
        mul(supply, raw)
    } else {
        raw
    };
    mul(input.detection, factor)
}

pub fn decryption(hour: &Hour<'_>, owner: u32, target: u32) -> Fact<i64> {
    let owner_id = hour
        .country_id(owner)
        .map_err(|reason| format!("convoy decryption owner country is unresolved: {reason}"))?;
    if (target as i32) <= 0 {
        return Ok(decryption_value(0, hour.content.convoy));
    }
    let target_id = hour
        .country_id(target)
        .map_err(|reason| format!("convoy decryption target country is unresolved: {reason}"))?;
    let counters = data::<tables::CountryCounterInputs>(hour.frame)?;
    let target_has_states = counters
        .binary_search_by_key(&target_id, |row| row.country)
        .ok()
        .map(|at| counters[at].owned_state_count > 0)
        .ok_or_else(|| {
            format!("convoy decryption target country {target_id} owned-state count is unavailable")
        })?;
    if !target_has_states {
        return Ok(decryption_value(0, hour.content.convoy));
    }
    let owner_slot = hour
        .links
        .get(&owner)
        .copied()
        .ok_or("convoy spotting owner country link is unavailable")?;
    let target_slot = hour
        .links
        .get(&target)
        .copied()
        .ok_or("convoy spotting target country link is unavailable")?;
    let rows = data::<tables::CountryIntel>(hour.frame)?;
    let navy = |owner, index| {
        if owner == 0 {
            return 0;
        }
        rows.binary_search_by_key(&(owner, index), |row| (row.owner_country, row.target_index))
            .ok()
            .map_or(0, |at| rows[at].navy)
    };
    Ok(decryption_value(
        navy(owner_id, target_slot).wrapping_sub(navy(target_id, owner_slot)),
        hour.content.convoy,
    ))
}

fn decryption_value(difference: i64, defines: Defines) -> i64 {
    let above = difference
        .wrapping_abs()
        .wrapping_sub(defines.decryption_min);
    if above < 0 {
        return 0;
    }
    let span = defines.decryption_max.wrapping_sub(defines.decryption_min);
    let numerator = mul(above.min(span), defines.decryption_bonus);
    let bonus = if span == 0 {
        0
    } else if (numerator as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd {
        numerator.wrapping_mul(100_000).wrapping_div(span)
    } else {
        0xffff_ffff
    };
    if difference < 1 {
        bonus.wrapping_neg()
    } else {
        bonus
    }
}

fn terrain_speed(hour: &Hour<'_>, handle: u64, class: i32) -> Fact<i64> {
    let force = hour.force(handle)?;
    if force.province == 0 {
        return Ok(0);
    }
    let region = hour
        .anchors
        .region_of
        .get(&force.province)
        .copied()
        .ok_or("convoy spotting source strategic region is unavailable")?;
    let selected = hour
        .content
        .naval_terrain
        .get(&region)
        .and_then(|key| hour.content.terrain.category(key))
        .or_else(|| hour.content.terrain.category(b"ocean"));
    let Some(at) = selected else {
        return Ok(0);
    };
    let terrain = &hour.content.terrain.categories[at];
    let movement = |class: i32| -> Fact<i64> {
        let common = terrain.common_adjuster.movement;
        if terrain.classes.is_empty() {
            return Ok(common);
        }
        let names = hour.names.as_ref().map_err(Clone::clone)?;
        let name = match names.name(class as u32) {
            Ok(name) => name,
            Err(TokenError::Rejected | TokenError::Absent(_)) => return Ok(common),
            Err(reason) => return Err(reason.to_string()),
        };
        let Some(at) = terrain
            .classes
            .iter()
            .position(|(key, _)| key.eq_ignore_ascii_case(name.as_bytes()))
        else {
            return Ok(common);
        };
        if hour
            .anchors
            .subunit_tokens
            .as_ref()
            .map_err(Clone::clone)?
            .resolve(name)?
            .is_none()
        {
            return Ok(common);
        }
        Ok(common.wrapping_add(
            terrain
                .class_adjusters
                .get(at)
                .ok_or("convoy spotting terrain class adjustment is unavailable")?
                .movement,
        ))
    };
    if class >= 0 {
        return movement(class);
    }
    let mut classes = HashMap::<i32, i32>::new();
    for ship in hour.ships(handle)? {
        *classes.entry(ship.class_id).or_default() += 1;
    }
    Ok(derive::terrain_mean(
        classes
            .into_iter()
            .map(|(class, count)| movement(class).map(|value| (count, value)))
            .collect::<Fact<Vec<_>>>()?,
    ))
}

fn night(hour: &Hour<'_>, province: u32) -> Fact<bool> {
    let rows = data::<tables::WeatherProvinces>(hour.frame)?;
    let row = rows
        .binary_search_by_key(&province, |row| row.id)
        .ok()
        .map(|at| &rows[at])
        .ok_or("convoy spotting source day/night record is unavailable")?;
    let clock = match row.clock.state(province).map_err(str::to_owned)? {
        ClockState::NotRecorded => {
            return Err("convoy spotting source day/night clock was not recorded".into());
        }
        ClockState::Held(clock) => clock,
        ClockState::Updated => {
            let (_, geometry) = hour.topology.as_ref().map_err(Clone::clone)?;
            let centroid = geometry
                .centroids
                .get(&province)
                .copied()
                .ok_or("convoy spotting source centroid is unavailable")?;
            let defines = &hour.content.convoy.day_night;
            replay_data::day_night::ProvinceClock::new(
                centroid,
                [geometry.width, geometry.height],
                defines,
            )
            .update(
                &replay_data::day_night::HourOperands::new(hour.frame.hour, defines)
                    .map_err(str::to_owned)?,
            )
        }
    };
    Ok(clock.night_factor > 80_000)
}

fn read(hour: &mut Hour<'_>, handle: u64, row: &NavalMissionTaskForceRow) -> Fact<Inputs> {
    let live = row
        .raiding_targets
        .ok_or("convoy spotting target lifetimes are unavailable")?;
    let trade = live[0].ok_or("convoy spotting trade-target lifetime is unavailable")?;
    let kind = if trade {
        0
    } else {
        if !live[1].ok_or("convoy spotting transport-target lifetime is unavailable")? {
            return Err("convoy spotting has no live target".into());
        }
        if row
            .raiding_transport_invasion
            .ok_or("convoy spotting transport invasion flag is unavailable")?
            != 0
        {
            2
        } else {
            1
        }
    };
    let variant = row
        .raiding_convoy_variant
        .ok_or("convoy spotting target design roster is unavailable")?;
    if variant == 0 {
        return Ok(Inputs {
            kind,
            ..Default::default()
        });
    }
    let design = [
        row.raiding_convoy_visibility
            .ok_or("convoy spotting target design visibility is unavailable")?,
        row.raiding_convoy_surface_detection
            .ok_or("convoy spotting target design surface detection is unavailable")?,
        row.raiding_convoy_sub_detection
            .ok_or("convoy spotting target design submarine detection is unavailable")?,
        row.raiding_convoy_maximum_speed
            .ok_or("convoy spotting target design speed is unavailable")?,
    ];
    let packet = patrol::packet(hour, handle)?;
    let counter = hour
        .native
        .task_force(handle)
        .ok_or("convoy spotting source activation and supply are unavailable")?;
    let force = hour.force(handle)?;
    let fraction = patrol::carrier_fraction(hour, handle)?;
    let detection = mul(
        packet
            .surface_detection
            .ok_or("convoy spotting source surface detection is unavailable")?
            .wrapping_add(mul(
                fraction,
                packet
                    .carrier_detection
                    .ok_or("convoy spotting source carrier detection is unavailable")?,
            )),
        patrol::combined_detection(hour, handle)?.wrapping_add(100_000),
    )
    .max(0);
    let source_class = row
        .raiding_source_terrain_class
        .ok_or("convoy spotting source terrain class is unavailable")?;
    let source_base = row
        .raiding_source_speed
        .ok_or("convoy spotting source base speed is unavailable")?;
    let fuel = derive::fuel_fraction(row.fuel, row.fuel_requested);
    let modifier = hour
        .combined(handle, keys::NAVAL_SPEED_FACTOR)?
        .wrapping_add(terrain_speed(hour, handle, source_class)?)
        .wrapping_add(mul(
            100_000_i64.wrapping_sub(fuel),
            hour.content.convoy.out_of_fuel,
        ));
    let source_speed = mul(
        mul(source_base, modifier.wrapping_add(100_000).max(10_000)),
        hour.content.convoy.naval_speed,
    );
    let mut inputs = Inputs {
        kind,
        convoys: row
            .raiding_target_convoys
            .ok_or("convoy spotting target convoy count is unavailable")?,
        design: Some(design),
        detection,
        radar: packet
            .radar_cache
            .ok_or("convoy spotting retained radar input is unavailable")?,
        air_superiority: packet
            .air_cache
            .ok_or("convoy spotting retained Air-superiority input is unavailable")?,
        supply: (counter.is_in_regions != 0).then_some(counter.received_supply_ratio),
        visibility: [
            packet
                .surface_visibility
                .ok_or("convoy spotting source surface visibility is unavailable")?,
            packet
                .submarine_visibility
                .ok_or("convoy spotting source submarine visibility is unavailable")?,
        ],
        visibility_modifier: hour.combined(handle, keys::NAVY_VISIBILITY)?,
        repair: packet
            .formation_class
            .ok_or("convoy spotting source formation state is unavailable")?
            == 1,
        speed: source_speed,
        coordination: hour.combined(handle, keys::NAVAL_COORDINATION)?,
        ..Default::default()
    };
    if speed(inputs, hour.content.convoy)?.value > 0 {
        inputs.spotting_modifier = hour.combined(handle, keys::SPOTTING_CHANCE)?;
        if !packet.targeted_spotting_known {
            return Err("convoy spotting current relation modifier is unavailable".into());
        }
        inputs.navy_spotting_modifier =
            hour.effective(keys::SPOTTING_CHANCE_AGAINST, packet.targeted_spotting)?;
        inputs.night = night(hour, force.province)?;
        if inputs.night {
            inputs.night_modifier = hour.combined(handle, keys::NIGHT_SPOTTING_CHANCE)?;
        }
    }
    Ok(inputs)
}

pub(in crate::naval_mission_inputs) fn prepare(
    hour: &mut Hour<'_>,
    ledger: &NavalMissionLedgerRow,
    handle: u64,
) -> Prepared {
    let mut out = Prepared::default();
    let row = match hour.input(handle) {
        Ok(row) => row,
        Err(reason) => {
            out.refuse("convoy spotting", reason);
            return out;
        }
    };
    let progress = match row.raiding_spotting {
        Some(value) => value,
        None => {
            out.refuse(
                "NAVAL_SPOTTING_PROGRESS",
                "convoy spotting progress is unavailable".into(),
            );
            return out;
        }
    };
    let inputs = match read(hour, handle, row) {
        Ok(value) => value,
        Err(reason) => {
            out.line(
                false,
                0,
                "NAVAL_SPOTTING_PROGRESS",
                vec![(
                    "PROGRESS",
                    Value::Float(((progress as f32 / 100_000.0) / 100.0).to_bits()),
                )],
            );
            out.refuse("convoy spotting speed", reason);
            return out;
        }
    };
    match speed(inputs, hour.content.convoy) {
        Ok(value) => append(
            progress,
            value,
            inputs.design.is_some(),
            hour.content.convoy,
            &mut out,
        ),
        Err(reason) => out.refuse("convoy spotting speed", reason),
    }
    out.newline(true);
    let result = (|| {
        let Some(design) = inputs.design else {
            return Ok(None);
        };
        let target = row
            .raiding_target_country
            .ok_or("convoy spotting target country is unavailable")?;
        let decrypt = decryption(hour, row.mission_owner, target)?;
        patrol::Breakdown::read_convoy(
            hour,
            ledger,
            [mul(design[0], hour.content.convoy.visibility), 0],
            decrypt,
        )
        .map(Some)
    })();
    match result {
        Ok(Some(breakdown)) => breakdown.append(&mut out, true),
        Ok(None) => {}
        Err(reason) => out.refuse("convoy spotting base chance", reason),
    }
    out
}

fn daily(raw: i64) -> i64 {
    let raw = raw.wrapping_mul(2_400_000) / 100_000;
    if (raw as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd {
        raw.wrapping_mul(100_000) / 10_000_000
    } else {
        0xffff_ffff
    }
}

fn float(raw: i64, multiplier: f32) -> Value {
    Value::Float(((raw as f32 / 100_000.0) * multiplier).to_bits())
}
fn coloured(raw: i64, precision: i32, percent: bool) -> String {
    let number = if percent {
        content_reader_parser::fixed5::fixed5_percent(raw, precision, true)
    } else {
        content_reader_parser::fixed5::fixed5_to_string(raw, precision)
    };
    format!(
        "\u{11}{}{number}\u{11}!",
        if raw > 0 {
            'G'
        } else if raw < 0 {
            'R'
        } else {
            'Y'
        }
    )
}

pub(crate) fn append(
    progress: i64,
    speed: Speed,
    design: bool,
    defines: Defines,
    out: &mut Prepared,
) {
    out.line(
        false,
        0,
        "NAVAL_SPOTTING_PROGRESS",
        vec![(
            "PROGRESS",
            Value::Float(((progress as f32 / 100_000.0) / 100.0).to_bits()),
        )],
    );
    out.newline(false);
    out.line(
        false,
        0,
        "NAVAL_SPOTTING_DAILY_CHANGE",
        vec![("SPEED", Value::Fixed(daily(speed.value)))],
    );
    out.line(
        true,
        0,
        "NAVAL_SPOTTING_HOURLY_CHANGE",
        vec![(
            "SPEED",
            Value::Float(((speed.value as f32 / 100_000.0) / 100.0).to_bits()),
        )],
    );
    out.newline(true);
    if speed.base != 0 {
        out.line(true, 0, "BASE", vec![]);
        out.raw(
            true,
            "base spotting speed",
            Value::Text(format!(" {}\n", coloured(speed.base, 2, false))),
        );
    }
    if !design {
        return;
    }
    out.line(
        true,
        0,
        "NAVAL_SPOTTING_SURFACE_DETECTION",
        vec![("VAL", float(speed.surface, 0.00001))],
    );
    out.raw(true, "spotting indentation", Value::Text("\n  ".into()));
    out.line(
        true,
        0,
        "NAVAL_SPOTTING_OUR_DETECTION",
        vec![("VAL", float(speed.our_detection, 0.00001))],
    );
    out.raw(true, "spotting indentation", Value::Text("\n  ".into()));
    out.line(
        true,
        0,
        "NAVAL_SPOTTING_THEIR_DETECTION",
        vec![("VAL", float(speed.their_detection, 0.00001))],
    );
    out.newline(true);
    if defines.sub != 0 {
        out.line(
            true,
            0,
            "NAVAL_SPOTTING_SUB_DETECTION",
            vec![("VAL", float(speed.sub, 0.00001))],
        );
        out.raw(true, "spotting indentation", Value::Text("\n  ".into()));
        out.line(
            true,
            0,
            "NAVAL_SPOTTING_OUR_DETECTION",
            vec![("VAL", Value::Float(0))],
        );
        out.raw(true, "spotting indentation", Value::Text("\n  ".into()));
        out.line(
            true,
            0,
            "NAVAL_SPOTTING_THEIR_DETECTION",
            vec![("VAL", float(speed.their_sub_detection, 0.00001))],
        );
        out.newline(true);
    }
    out.line(
        true,
        0,
        "NAVAL_SPOTTING_DUE_TO_SPEED_DELTA",
        vec![("VAL", float(speed.delta, 0.01))],
    );
    out.newline(true);
    out.line(
        true,
        0,
        "NAVAL_SPOTTING_COORDINATION_FACTOR",
        vec![("VAL", Value::Fixed(speed.coordination))],
    );
    out.newline(true);
    if speed.spotting != 100_000 {
        out.line(true, 0, "MODIFIER_SPOTTING_CHANCE_MODIFIER", vec![]);
        out.raw(
            true,
            "spotting modifier",
            Value::Text(coloured(speed.spotting.wrapping_sub(100_000), 0, true)),
        );
        out.newline(true);
    }
    if let Some(night) = speed.night.filter(|_| speed.spotting != 100_000) {
        out.line(true, 0, "MODIFIER_NIGHT_SPOTTING_CHANCE_MODIFIER", vec![]);
        out.raw(
            true,
            "night spotting modifier",
            Value::Text(coloured(night, 0, true)),
        );
        out.newline(true);
    }
}

#[cfg(test)]
mod tests {
    use replay_data::rows::{CountryCounterInputRow, TableId, TableRows};
    use super::{CountryIntelRow, Defines, Inputs, Localisation, NativeInputs, Prepared, Speed, append, daily, decryption, decryption_value, speed};
    #[test]
    fn aliased_intel_owners_use_current_ids_and_canonical_target_slots() {
        use crate::naval_mission_inputs::tests as fixture;
        let mut frame = fixture::frame();
        fixture::set(
            &mut frame,
            TableRows::CountryIntel(vec![
                CountryIntelRow {
                    owner_country: 10,
                    target_index: 17,
                    navy: 50_000,
                    ..Default::default()
                },
                CountryIntelRow {
                    owner_country: 20,
                    target_index: 13,
                    navy: 20_000,
                    ..Default::default()
                },
            ]),
        );
        fixture::set(
            &mut frame,
            TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                country: 20,
                owned_state_count: 1,
                ..Default::default()
            }]),
        );
        let mut anchors = fixture::anchors();
        let mut content = fixture::content(b"categories={ocean={}}");
        content.convoy = Defines {
            decryption_max: 100_000,
            decryption_bonus: 100_000,
            ..Default::default()
        };
        anchors.mission_content = Ok(content);
        let native = NativeInputs::default();
        let links = [(7, 13), (9, 17)].into();
        let ids = [(13, 10), (17, 20)].into();
        let hour = fixture::hour(&frame, &anchors, &native, (&links, &ids));
        assert_eq!(decryption(&hour, 7, 9), Ok(30_000));
        assert!(
            decryption(&hour, 8, 9)
                .unwrap_err()
                .contains("convoy decryption owner country is unresolved")
        );
        assert!(
            decryption(&hour, 7, 8)
                .unwrap_err()
                .contains("convoy decryption target country is unresolved")
        );
    }

    #[test]
    fn owner_resolves_before_stateless_target_zeros_difference_without_intel() {
        use crate::naval_mission_inputs::tests as fixture;
        let mut frame = fixture::frame();
        fixture::set(
            &mut frame,
            TableRows::CountryIntel(vec![
                CountryIntelRow {
                    owner_country: 10,
                    target_index: 17,
                    navy: 50_000,
                    ..Default::default()
                },
                CountryIntelRow {
                    owner_country: 20,
                    target_index: 13,
                    navy: 90_000,
                    ..Default::default()
                },
            ]),
        );
        let mut anchors = fixture::anchors();
        let mut content = fixture::content(b"categories={ocean={}}");
        content.convoy = Defines {
            decryption_max: 100_000,
            decryption_bonus: 100_000,
            ..Default::default()
        };
        anchors.mission_content = Ok(content);
        let native = NativeInputs::default();
        let links = [(7, 13), (9, 17)].into();
        let ids = [(13, 10), (17, 20)].into();
        for (states, expected) in [(1, -40_000), (0, 0)] {
            fixture::set(
                &mut frame,
                TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                    country: 20,
                    owned_state_count: states,
                    ..Default::default()
                }]),
            );
            let hour = fixture::hour(&frame, &anchors, &native, (&links, &ids));
            assert_eq!(decryption(&hour, 7, 9), Ok(expected));
        }
        *frame.tables[TableId::CountryIntel.index()] =
            replay_frame::frame::TableView::Unavailable {
                reason: "intel unavailable behind the stateless-target gate".into(),
            };
        let hour = fixture::hour(&frame, &anchors, &native, (&links, &ids));
        assert_eq!(decryption(&hour, 7, 9), Ok(0));
        assert!(
            decryption(&hour, 8, 9)
                .unwrap_err()
                .contains("convoy decryption owner country is unresolved")
        );
        fixture::set(&mut frame, TableRows::CountryCounterInputs(Vec::new()));
        let hour = fixture::hour(&frame, &anchors, &native, (&links, &ids));
        assert!(
            decryption(&hour, 7, 9)
                .unwrap_err()
                .contains("target country 20 owned-state count")
        );
    }

    #[test]
    fn null_target_is_zero_after_owner_resolution_before_target_or_ownership_reads() {
        use crate::naval_mission_inputs::tests as fixture;
        let mut frame = fixture::frame();
        for table in [TableId::CountryCounterInputs, TableId::CountryIntel] {
            *frame.tables[table.index()] = replay_frame::frame::TableView::Unavailable {
                reason: "unavailable behind the null-target gate".into(),
            };
        }
        let mut anchors = fixture::anchors();
        anchors.mission_content = Ok(fixture::content(b"categories={ocean={}}"));
        let native = NativeInputs::default();
        let links = [(7, 13)].into();
        let ids = [(13, 10)].into();
        let hour = fixture::hour(&frame, &anchors, &native, (&links, &ids));
        for target in [0, u32::MAX] {
            assert_eq!(decryption(&hour, 7, target), Ok(0));
            assert!(
                decryption(&hour, 8, target)
                    .unwrap_err()
                    .contains("convoy decryption owner country is unresolved")
            );
        }
        assert!(
            decryption(&hour, 8, 9)
                .unwrap_err()
                .contains("convoy decryption owner country is unresolved")
        );
    }

    #[test]
    fn absent_convoy_design_is_known_zero_before_task_force_inputs() {
        assert_eq!(
            speed(
                Inputs {
                    detection: i64::MAX,
                    ..Default::default()
                },
                Defines::default()
            )
            .unwrap(),
            Speed::default()
        );
    }
    #[test]
    fn daily_change_wraps_before_guard_and_truncates() {
        assert_eq!(daily(100_000), 24_000);
        assert_eq!(daily(-100_000), -24_000);
    }
    #[test]
    fn decryption_uses_absolute_threshold_then_original_sign() {
        let defines = Defines {
            decryption_min: 500_000,
            decryption_max: 4_000_000,
            decryption_bonus: 1_000_000,
            ..Default::default()
        };
        assert_eq!(decryption_value(499_999, defines), 0);
        assert_eq!(decryption_value(2_250_000, defines), 500_000);
        assert_eq!(decryption_value(-2_250_000, defines), -500_000);
        assert_eq!(decryption_value(5_000_000, defines), 1_000_000);
    }
    #[test]
    fn speed_keeps_the_submarine_term_outside_the_convoy_kind_multiplier() {
        let defines = Defines {
            multiplier: [100_000, 500_000, 1_000_000],
            surface: 100_000,
            sub: 100_000,
            count_power: 100_000,
            visibility: 100_000,
            detection: 100_000,
            enemy_running: 100_000,
            ..Default::default()
        };
        let input = Inputs {
            convoys: 1,
            design: Some([100_000, 0, 100_000, 0]),
            detection: 100_000,
            visibility: [0, 100_000],
            ..Default::default()
        };
        let one = speed(input, defines).unwrap();
        let five = speed(Inputs { kind: 1, ..input }, defines).unwrap();
        assert_eq!(one.sub, five.sub);
        assert_eq!(one.surface.wrapping_mul(5), five.surface);
    }
    #[test]
    fn positive_speed_applies_spotting_then_night_and_keeps_native_description_gate() {
        let defines = Defines {
            base: [100_000, 0, 0],
            ..Default::default()
        };
        let input = Inputs {
            design: Some([0; 4]),
            spotting_modifier: 50_000,
            night: true,
            night_modifier: 100_000,
            ..Default::default()
        };
        let speed = speed(input, defines).unwrap();
        assert_eq!(speed.value, 300_000);
        let mut out = Prepared::default();
        append(0, speed, true, defines, &mut out);
        let text = out.render(&Localisation::default(), "english", None);
        assert!(
            text.body
                .contains("MODIFIER_NIGHT_SPOTTING_CHANCE_MODIFIER")
        );
        let speed = super::speed(
            Inputs {
                spotting_modifier: 0,
                ..input
            },
            defines,
        )
        .unwrap();
        assert_eq!(speed.value, 200_000);
        let mut out = Prepared::default();
        append(0, speed, true, defines, &mut out);
        assert!(
            !out.render(&Localisation::default(), "english", None)
                .body
                .contains("MODIFIER_NIGHT_SPOTTING_CHANCE_MODIFIER")
        );
    }
}
