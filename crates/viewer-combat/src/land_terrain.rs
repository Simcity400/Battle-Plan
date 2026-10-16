//! Land combat terrain hovers.
use installed_content::{
    localisation::{Argument, Localisation},
    terrain::TerrainCategory,
};
use replay::{frame::DecodedFrame, rows::tables};
use std::io;
use viewer_foundation::tooltip_text::ensure_newlines;
use viewer_foundation::tooltip_text::tooltip_keys;
use {super::content::Content as StaticContent, viewer_foundation::table_access::rows};
use {
    super::{
        Selection,
        binding::Bindings,
        modifier_text::{Block, Definitions},
        tooltips::{hover_text, set_hover},
    },
    viewer_foundation::tooltip_text::{TextIcons, builtin, token},
};

// Localisation keys for terrain and unit adjuster descriptions.
tooltip_keys! {
    UNIT_TERRAIN_MODIFIER = "UNIT_TERRAIN_MODIFIER",
    STAT_ADJUSTER_MOVEMENT = "STAT_ADJUSTER_MOVEMENT",
    STAT_ADJUSTER_MOVEMENT_VALUE = "STAT_ADJUSTER_MOVEMENT_VALUE",
    STAT_ADJUSTER_ATTACK = "STAT_ADJUSTER_ATTACK",
    STAT_ADJUSTER_ATTACK_VALUE = "STAT_ADJUSTER_ATTACK_VALUE",
    STAT_ADJUSTER_DEFENCE = "STAT_ADJUSTER_DEFENCE",
    STAT_ADJUSTER_DEFENCE_VALUE = "STAT_ADJUSTER_DEFENCE_VALUE",
    TERRAIN_COMBAT_WIDTH_COMB = "TERRAIN_COMBAT_WIDTH_COMB",
    TERRAIN_SUPPLY_FLOW_REDUCTION = "TERRAIN_SUPPLY_FLOW_REDUCTION",
    MOVEMENT_COST = "MOVEMENT_COST",
}

/// common unit adjuster, nonzero width, common modifiers,
/// supply-flow reduction (even zero), then movement cost (even zero).
fn effects(
    terrain: &TerrainCategory,
    definitions: &Definitions<'_>,
    icons: &dyn TextIcons,
    loc: &Localisation,
    dropped: &mut Vec<String>,
) -> Result<String, String> {
    let adjuster = terrain.common_adjuster;
    let mut text = String::new();
    if adjuster.movement != 0 || adjuster.attack != 0 || adjuster.defence != 0 {
        text.push_str(&format!(
            "\u{11}H{}\u{11}!\n",
            builtin(icons, loc, UNIT_TERRAIN_MODIFIER, &[])?
        ));
        // Describe(difference=0, heading=1, indentation=0): each nonzero
        // term is shown as 1 + value, in movement/attack/defence order.
        for (name, value_key, value) in [
            (
                STAT_ADJUSTER_MOVEMENT,
                STAT_ADJUSTER_MOVEMENT_VALUE,
                adjuster.movement,
            ),
            (
                STAT_ADJUSTER_ATTACK,
                STAT_ADJUSTER_ATTACK_VALUE,
                adjuster.attack,
            ),
            (
                STAT_ADJUSTER_DEFENCE,
                STAT_ADJUSTER_DEFENCE_VALUE,
                adjuster.defence,
            ),
        ] {
            if value != 0 {
                text.push_str(&builtin(icons, loc, name, &[])?);
                text.push_str(&builtin(
                    icons,
                    loc,
                    value_key,
                    &[("VALUE", Argument::Fixed5(value.wrapping_add(100_000)))],
                )?);
            }
        }
        ensure_newlines(&mut text, 1);
    }
    if terrain.combat_width != 0 {
        text.push_str(&builtin(
            icons,
            loc,
            TERRAIN_COMBAT_WIDTH_COMB,
            &[
                ("VALUE", Argument::Fixed5(terrain.combat_width)),
                ("ADD", Argument::Fixed5(terrain.combat_support_width)),
            ],
        )?);
        ensure_newlines(&mut text, 1);
    }
    // The shared Block::describe contract: precision 2, no colour, all categories
    // and scale 100000.
    let (block, unknown) =
        Block::build(definitions, &terrain.modifiers).map_err(|why| why.to_string())?;
    dropped.extend(unknown);
    text.push_str(
        &block
            .describe(definitions, icons, loc, 100_000, "", u32::MAX)
            .map_err(|why| why.to_string())?,
    );
    ensure_newlines(&mut text, 1);
    text.push_str(&builtin(
        icons,
        loc,
        TERRAIN_SUPPLY_FLOW_REDUCTION,
        &[(
            "RIGHT",
            Argument::Fixed5(terrain.supply_flow_penalty_factor),
        )],
    )?);
    ensure_newlines(&mut text, 1);
    text.push_str(&builtin(
        icons,
        loc,
        MOVEMENT_COST,
        &[("VALUE", Argument::Fixed5(terrain.movement_cost))],
    )?);
    Ok(text)
}

pub(super) fn bind(
    out: &mut Bindings,
    static_content: &StaticContent,
    frame: &DecodedFrame,
    selection: Selection,
) -> io::Result<()> {
    let Selection::Land(id) = selection else {
        return Ok(());
    };
    let icons = static_content.assets.as_ref();
    let loc = &static_content.localisation;
    let mut dropped = Vec::new();
    let result = (|| {
        let combats = rows::<tables::LandCombats>(frame).map_err(|why| why.to_string())?;
        let combat = combats
            .rows()
            .iter()
            .find(|row| row.id == id)
            .ok_or("LandCombats row is unavailable")?;
        let name = static_content
            .terrain
            .combat_terrain
            .get(&combat.province)
            .ok_or("the combat province has no content terrain category")?;
        let content = icons.land_terrain.as_ref().map_err(Clone::clone)?;
        let terrain = content
            .category(name.as_bytes())
            .ok_or_else(|| format!("content terrain category {name} is absent"))?;
        for why in &content.skipped {
            out.refuse(format!("Land terrain reader: {why}"));
        }
        let terrain = &content.categories[terrain];
        let title = token(icons, loc, &terrain.name)?; // The terrain name is immediate text.
        let definitions = Definitions::read(frame, &static_content.modifier_names)
            .map_err(|why| why.to_string())?;
        let body = effects(
            terrain,
            definitions.as_ref().map_err(|why| why.to_string())?,
            icons,
            loc,
            &mut dropped,
        )?;
        Ok::<_, String>((title, body))
    })();
    let (title, body) = match result {
        Ok((title, body)) => (hover_text(&title), hover_text(&body)),
        Err(why) => {
            out.refuse(format!("Land combat {id} terrain hover is omitted: {why}."));
            (None, None)
        }
    };
    for name in dropped {
        out.refuse(format!(
            "Land combat {id} terrain modifier {name} is not registered; its hover line is omitted."
        ));
    }
    // the game compares the hovered element to terrain_picture.
    set_hover(&mut out.values, "terrain_picture", title, body);
    Ok(())
}

#[cfg(test)]
mod tests;
