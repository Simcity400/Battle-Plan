//! Name projections of the three native doctrine databases.
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
    // Tracks 1409D9E60, grand 1409C1300, sub 1409D9920; shared base
    // 1409BF8C0 and 141517770. Names consume exactly their native reader.
    let read = native::read(
        bytes, native::Mode::KeepMark, native::Top::File(Node::File), Some(names),
        &mut |node, field| match (node, field.keyword().as_deref()) {
            (Node::File, _) => Shape::Object(Node::Definition),
            (Node::Definition, Some(b"name")) if kind == 2 =>
                crate::effect_keys::bound_localization_shape(field).map(|()| Node::Definition),
            (Node::Definition, Some(b"name" | b"icon")) => Shape::One,
            (Node::Definition, Some(b"xp_type" | b"xp_cost")) if kind != 2 => Shape::One,
            (Node::Definition, Some(b"folder" | b"max_track_rows" | b"max_track_columns")) if kind == 0 => Shape::One,
            (Node::Definition, Some(b"progress_type" | b"background" | b"icon_frame" | b"background_offset")) if kind == 2 => Shape::One,
            (Node::Definition, Some(b"reward_gfx" | b"allow_in_multiple_tracks")) if kind == 1 => Shape::One,
            (Node::Definition, Some(b"description")) if kind != 2 =>
                crate::effect_keys::bound_localization_shape(field).map(|()| Node::Definition),
            (Node::Definition, Some(b"active")) if kind == 2 => Shape::Trigger,
            (Node::Definition, Some(b"visible" | b"available")) if kind != 2 => Shape::Trigger,
            (Node::Definition, Some(b"ai_will_do")) if kind != 2 => Shape::Object(Node::Mtth),
            (Node::Mtth, _) => native::mean_time_to_happen(field),
            (Node::Definition, Some(b"mastery")) if kind != 0 => Shape::Object(Node::Mastery),
            // 1409D8450 copies each raw track token into the reader's value slot.
            (Node::Definition, Some(b"track")) if kind == 1 && field.opens() => Shape::ValueList,
            (Node::Definition, Some(b"track")) if kind == 1 => Shape::One,
            (Node::Definition, Some(b"enable_tactic")) if kind != 2 => Shape::One,
            (Node::Reward | Node::Milestone, Some(b"enable_tactic")) => Shape::One,
            // ReadDynamicTokenIdList 1403B6DD0 retains the enclosing value slot.
            (Node::Definition, Some(b"xor")) if kind == 1 => Shape::RawList,
            (Node::Definition, Some(b"rewards")) if kind == 1 => Shape::Object(Node::Rewards),
            (Node::Rewards, _) => Shape::Object(Node::Reward),
            // Reward 1409D83F0 and milestone 1409BFC00 have one scalar each.
            (Node::Reward, Some(b"mastery")) => Shape::One,
            (Node::Definition, Some(b"milestones")) if kind == 0 => Shape::ObjectList(Node::Milestone),
            (Node::Milestone, Some(b"required_progress")) => Shape::One,
            // 1409C1300 -> 1409BFE50: statements reader, not a raw token list.
            (Node::Definition, Some(b"tracks")) if kind == 0 => Shape::Statements(Node::TrackNames),
            (Node::TrackNames, _) if field.fresh || field.opens() => Shape::Skip,
            (Node::TrackNames, _) => Shape::One,
            (Node::Mastery, Some(b"multiplier")) => Shape::One,
            // 1409D62C0/1409D66E0/1409D5EA0 write the current entry into +0xC0.
            (Node::Mastery, Some(b"categories" | b"equipment" | b"sub_units")) => Shape::ValueList,
            (Node::Definition, Some(b"effect")) if kind != 2 => Shape::Object(Node::Effect),
            (Node::Reward | Node::Milestone, Some(b"effect")) => Shape::Object(Node::Effect),
            (Node::Effect, _) => crate::effect_keys::shape(Some(names), field).map(|()| Node::Effect),
            (Node::Definition, Some(b"modifier")) if kind != 2 => Shape::Object(Node::Modifier(modifiers::Node::Modifier)),
            (Node::Reward | Node::Milestone, Some(b"modifier")) => Shape::Object(Node::Modifier(modifiers::Node::Modifier)),
            (Node::Modifier(node), _) => modifiers::shape(node, field, &|key| modifiers::registered(key, Some(names)), &mut Vec::new()).map(Node::Modifier),
            _ => Shape::Skip,
        },
    );
    database.notes.extend(read.misreads.iter().map(|note| format!("{source}: {note}")));
    if let Some(why) = read.unsettled {
        database.unread = Some(format!("{source}: doctrine names unread: {why}"));
    }
    let destination = match kind { 0 => &mut database.grand, 1 => &mut database.sub, _ => &mut database.tracks };
    for item in &read.items {
        // 14012F900/1401310E0/140131500 reuse repeated token objects;
        // omitted fields retain the previous value, constructor names are empty.
        let name = destination.entry(Token::c_str(item.key.as_deref().unwrap_or_default())).or_default();
        for field in item.value.items().unwrap_or_default().iter().filter(|field| field.keyword_is("name")) {
            set_name(name, field, kind == 2, source);
        }
    }
}

