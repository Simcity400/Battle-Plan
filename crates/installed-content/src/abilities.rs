//! The sprite each ability definition gives the map counter's ability grid.
//!
//! `CAbility::CAbility` (140FF44E0) starts from `GFX_ability_<token>`;
//! `CAbility::parse_key` (140FF5EC0) replaces that base with `icon`. Its
//! finalizer (140FF5B20) appends `_animated`, and the map-counter update
//! (1418C5CE0) gives that sprite to each active-ability child. Recordings hold
//! only the ability token; the sprite is resolved here from the content stack.

use crate::key_names::KeyNames;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use crate::{
    ResolvedLayers,
    fields::{block, invalid},
    modifiers,
    parser::native::{self, Shape},
};

/// Static token 0x165, the token CUnitAdjuster's constructor leaves in +8
/// (InitializeDefaultTokenField 1424A9460); the engine registers its
/// spelling (140088938 loads `none` at 14271C2A4), as unit_leader cites.
const DEFAULT_TOKEN_KEY: &[u8] = b"none";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AbilitySprites {
    /// Ability token to the `GFX_...` sprite its map-counter child shows.
    pub by_token: BTreeMap<Vec<u8>, String>,
    /// Native reading notes and counted deferred effect warnings, with
    /// file/line context when loaded through the recorded stack.
    pub warnings: Vec<String>,
}

impl AbilitySprites {
    pub fn sprite(&self, token: &[u8]) -> Option<&str> {
        self.by_token.get(token).map(String::as_str)
    }
}

