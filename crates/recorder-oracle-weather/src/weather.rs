
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use content_reader_save::{Decimal, Save};
use content_reader_weather::{
    weather::GROUND_SNOW_MODIFIERS, weather::MUD_MODIFIER, weather::ProvinceModifierRules,
    weather::TEMPERATURE_MODIFIERS,
};
use replay_data::{
    rows::ClockState, rows::ProvinceStaticModifiers, rows::ProvinceWeatherModifiers,
    rows::Recorded, rows::TemperatureRecord, rows::tables,
};
use replay_derive::modifier_registry::Registry;
use replay_frame::frame::DecodedFrame;

use super::{Report, complete, invalid};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProvinceModifierContent {
    pub rules: ProvinceModifierRules,
    pub statics: BTreeMap<String, Result<content_reader_script::modifiers::Modifiers, String>>,
    pub notes: Vec<String>,
}

impl ProvinceModifierContent {
    pub fn read(layers: &content_reader_layers::ResolvedLayers, saved: &Save) -> io::Result<Self> {
        let rules = content_reader_weather::weather::province_modifier_rules(layers)?;
        let mut names: BTreeSet<&str> = TEMPERATURE_MODIFIERS
            .into_iter()
            .chain(GROUND_SNOW_MODIFIERS)
            .chain([MUD_MODIFIER])
            .collect();
        for province in saved.weather.provinces.iter().flatten() {
            names.extend(
                province
                    .custom_modifiers
                    .iter()
                    .flatten()
                    .map(|(name, _)| name.as_str()),
            );
        }
        let names: Vec<&str> = names.into_iter().collect();
        let blocks = content_reader_politics::static_modifiers::read_blocks(layers, &names)?;
        let statics = names
            .iter()
            .zip(blocks.blocks)
            .map(|(name, block)| {
                let block = block.map_err(|refusal| {
                    format!(
                        "{} in {}: {}",
                        refusal.modifier, refusal.file, refusal.reason
                    )
                });
                ((*name).to_owned(), block)
            })
            .collect();
        Ok(Self {
            rules,
            statics,
            notes: blocks.notes,
        })
    }
}

pub fn compare(
    frame: &DecodedFrame,
    saved: &Save,
    temperature: Result<&replay_data::temperature::Content, String>,
    province_modifiers: Result<&ProvinceModifierContent, String>,
    report: &mut Report,
) -> io::Result<()> {
    let provinces = report.attempt("WeatherProvinces", |report| {
        compare_provinces(frame, saved, temperature, province_modifiers, report)
    });
    let regions = report.attempt("WeatherRegions", |report| {
        compare_regions(frame, saved, report)
    });
    report.limitation(
        "the save holds neither the snow ceiling nor permanent snow; a load recomputes both from content",
        "WeatherRegions.max_snow, WeatherProvinces.permanent_snow: census only",
    );
    report.limitation(
        "the save does not retain held day/night direction or feather words; completed-update direction derives from date and content, while suspended/startup operands require matched native observation",
        "WeatherProvinces.daylight: runtime only",
    );
    report.limitation(
        "the save does not retain the live regional weather modifier accumulator or its known-read gate; matched native observation must verify its independent raw map terms",
        "WeatherRegions.map_modifiers_known, naval_detection, air_attack, air_defence: runtime only",
    );
    report.limitation(
        "the weather boolean writer preserves zero/nonzero truth only; nonzero native byte values are indistinguishable in text saves",
        "WeatherRegions.light_rain, heavy_rain, snow, blizzard, sandstorm, arctic_water; WeatherProvinces.raw_mud",
    );
    if provinces && regions {
        Ok(())
    } else {
        Err(invalid(
            "weather comparison incomplete; see per-table errors",
        ))
    }
}

