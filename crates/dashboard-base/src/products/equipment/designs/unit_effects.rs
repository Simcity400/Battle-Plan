//! Selected-unit XP and ace contributors. No unit/design result is recorded.
use super::*;
use installed_content_core::{
    modifiers,
    parser::native::{self, Shape},
};
use std::{collections::BTreeMap, io};

const SCALE: i64 = 100_000;
const INVALID: i64 = u32::MAX as i64;
const CARRIER_CATEGORIES: u64 = (1 << 22) | (1 << 37);
const NO_WING_EXPERIENCE: u64 = 0x1c00010000;
const AIR_EXPERIENCE_TRAINING: u32 = 0x73;

#[derive(Default)]
pub struct AceContent {
    definitions: BTreeMap<Vec<u8>, modifiers::Modifiers>,
    refusal: Option<String>,
}
#[derive(Clone, Copy)]
enum Node {
    File,
    Entries,
    Definition,
    Modifier(modifiers::Node),
}
impl AceContent {
    pub fn load(layers: &installed_content_core::ResolvedLayers) -> io::Result<Self> {
        let names = installed_content_core::key_names::KeyNames::read(layers)?;
        let mut result = Self::default();
        for path in layers.directory_files("common/aces", ".txt")? {
            result.read(&path.read(layers)?, &names);
        }
        Ok(result)
    }
    fn read(&mut self, bytes: &[u8], names: &installed_content_core::key_names::KeyNames) {
        let mut undecided = Vec::new();
        let parsed = native::read(
            bytes,
            native::Mode::KeepMark,
            native::Top::File(Node::File),
            Some(names),
            &mut |node, field| match node {
                Node::File if field.keyword_is("modifiers") => Shape::Object(Node::Entries),
                Node::File => Shape::Skip,
                Node::Entries => Shape::Object(Node::Definition),
                Node::Definition if field.keyword_is("effect") => {
                    Shape::Object(Node::Modifier(modifiers::Node::Modifier))
                }
                Node::Definition if field.keyword_is("type") => Shape::ListIfOpen,
                Node::Definition if field.keyword_is("chance") => Shape::One,
                Node::Definition => Shape::Skip,
                Node::Modifier(node) => modifiers::shape(
                    node,
                    field,
                    &|key| modifiers::registered(key, Some(names)),
                    &mut undecided,
                )
                .map(Node::Modifier),
            },
        );
        if let Some(reason) = parsed
            .unsettled
            .as_ref()
            .map(ToString::to_string)
            .or_else(|| undecided.first().map(ToString::to_string))
        {
            self.refusal = Some(format!("ace effect content: {reason} (count: 1)"));
            // Unsettled consumption may replace any earlier definition.
            // A later fully read file can establish an exact name again.
            self.definitions.clear();
            return;
        }
        for group in parsed
            .items
            .iter()
            .filter(|item| item.keyword_is("modifiers"))
        {
            for definition in group.value.items().unwrap_or_default() {
                let Some(name) = definition.key.as_deref() else {
                    continue;
                };
                let mut effects = modifiers::Modifiers::default();
                for effect in definition
                    .value
                    .items()
                    .unwrap_or_default()
                    .iter()
                    .filter(|item| item.keyword_is("effect"))
                {
                    effects.read_all(effect.value.items().unwrap_or_default());
                }
                // Repeated definitions reset their stored modifier object.
                self.definitions
                    .insert(installed_content_core::parser::c_string(name).to_vec(), effects);
            }
        }
    }
    fn definition(&self, name: &[u8]) -> Result<&modifiers::Modifiers, String> {
        if let Some(definition) = self.definitions.get(installed_content_core::parser::c_string(name)) {
            return Ok(definition);
        }
        if let Some(reason) = &self.refusal {
            return Err(format!("ace {}: {reason}", String::from_utf8_lossy(name)));
        }
        self.definitions
            .get(installed_content_core::parser::c_string(name))
            .ok_or_else(|| {
                format!(
                    "ace definition {} is absent (count: 1)",
                    String::from_utf8_lossy(name)
                )
            })
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Effects {
    pub level: Option<usize>,
    pub sources: Vec<(String, i64)>,
    pub values: Vec<(String, i64)>,
    pub refusals: Vec<String>,
}
fn finite(value: i64) -> bool {
    (value as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd
}
fn divide(value: i64, denominator: i64) -> i64 {
    if denominator == 0 || !finite(value) {
        INVALID
    } else {
        value.wrapping_mul(SCALE).wrapping_div(denominator)
    }
}
fn level(ratio: i64, thresholds: &[i64]) -> Result<usize, String> {
    if thresholds.is_empty() {
        return Err("experience threshold array is empty (count: 1)".into());
    }
    Ok(thresholds
        .iter()
        .take_while(|threshold| ratio >= **threshold)
        .count())
}
fn ship_scale(level: usize, base: i32, count: usize) -> i64 {
    let delta = (level as i64)
        .wrapping_sub(i64::from(base))
        .wrapping_mul(SCALE);
    if delta < 0 {
        divide(delta.wrapping_neg(), i64::from(base).wrapping_mul(SCALE))
    } else if delta == 0 {
        0
    } else {
        divide(
            delta,
            (count as i64)
                .wrapping_sub(i64::from(base))
                .wrapping_mul(SCALE),
        )
    }
}
fn wing_scale(xp: i64, thresholds: &[i64]) -> Result<i64, String> {
    let (&first, rest) = thresholds
        .split_first()
        .ok_or("wing experience threshold array is empty (count: 1)")?;
    let last = *rest.last().unwrap_or(&first);
    let value = if xp < first {
        let raw = divide(xp, first).wrapping_sub(SCALE);
        if raw < -SCALE { raw } else { raw.min(0) }
    } else {
        divide(xp.wrapping_sub(first), last.wrapping_sub(first)).clamp(0, SCALE)
    };
    // The native cached contributor truncates to tenths, towards zero.
    Ok(i64::from((value.wrapping_mul(1_000_000) / 10_000_000_000) as i32).wrapping_mul(10_000))
}
fn metadata<'a>(
    frame: &'a DecodedFrame,
    name: &str,
) -> Result<Option<&'a rows::ModifierDefinitionRow>, String> {
    let definitions = frame
        .complete::<tables::ModifierDefinitions>()
        .map_err(|e| e.to_string())?;
    for row in definitions {
        if queries::spelling(frame, row.token)?.eq_ignore_ascii_case(name.as_bytes()) {
            return Ok(Some(row));
        }
    }
    Ok(None)
}
fn effective(frame: &DecodedFrame, name: &str, value: i64) -> Result<i64, String> {
    let Some(definition) = metadata(frame, name)? else {
        return Ok(0);
    };
    if definition.feature_selector != 0 {
        if definition.eligibility == 0 {
            return Ok(0);
        }
        let [features] = frame
            .complete::<tables::SimulationFeatures>()
            .map_err(|e| e.to_string())?
        else {
            return Err(format!(
                "unit modifier {name} feature mask is not unique (count: 1)"
            ));
        };
        if features.dlc_mask & definition.feature_selector == 0 {
            return Ok(0);
        }
    }
    Ok(
        if definition.flags & engine_native::modifier_definition::CLAMP_MAXIMUM != 0 {
            value.clamp(0, SCALE)
        } else {
            value
        },
    )
}
fn scaled_source(
    frame: &DecodedFrame,
    source: &str,
    block: &modifiers::Modifiers,
    scale: i64,
    values: &mut BTreeMap<String, i64>,
    result: &mut Effects,
) -> Result<(), String> {
    if let Some(reason) = block.unheld.cause() {
        return Err(format!("{source}: {reason} (count: 1)"));
    }
    result.sources.push((source.to_owned(), scale));
    for entry in &block.values {
        let Some(definition) = metadata(frame, &entry.name)? else {
            continue;
        };
        if definition.category_mask == 0 {
            continue;
        }
        let value = entry.value.wrapping_mul(scale) / SCALE;
        result
            .sources
            .push((format!("{source} / {}", entry.name), value));
        let old = values.entry(entry.name.clone()).or_default();
        *old = old.wrapping_add(value);
    }
    Ok(())
}
fn static_source(
    frame: &DecodedFrame,
    layers: &installed_content_core::ResolvedLayers,
    name: &str,
    scale: i64,
    values: &mut BTreeMap<String, i64>,
    result: &mut Effects,
) -> Result<(), String> {
    if scale == 0 {
        return Ok(());
    }
    let (_, block) = layers
        .derived::<installed_content_core::static_modifiers::Descriptions>()
        .get(layers, name)?;
    scaled_source(frame, name, &block, scale, values, result)
}
fn finish(frame: &DecodedFrame, values: BTreeMap<String, i64>, result: &mut Effects) {
    for (name, value) in values {
        match effective(frame, &name, value) {
            Ok(value) => result.values.push((name, value)),
            Err(reason) => result.refusals.push(reason),
        }
    }
}
fn carrier(model: &rows::ShipModelRow) -> Result<bool, String> {
    if model.formation_flags & CARRIER_CATEGORIES != 0 {
        return Ok(true);
    }
    model
        .counter
        .as_ref()
        .map(|counter| counter.positive_equipment_flags & CARRIER_CATEGORIES != 0)
        .ok_or_else(|| {
            "ShipModels.counter.positive_equipment_flags is unavailable (count: 1)".into()
        })
}

pub fn ship(
    frame: &DecodedFrame,
    variant: u64,
    ship: u64,
    layers: &installed_content_core::ResolvedLayers,
    defines: &installed_content_core::defines::Defines,
) -> Result<Effects, String> {
    let row = queries::ship_context(frame, variant, ship)?;
    let thresholds = defines
        .fixed5_array("NMilitary", "UNIT_EXP_LEVELS")
        .map_err(|e| e.to_string())?;
    let maximum = defines
        .fixed5("NNavy", "NAVY_MAX_XP")
        .map_err(|e| e.to_string())?;
    let ratio = divide(row.experience.max(0), maximum);
    let current = level(ratio, &thresholds)?;
    if row.experience >= 0
        && level(
            divide(row.experience.saturating_add(999), maximum),
            &thresholds,
        )? != current
    {
        return Err("Ships.experience display step crosses an authored level threshold (count: 1); finer per-ship capture is not demanded".into());
    }
    let scale = ship_scale(
        current,
        defines
            .int32("NMilitary", "ARMY_EXP_BASE_LEVEL")
            .map_err(|e| e.to_string())?,
        thresholds.len(),
    );
    let mut result = Effects {
        level: Some(current),
        ..Default::default()
    };
    let mut values = BTreeMap::new();
    let model = frame
        .complete::<tables::ShipModels>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.ship == ship)
        .unwrap();
    if scale != 0 {
        let suffix = if (current as i64)
            < i64::from(
                defines
                    .int32("NMilitary", "ARMY_EXP_BASE_LEVEL")
                    .map_err(|e| e.to_string())?,
            ) {
            "malus_min"
        } else {
            "bonus_max"
        };
        static_source(
            frame,
            layers,
            &format!("ship_experience_{suffix}"),
            scale,
            &mut values,
            &mut result,
        )?;
        match carrier(model) {
            Ok(true) => static_source(
                frame,
                layers,
                &format!("carrier_experience_{suffix}"),
                scale,
                &mut values,
                &mut result,
            )?,
            Ok(false) => {}
            Err(reason) => result.refusals.push(reason),
        }
    }
    finish(frame, values, &mut result);
    Ok(result)
}

