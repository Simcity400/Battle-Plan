//! Name projections of the track, grand-doctrine, and sub-doctrine databases.
use super::*;
use crate::{modifiers, parser::Item};
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
    Effect,
}

fn append(database: &mut Database, bytes: &[u8], source: &str, kind: u8, names: &KeyNames) {
    // Tracks, grand doctrines, and sub-doctrines share base fields while
    // name fields retain their distinct consumption rules.
    let read = native::read(
        bytes,
        native::Mode::KeepMark,
        native::Top::File(Node::File),
        Some(names),
        &mut |node, field| match (node, field.keyword().as_deref()) {
            (Node::File, _) => Shape::Object(Node::Definition),
            (Node::Definition, Some(b"name")) if kind == 2 => {
                crate::effect_keys::bound_localization_shape(field).map(|()| Node::Definition)
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
                crate::effect_keys::bound_localization_shape(field).map(|()| Node::Definition)
            }
            (Node::Definition, Some(b"active")) if kind == 2 => Shape::Trigger,
            (Node::Definition, Some(b"visible" | b"available")) if kind != 2 => Shape::Trigger,
            (Node::Definition, Some(b"ai_will_do")) if kind != 2 => Shape::Object(Node::Mtth),
            (Node::Mtth, _) => native::mean_time_to_happen(field),
            (Node::Definition, Some(b"mastery")) if kind != 0 => Shape::Object(Node::Mastery),
            // Copy each raw track token into the reader's value slot.
            (Node::Definition, Some(b"track")) if kind == 1 && field.opens() => Shape::ValueList,
            (Node::Definition, Some(b"track")) if kind == 1 => Shape::One,
            (Node::Definition, Some(b"enable_tactic")) if kind != 2 => Shape::One,
            (Node::Reward | Node::Milestone, Some(b"enable_tactic")) => Shape::One,
            // Dynamic-token lists retain the enclosing value slot.
            (Node::Definition, Some(b"xor")) if kind == 1 => Shape::RawList,
            (Node::Definition, Some(b"rewards")) if kind == 1 => Shape::Object(Node::Rewards),
            (Node::Rewards, _) => Shape::Object(Node::Reward),
            // Reward and milestone have one scalar each.
            (Node::Reward, Some(b"mastery")) => Shape::One,
            (Node::Definition, Some(b"milestones")) if kind == 0 => {
                Shape::ObjectList(Node::Milestone)
            }
            (Node::Milestone, Some(b"required_progress")) => Shape::One,
            // Entries use statement reads rather than a raw token list.
            (Node::Definition, Some(b"tracks")) if kind == 0 => Shape::Statements(Node::TrackNames),
            (Node::TrackNames, _) if field.fresh || field.opens() => Shape::Skip,
            (Node::TrackNames, _) => Shape::One,
            (Node::Mastery, Some(b"multiplier")) => Shape::One,
            // Each entry updates the current value slot.
            (Node::Mastery, Some(b"categories" | b"equipment" | b"sub_units")) => Shape::ValueList,
            (Node::Definition, Some(b"effect")) if kind != 2 => Shape::Object(Node::Effect),
            (Node::Reward | Node::Milestone, Some(b"effect")) => Shape::Object(Node::Effect),
            (Node::Effect, _) => {
                crate::effect_keys::shape(Some(names), field).map(|()| Node::Effect)
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
            _ => Shape::Skip,
        },
    );
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
        super::sources::append(&mut database.tactic_sources, item, kind);
    }
    let destination = match kind {
        0 => &mut database.grand,
        1 => &mut database.sub,
        _ => &mut database.tracks,
    };
    for item in &read.items {
        // Repeated tokens reuse their objects. Omitted fields retain previous
        // values; constructor names are empty.
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
    // Load tracks before grand and sub-doctrines so track lookup sees the
    // completed registry. Tracks and grand doctrines use direct files;
    // sub-doctrines load immediate subdirectories before root files.
    for (directory, kind) in [
        ("common/doctrines/tracks", 2),
        ("common/doctrines/grand_doctrines", 0),
    ] {
        for path in layers.directory_files(directory, ".txt")? {
            append(database, &layers.read(&path)?, &path, kind, names);
        }
    }
    let root = "common/doctrines/subdoctrines";
    let children: BTreeSet<_> = layers
        .descriptor_paths(root, &[".txt"])?
        .into_iter()
        .filter_map(|path| {
            path.strip_prefix(&format!("{root}/")).and_then(|tail| {
                tail.split_once('/')
                    .map(|(directory, _)| directory.to_owned())
            })
        })
        .collect();
    for directory in children
        .into_iter()
        .map(|child| format!("{root}/{child}"))
        .chain(std::iter::once(root.to_owned()))
    {
        for path in layers.directory_files(&directory, ".txt")? {
            append(database, &layers.read(&path)?, &path, 1, names);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn doctrine_names_merge_by_token_and_preserve_omitted_fields() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    fn tactic_sources_keep_pointer_occurrences_append_lists_and_bound_tracks() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        assert_eq!(grand.available.len(), 1); // Visible is not the source gate.
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
            crate::gui::native::BoundLocalization::from_value(&crate::parser::Value::Scalar(
                b"LAST".to_vec()
            ))
        );
    }

    #[test]
    fn retained_track_name_preserves_nested_binding_and_base_name_string_read() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
