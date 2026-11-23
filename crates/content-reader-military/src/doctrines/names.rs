use super::*;
use content_reader_script::modifiers;
use content_reader_parser::parser::Item;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default)]
pub struct Name {
    pub value: Vec<u8>,
    pub refusal: Option<String>,
}

#[derive(Clone, Copy)]
enum Node {
    File,
    Definition,
    Mastery,
    Rewards,
    Reward,
    Milestone,
    TrackNames,
    Mtth,
    Modifier(modifiers::Node),
    Unit(crate::technology::units::Node),
    Effect,
}

fn append(database: &mut Database, bytes: &[u8], source: &str, kind: u8, names: &KeyNames) {
    let registry = crate::technology::units::Registry {
        units: names
            .sub_units
            .as_ref()
            .map(|units| {
                units
                    .registered()
                    .map(|name| name.to_ascii_lowercase())
                    .collect()
            })
            .unwrap_or_default(),
        categories: names
            .sub_units
            .as_ref()
            .map(|units| units.categories().clone())
            .unwrap_or_default(),
        terrains: names.effects.terrains.clone(),
        equipment: names.effects.equipment.clone(),
    };
    let mut undecided = Vec::new();
    let mut refused = Vec::new();
    let read = native::read(
        bytes,
        native::Mode::KeepMark,
        native::Top::File(Node::File),
        Some(names),
        &mut |node, field| match (node, field.keyword().as_deref()) {
            (Node::File, _) => Shape::Object(Node::Definition),
            (Node::Definition, Some(b"name")) if kind == 2 => {
                content_reader_script::effect_keys::bound_localization_shape(field).map(|()| Node::Definition)
            }
            (Node::Definition, Some(b"name" | b"icon")) => Shape::One,
            (Node::Definition, Some(b"xp_type" | b"xp_cost")) if kind != 2 => Shape::One,
            (Node::Definition, Some(b"folder" | b"max_track_rows" | b"max_track_columns"))
                if kind == 0 =>
            {
                Shape::One
            }
            (
                Node::Definition,
                Some(b"progress_type" | b"background" | b"icon_frame" | b"background_offset"),
            ) if kind == 2 => Shape::One,
            (Node::Definition, Some(b"reward_gfx" | b"allow_in_multiple_tracks")) if kind == 1 => {
                Shape::One
            }
            (Node::Definition, Some(b"description")) if kind != 2 => {
                content_reader_script::effect_keys::bound_localization_shape(field).map(|()| Node::Definition)
            }
            (Node::Definition, Some(b"active")) if kind == 2 => Shape::Trigger,
            (Node::Definition, Some(b"visible" | b"available")) if kind != 2 => Shape::Trigger,
            (Node::Definition, Some(b"ai_will_do")) if kind != 2 => Shape::Object(Node::Mtth),
            (Node::Mtth, _) => native::mean_time_to_happen(field),
            (Node::Definition, Some(b"mastery")) if kind != 0 => Shape::Object(Node::Mastery),
            (Node::Definition, Some(b"track")) if kind == 1 && field.opens() => Shape::ValueList,
            (Node::Definition, Some(b"track")) if kind == 1 => Shape::One,
            (Node::Definition, Some(b"enable_tactic")) if kind != 2 => Shape::One,
            (Node::Reward | Node::Milestone, Some(b"enable_tactic")) => Shape::One,
            (Node::Definition, Some(b"xor")) if kind == 1 => Shape::RawList,
            (Node::Definition, Some(b"rewards")) if kind == 1 => Shape::Object(Node::Rewards),
            (Node::Rewards, _) => Shape::Object(Node::Reward),
            (Node::Reward, Some(b"mastery")) => Shape::One,
            (Node::Definition, Some(b"milestones")) if kind == 0 => {
                Shape::ObjectList(Node::Milestone)
            }
            (Node::Milestone, Some(b"required_progress")) => Shape::One,
            (Node::Definition, Some(b"tracks")) if kind == 0 => Shape::Statements(Node::TrackNames),
            (Node::TrackNames, _) if field.fresh || field.opens() => Shape::Skip,
            (Node::TrackNames, _) => Shape::One,
            (Node::Mastery, Some(b"multiplier")) => Shape::One,
            (Node::Mastery, Some(b"categories" | b"equipment" | b"sub_units")) => Shape::ValueList,
            (Node::Definition, Some(b"effect")) if kind != 2 => Shape::Object(Node::Effect),
            (Node::Reward | Node::Milestone, Some(b"effect")) => Shape::Object(Node::Effect),
            (Node::Effect, _) => {
                content_reader_script::effect_keys::shape(Some(names), field).map(|()| Node::Effect)
            }
            (Node::Definition, Some(b"modifier")) if kind != 2 => {
                Shape::Object(Node::Modifier(modifiers::Node::Modifier))
            }
            (Node::Reward | Node::Milestone, Some(b"modifier")) => {
                Shape::Object(Node::Modifier(modifiers::Node::Modifier))
            }
            (Node::Modifier(node), _) => modifiers::shape(
                node,
                field,
                &|key| modifiers::registered(key, Some(names)),
                &mut Vec::new(),
            )
            .map(Node::Modifier),
            (Node::Definition | Node::Reward | Node::Milestone, Some(key))
                if registry.units.contains(key) || registry.categories.contains(key) =>
            {
                Shape::Object(Node::Unit(crate::technology::units::Node::Stats))
            }
            (Node::Unit(node), _) => registry
                .shape(node, field, &mut undecided, &mut refused)
                .map(Node::Unit),
            (Node::Definition | Node::Reward | Node::Milestone, _) if kind != 2 => {
                modifiers::shape(
                    modifiers::Node::Modifier,
                    field,
                    &|key| modifiers::registered(key, Some(names)),
                    &mut undecided,
                )
                .map(Node::Modifier)
            }
            _ => Shape::Skip,
        },
    );
    database
        .notes
        .extend(undecided.iter().map(|why| format!("{source}: {why}")));
    database
        .notes
        .extend(refused.iter().map(|why| format!("{source}: {why}")));
    database
        .notes
        .extend(read.misreads.iter().map(|note| format!("{source}: {note}")));
    if let Some(why) = read.unsettled {
        database.unread = Some(format!("{source}: doctrine names unread: {why}"));
        let gap = match kind {
            0 => &mut database.tactic_sources.grand_unread,
            1 => &mut database.tactic_sources.sub_unread,
            _ => &mut database.tactic_sources.tracks_unread,
        };
        *gap = Some(format!("{source}: doctrine tactic sources unread: {why}"));
    }
    for item in &read.items {
        super::sources::append(&mut database.tactic_sources, item, kind, names);
    }
    let presentations = match kind {
        0 => &mut database.grand_presentations,
        1 => &mut database.sub_presentations,
        _ => &mut database.track_presentations,
    };
    for item in &read.items {
        let authored = parser::c_string(item.key.as_deref().unwrap_or_default());
        let presentation = presentations
            .entry(Token::c_str(authored))
            .or_insert_with(|| Presentation {
                token: authored.to_vec(),
                ..Default::default()
            });
        for field in item.value.items().unwrap_or_default() {
            if field.keyword_is("icon") {
                if let Some(value) = field.value.read_string() {
                    presentation.icon = value.to_vec();
                }
            } else if kind == 0
                && field.keyword_is("folder")
                && let Some(value) = field.value.scalar()
            {
                presentation.folder = Some(Token::c_str(value));
            }
        }
    }
    let destination = match kind {
        0 => &mut database.grand,
        1 => &mut database.sub,
        _ => &mut database.tracks,
    };
    for item in &read.items {
        let name = destination
            .entry(Token::c_str(item.key.as_deref().unwrap_or_default()))
            .or_default();
        for field in item
            .value
            .items()
            .unwrap_or_default()
            .iter()
            .filter(|field| field.keyword_is("name"))
        {
            set_name(name, field, kind == 2, source);
        }
    }
}

