//! Hourly commander presentation, with native portrait and assigned-trait selection.
use super::{
    Selection, Values, Widget,
    binding::{Bindings, FLAG, SPRITE, country_flag, hide, sprite},
};
use crate::{document::Document, map::tables::rows};
use replay::{
    frame::DecodedFrame,
    rows::{
        CombatCommanderPortraitEntry, CombatCommanderPresentation, LandCombatCommanderTrait, tables,
    },
};
use std::io;

/// The caller loads direct textures once, through the resolved content layers.
/// Keep the declaration and texture namespaces separate: native icon kind 0/1
/// means a GFX declaration / a file path (141388180, 140B37430).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum PortraitAsset {
    Declaration(String),
    Texture(String),
}
impl PortraitAsset {
    pub(super) fn key(&self) -> String {
        match self {
            Self::Declaration(name) => name.clone(),
            Self::Texture(path) => format!("combat_portrait:{path}"),
        }
    }
}

fn identity(bytes: &[u8]) -> Result<String, &'static str> {
    let value = std::str::from_utf8(bytes).map_err(|_| "non-UTF8 portrait identity")?;
    if value.is_empty() {
        return Err("empty portrait identity");
    }
    Ok(value.to_owned())
}
fn texture(bytes: &[u8]) -> Result<PortraitAsset, &'static str> {
    let path = identity(bytes)?.replace('\\', "/");
    // A recorded identity is content-relative. It must never authorize a read
    // outside the resolved game content, including drive or parent traversal.
    if path.starts_with('/') || path.contains(':') || path.split('/').any(|p| p == "..") {
        return Err("portrait path outside relative game content");
    }
    Ok(PortraitAsset::Texture(
        path.split('/')
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join("/"),
    ))
}

/// 140B3A100 asks for size 1 and maps commander categories to character roles.
/// 141389840 selects an exact role/size, otherwise the first role at that size.
/// It does not skip an unset exact match in favour of another role.
fn portrait_entry(
    presentation: &CombatCommanderPresentation,
    category: i32,
) -> Option<&CombatCommanderPortraitEntry> {
    let role = match category {
        2 => 2,
        3 => 4,
        _ => 1,
    };
    presentation
        .portraits
        .iter()
        .find(|entry| entry.size == 1 && entry.role == role)
        .or_else(|| {
            presentation
                .portraits
                .iter()
                .filter(|entry| entry.size == 1)
                .min_by_key(|entry| entry.role)
        })
}

pub(super) fn select_portrait(
    presentation: &CombatCommanderPresentation,
    category: i32,
) -> Result<PortraitAsset, &'static str> {
    if presentation.character_state == 2 {
        let entry = portrait_entry(presentation, category)
            .filter(|entry| entry.is_set != 0)
            .ok_or("generated character portrait selection is UNVERIFIED (141388EC0/1406A8D80)")?;
        return match entry.kind {
            0 => Ok(PortraitAsset::Declaration(identity(&entry.text)?)),
            1 => texture(&entry.text),
            _ => Err("unsupported character portrait kind"),
        };
    }
    // 140C09910 uses +C0 directly when present. +E0 constructs a country
    // graphics-tag path; do not substitute the political or cosmetic tag until
    // that distinct 1406DF830 -> 140BA5C20 content selection is ported.
    if !presentation.fallback_text[0].is_empty() {
        return texture(&presentation.fallback_text[0]);
    }
    if !presentation.fallback_text[1].is_empty() {
        return Err("country graphics-tag portrait suffix selection is UNVERIFIED (140C09910)");
    }
    if !presentation.fallback_text[2].is_empty() {
        return Ok(PortraitAsset::Declaration(identity(
            &presentation.fallback_text[2],
        )?));
    }
    Ok(PortraitAsset::Declaration("GFX_leader_unknown".into()))
}

/// A present commander's portrait. ResolveLeaderPortrait (140B3A100)
/// selects it ([`select_portrait`], `GFX_leader_unknown` when nothing is
/// set) and sets it by name: CIcon::SetSpriteByName (1422E6410) does nothing
/// when the content does not declare the sprite, so the widget keeps what it
/// showed before. The viewer then leaves the widget unbound (the .gui's own
/// sprite shows) and names the gap. A portrait whose selection the port
/// cannot settle is hidden with the named refusal.
fn portrait(
    out: &mut Bindings,
    has_sprite: &dyn Fn(&str) -> bool,
    key: &str,
    presentation: &CombatCommanderPresentation,
    category: i32,
) {
    match select_portrait(presentation, category) {
        Ok(asset) if has_sprite(&asset.key()) => out.bind(key, SPRITE, Ok(asset.key())),
        Ok(PortraitAsset::Declaration(name)) => out.refuse(format!(
            "Combat portrait {key}: the content does not declare sprite {name}, so the game's sprite setter (1422E6410) keeps the widget's previous image; the viewer shows the window's own sprite"
        )),
        Ok(asset) => out.bind(
            key,
            SPRITE,
            Err(format!(
                "Combat portrait {key} is hidden: texture {} cannot be drawn (what the game's loader 1423F2BB0 gives for it is unverified)",
                asset.key()
            )),
        ),
        Err(reason) => out.bind(
            key,
            SPRITE,
            Err(format!("Combat portrait {key} is hidden: {reason}")),
        ),
    }
}