fn country_slot(frame: &DecodedFrame, raw: u32) -> Result<u32, String> {
    frame
        .complete::<tables::CountryLinks>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.id == raw)
        .map(|row| row.canonical_slot)
        .ok_or_else(|| format!("unit country {raw} canonical slot is absent (count: 1)"))
}
fn ace_country(frame: &DecodedFrame, raw: u32, effectiveness: bool) -> Result<i64, String> {
    let slot = country_slot(frame, raw)?;
    let row = frame
        .complete::<tables::CountryCommandPower>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.canonical_slot == slot)
        .ok_or_else(|| {
            format!("ace country slot {slot} accumulator values are absent (count: 1)")
        })?;
    (if effectiveness {
        row.ace_effectiveness_factor
    } else {
        row.air_ace_bonuses_factor
    })
    .ok_or_else(|| {
        format!(
            "ace country slot {slot} {} registration is absent (count: 1)",
            if effectiveness {
                "ace_effectiveness_factor"
            } else {
                "air_ace_bonuses_factor"
            }
        )
    })
}
fn ace_scale(planes: Option<i32>, size: i64, maximum: i64) -> i64 {
    let Some(planes) = planes.filter(|planes| *planes != 0) else {
        return SCALE;
    };
    divide(size, i64::from(planes).wrapping_mul(SCALE)).min(maximum)
}

