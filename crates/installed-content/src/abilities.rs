//! The sprite each ability definition gives the map counter's ability grid.
//!
//! `CAbility::CAbility` (140FF44E0) starts from `GFX_ability_<token>`;
//! `CAbility::parse_key` (140FF5EC0) replaces that base with `icon`. Its
//! finalizer (140FF5B20) appends `_animated`, and the map-counter update
//! (1418C5CE0) gives that sprite to each active-ability child. Recordings hold
//! only the ability token; the sprite is resolved here from the content stack.

use std::{collections::BTreeMap, io};

use crate::{
    ResolvedLayers,
    fields::{all, block, invalid},
    parser,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AbilitySprites {
    /// Ability token to the `GFX_...` sprite its map-counter child shows.
    pub by_token: BTreeMap<Vec<u8>, String>,
}

impl AbilitySprites {
    pub fn sprite(&self, token: &[u8]) -> Option<&str> {
        self.by_token.get(token).map(String::as_str)
    }
}

fn apply(input: &[u8], sprites: &mut AbilitySprites) -> io::Result<()> {
    let document = parser::parse(input)?;
    for root in all(&document.items, b"ability") {
        for item in block(root, "ability")? {
            let token = item
                .key
                .as_deref()
                .filter(|token| !token.is_empty())
                .ok_or_else(|| invalid("ability has no token"))?;
            let name =
                std::str::from_utf8(token).map_err(|_| invalid("ability token is not UTF-8"))?;
            let fields = block(&item.value, "ability definition")?;
            let mut base = format!("GFX_ability_{name}");
            for icon in all(fields, b"icon") {
                let Some(value) = icon.scalar_str().filter(|value| !value.is_empty()) else {
                    return Err(invalid(format!("ability {name} icon is not a sprite name")));
                };
                base = value.to_owned();
            }
            sprites
                .by_token
                .insert(token.to_vec(), format!("{base}_animated"));
        }
    }
    Ok(())
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
    let mut sprites = AbilitySprites::default();
    for path in paths {
        apply(&layers.read(&path)?, &mut sprites)
            .map_err(|error| invalid(format!("{path}: {error}")))?;
    }
    Ok(sprites)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_comment_only_file_declares_nothing() {
        let mut sprites = AbilitySprites::default();
        apply(b"# no abilities in this layer\n", &mut sprites).unwrap();
        assert!(sprites.by_token.is_empty());
    }
}