fn apply_with_terrain(
    input: &[u8],
    sprites: &mut AbilitySprites,
    terrains: &BTreeSet<Vec<u8>>,
    names: &KeyNames,
) -> io::Result<Vec<String>> {
    /// Each declaration is a new CAbility (140606A40 -> 140FF44E0), its
    /// adjusters (+0x538) shared by all its `unit_modifiers` blocks.
    #[derive(Clone, Copy)]
    enum Node {
        File,
        Abilities,
        Ability(usize),
        Mtth,
        UnitModifiers(usize),
        Adjuster,
        Modifier(modifiers::Node),
    }
    let mut undecided = Vec::new();
    let mut declared = 0;
    let mut adjusters: BTreeMap<usize, Vec<Vec<u8>>> = BTreeMap::new();
    // 140606A40: file and ability loops. CAbility::parse_key 140FF5EC0
    // ignores unknown fields without skipping; invalid type values record
    // an error and skip. Effects read by key, allowed through LoadBody.
    // 1406073D0 constructs each file's lexer in mode 0 (1424A6370).
    let document = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(Node::File),
        Some(names),
        &mut |node, field| match node {
            Node::File if field.keyword_is("ability") => Shape::Object(Node::Abilities),
            Node::File => Shape::Skip,
            Node::Abilities => {
                declared += 1;
                Shape::Object(Node::Ability(declared))
            }
            Node::Ability(ability) => match field.keyword().as_deref() {
                Some(b"allowed") => Shape::Trigger,
                Some(b"ai_will_do") => Shape::Object(Node::Mtth),
                Some(b"one_time_effect" | b"hourly_effect" | b"daily_effect" | b"end_effect") => {
                    Shape::Effect
                }
                Some(b"country_modifiers" | b"leader_modifiers") => {
                    Shape::Object(Node::Modifier(modifiers::Node::Modifier))
                }
                Some(b"unit_modifiers") => Shape::Object(Node::UnitModifiers(ability)),
                Some(b"type") => match field.value_keyword().as_deref() {
                    Some(
                        b"army_leader" | b"navy_leader" | b"target" | b"state" | b"country"
                        | b"air_zone" | b"naval_zone",
                    ) => Shape::One,
                    _ => Shape::Skip,
                },
                _ => Shape::One,
            },
            Node::Mtth => native::mean_time_to_happen(field),
            // 140FF5EC0 finds the ability's adjuster by the key's token, else
            // CreateUnitAdjusterForToken 141001790 makes one for these five
            // built-in tokens or a registered terrain and reads it
            // (CUnitAdjuster::vf4 141001740); either is then read (again) only
            // while its token is the default `none` (0x165).
            Node::UnitModifiers(ability)
                if field.keyword().is_some_and(|key| {
                    matches!(
                        key.as_slice(),
                        b"night" | b"amphibious" | b"river" | b"fort" | b"snow"
                    ) || terrains.contains(&key)
                }) =>
            {
                let key = field.keyword().unwrap_or_default();
                let seen = adjusters.entry(ability).or_default();
                let repeated = seen.contains(&key);
                let default = key.as_slice() == DEFAULT_TOKEN_KEY;
                seen.push(key);
                match (repeated, default) {
                    (true, true) | (false, false) => Shape::Object(Node::Adjuster),
                    (true, false) => Shape::One,
                    (false, true) => Shape::Twice(Node::Adjuster),
                }
            }
            Node::UnitModifiers(_) => modifiers::shape(
                modifiers::Node::Modifier,
                field,
                modifiers::UNKNOWN,
                &mut undecided,
            )
            .map(Node::Modifier),
            Node::Adjuster => match field.keyword().as_deref() {
                Some(b"attack" | b"defence" | b"movement") => Shape::One,
                _ => Shape::Skip,
            },
            Node::Modifier(node) => {
                modifiers::shape(node, field, modifiers::UNKNOWN, &mut undecided)
                    .map(Node::Modifier)
            }
        },
    );
    if let Some(cause) = document.unsettled {
        return Err(invalid(format!("abilities: {cause}")));
    }
    if let Some(cause) = undecided.first() {
        return Err(invalid(format!("abilities: {cause}")));
    }
    for root in document
        .items
        .iter()
        .filter(|item| item.keyword_is("ability"))
    {
        for item in block(&root.value, "ability")? {
            let token = item
                .key
                .as_deref()
                .filter(|token| !token.is_empty())
                .ok_or_else(|| invalid("ability has no token"))?;
            let name =
                std::str::from_utf8(token).map_err(|_| invalid("ability token is not UTF-8"))?;
            let fields = block(&item.value, "ability definition")?;
            let mut base = format!("GFX_ability_{name}");
            for icon in fields.iter().filter(|item| item.keyword_is("icon")) {
                let value = icon
                    .value
                    .read_str()
                    .map_err(|error| invalid(format!("ability {name} icon: {error}")))?;
                if value.is_empty() || value == "{" {
                    return Err(invalid(format!(
                        "ability {name} icon {value:?} is not a supported sprite name"
                    )));
                }
                base = value.to_owned();
            }
            sprites
                .by_token
                .insert(token.to_vec(), format!("{base}_animated"));
        }
    }
    Ok(document.misreads.iter().map(ToString::to_string).collect())
}

