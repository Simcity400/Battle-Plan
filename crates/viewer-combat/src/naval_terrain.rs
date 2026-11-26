use content_reader_layers::ResolvedLayers;
use content_reader_defines::defines::Defines;
use content_reader_key_names::terrain::{TerrainAdjuster, TerrainDefinitions};
use content_reader_localisation::{localisation::Argument, localisation::Localisation};
use std::{io, sync::Arc};
use replay_derive::naval_math::dominance_value;
use viewer_foundation::tooltip_text::country_localised;
use viewer_foundation::tooltip_text::displayable;
use viewer_foundation::tooltip_text::tooltip_keys;
use {
    super::modifier_text::{Block, Definitions},
    viewer_foundation::tooltip_text::{TextIcons, builtin, token},
};

const ONE: i64 = 100_000;
tooltip_keys! {
    MINIMUM_SEAZONE_DOMINANCE_NEEDED = "MINIMUM_SEAZONE_DOMINANCE_NEEDED",
    ALL_SHIPS = "ALL_SHIPS",
    OTHER_SHIPS = "OTHER_SHIPS",
    STAT_ADJUSTER_MOVEMENT = "STAT_ADJUSTER_MOVEMENT",
    STAT_ADJUSTER_MOVEMENT_DIFF = "STAT_ADJUSTER_MOVEMENT_DIFF",
    STAT_ADJUSTER_ATTACK = "STAT_ADJUSTER_ATTACK",
    STAT_ADJUSTER_ATTACK_DIFF = "STAT_ADJUSTER_ATTACK_DIFF",
    STAT_ADJUSTER_DEFENCE = "STAT_ADJUSTER_DEFENCE",
    STAT_ADJUSTER_DEFENCE_DIFF = "STAT_ADJUSTER_DEFENCE_DIFF",
}

fn add_adjusters(left: TerrainAdjuster, right: TerrainAdjuster) -> TerrainAdjuster {
    TerrainAdjuster {
        movement: left.movement.wrapping_add(right.movement),
        attack: left.attack.wrapping_add(right.attack),
        defence: left.defence.wrapping_add(right.defence),
    }
}

fn nonzero_adjuster(value: TerrainAdjuster) -> bool {
    value.movement != 0 || value.attack != 0 || value.defence != 0
}

fn describe_adjuster(
    value: TerrainAdjuster,
    icons: &dyn TextIcons,
    loc: &Localisation,
) -> Result<String, String> {
    let mut out = String::new();
    for (key, diff, value) in [
        (
            STAT_ADJUSTER_MOVEMENT,
            STAT_ADJUSTER_MOVEMENT_DIFF,
            value.movement,
        ),
        (
            STAT_ADJUSTER_ATTACK,
            STAT_ADJUSTER_ATTACK_DIFF,
            value.attack,
        ),
        (
            STAT_ADJUSTER_DEFENCE,
            STAT_ADJUSTER_DEFENCE_DIFF,
            value.defence,
        ),
    ] {
        if value == 0 {
            continue;
        }
        out.push_str(&builtin(icons, loc, key, &[])?);
        out.push_str(&builtin(
            icons,
            loc,
            diff,
            &[("VALUE", Argument::Fixed5(value))],
        )?);
        out.push('\n');
    }
    Ok(out)
}

pub(super) struct Content {
    terrain: Arc<TerrainDefinitions>,
    ship_classes: Vec<Vec<u8>>,
    dominance_ratio: Result<i64, String>,
}

impl Content {
    pub(super) fn load(
        layers: &ResolvedLayers,
        defines: &Defines,
        terrain: Arc<TerrainDefinitions>,
    ) -> io::Result<Self> {
        let sub_units = content_reader_key_names::unit_leader::sub_units(layers)?;
        let mut ship_classes = Vec::new();
        for index in 1.. {
            let Some(name) = sub_units.name(index) else {
                break;
            };
            ship_classes.push(name.map_err(super::invalid)?.as_bytes().to_vec());
        }
        Ok(Self {
            terrain,
            ship_classes,
            dominance_ratio: defines
                .fixed5("NNavy", "DOMINANCE_CONTROLLED_THRESHOLD_RATIO")
                .map_err(|error| error.to_string()),
        })
    }