fn compare_provinces(
    frame: &DecodedFrame,
    saved: &Save,
    temperature: Result<&replay_data::temperature::Content, String>,
    province_modifiers: Result<&ProvinceModifierContent, String>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::WeatherProvinces>(frame)?;
    let rebuild = ModifierRebuild::prepare(frame, rows, province_modifiers, report);
    let unrecorded = rows
        .iter()
        .all(|row| matches!(row.temperature.record(), Ok(TemperatureRecord::NotRecorded)));
    let temperatures = replay_data::temperature::Memo::default()
        .states(
            frame.branch,
            frame.hour,
            rows,
            temperature
                .as_ref()
                .map(|content| *content)
                .map_err(String::as_str),
        )
        .map(|states| {
            states
                .iter()
                .map(|state| state.temperature)
                .collect::<Vec<_>>()
        });
    if let (false, Err(reason)) = (unrecorded, &temperatures) {
        report.failed_comparisons.insert(
            "WeatherProvinces.temperature".into(),
            format!(
                "WeatherProvinces.temperature refused: {reason}; no province's temperature compared"
            ),
        );
    }
    let mut provinces = BTreeMap::new();
    for row in rows {
        if provinces.insert(row.id, row).is_some() {
            return Err(invalid("duplicate native weather province id"));
        }
        if matches!(row.clock.state(row.id), Ok(ClockState::NotRecorded)) {
            report.recorded(
                &replay_data::rows::Recorded::<()>::NotRecorded,
                "WeatherProvinces.clock",
                format!("weather province {}.clock", row.id),
            );
        }
        if row.id == 0
            && report
                .recorded(
                    &row.display,
                    "WeatherProvinces.display",
                    "weather display boundary",
                )
                .is_some_and(Option::is_none)
        {
            report.limitation(
                "WeatherProvinces.display unavailable",
                "1 live snow/mud boundary acquisition refused",
            );
        }
    }
    for (ordinal, &id) in provinces.keys().enumerate() {
        dense_slot(ordinal, id, "captured weather provinces")?;
    }

    let mut compared_provinces = 0;
    let mut compared_modifiers = 0;
    let mut compared_statics = 0;
    if let Some(expected) = &saved.weather.provinces {
        for (ordinal, expected) in expected.iter().enumerate() {
            dense_slot(ordinal, expected.slot, "saved weather provinces")?;
            let label = format!("weather province {}", expected.slot);
            let Some(actual) = provinces.remove(&expected.slot) else {
                report.difference(label, "absent", "present");
                continue;
            };
            compared_provinces += 1;
            if Some(actual.id) != expected.province {
                report.difference(
                    format!("{label}.province"),
                    Some(actual.id),
                    expected.province,
                );
            }
            sparse_scalar(report, &label, "snow", actual.snow, expected.snow.as_ref());
            let recorded = match &temperatures {
                _ if unrecorded => Some(replay_data::rows::Recorded::NotRecorded),
                Ok(values) => Some(
                    values
                        .get(actual.id as usize)
                        .copied()
                        .ok_or_else(|| {
                            invalid("derived temperatures do not cover every recorded province")
                        })?
                        .into(),
                ),
                Err(_) => None,
            };
            if let Some(&temperature) = recorded.as_ref().and_then(|recorded| {
                report.recorded(
                    recorded,
                    "WeatherProvinces.temperature",
                    format!("{label}.temperature"),
                )
            }) {
                report.scalar_checks += 1;
                if expected.temperature.as_ref().map(|value| value.raw) != Some(temperature) {
                    report.difference(
                        format!("{label}.temperature"),
                        Some(temperature),
                        expected.temperature.as_ref().map(|value| value.raw),
                    );
                }
            }
            if let Some(&mud) = report.recorded(
                &actual.raw_mud,
                "WeatherProvinces.raw_mud",
                format!("{label}.mud"),
            ) {
                truth(report, &format!("{label}.mud"), mud, expected.mud);
            }
            let field = format!("{label}.modifiers");
            match report.recorded(&actual.modifiers, "WeatherProvinces.modifiers", &field) {
                Some(None) => {
                    report.failed_comparisons.insert(
                        field.clone(),
                        format!("{field}: read refused, not compared"),
                    );
                }
                Some(Some(recorded)) => {
                    if let Some(rebuild) = &rebuild {
                        let entries: BTreeMap<u32, i64> = recorded
                            .entries
                            .iter()
                            .map(|entry| (entry.key, entry.value))
                            .collect();
                        let rebuilt =
                            rebuild
                                .expected(expected, actual.permanent_snow)
                                .and_then(|rebuilt| {
                                    Ok((rebuild.saved_part_holds(expected, &entries)?, rebuilt))
                                });
                        match rebuilt {
                            Ok((Fit::One, _)) => compared_modifiers += 1,
                            Ok((Fit::Several(states), _)) => {
                                report.failed_comparisons.insert(
                                    field.clone(),
                                    format!("{field} save-backed part ambiguous: {states} distinct mud, thermal and ground snow states the content can select fit"),
                                );
                            }
                            Ok((Fit::None, rebuilt)) => {
                                compared_modifiers += 1;
                                rebuild.compare(report, &field, recorded, &rebuilt);
                            }
                            Err(reason) => {
                                report.failed_comparisons.insert(
                                    field.clone(),
                                    format!("{field} not rebuilt: {reason}"),
                                );
                            }
                        }
                    }
                }
                None => {}
            }
            let field = format!("{label}.static_modifiers");
            match report.recorded(
                &actual.static_modifiers,
                "WeatherProvinces.static_modifiers",
                &field,
            ) {
                Some(None) => {
                    report.failed_comparisons.insert(
                        field.clone(),
                        format!("{field}: read refused, not compared"),
                    );
                }
                Some(Some(recorded)) => {
                    compared_statics += 1;
                    report.scalar_checks += 1;
                    if !statics_match(recorded, expected.custom_modifiers.as_deref()) {
                        report.difference(
                            field,
                            static_list(recorded),
                            expected.custom_modifiers.as_deref().unwrap_or_default(),
                        );
                    }
                }
                None => {}
            }
        }
        for id in provinces.keys() {
            report.difference(format!("weather province {id}"), "present", "absent");
        }
    } else {
        report.difference(
            "weather.provinces",
            format!("{} decoded native rows", provinces.len()),
            "missing serialized dense roster",
        );
    }
    report.compared("WeatherProvinces", compared_provinces);
    report.compared("WeatherProvinces modifiers", compared_modifiers);
    report.compared("WeatherProvinces static modifiers", compared_statics);
    if compared_modifiers != 0 {
        report.limitation(
            "the weather manager writer saves no province modifier accumulator; a load rebuilds it from the saved mud, temperature, snow and list, while the live one keeps the mud, thermal and ground snow modifiers of its last rebuild (the hourly window rebuilds only on a change against the record's previous hour, and temperatures and mud also change outside that test). The list's static modifiers and their arithmetic are compared exactly",
            "WeatherProvinces.modifiers mud, thermal and ground snow part: runtime-only; no text-save equality claim",
        );
    }
    report.limitation(
        "the weather manager writer omits the display byte arrays; a load rebuilds them from the passes",
        "WeatherProvinces.display_snow, display_mud: census only",
    );
    report.limitation(
        "the weather manager writer omits the stored day/night array; held clocks require a matched native read witness, and updated clocks are the hour's derivation from content (`recorder derived` compares it with recorded clocks)",
        "WeatherProvinces.clock: census only",
    );
    report.limitation(
        "the manager serializer omits the graphics grid, refresh interval and refresh identity; retained boundary and local fade require matched native graphics witnesses",
        "WeatherProvinces.display: runtime-only; no text-save equality claim",
    );
    Ok(())
}

