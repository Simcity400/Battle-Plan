use content_reader_key_names::key_name_products::KeyNamesExt as _;

use crate::key_names::KeyNames;
use std::{collections::BTreeMap, io};

use content_reader_script::native::{self, Shape};
use crate::{
    ResolvedLayers,
    fields::{block, invalid},
    modifiers::{self, Modifiers},
    parser::Item,
    scope::ScopeKind,
};

const DEFAULT_TOKEN_KEY: &[u8] = b"none";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AbilitySprites {
    pub by_token: BTreeMap<Vec<u8>, String>,
    pub definitions: BTreeMap<Vec<u8>, Definition>,
    pub warnings: Vec<crate::Diagnostic>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Definition {
    pub name: Vec<u8>,
    pub description: Vec<u8>,
    pub modifiers: [Modifiers; 3],
    pub adjusters: Vec<crate::unit_leader::Adjuster>,
    pub effects: [Vec<Item>; 3],
}

impl AbilitySprites {
    pub fn sprite(&self, token: &[u8]) -> Option<&str> {
        self.by_token.get(token).map(String::as_str)
    }

    pub fn static_sprite(&self, token: &[u8]) -> Option<&str> {
        self.sprite(token)?.strip_suffix("_animated")
    }

    pub fn name_key(&self, token: &[u8]) -> Option<&str> {
        std::str::from_utf8(&self.definition(token)?.name).ok()
    }

    pub fn definition(&self, token: &[u8]) -> Option<&Definition> {
        self.definitions.get(token)
    }
}

fn apply_with_terrain(
    input: &[u8],
    sprites: &mut AbilitySprites,
    terrains: &BTreeMap<Vec<u8>, Vec<u8>>,
    names: &KeyNames,
) -> io::Result<Vec<crate::Diagnostic>> {
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
    let mut ability_scopes = Vec::new();
    let mut adjusters: BTreeMap<usize, Vec<Vec<u8>>> = BTreeMap::new();
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
                ability_scopes.push(ScopeKind::Country.bit());
                Shape::Object(Node::Ability(declared))
            }
            Node::Ability(ability) => match field.keyword().as_deref() {
                Some(b"allowed") => Shape::Trigger,
                Some(b"ai_will_do") => Shape::Object(Node::Mtth),
                Some(b"one_time_effect" | b"hourly_effect" | b"daily_effect" | b"end_effect") => {
                    Shape::EffectIn(ability_scopes[ability - 1])
                }
                Some(b"country_modifiers" | b"leader_modifiers") => {
                    Shape::Object(Node::Modifier(modifiers::Node::Modifier))
                }
                Some(b"unit_modifiers") => Shape::Object(Node::UnitModifiers(ability)),
                Some(b"type") => match field.value_keyword().as_deref() {
                    Some(
                        kind @ (b"army_leader" | b"navy_leader" | b"target" | b"state" | b"country"
                        | b"air_zone" | b"naval_zone"),
                    ) => {
                        ability_scopes[ability - 1] = match kind {
                            b"army_leader" | b"navy_leader" => ScopeKind::Character.bit(),
                            b"state" => ScopeKind::State.bit(),
                            b"air_zone" | b"naval_zone" => ScopeKind::StrategicRegion.bit(),
                            _ => ScopeKind::Country.bit(),
                        };
                        Shape::One
                    }
                    _ => Shape::Skip,
                },
                _ => Shape::One,
            },
            Node::Mtth => native::mean_time_to_happen(field),
            Node::UnitModifiers(ability)
                if field.keyword().is_some_and(|key| {
                    matches!(
                        key.as_slice(),
                        b"night" | b"amphibious" | b"river" | b"fort" | b"snow"
                    ) || terrains.contains_key(&key)
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
            let token = crate::parser::c_string(
                item.key
                    .as_deref()
                    .ok_or_else(|| invalid("ability has no token"))?,
            );
            let name =
                std::str::from_utf8(token).map_err(|_| invalid("ability token is not UTF-8"))?;
            let fields = block(&item.value, "ability definition")?;
            let mut base = format!("GFX_ability_{name}");
            let mut definition = Definition {
                name: token.to_vec(),
                description: [token, b"_desc"].concat(),
                ..Default::default()
            };
            for field in fields {
                if field.keyword_is("name") || field.keyword_is("desc") {
                    let value = field.value.read_string().ok_or_else(|| {
                        invalid(format!("ability {name} display key is not a string"))
                    })?;
                    if field.keyword_is("name") {
                        definition.name = value.to_vec();
                    } else {
                        definition.description = value.to_vec();
                    }
                } else if field.keyword_is("icon") {
                    let value = field
                        .value
                        .read_str()
                        .map_err(|error| invalid(format!("ability {name} icon: {error}")))?;
                    if value.is_empty() || value == "{" {
                        return Err(invalid(format!(
                            "ability {name} icon {value:?} is not a supported sprite name"
                        )));
                    }
                    base = value.to_owned();
                } else if let Some(slot) =
                    ["country_modifiers", "leader_modifiers", "unit_modifiers"]
                        .iter()
                        .position(|key| field.keyword_is(key))
                {
                    for entry in field.value.items().unwrap_or_default() {
                        if slot == 2 {
                            let adjuster_key = entry.keyword().and_then(|key| {
                                let key = key.folded();
                                if matches!(
                                    key.as_slice(),
                                    b"night" | b"amphibious" | b"river" | b"fort" | b"snow"
                                ) {
                                    Some(key)
                                } else {
                                    terrains.get(&key).cloned()
                                }
                            });
                            if let Some(key) = adjuster_key {
                                let key = String::from_utf8(key)
                                    .map_err(|_| invalid("ability adjuster token is not UTF-8"))?;
                                let index = definition
                                    .adjusters
                                    .iter()
                                    .position(|old| old.key == key)
                                    .unwrap_or_else(|| {
                                        definition.adjusters.push(crate::unit_leader::Adjuster {
                                            key,
                                            ..Default::default()
                                        });
                                        definition.adjusters.len() - 1
                                    });
                                let adjuster = &mut definition.adjusters[index];
                                for component in entry.value.items().unwrap_or_default() {
                                    let target = match component
                                        .keyword()
                                        .map(|key| key.folded())
                                        .as_deref()
                                    {
                                        Some(b"attack") => &mut adjuster.attack,
                                        Some(b"defence") => &mut adjuster.defence,
                                        Some(b"movement") => &mut adjuster.movement,
                                        _ => continue,
                                    };
                                    if let Some(value) = modifiers::read_fixed5(component) {
                                        *target = value;
                                    }
                                }
                                continue;
                            }
                        }
                        definition.modifiers[slot].read(entry);
                    }
                } else if let Some(slot) = ["one_time_effect", "hourly_effect", "daily_effect"]
                    .iter()
                    .position(|key| field.keyword_is(key))
                {
                    definition.effects[slot]
                        .extend(field.value.items().unwrap_or_default().iter().cloned());
                }
            }
            sprites
                .by_token
                .entry(token.to_vec())
                .or_insert_with(|| format!("{base}_animated"));
            sprites
                .definitions
                .entry(token.to_vec())
                .or_insert(definition);
        }
    }
    Ok(document
        .misreads
        .iter()
        .map(|note| (note.kind, note.to_string()))
        .collect())
}