/// A side's tactic picture (`{prefix}_tactic`) for a selected combat tactic
/// (kind 2): the registry entry's own picture, through the registry's
/// identity and refusal checks (1406B0A70's +0x98 ID must name the recorded
/// token; 1406B3CB0, 141C5DD20), when the content has the sprite; else the
/// widget is hidden with the named refusal.
fn tactic(
    out: &mut Bindings,
    has_sprite: &dyn Fn(&str) -> bool,
    tactics: &super::tactics::Content,
    key: &str,
    id: i32,
    token: &[u8],
) {
    let name = String::from_utf8_lossy(token);
    let value = match tactics.picture(id, token) {
        Ok(picture) if has_sprite(picture) => Ok(picture.to_owned()),
        Ok(picture) => Err(format!(
            "Combat tactic picture {key} is hidden: the content lacks sprite {picture} for tactic {name}"
        )),
        Err(reason) => Err(format!("Combat tactic picture {key} is hidden: {reason}")),
    };
    out.bind(key, SPRITE, value);
}

fn exile_entry(out: &mut Bindings, entry: &mut Values, flag: Result<String, String>) {
    // 141C61C40 fills the exile flag through 140B35990.
    match flag {
        Ok(flag) => out.bind_in(entry, "exile_flag", FLAG, Ok(flag)),
        Err(why) => {
            hide(entry, "exile_flag");
            hide(entry, "exile_flag_frame");
            out.refuse(format!("Combat exile trait omitted: {why}"));
        }
    }
}

fn traits(
    out: &mut Bindings,
    document: &Document,
    key: &str,
    traits: &[LandCombatCommanderTrait],
    exile: &Result<String, String>,
) {
    // Both filters are enabled by 1415C6AB0 (land), 1417D81B0 (naval),
    // and 1417C6800 (results). Preserve native assigned order, not token order.
    let mut entries = Vec::new();
    let mut entry_templates = std::collections::BTreeMap::new();
    // Trait sprites that are not loaded, counted by name and why.
    let mut missing: std::collections::BTreeMap<(String, String), usize> = Default::default();
    let names = document.icons.combat.tactics.trait_names();
    let not_declared = || "which the content does not declare".to_owned();
    for value in traits.iter().filter(|t| t.kind != 0 && t.relevant != 0) {
        let mut entry = Values::new();
        if value.kind == 6 {
            // 141C5EF70 / 141C60E40 switch the assigned-trait entry template.
            entry_templates.insert(entries.len(), "unit_leader_exile_trait_entry".into());
            exile_entry(out, &mut entry, exile.clone());
        } else {
            let token = String::from_utf8_lossy(&value.token);
            let requested = format!("GFX_trait_{token}");
            if document.icons.combat.has_sprite(&requested) {
                sprite(&mut entry, "Background", requested);
            } else {
                // 141C61C40's actual missing-declaration fallback; the
                // missing sprite is still reported by name. Only the
                // content's trait names have their sprites loaded
                // (installed_content::unit_leader::Traits::registered).
                let why = if names.contains(&value.token) {
                    not_declared()
                } else if names.unsupported.is_empty() {
                    "which is not loaded: no leader_traits entry of the content names the trait"
                        .to_owned()
                } else {
                    format!(
                        "which is not loaded: the trait is not among the proven trait names, and {}",
                        names.unsupported.join("; ")
                    )
                };
                *missing.entry((requested, why)).or_default() += 1;
                if document.icons.combat.has_sprite("GFX_trait_unknown") {
                    sprite(&mut entry, "Background", "GFX_trait_unknown".into());
                } else {
                    *missing
                        .entry(("GFX_trait_unknown".into(), not_declared()))
                        .or_default() += 1;
                }
            }
        }
        entries.push(entry);
    }
    for ((name, why), count) in missing {
        out.warnings.push(format!(
            "Combat traits {key}: {count} trait entries name sprite {name}, {why}."
        ));
    }
    out.values.insert(
        key.into(),
        Widget {
            entry_template: Some("unit_leader_trait_entry".into()),
            entry_templates,
            entries,
            ..Widget::default()
        },
    );
}

