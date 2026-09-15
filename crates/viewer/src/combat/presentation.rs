//! Hourly commander presentation, with native portrait and assigned-trait selection.
use super::{
    Selection, Values, Widget,
    binding::{Bindings, hide, sprite},
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

fn portrait(
    out: &mut Bindings,
    document: &Document,
    key: &str,
    presentation: &CombatCommanderPresentation,
    category: i32,
) {
    // A present leader whose portrait cannot resolve uses this native default,
    // not land combat's distinct no-commander silhouette (140B3A100).
    if document
        .icons
        .combat
        .sprites
        .contains_key("GFX_leader_unknown")
    {
        sprite(&mut out.values, key, "GFX_leader_unknown".into());
        out.values.entry(key.into()).or_default().hidden = false;
    }
    match select_portrait(presentation, category) {
        Ok(asset) if document.icons.combat.sprites.contains_key(&asset.key()) => {
            sprite(&mut out.values, key, asset.key());
            out.values.entry(key.into()).or_default().hidden = false;
        }
        Ok(asset) => out.warnings.push(format!(
            "Combat portrait {key}: {} is absent from the verified static asset inventory; the unknown portrait remains. Runtime texture loading and native generated fallback are UNVERIFIED.",
            asset.key()
        )),
        Err(reason) => out.warnings.push(format!(
            "Combat portrait {key}: {reason}; the unknown portrait remains. Matching static picker evidence and a same-hour format-22 portrait capture would settle this."
        )),
    }
}

fn traits(out: &mut Bindings, document: &Document, key: &str, traits: &[LandCombatCommanderTrait]) {
    // Both filters are enabled by 1415C6AB0 (land), 1417D81B0 (naval),
    // and 1417C6800 (results). Preserve native assigned order, not token order.
    let mut entries = Vec::new();
    for value in traits.iter().filter(|t| t.kind != 0 && t.relevant != 0) {
        let mut entry = Values::new();
        if value.kind == 6 {
            // Native uses unit_leader_exile_trait_entry and leader+ED4's country.
            // The current context does not record that exile-country operand.
            // Retain its slot but leave it empty instead of inventing a flag.
            hide(&mut entry, "Background");
            out.warnings.push("Combat trait: exile flag source leader+ED4 is unavailable; its assigned slot remains empty. UNVERIFIED until that source is recorded and the exile entry is captured.".into());
        } else {
            let token = String::from_utf8_lossy(&value.token);
            let requested = format!("GFX_trait_{token}");
            let selected = if document.icons.combat.sprites.contains_key(&requested) {
                requested
            } else {
                // 141C61C40's actual missing-declaration fallback.
                "GFX_trait_unknown".into()
            };
            sprite(&mut entry, "Background", selected);
        }
        entries.push(entry);
    }
    out.values.insert(
        key.into(),
        Widget {
            entry_template: Some("unit_leader_trait_entry".into()),
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
) -> io::Result<()> {
    match selection {
        Selection::Land(id) => {
            let details = rows::<tables::LandCombatDetails>(frame)?;
            let Some(detail) = details.rows().iter().find(|row| row.combat == id) else {
                return Ok(());
            };
            for (index, prefix) in ["attacker", "defender"].into_iter().enumerate() {
                let side = &detail.sides[index];
                if side.commander != 0 {
                    portrait(
                        out,
                        document,
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
                );
                let key = format!("{prefix}_tactic");
                if side.tactic_kind == 2 {
                    let token = String::from_utf8_lossy(&side.tactic_token);
                    if let Some(picture) = document
                        .icons
                        .combat
                        .tactic_pictures
                        .get(token.as_ref())
                        .filter(|name| document.icons.combat.sprites.contains_key(*name))
                    {
                        sprite(&mut out.values, &key, picture.clone());
                        out.values.entry(key).or_default().hidden = false;
                    } else {
                        out.warnings.push(format!("Combat tactic {token}: matching verified picture is unavailable; widget remains empty."));
                    }
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
                return Ok(());
            };
            for (side, prefix) in detail.sides.iter().zip(["attacker", "defender"]) {
                if let Some(commander) = &side.commander_inputs {
                    if commander.commander != 0 {
                        portrait(
                            out,
                            document,
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
                    );
                }
            }
        }
        Selection::NavalResult(id) => {
            let details = rows::<tables::NavalCombatResultDetails>(frame)?;
            let Some(detail) = details.rows().iter().find(|row| row.result == id) else {
                return Ok(());
            };
            for (side, prefix) in detail.sides.iter().zip(["attacker", "defender"]) {
                if let Some(commander) = &side.commander_context {
                    let root = format!("{prefix}_leader");
                    if commander.commander != 0 {
                        portrait(
                            out,
                            document,
                            &format!("{root}/leader_portrait"),
                            &commander.presentation,
                            commander.category,
                        );
                        traits(
                            out,
                            document,
                            &format!("{root}/traits_grid"),
                            &commander.traits,
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
                        document,
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
mod tests {
    use super::*;

    #[test]
    fn portrait_role_fallback_stays_at_the_requested_size() {
        let mut value = CombatCommanderPresentation {
            character_state: 2,
            ..Default::default()
        };
        for (role, size, text) in [(1, 0, "wrong_size"), (4, 1, "naval"), (1, 1, "first_role")] {
            value.portraits.push(CombatCommanderPortraitEntry {
                role,
                size,
                is_set: 1,
                kind: 0,
                text: text.as_bytes().into(),
            });
        }
        assert_eq!(
            select_portrait(&value, 3).unwrap(),
            PortraitAsset::Declaration("naval".into())
        );
        assert_eq!(
            select_portrait(&value, 2).unwrap(),
            PortraitAsset::Declaration("first_role".into())
        );
        value.portraits[1].is_set = 0;
        assert!(select_portrait(&value, 3).is_err());
    }

    #[test]
    fn authenticated_character_does_not_use_unresolved_leader_fallback() {
        let value = CombatCommanderPresentation {
            character_state: 2,
            fallback_text: [b"gfx/leaders/old.dds".to_vec(), Vec::new(), Vec::new()],
            ..Default::default()
        };
        assert!(select_portrait(&value, 0).is_err());
        assert!(texture(b"../outside.dds").is_err());
        assert!(texture(b"C:\\outside.dds").is_err());
    }
}