pub fn wing(
    frame: &DecodedFrame,
    variant: u64,
    wing: u64,
    layers: &installed_content_core::ResolvedLayers,
    defines: &installed_content_core::defines::Defines,
) -> Result<Effects, String> {
    let context = queries::wing_context(frame, variant, wing)?;
    let mut result = Effects::default();
    let mut values = BTreeMap::new();
    let pool = context
        .wing
        .pool_subunit
        .require("AirWings.pool_subunit")
        .map_err(|e| e.to_string())?
        .ok_or("wing equipment-pool type is unavailable (count: 1)")?;
    let subunits = layers
        .derived::<crate::catalogue::CachedResult<installed_content_core::unit_leader::SubUnits>>()
        .get_or_init(|| installed_content_core::unit_leader::sub_units(layers))
        .map_err(|e| e.to_string())?;
    let categories = if pool == 0 {
        0
    } else {
        let name = subunits
            .name(pool)
            .ok_or_else(|| format!("wing pool sub-unit index {pool} is absent (count: 1)"))??;
        subunits.category(name.as_bytes())?
    };
    if categories & NO_WING_EXPERIENCE == 0 {
        let thresholds = defines
            .fixed5_array("NAir", "AIR_WING_XP_LEVELS")
            .map_err(|e| e.to_string())?;
        result.level = Some(level(context.experience, &thresholds)?);
        let mut scale = wing_scale(context.experience, &thresholds)?;
        if scale < 0 && context.wing.country_override > 0 {
            match country_modifier(
                frame,
                context.wing.country_override as u32,
                AIR_EXPERIENCE_TRAINING,
            ) {
                Ok(factor) => {
                    scale = scale
                        .wrapping_neg()
                        .wrapping_mul(factor.max(-SCALE).wrapping_add(SCALE))
                        / SCALE;
                }
                Err(reason) => {
                    result.refusals.push(format!("wing XP training factor: {reason} (count: 1); owner scope outside existing air-design demands is not expanded into a wing census"));
                    scale = 0;
                }
            }
        } else if scale < 0 {
            scale = scale.wrapping_neg();
        }
        let below = context.experience < thresholds[0];
        static_source(
            frame,
            layers,
            if below {
                "air_wing_experience_malus_min"
            } else {
                "air_wing_experience_bonus_max"
            },
            scale,
            &mut values,
            &mut result,
        )?;
    }
    finish(frame, values, &mut result);
    if let Some(ace) = context.ace.filter(|ace| ace.alive != 0) {
        let applied = (|| {
            let presentation = ace
                .presentation
                .require("AirAces.presentation")
                .map_err(|e| e.to_string())?
                .as_ref()
                .ok_or("ace definition read is unavailable (count: 1)")?;
            let Some(name) = &presentation.definition else {
                return Ok::<_, String>(());
            };
            let content = layers
                .derived::<crate::catalogue::CachedResult<AceContent>>()
                .get_or_init(|| AceContent::load(layers))
                .map_err(|e| e.to_string())?;
            let block = content.definition(name)?;
            if let Some(reason) = block.unheld.cause() {
                return Err(format!("ace effect: {reason} (count: 1)"));
            }
            let assigned = if ace.assigned_wing == 0 {
                None
            } else {
                frame
                    .complete::<tables::AirWings>()
                    .map_err(|e| e.to_string())?
                    .iter()
                    .find(|row| row.id == ace.assigned_wing)
                    .map(|row| row.planes)
            };
            let size = ace_scale(
                assigned,
                defines
                    .fixed5("NAir", "ACE_WING_SIZE")
                    .map_err(|e| e.to_string())?,
                defines
                    .fixed5("NAir", "ACE_WING_SIZE_MAX_BONUS")
                    .map_err(|e| e.to_string())?,
            );
            let mut effectiveness = None;
            let mut bonus = None;
            result.sources.push(("Ace wing-size scale".into(), size));
            for entry in &block.values {
                if metadata(frame, &entry.name)?.is_none() {
                    continue;
                }
                let own = effective(frame, &entry.name, entry.value)?;
                result
                    .sources
                    .push((format!("Ace definition / {}", entry.name), entry.value));
                result
                    .sources
                    .push((format!("Ace own effective / {}", entry.name), own));
                let factor = match effectiveness {
                    Some(value) => value,
                    None => {
                        let value = ace_country(frame, ace.country, true)?.wrapping_add(SCALE);
                        result
                            .sources
                            .push(("Ace owner effectiveness multiplier".into(), value));
                        *effectiveness.insert(value)
                    }
                };
                let scaled = (own.wrapping_mul(size) / SCALE).wrapping_mul(factor) / SCALE;
                result
                    .sources
                    .push((format!("Ace / {}", entry.name), scaled));
                // Getter-specific extra country terms are separate stages;
                // the ace's own scaled contribution remains available alone.
                let extra = match entry.name.as_str() {
                    "air_maximum_speed_factor" | "air_range_factor" => own != 0,
                    _ => false,
                };
                if extra {
                    let raw = if context.wing.country_override > 0 {
                        context.wing.country_override as u32
                    } else {
                        context.wing.owner
                    };
                    let value = match bonus {
                        Some(value) => value,
                        None => *bonus.insert(ace_country(frame, raw, false)?),
                    };
                    result
                        .sources
                        .push((format!("Ace country stage / {}", entry.name), value));
                }
            }
            Ok(())
        })();
        if let Err(reason) = applied {
            result.refusals.push(reason);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn carrier_experience_joins_the_full_positive_pool_flags_and_keeps_lazy_formation_branch() {
        let mut model = rows::ShipModelRow::default();
        assert!(
            carrier(&model)
                .unwrap_err()
                .contains("positive_equipment_flags")
        );
        model.formation_flags = 1 << 37;
        assert_eq!(carrier(&model), Ok(true));
        model.formation_flags = 0;
        model.counter = Some(rows::ShipCounterInputs {
            positive_equipment_flags: 1 << 22,
            ..Default::default()
        });
        assert_eq!(carrier(&model), Ok(true));
        model.counter.as_mut().unwrap().positive_equipment_flags = 0;
        assert_eq!(carrier(&model), Ok(false));
    }
    #[test]
    fn unit_effects_keep_source_truncation_own_entries_and_recorded_getter_eligibility() {
        use replay_data::rows::{TableId, TableRows};
        use replay_frame::frame::{Metrics, TableView};
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 7,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "synthetic missing operand".into(),
                }
                .into()
            }),
        };
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![rows::LexicalTokenRow {
                token: 5000,
                literal: b"synthetic_factor".to_vec(),
            }]));
        *frame.tables[TableId::ModifierDefinitions.index()] =
            TableView::Complete(TableRows::ModifierDefinitions(vec![
                rows::ModifierDefinitionRow {
                    index: 7,
                    token: 5000,
                    category_mask: u32::MAX,
                    flags: engine_native::modifier_definition::CLAMP_MAXIMUM,
                    ..Default::default()
                },
            ]));
        let block = modifiers::values(
            &installed_content_core::parser::parse(
                b"synthetic_factor=4.00001 added_modifier={synthetic_factor=100}",
            )
            .unwrap()
            .items,
        );
        let mut values = BTreeMap::new();
        let mut result = Effects::default();
        scaled_source(&frame, "test XP", &block, 33_333, &mut values, &mut result).unwrap();
        assert_eq!(values["synthetic_factor"], 133_332);
        finish(&frame, values, &mut result);
        assert_eq!(result.values, [("synthetic_factor".into(), 100_000)]);
        let TableView::Complete(TableRows::ModifierDefinitions(definitions)) =
            &mut *frame.tables[TableId::ModifierDefinitions.index()]
        else {
            unreachable!()
        };
        definitions[0].feature_selector = 2;
        definitions[0].eligibility = 0;
        assert_eq!(effective(&frame, "synthetic_factor", 400_001), Ok(0));
        let TableView::Complete(TableRows::ModifierDefinitions(definitions)) =
            &mut *frame.tables[TableId::ModifierDefinitions.index()]
        else {
            unreachable!()
        };
        definitions[0].eligibility = 1;
        assert!(
            effective(&frame, "synthetic_factor", 400_001)
                .unwrap_err()
                .contains("synthetic missing operand")
        );
    }
    #[test]
    fn experience_uses_threshold_order_discrete_ship_scales_and_wing_tenths() {
        assert_eq!(level(30_000, &[10_000, 30_000, 75_000, 90_000]), Ok(2));
        assert_eq!(level(29_999, &[10_000, 30_000, 75_000, 90_000]), Ok(1));
        assert_eq!(ship_scale(0, 1, 4), 100_000);
        assert_eq!(ship_scale(1, 1, 4), 0);
        assert_eq!(ship_scale(2, 1, 4), 33_333);
        assert_eq!(ship_scale(4, 1, 4), 100_000);
        let thresholds = [10_000_000, 30_000_000, 70_000_000, 90_000_000];
        for (xp, want) in [
            (0, -100_000),
            (500_000, -90_000),
            (9_999_999, 0),
            (10_000_000, 0),
            (18_000_000, 10_000),
            (90_000_000, 100_000),
            (100_000_000, 100_000),
        ] {
            assert_eq!(wing_scale(xp, &thresholds), Ok(want));
        }
        assert!(level(0, &[]).is_err());
        assert!(wing_scale(0, &[]).is_err());
    }
    #[test]
    fn ace_size_keeps_null_zero_negative_finite_and_cap_branches() {
        assert_eq!(ace_scale(None, 10_000_000, 500_000), 100_000);
        assert_eq!(ace_scale(Some(0), 10_000_000, 500_000), 100_000);
        assert_eq!(ace_scale(Some(20), 10_000_000, 500_000), 500_000);
        assert_eq!(ace_scale(Some(200), 10_000_000, 500_000), 50_000);
        assert_eq!(ace_scale(Some(-20), 10_000_000, 500_000), -500_000);
        assert_eq!(ace_scale(Some(20), i64::MAX, 500_000), 500_000);
    }
    #[test]
    fn ace_content_resolves_variables_replaces_definitions_and_keeps_hidden_own_entries() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let mut content = AceContent::default();
        content.read(b"@v=0.125 modifiers={synthetic={effect={air_attack_factor=@v hidden_modifier={air_range_factor=0.2} added_modifier={air_agility_factor=0.9}}}}",&Default::default());
        let first = content.definition(b"synthetic").unwrap();
        assert_eq!(
            first.values.iter().map(|v| v.value).collect::<Vec<_>>(),
            [12_500, 20_000]
        );
        assert_eq!(first.children.len(), 1);
        assert!(content.definition(b"SYNTHETIC").is_err());
        content.read(
            b"modifiers={synthetic={effect={air_attack_factor=0.25}}}",
            &Default::default(),
        );
        assert_eq!(content.definition(b"synthetic").unwrap().values.len(), 1);
        assert_eq!(
            content.definition(b"synthetic").unwrap().values[0].value,
            25_000
        );
        let mut names = installed_content_core::key_names::KeyNames::default();
        names.effects.modifiers_gap = Some("synthetic incomplete registration".into());
        content.read(
            b"modifiers={other={effect={synthetic_unknown={x=y}}}}",
            &names,
        );
        assert!(
            content
                .definition(b"synthetic")
                .unwrap_err()
                .contains("synthetic")
        );
        content.read(
            b"modifiers={synthetic={effect={air_attack_factor=0.5}}}",
            &Default::default(),
        );
        assert_eq!(
            content.definition(b"synthetic").unwrap().values[0].value,
            50_000
        );
        assert!(content.definition(b"other").is_err());
    }
}