    #[cfg(test)]
    pub(super) fn test_empty(name: &[u8]) -> Self {
        Self {
            terrain: Arc::new(TerrainDefinitions {
                categories: vec![content_reader_key_names::terrain::TerrainCategory {
                    name: name.to_vec(),
                    control_weight: Some(0),
                    movement_cost: 0,
                    combat_width: 0,
                    combat_support_width: 0,
                    supply_flow_penalty_factor: 0,
                    modifiers: Default::default(),
                    classes: Vec::new(),
                    minimum_seazone_dominance: 0,
                    common_adjuster: TerrainAdjuster::default(),
                    class_adjusters: Vec::new(),
                    color: Ok([1.0_f32.to_bits(); 4]),
                }],
                ..Default::default()
            }),
            ship_classes: Vec::new(),
            dominance_ratio: Ok(0),
        }
    }

    pub(super) fn has_category(&self, name: &[u8]) -> bool {
        self.terrain.categories.iter().any(|entry| entry.name == name)
    }

    pub(super) fn prefix(
        &self,
        name: &[u8],
        definitions: &Definitions<'_>,
        icons: &dyn TextIcons,
        loc: &Localisation,
        viewing_tag: Result<&str, &str>,
        dropped: &mut Vec<String>,
    ) -> Result<String, String> {
        let category = self
            .terrain
            .categories
            .iter()
            .find(|entry| entry.name == name)
            .ok_or_else(|| {
                format!(
                    "terrain category {} is absent from the recorded content stack",
                    String::from_utf8_lossy(name)
                )
            })?;
        let mut out = format!("\u{11}H{}\u{11}!\n", token(icons, loc, &category.name)?);
        if category.minimum_seazone_dominance > 0 {
            let ratio = self.dominance_ratio.as_ref().map_err(Clone::clone)?;
            let value = dominance_value(category.minimum_seazone_dominance, *ratio);
            out.push_str(&builtin(
                icons,
                loc,
                MINIMUM_SEAZONE_DOMINANCE_NEEDED,
                &[("VALUE", Argument::Fixed5(value))],
            )?);
            out.push('\n');
        }
        let (common, common_dropped) =
            Block::build(definitions, &category.modifiers).map_err(|error| error.to_string())?;
        dropped.extend(common_dropped.into_iter().map(|key| format!(
            "Naval terrain {} common modifier {key} has no recorded definition and is dropped as the game drops it.",
            String::from_utf8_lossy(name)
        )));
        let mut class_shown = false;
        for name in &self.ship_classes {
            let Some((at, (_, authored))) = category
                .classes
                .iter()
                .enumerate()
                .find(|(_, (key, _))| key == name)
            else {
                continue;
            };
            let (mut block, class_dropped) =
                Block::build(definitions, authored).map_err(|error| error.to_string())?;
            dropped.extend(class_dropped.into_iter().map(|key| format!(
                "Naval terrain {} class {} modifier {key} has no recorded definition and is dropped as the game drops it.",
                String::from_utf8_lossy(&category.name), String::from_utf8_lossy(name)
            )));
            for (&key, &value) in &common.values {
                let entry = block.values.entry(key).or_default();
                *entry = entry.wrapping_add(value);
            }
            let adjuster = add_adjusters(
                category
                    .class_adjusters
                    .get(at)
                    .copied()
                    .unwrap_or_default(),
                category.common_adjuster,
            );
            if block.values.is_empty() && !nonzero_adjuster(adjuster) {
                continue;
            }
            class_shown = true;
            let key = std::str::from_utf8(name).map_err(|_| {
                format!(
                    "naval ship class {} has a non-UTF-8 name",
                    String::from_utf8_lossy(name)
                )
            })?;
            let heading = country_localised(loc, viewing_tag, key)?;
            let heading = displayable(icons, heading)?;
            out.push_str(&format!("\n\u{11}H{heading}\u{11}!\n"));
            let description = block
                .describe(definitions, icons, loc, ONE, "", u32::MAX)
                .map_err(|error| error.to_string())?;
            out.push_str(&description);
            if !block.values.is_empty() {
                out.push('\n');
            }
            out.push_str(&describe_adjuster(adjuster, icons, loc)?);
        }
        if !common.values.is_empty() || nonzero_adjuster(category.common_adjuster) {
            let heading = if class_shown { OTHER_SHIPS } else { ALL_SHIPS };
            out.push_str(&format!(
                "\n\u{11}H{}\u{11}!\n",
                builtin(icons, loc, heading, &[])?
            ));
            let description = common
                .describe(definitions, icons, loc, ONE, "", u32::MAX)
                .map_err(|error| error.to_string())?;
            out.push_str(&description);
            if !common.values.is_empty() {
                out.push('\n');
            }
            out.push_str(&describe_adjuster(category.common_adjuster, icons, loc)?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