pub(super) fn bind(
    out: &mut Bindings,
    document: &Document,
    frame: &DecodedFrame,
    selection: Selection,
    flags: &std::collections::BTreeMap<u32, String>,
) -> io::Result<()> {
    let has_sprite = |name: &str| document.icons.combat.has_sprite(name);
    let leaders = rows::<tables::UnitLeaders>(frame);
    let exile = |commander: u64| -> Result<String, String> {
        let rows = leaders
            .as_ref()
            .map_err(|e| format!("UnitLeaders unavailable: {e}"))?;
        let row = rows
            .rows()
            .iter()
            .find(|row| (u64::from(row.id) << 32 | u64::from(row.reference_type)) == commander)
            .ok_or_else(|| format!("UnitLeaders lacks commander {commander}"))?;
        country_flag(
            flags,
            *row.exile_country.require("UnitLeaders.exile_country")? as u32,
        )
    };
    match selection {
        Selection::Land(id) => {
            let details = rows::<tables::LandCombatDetails>(frame)?;
            let Some(detail) = details.rows().iter().find(|row| row.combat == id) else {
                out.warnings.push(format!(
                    "Land combat {id}: no LandCombatDetails row, so the commander portraits, traits and tactic pictures are omitted."
                ));
                return Ok(());
            };
            for (index, prefix) in ["attacker", "defender"].into_iter().enumerate() {
                let side = &detail.sides[index];
                if side.commander != 0 {
                    portrait(
                        out,
                        &has_sprite,
                        &format!("{prefix}_leader"),
                        &side.presentation,
                        side.commander_category,
                    );
                }
                traits(
                    out,
                    document,
                    &format!("{prefix}_traits_grid"),
                    &side.traits,
                    &exile(side.commander),
                );
                if side.tactic_kind == 2 {
                    tactic(
                        out,
                        &has_sprite,
                        &document.icons.combat.tactics,
                        &format!("{prefix}_tactic"),
                        side.tactic_id,
                        &side.tactic_token,
                    );
                }
                // 1406B3670 compares this tactic's counter id to the other id.
                // Null and CNullCombatTactic are distinct recorded kinds; neither
                // provides an authenticated selected tactic id for this binding.
                let other = &detail.sides[1 - index];
                out.values
                    .entry(format!("{prefix}_countered_tactic_overlay"))
                    .or_default()
                    .hidden = side.tactic_kind != 2
                    || other.tactic_kind != 2
                    || side.tactic_counter_id != other.tactic_id as u32;
            }
        }
        Selection::Naval(id) => {
            let details = rows::<tables::NavalCombatDetails>(frame)?;
            let Some(detail) = details.rows().iter().find(|row| row.combat == id) else {
                out.warnings.push(format!(
                    "Naval combat {id}: no NavalCombatDetails row, so the commander portraits and traits are omitted."
                ));
                return Ok(());
            };
            for (side, prefix) in detail.sides.iter().zip(["attacker", "defender"]) {
                if side.commander_inputs.is_none() && side.commander != 0 {
                    out.warnings.push(format!(
                        "Naval combat {id} {prefix} commander {}: its commander inputs are not recorded, so the portrait and traits are omitted.",
                        side.commander
                    ));
                }
                if let Some(commander) = &side.commander_inputs {
                    if commander.commander != 0 {
                        portrait(
                            out,
                            &has_sprite,
                            &format!("header_container/leader_{prefix}"),
                            &commander.presentation,
                            commander.category,
                        );
                    }
                    traits(
                        out,
                        document,
                        &format!("header_container/traits_grid_{prefix}"),
                        &commander.traits,
                        &exile(commander.commander),
                    );
                }
            }
        }
        Selection::NavalResult(id) => {
            let details = rows::<tables::NavalCombatResultDetails>(frame)?;
            let Some(detail) = details.rows().iter().find(|row| row.result == id) else {
                out.warnings.push(format!(
                    "Naval result {id}: no NavalCombatResultDetails row, so the commander portraits and traits are omitted."
                ));
                return Ok(());
            };
            for (side, prefix) in detail.sides.iter().zip(["attacker", "defender"]) {
                if side.commander_context.is_none() && side.leader != 0 {
                    out.warnings.push(format!(
                        "Naval result {id} {prefix} commander {}: its commander context is not recorded, so the portrait and traits are omitted.",
                        side.leader
                    ));
                }
                if side.hq_commander_context.is_none() && side.hq_leader != 0 {
                    out.warnings.push(format!(
                        "Naval result {id} {prefix} HQ commander {}: its commander context is not recorded, so the HQ portrait is omitted.",
                        side.hq_leader
                    ));
                }
                if let Some(commander) = &side.commander_context {
                    let root = format!("{prefix}_leader");
                    if commander.commander != 0 {
                        portrait(
                            out,
                            &has_sprite,
                            &format!("{root}/leader_portrait"),
                            &commander.presentation,
                            commander.category,
                        );
                        traits(
                            out,
                            document,
                            &format!("{root}/traits_grid"),
                            &commander.traits,
                            &exile(commander.commander),
                        );
                    }
                }
                if let Some(commander) = &side.hq_commander_context
                    && commander.commander != 0
                {
                    let root = format!("{prefix}_hq_leader");
                    out.values.entry(root.clone()).or_default().hidden = false;
                    portrait(
                        out,
                        &has_sprite,
                        &format!("{root}/hq_leader_portrait"),
                        &commander.presentation,
                        commander.category,
                    );
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