fn statics_match(recorded: &ProvinceStaticModifiers, saved: Option<&[(String, i64)]>) -> bool {
    let saved = saved.unwrap_or_default();
    recorded.entries.len() == saved.len()
        && recorded
            .entries
            .iter()
            .zip(saved)
            .all(|(entry, (name, days))| {
                entry.name == name.as_bytes() && i64::from(entry.days) == *days
            })
}

fn static_list(recorded: &ProvinceStaticModifiers) -> Vec<(String, i64)> {
    recorded
        .entries
        .iter()
        .map(|entry| {
            (
                String::from_utf8_lossy(&entry.name).into_owned(),
                i64::from(entry.days),
            )
        })
        .collect()
}

type Entries = Vec<(u32, i64)>;

const ONE: i64 = 100_000;

fn scaled(value: i64) -> i64 {
    value.wrapping_mul(ONE) / ONE
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fit {
    None,
    One,
    Several(usize),
}

struct ModifierRebuild<'a> {
    rules: &'a ProvinceModifierRules,
    statics: BTreeMap<&'a str, Result<Entries, String>>,
    names: BTreeMap<u32, &'a str>,
}

impl<'a> ModifierRebuild<'a> {
    fn prepare(
        frame: &DecodedFrame,
        rows: &[replay_data::rows::WeatherProvinceRow],
        content: Result<&'a ProvinceModifierContent, String>,
        report: &mut Report,
    ) -> Option<Self> {
        if !rows
            .iter()
            .any(|row| matches!(row.modifiers, Recorded::Value(Some(_))))
        {
            return None;
        }
        let recorded = rows
            .iter()
            .filter(|row| matches!(row.modifiers, Recorded::Value(Some(_))))
            .count();
        let refused = |report: &mut Report, reason: &str| {
            report.failed_comparisons.insert(
                "WeatherProvinces.modifiers".into(),
                format!("WeatherProvinces.modifiers refused: {reason}; {recorded} recorded accumulators not compared"),
            );
        };
        let content = match content {
            Ok(content) => content,
            Err(reason) => {
                refused(report, &reason);
                return None;
            }
        };
        let registry = match Registry::from_frame(frame) {
            Ok(registry) => registry,
            Err(reason) => {
                refused(report, &format!("modifier definitions: {reason}"));
                return None;
            }
        };
        let mut names = BTreeMap::new();
        let statics: BTreeMap<&str, Result<Entries, String>> = content
            .statics
            .iter()
            .map(|(name, block)| {
                let values = block
                    .as_ref()
                    .map_err(Clone::clone)
                    .and_then(|block| Self::added(&registry, name, block, &mut names));
                (name.as_str(), values)
            })
            .collect();
        for note in &content.notes {
            report.limitation(
                "static modifier reading notes; values follow the game's reading",
                format!("WeatherProvinces.modifiers: {note}"),
            );
        }
        report.limitation(
            "the save does not hold permanent snow; a load recomputes it from content, so the ground snow rebuild uses the recorded value",
            "WeatherProvinces.modifiers: ground snow from saved snow and recorded permanent_snow",
        );
        Some(Self {
            rules: &content.rules,
            statics,
            names,
        })
    }