fn set_name(name: &mut Name, field: &Item, bound: bool, source: &str) {
    if let Some(value) = field.value.scalar() {
        name.value = parser::c_string(value).to_vec();
        name.refusal = if bound && value.contains(&b'|') || bound && value.windows(6).any(|part| part == b"token:") {
            Some(format!("{source}: track name requires the core bound-localisation formatter (14051E570)"))
        } else { None };
    } else {
        name.refusal = Some(format!("{source}: track bound name requires the core bound-localisation formatter (14051E570)"));
    }
}

pub(super) fn read(layers: &ResolvedLayers, names: &KeyNames, database: &mut Database) -> io::Result<()> {
    // 140188440/14018B1A0 enumerate direct files. 140193720 loads immediate
    // subdirectories first (140193540), then root; never arbitrary recursion.
    for (directory, kind) in [("common/doctrines/grand_doctrines", 0), ("common/doctrines/tracks", 2)] {
        for path in layers.directory_files(directory, ".txt")? {
            append(database, &layers.read(&path)?, &path, kind, names);
        }
    }
    let root = "common/doctrines/subdoctrines";
    let children: BTreeSet<_> = layers.descriptor_paths(root, &[".txt"])?
        .into_iter().filter_map(|path| path.strip_prefix(&format!("{root}/")).and_then(|tail| tail.split_once('/').map(|(directory, _)| directory.to_owned()))).collect();
    for directory in children.into_iter().map(|child| format!("{root}/{child}")).chain(std::iter::once(root.to_owned())) {
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
        let mut db = Database::default();
        for (kind, definitions) in [(0, "grand={name=grand_name} GRAND={icon=unused}"), (1, "sub={name=sub_name}"), (2, "track={name=track_name}")] {
            append(&mut db, definitions.as_bytes(), "synthetic", kind, &KeyNames::default());
        }
        assert_eq!(db.grand[&Token::c_str(b"grand")].value, b"grand_name");
        assert_eq!(db.sub[&Token::c_str(b"sub")].value, b"sub_name");
        assert_eq!(db.tracks[&Token::c_str(b"track")].value, b"track_name");
    }

    #[test]
    fn catalogue_specific_readers_keep_names_after_nested_fields() {
        let mut db = Database::default();
        for (kind, definitions) in [
            (0, "grand={tracks={unused={name=ignored}} milestones={{name={name=ignored} required_progress=0.5}} name=grand_name}"),
            (1, "sub={track={unused} rewards={reward={name={name=ignored} mastery=1}} name=sub_name}"),
            (2, "track={xp_type={name=ignored} mastery={categories={unused}} name=track_name}"),
        ] {
            append(&mut db, definitions.as_bytes(), "synthetic", kind, &KeyNames::default());
        }
        assert_eq!(db.grand[&Token::c_str(b"grand")].value, b"grand_name");
        assert_eq!(db.sub[&Token::c_str(b"sub")].value, b"sub_name");
        assert_eq!(db.tracks[&Token::c_str(b"track")].value, b"track_name");
    }
}
