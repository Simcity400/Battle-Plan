//! Immutable attrition facts for content loading and counter hover lines.
use crate::map::counter_inputs::SCALE as ONE;
use std::io;

/// The content the lines read: three defines and each terrain category's
/// stored `attrition` entry.
pub struct Content {
    /// `NMilitary.TRAINING_ATTRITION`.
    pub training: i64,
    /// `NMilitary.BORDER_WAR_ATTRITION_FACTOR`.
    pub border_factor: i64,
    /// `NSupply.SUPPLY_THRESHOLD_FOR_ARMY_ATTRITION`, which its
    /// loader clamps to [0, 100000].
    pub supply_threshold: i64,
    /// The terrain database in load order: one-based ordinal
    /// ordinal minus one.
    pub terrains: Vec<Terrain>,
}

pub struct Terrain {
    /// The category's name, which `ATTRITION_TERRAIN`
    /// localises.
    pub name: Vec<u8>,
    /// The stored `attrition` entry of the category's common modifier
    /// operand; `None` when the category has none.
    pub attrition: Option<i64>,
}

impl Content {
    pub fn read(
        defines: &installed_content::defines::Defines,
        terrain: &installed_content::terrain::TerrainDefinitions,
    ) -> io::Result<Self> {
        Ok(Self {
            training: defines.fixed5("NMilitary", "TRAINING_ATTRITION")?,
            border_factor: defines.fixed5("NMilitary", "BORDER_WAR_ATTRITION_FACTOR")?,
            supply_threshold: defines
                .fixed5("NSupply", "SUPPLY_THRESHOLD_FOR_ARMY_ATTRITION")?
                .clamp(0, ONE),
            terrains: terrain
                .categories
                .iter()
                .map(|category| Terrain {
                    name: category.name.clone(),
                    attrition: category
                        .modifiers
                        .values
                        .iter()
                        .find(|value| value.name == "attrition")
                        .map(|value| value.value),
                })
                .collect(),
        })
    }
}