    fn added(
        registry: &Registry<'_>,
        name: &str,
        block: &'a content_reader_script::modifiers::Modifiers,
        names: &mut BTreeMap<u32, &'a str>,
    ) -> Result<Vec<(u32, i64)>, String> {
        let mut entries = Vec::new();
        let mut eligible = false;
        for value in &block.values {
            let Ok(definition) = registry.named(&value.name) else {
                continue;
            };
            eligible |= registry
                .eligible(definition)
                .map_err(|reason| format!("static modifier {name}: {reason}"))?;
            names.insert(definition.index, value.name.as_str());
            if definition.category_mask != 0 {
                entries.push((definition.index, value.value));
            }
        }
        let empty = !eligible && block.tooltip_table().is_empty() && block.children.is_empty();
        Ok(if empty { Vec::new() } else { entries })
    }

    fn with(
        &self,
        saved: &content_reader_save::weather::Province,
        mud: bool,
        thermal: Option<&str>,
        snow: Option<&str>,
    ) -> Result<BTreeMap<u32, i64>, String> {
        let applied = mud
            .then_some(MUD_MODIFIER)
            .into_iter()
            .chain(thermal)
            .chain(
                saved
                    .custom_modifiers
                    .iter()
                    .flatten()
                    .map(|(name, _)| name.as_str()),
            )
            .chain(snow);
        let mut accumulator = BTreeMap::new();
        for name in applied {
            let values = self
                .statics
                .get(name)
                .ok_or_else(|| format!("static modifier {name} was not read"))?
                .as_ref()
                .map_err(Clone::clone)?;
            for &(key, value) in values {
                let entry = accumulator.entry(key).or_insert(0_i64);
                *entry = entry.wrapping_add(scaled(value));
            }
        }
        Ok(accumulator)
    }

    fn expected(
        &self,
        saved: &content_reader_save::weather::Province,
        permanent_snow: i64,
    ) -> Result<BTreeMap<u32, i64>, String> {
        let temperature = saved
            .temperature
            .as_ref()
            .ok_or("the saved province has no temperature")?
            .raw;
        let snow = saved.snow.as_ref().map_or(0, |snow| snow.raw);
        self.with(
            saved,
            saved.mud.unwrap_or(false),
            self.rules.temperature_modifier(temperature),
            self.rules.ground_snow_modifier(snow, permanent_snow),
        )
    }

    fn saved_part_holds(
        &self,
        saved: &content_reader_save::weather::Province,
        recorded: &BTreeMap<u32, i64>,
    ) -> Result<Fit, String> {
        let thermals = self.rules.reachable_temperature_modifiers();
        let snows = self.rules.reachable_ground_snow_modifiers();
        let empty = content_reader_save::weather::Province {
            custom_modifiers: None,
            ..saved.clone()
        };
        let mut fitting: Vec<BTreeMap<u32, i64>> = Vec::new();
        for mud in [false, true] {
            for &thermal in &thermals {
                for &snow in &snows {
                    if self.with(saved, mud, thermal, snow)? == *recorded {
                        let state = self.with(&empty, mud, thermal, snow)?;
                        if !fitting.contains(&state) {
                            fitting.push(state);
                        }
                    }
                }
            }
        }
        Ok(match fitting.len() {
            0 => Fit::None,
            1 => Fit::One,
            states => Fit::Several(states),
        })
    }