/// Every effective `common/abilities/*.txt` in the byte order the database
/// loader enumerates them (the same folder scan as `common/buildings`); a later
/// definition of a token replaces an earlier one.
pub fn read(layers: &ResolvedLayers) -> io::Result<AbilitySprites> {
    let mut paths: Vec<String> = layers
        .paths()
        .filter(|path| {
            path.to_ascii_lowercase()
                .strip_prefix("common/abilities/")
                .is_some_and(|name| !name.contains('/') && name.ends_with(".txt"))
        })
        .map(str::to_owned)
        .collect();
    paths.sort();
    let terrains = crate::terrain::read_definitions(layers)?
        .categories
        .into_iter()
        .map(|category| category.name.to_ascii_lowercase())
        .collect();
    let mut sprites = AbilitySprites::default();
    let names = layers.key_names()?;
    for path in paths {
        let notes = apply_with_terrain(&layers.read(&path)?, &mut sprites, &terrains, &names)
            .map_err(|error| invalid(format!("{path}: {error}")))?;
        sprites
            .warnings
            .extend(notes.into_iter().map(|note| format!("{path}: {note}")));
    }
    Ok(sprites)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(input: &[u8], sprites: &mut AbilitySprites) -> io::Result<()> {
        let notes = apply_with_terrain(input, sprites, &BTreeSet::new(), &KeyNames::default())?;
        sprites.warnings.extend(notes);
        Ok(())
    }

    #[test]
    fn native_unknown_fields_leave_contents_in_the_ability() {
        let mut sprites = AbilitySprites::default();
        apply(
            br#"@sprite=custom ability={a={"icon"=ignored unknown={ICON=@sprite}}}"#,
            &mut sprites,
        )
        .unwrap();
        assert_eq!(sprites.sprite(b"a"), Some("custom_animated"));
        let mut sprites = AbilitySprites::default();
        apply(
            br#"ability={a={type={icon=ignored} icon=kept}}"#,
            &mut sprites,
        )
        .unwrap();
        assert_eq!(sprites.sprite(b"a"), Some("kept_animated"));
        let error = apply(
            b"ability={a={country_modifiers={unresolved={x=1}}}}",
            &mut sprites,
        )
        .unwrap_err();
        assert!(error.to_string().contains("unresolved"));
    }

    #[test]
    fn default_and_authored_sprites_take_the_animated_suffix() {
        let mut sprites = AbilitySprites::default();
        apply(
            b"ability={ force_attack={ duration=168 } special={ icon=GFX_custom } }",
            &mut sprites,
        )
        .unwrap();
        assert_eq!(
            sprites.sprite(b"force_attack"),
            Some("GFX_ability_force_attack_animated")
        );
        assert_eq!(sprites.sprite(b"special"), Some("GFX_custom_animated"));
        assert_eq!(sprites.sprite(b"absent"), None);
    }

    #[test]
    fn a_later_definition_replaces_the_token_and_a_malformed_icon_fails() {
        let mut sprites = AbilitySprites::default();
        apply(b"ability={ special={ icon=GFX_old } }", &mut sprites).unwrap();
        apply(b"ability={ special={ icon=GFX_new } }", &mut sprites).unwrap();
        assert_eq!(sprites.sprite(b"special"), Some("GFX_new_animated"));
        assert!(
            apply(
                b"ability={ special={ icon={ selected=GFX_new } } }",
                &mut sprites
            )
            .is_err()
        );
    }

    #[test]
    fn repeated_adjusters_are_read_again_only_on_the_default_token() {
        let terrains = BTreeSet::from([b"none".to_vec(), b"plains".to_vec()]);
        let sprite = |text: &[u8]| {
            let mut sprites = AbilitySprites::default();
            apply_with_terrain(text, &mut sprites, &terrains, &KeyNames::default()).unwrap();
            sprites.sprite(b"a").map(str::to_owned)
        };
        // A repeated `night` (token not 0x165) reads its value token alone:
        // the block's `}` ends unit_modifiers, the next ends the ability, so
        // `icon` is no field of `a`.
        assert_eq!(
            sprite(b"ability={a={unit_modifiers={night={} night={}} icon=custom}}").as_deref(),
            Some("GFX_ability_a_animated")
        );
        // Across blocks too: the adjusters are the ability's.
        assert_eq!(
            sprite(
                b"ability={a={unit_modifiers={plains={}} unit_modifiers={plains={}} icon=custom}}"
            )
            .as_deref(),
            Some("GFX_ability_a_animated")
        );
        // A new declaration has new adjusters.
        assert_eq!(
            sprite(b"ability={a={unit_modifiers={night={}}} a={unit_modifiers={night={}} icon=custom}}")
                .as_deref(),
            Some("custom_animated")
        );
        // A terrain named `none` is read twice when new (141001790, then
        // 140FF5EC0), and again when repeated: the second read takes the
        // tokens after its block, so `icon` is read inside it.
        assert_eq!(
            sprite(b"ability={a={unit_modifiers={none={} icon=custom}}}").as_deref(),
            Some("GFX_ability_a_animated")
        );
    }

    #[test]
    fn a_comment_only_file_declares_nothing() {
        let mut sprites = AbilitySprites::default();
        apply(b"# no abilities in this layer\n", &mut sprites).unwrap();
        assert!(sprites.by_token.is_empty());
    }
}