pub fn read(layers: &ResolvedLayers) -> io::Result<AbilitySprites> {
    let paths = content_reader_layers::script_files::database_paths(layers, "common/abilities/")?;
    let terrains = crate::terrain::read_definitions(layers)?
        .categories
        .into_iter()
        .map(|category| (category.name.to_ascii_lowercase(), category.name))
        .collect();
    let mut sprites = AbilitySprites::default();
    let names = layers.key_names()?;
    for path in paths {
        let notes = apply_with_terrain(&path.read(layers)?, &mut sprites, &terrains, &names)
            .map_err(|error| invalid(format!("{path}: {error}")))?;
        for (kind, note) in notes {
            sprites.warnings.push((kind, format!("{path}: {note}")));
        }
    }
    Ok(sprites)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ability_notes_keep_kind() {
        content_reader_script::test_game::enter();
        let notes = apply_with_terrain(
            b"ability = { a = { duration = { x = 1 } } }",
            &mut AbilitySprites::default(),
            &BTreeMap::new(),
            &KeyNames::default(),
        )
        .unwrap();
        assert!(
            notes
                .iter()
                .any(|(kind, _)| *kind == crate::DiagnosticKind::Misread),
            "{notes:?}"
        );
    }

    fn apply(input: &[u8], sprites: &mut AbilitySprites) -> io::Result<()> {
        let notes = apply_with_terrain(input, sprites, &BTreeMap::new(), &KeyNames::default())?;
        sprites.warnings.extend(notes);
        Ok(())
    }

    #[test]
    fn unknown_fields_leave_contents_in_the_ability() {
        content_reader_script::test_game::enter();
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
        content_reader_script::test_game::enter();
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
    fn lookup_keeps_the_first_exact_token_and_later_malformed_fields_still_fail() {
        content_reader_script::test_game::enter();
        let mut sprites = AbilitySprites::default();
        apply(b"ability={ special={ icon=GFX_old } }", &mut sprites).unwrap();
        apply(b"ability={ special={ icon=GFX_new } }", &mut sprites).unwrap();
        assert_eq!(sprites.sprite(b"special"), Some("GFX_old_animated"));
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
        content_reader_script::test_game::enter();
        let terrains = BTreeMap::from([
            (b"none".to_vec(), b"none".to_vec()),
            (b"plains".to_vec(), b"plains".to_vec()),
        ]);
        let sprite = |text: &[u8]| {
            let mut sprites = AbilitySprites::default();
            apply_with_terrain(text, &mut sprites, &terrains, &KeyNames::default()).unwrap();
            sprites.sprite(b"a").map(str::to_owned)
        };
        assert_eq!(
            sprite(b"ability={a={unit_modifiers={night={} night={}} icon=custom}}").as_deref(),
            Some("GFX_ability_a_animated")
        );
        assert_eq!(
            sprite(
                b"ability={a={unit_modifiers={plains={}} unit_modifiers={plains={}} icon=custom}}"
            )
            .as_deref(),
            Some("GFX_ability_a_animated")
        );
        assert_eq!(
            sprite(b"ability={a={unit_modifiers={night={}}} a={unit_modifiers={night={}} icon=custom}}")
                .as_deref(),
            Some("GFX_ability_a_animated")
        );
        assert_eq!(
            sprite(b"ability={a={unit_modifiers={none={} icon=custom}}}").as_deref(),
            Some("GFX_ability_a_animated")
        );
    }

    #[test]
    fn a_comment_only_file_declares_nothing() {
        content_reader_script::test_game::enter();
        let mut sprites = AbilitySprites::default();
        apply(b"# no abilities in this layer\n", &mut sprites).unwrap();
        assert!(sprites.by_token.is_empty());
    }

    #[test]
    fn value_effect_scopes_follow_ability_type_in_authored_order() {
        content_reader_script::test_game::enter();
        let mut sprites = AbilitySprites::default();
        let notes = apply_with_terrain(br#"
        ability={
            before={one_time_effect={remove_unit_leader_role={@before=3 character=fixture}} type=army_leader icon=@before}
            character={type=army_leader daily_effect={remove_unit_leader_role={@hidden=7 character=fixture}} icon=@hidden}
            reset={type=army_leader type=country end_effect={remove_unit_leader_role={@after=5 character=fixture}} icon=@after}
        }
    "#, &mut sprites, &BTreeMap::new(), &KeyNames::default()).unwrap();
        assert_eq!(sprites.sprite(b"before"), Some("3_animated"));
        assert_eq!(sprites.sprite(b"character"), Some("@hidden_animated"));
        assert_eq!(sprites.sprite(b"reset"), Some("5_animated"));
        assert!(
            !notes.iter().any(|note| note.1.contains("Deferred effect")),
            "{notes:?}"
        );
    }

    #[test]
    fn description_keeps_accumulators_adjusters_and_appended_effect_order() {
        content_reader_script::test_game::enter();
        let mut catalog = AbilitySprites::default();
        let terrains = BTreeMap::from([(b"synthetic".to_vec(), b"SYNTHETIC".to_vec())]);
        apply_with_terrain(
            br#"ability={ a={
                name=FIRST name=LAST "name"=IGNORED desc=BODY
                country_modifiers={political_power_gain=0.1}
                country_modifiers={political_power_gain=0.2 custom_modifier_tooltip=TIP}
                leader_modifiers={attack_bonus_against=0.3}
                unit_modifiers={synthetic={attack=0.4 attack=bad defence=0.5} movement_speed=0.6}
                one_time_effect={custom_effect_tooltip=FIRST}
                one_time_effect={custom_effect_tooltip=SECOND}
                hourly_effect={custom_effect_tooltip=HOURLY}
                daily_effect={custom_effect_tooltip=DAILY}
                end_effect={custom_effect_tooltip=END}
            }}"#,
            &mut catalog,
            &terrains,
            &KeyNames::default(),
        )
        .unwrap();
        let definition = catalog.definition(b"a").unwrap();
        assert_eq!(definition.name, b"LAST");
        assert_eq!(definition.description, b"BODY");
        assert_eq!(definition.modifiers[0].values[0].value, 20_000);
        assert_eq!(definition.modifiers[0].tooltips(), ["TIP"]);
        assert_eq!(definition.modifiers[1].values[0].value, 30_000);
        assert_eq!(definition.modifiers[2].values[0].value, 60_000);
        assert_eq!(
            definition.adjusters,
            [crate::unit_leader::Adjuster {
                key: "SYNTHETIC".into(),
                attack: 40_000,
                defence: 50_000,
                movement: 0,
            }]
        );
        assert_eq!(definition.effects[0].len(), 2);
        assert_eq!(
            definition.effects[0][0].value.read_string(),
            Some(b"FIRST".as_slice())
        );
        assert_eq!(
            definition.effects[0][1].value.read_string(),
            Some(b"SECOND".as_slice())
        );
        assert_eq!(
            definition.effects[1][0].value.read_string(),
            Some(b"HOURLY".as_slice())
        );
        assert_eq!(
            definition.effects[2][0].value.read_string(),
            Some(b"DAILY".as_slice())
        );
    }

    #[test]
    fn description_defaults_and_duplicate_lookup_preserve_exact_case() {
        content_reader_script::test_game::enter();
        let mut catalog = AbilitySprites::default();
        apply(
            b"ability={a={name=FIRST} a={name=LATER} A={name=UPPER}}",
            &mut catalog,
        )
        .unwrap();
        assert_eq!(catalog.definition(b"a").unwrap().name, b"FIRST");
        assert_eq!(catalog.definition(b"a").unwrap().description, b"a_desc");
        assert_eq!(catalog.definition(b"A").unwrap().name, b"UPPER");
    }
}