    fn compare(
        &self,
        report: &mut Report,
        field: &str,
        recorded: &ProvinceWeatherModifiers,
        rebuilt: &BTreeMap<u32, i64>,
    ) {
        let recorded: BTreeMap<u32, i64> = recorded
            .entries
            .iter()
            .map(|entry| (entry.key, entry.value))
            .collect();
        let keys: BTreeSet<u32> = recorded.keys().chain(rebuilt.keys()).copied().collect();
        for key in keys {
            let label = match self.names.get(&key) {
                Some(name) => format!("{field}.{name} (definition {key})"),
                None => format!("{field}.definition {key}"),
            };
            match (recorded.get(&key), rebuilt.get(&key)) {
                (Some(captured), Some(expected)) if captured != expected => {
                    report.difference(label, captured, expected);
                }
                (Some(captured), None) => report.difference(label, captured, "absent"),
                (None, Some(expected)) => report.difference(label, "absent", expected),
                _ => {}
            }
        }
    }
}

fn compare_regions(frame: &DecodedFrame, saved: &Save, report: &mut Report) -> io::Result<()> {
    let mut regions = BTreeMap::new();
    for row in complete::<tables::WeatherRegions>(frame)? {
        if regions.insert(row.id, row).is_some() {
            return Err(invalid("duplicate native weather region id"));
        }
    }
    for (ordinal, &id) in regions.keys().enumerate() {
        dense_slot(ordinal, id, "captured weather regions")?;
    }
    let mut compared_regions = 0;
    if let Some(expected) = &saved.weather.regions {
        for (ordinal, expected) in expected.iter().enumerate() {
            dense_slot(ordinal, expected.slot, "saved weather regions")?;
            let label = format!("weather region {}", expected.slot);
            let Some(actual) = regions.remove(&expected.slot) else {
                report.difference(label, "absent", "present");
                continue;
            };
            compared_regions += 1;
            if Some(actual.id) != expected.region {
                report.difference(format!("{label}.region"), Some(actual.id), expected.region);
            }
            for (name, captured, saved) in [
                ("rain_light", actual.light_rain, expected.rain_light),
                ("rain_heavy", actual.heavy_rain, expected.rain_heavy),
                ("snow", actual.snow, expected.snow),
                ("blizzard", actual.blizzard, expected.blizzard),
                ("sandstorm", actual.sandstorm, expected.sandstorm),
                ("arctic_water", actual.arctic_water, expected.arctic_water),
            ] {
                truth(report, &format!("{label}.{name}"), captured, saved);
            }
            custom_sea(
                report,
                &label,
                &actual.custom_sea,
                expected.active_modifiers.as_deref(),
            );
        }
        for id in regions.keys() {
            report.difference(format!("weather region {id}"), "present", "absent");
        }
    } else {
        report.difference(
            "weather.regions",
            format!("{} decoded native rows", regions.len()),
            "missing serialized dense roster",
        );
    }
    report.compared("WeatherRegions", compared_regions);
    Ok(())
}

fn dense_slot(ordinal: usize, slot: u32, label: &str) -> io::Result<()> {
    if u32::try_from(ordinal).ok() != Some(slot) {
        return Err(invalid(format!(
            "{label} must retain every dense slot in order from zero: position {ordinal}, slot {slot}"
        )));
    }
    Ok(())
}

fn sparse_scalar(
    report: &mut Report,
    label: &str,
    field: &str,
    captured: i64,
    saved: Option<&Decimal>,
) {
    let expected = saved.map_or(0, |value| value.raw);
    report.scalar_checks += 1;
    if captured != expected {
        report.difference(format!("{label}.{field}"), captured, expected);
    }
}

fn custom_sea(
    report: &mut Report,
    label: &str,
    captured: &replay_data::rows::Recorded<Option<Vec<i16>>>,
    saved: Option<&[i64]>,
) {
    let expected = saved.unwrap_or_default();
    let label = format!("{label}.active_modifiers");
    match captured {
        replay_data::rows::Recorded::Value(Some(values)) => {
            if !values.iter().map(|&value| i64::from(value)).eq(expected.iter().copied()) {
                report.difference(label, values, expected);
            }
        }
        replay_data::rows::Recorded::Value(None) => report.difference(label, "refused", expected),
        replay_data::rows::Recorded::NotRecorded => {
            report.limitation("WeatherRegions.custom_sea not recorded", label)
        }
    }
}

fn truth(report: &mut Report, label: &str, captured: u8, saved: Option<bool>) {
    let expected = saved.unwrap_or(false);
    if (captured != 0) != expected {
        report.difference(label, captured, expected);
    }
}

#[cfg(test)]
mod tests;