fn set_name(name: &mut Name, field: &Item, bound: bool, source: &str) {
    if let Some(value) = field.value.scalar() {
        name.value = if bound {
            parser::c_string(value)
        } else {
            parser::read_string(value)
        }
        .to_vec();
        name.refusal =
            if bound && (value.contains(&b'|') || value.windows(6).any(|part| part == b"token:")) {
                Some(format!(
                    "{source}: track name requires the core bound-localisation formatter"
                ))
            } else {
                None
            };
    } else {
        name.refusal = Some(format!(
            "{source}: track bound name requires the core bound-localisation formatter"
        ));
    }
}

pub(super) fn read(
    layers: &ResolvedLayers,
    names: &KeyNames,
    database: &mut Database,
) -> io::Result<()> {
    for (directory, kind) in [
        ("common/doctrines/tracks", 2),
        ("common/doctrines/grand_doctrines", 0),
    ] {
        for path in layers.directory_files(directory, ".txt")? {
            append(database, &path.read(layers)?, &path.text(), kind, names);
        }
    }
    let root = "common/doctrines/subdoctrines";
    let children: BTreeSet<Vec<u8>> = layers
        .descriptor_paths(root, &[".txt"])?
        .into_iter()
        .filter_map(|path| {
            let tail = path.as_bytes().strip_prefix(format!("{root}/").as_bytes())?;
            let end = tail.iter().position(|byte| *byte == b'/')?;
            Some(tail[..end].to_vec())
        })
        .collect();
    for directory in children
        .into_iter()
        .map(|child| [format!("{root}/").into_bytes(), child].concat())
        .chain(std::iter::once(root.as_bytes().to_vec()))
    {
        for path in layers.directory_files(&directory, ".txt")? {
            append(database, &path.read(layers)?, &path.text(), 1, names);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn doctrine_names_merge_by_token_and_preserve_omitted_fields() {
        content_reader_script::test_game::enter();
        let mut db = Database::default();
        for (kind, definitions) in [
            (0, "grand={name=grand_name} GRAND={icon=unused}"),
            (1, "sub={name=sub_name}"),
            (2, "track={name=track_name}"),
        ] {
            append(
                &mut db,
                definitions.as_bytes(),
                "synthetic",
                kind,
                &KeyNames::default(),
            );
        }
        assert_eq!(db.grand[&Token::c_str(b"grand")].value, b"grand_name");
        assert_eq!(db.sub[&Token::c_str(b"sub")].value, b"sub_name");
        assert_eq!(db.tracks[&Token::c_str(b"track")].value, b"track_name");
    }

    #[test]
    fn catalogue_specific_readers_keep_names_after_nested_fields() {
        content_reader_script::test_game::enter();
        let mut db = Database::default();
        for (kind, definitions) in [
            (
                0,
                "grand={tracks={unused={name=ignored}} milestones={{name={name=ignored} required_progress=0.5}} name=grand_name}",
            ),
            (
                1,
                "sub={track={unused} rewards={reward={name={name=ignored} mastery=1}} name=sub_name}",
            ),
            (
                2,
                "track={xp_type={name=ignored} mastery={categories={unused}} name=track_name}",
            ),
        ] {
            append(
                &mut db,
                definitions.as_bytes(),
                "synthetic",
                kind,
                &KeyNames::default(),
            );
        }
        assert_eq!(db.grand[&Token::c_str(b"grand")].value, b"grand_name");
        assert_eq!(db.sub[&Token::c_str(b"sub")].value, b"sub_name");
        assert_eq!(db.tracks[&Token::c_str(b"track")].value, b"track_name");
    }

    #[test]
    fn doctrine_rewards_keep_mod_unit_stats_modifiers_effects_and_costs() {
        content_reader_script::test_game::enter();
        let mut units = content_reader_script::sub_units::SubUnits::default();
        units.read(
            b"sub_units={mod_unit={}} sub_unit_categories={mod_category}",
            "synthetic units",
        );
        let names = KeyNames {
            sub_units: Some(units),
            ..Default::default()
        };
        let mut db = Database::default();
        append(&mut db, br#"mod_sub={xp_cost=73 xp_type=army description=MOD_DESC
            modifier={army_org_factor=0.1} effect={add_political_power=2}
            rewards={mod_reward={mastery=1.25 mod_unit={max_organisation=2} mod_category={default_morale=0.05}}}}
            MOD_SUB={rewards={second={mastery=2}}}"#, "synthetic doctrine", 1, &names);
        let definition = &db.tactic_sources.sub[&Token::c_str(b"mod_sub")];
        assert_eq!(definition.xp_cost, 7_300_000);
        assert_eq!(definition.xp_type, b"army");
        assert!(definition.benefits.prose.is_some());
        assert_eq!(definition.benefits.effects.len(), 1);
        assert_eq!(definition.benefits.modifiers.values.len(), 1);
        assert_eq!(definition.rewards.len(), 2);
        assert_eq!(definition.rewards[0].mastery, 125_000);
        assert_eq!(definition.rewards[0].token, b"mod_reward");
        assert_eq!(definition.rewards[0].benefits.units.len(), 2);
    }

    #[test]
    fn tactic_sources_keep_pointer_occurrences_append_lists_and_bound_tracks() {
        content_reader_script::test_game::enter();
        let mut db = Database::default();
        let names = KeyNames::default();
        append(&mut db, br#"a={name={localization_key=TRACK arg={localization_key=INNER n=VALUE}}} b={} A={name=LAST}"#,
            "tracks", 2, &names);
        append(
            &mut db,
            br#"g={name=GRAND available={always=yes} visible={always=no}
            enable_tactic=Exact tracks={a absent a b assigned=a nested={a}}
            milestones={{enable_tactic=Exact enable_tactic=Exact} {enable_tactic=Other} {}}}
            G={enable_tactic=Late tracks={a} milestones={{enable_tactic=Exact}}}"#,
            "grand",
            0,
            &names,
        );
        append(&mut db, br#"s={name=SUB track={a absent a b} enable_tactic=Exact
            rewards={r={mastery=0 enable_tactic=Other} r={mastery=1.25 mastery=bad enable_tactic=Exact}}}
            S={track=b rewards={r={enable_tactic=Exact}}}"#,
            "sub", 1, &names);
        let token = Token::c_str;
        let sources = &db.tactic_sources;
        assert_eq!(sources.grand_order, [token(b"g"), token(b"G")]);
        assert_eq!(sources.sub_order, [token(b"s"), token(b"S")]);
        let grand = &sources.grand[&token(b"g")];
        assert_eq!(
            grand.tracks,
            [token(b"a"), token(b"a"), token(b"b"), token(b"a")]
        );
        assert_eq!(grand.tactics, [b"Exact".to_vec(), b"Late".to_vec()]);
        assert_eq!(grand.available.len(), 1);
        assert_eq!(grand.milestones.len(), 4);
        assert_eq!(grand.milestones[0], [b"Exact".to_vec(), b"Exact".to_vec()]);
        let sub = &sources.sub[&token(b"s")];
        assert_eq!(
            sub.tracks,
            [token(b"a"), token(b"a"), token(b"b"), token(b"b")]
        );
        assert_eq!(sub.rewards.len(), 3);
        assert_eq!(sub.rewards[0].mastery, 0);
        assert_eq!(sub.rewards[1].mastery, 125_000);
        assert_eq!(sub.rewards[2].mastery, 0);
        assert_eq!(
            sources.tracks[&token(b"a")],
            content_reader_script::localization_binding::BoundLocalization::from_value(&content_reader_parser::parser::Value::Scalar(
                b"LAST".to_vec()
            ))
        );
    }

    #[test]
    fn retained_track_name_preserves_nested_binding_and_base_name_string_read() {
        content_reader_script::test_game::enter();
        let mut db = Database::default();
        append(
            &mut db,
            b"t={name={localization_key=TRACK arg={localization_key=INNER n=VALUE}}}",
            "tracks",
            2,
            &KeyNames::default(),
        );
        let bound = &db.tactic_sources.tracks[&Token::c_str(b"t")];
        assert_eq!(bound.nodes.len(), 2);
        assert_eq!(bound.nodes[0].arguments[0].name, b"arg");
        append(
            &mut db,
            b"g={name=\"line\nfeed\"}",
            "grand",
            0,
            &KeyNames::default(),
        );
        assert_eq!(db.grand[&Token::c_str(b"g")].value, b"Unreadable String");
    }
}
