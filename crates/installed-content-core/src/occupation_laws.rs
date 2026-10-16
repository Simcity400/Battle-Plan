//! Occupation-law names and sprite/frame pairs from the recorded content stack.
use crate::{
    ResolvedLayers,
    fields::invalid,
    fixed5::decimal_i32,
    key_names::KeyNames,
    modifiers,
    parser::native::{self, Shape},
    script_files,
};
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Law {
    pub token: String,
    pub sprite: String,
    pub frame: i32,
    /// Visibility and availability triggers are stored separately.
    pub visible: Vec<crate::parser::Item>,
    pub available: Vec<crate::parser::Item>,
}

/// Default occupation-law sprite name.
const DEFAULT_SPRITE: &str = "GFX_occupation_policy_icon_strip";

/// The sprite/frame pair reads one token, not an icon object. Digits
/// change only the frame; `sprite:frame` changes both, and a sprite alone
/// retains the frame. A frame token must contain only digits.
pub fn parse(input: &[u8], names: &KeyNames) -> io::Result<Vec<Law>> {
    #[derive(Clone, Copy)]
    enum Node {
        File,
        Law,
        Mtth,
        Modifier(modifiers::Node),
    }
    let mut undecided = Vec::new();
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(Node::File),
        Some(names),
        &mut |node, field| match node {
            Node::File => Shape::Object(Node::Law),
            Node::Law => match field.keyword().as_deref() {
                Some(
                    b"icon"
                    | b"tooltip"
                    | b"sound_effect"
                    | b"missing_garrison_law"
                    | b"starting_law"
                    | b"gui_order"
                    | b"fallback_law"
                    | b"default_law",
                ) => Shape::One,
                Some(b"visible" | b"available") => Shape::Trigger,
                Some(b"ai_will_do") => Shape::Object(Node::Mtth),
                Some(b"state_modifier" | b"suppressed_state_modifier") => {
                    Shape::Object(Node::Modifier(modifiers::Node::Modifier))
                }
                _ => Shape::Skip,
            },
            Node::Mtth => native::mean_time_to_happen(field),
            Node::Modifier(node) => {
                modifiers::shape(node, field, modifiers::UNKNOWN, &mut undecided)
                    .map(Node::Modifier)
            }
        },
    );
    if let Some(cause) = read.unsettled {
        return Err(invalid(format!("occupation laws: {cause}")));
    }
    if let Some(cause) = undecided.first() {
        return Err(invalid(format!("occupation laws: {cause}")));
    }
    let mut laws = Vec::new();
    for item in read.items {
        let token = item
            .key_str()
            .map_err(|e| invalid(format!("occupation law name: {e}")))?
            .to_owned();
        let mut law = Law {
            token,
            sprite: DEFAULT_SPRITE.into(),
            frame: 1,
            visible: Vec::new(),
            available: Vec::new(),
        };
        for field in item.value.items().unwrap_or_default() {
            if field.keyword_is("visible") {
                law.visible
                    .extend_from_slice(field.value.items().unwrap_or_default());
                continue;
            }
            if field.keyword_is("available") {
                law.available
                    .extend_from_slice(field.value.items().unwrap_or_default());
                continue;
            }
            if !field.keyword_is("icon") {
                continue;
            }
            let bytes = crate::parser::c_string(field.value.scalar().unwrap_or_default());
            if !bytes.is_empty() && bytes.iter().all(u8::is_ascii_digit) {
                law.frame = decimal_i32(bytes);
            } else if let Some(colon) = bytes.iter().position(|b| *b == b':') {
                law.sprite = std::str::from_utf8(&bytes[..colon])
                    .map_err(|e| invalid(format!("{} icon: {e}", law.token)))?
                    .into();
                law.frame = decimal_i32(&bytes[colon + 1..]);
            } else {
                law.sprite = std::str::from_utf8(bytes)
                    .map_err(|e| invalid(format!("{} icon: {e}", law.token)))?
                    .into();
            }
        }
        laws.push(law);
    }
    Ok(laws)
}

#[derive(Default)]
pub struct Laws {
    pub definitions: BTreeMap<String, Law>,
    /// An unread tail can replace any earlier law. Refuse lookups rather
    /// than presenting an earlier definition as the final one.
    pub warnings: Vec<String>,
}

impl Laws {
    pub fn find(&self, token: &str) -> Result<Option<&Law>, String> {
        if self.warnings.is_empty() {
            Ok(self.definitions.get(token))
        } else {
            Err(format!(
                "{token}: occupation-law definitions uncertain: {}",
                self.warnings.join("; ")
            ))
        }
    }
}

pub fn read(layers: &ResolvedLayers) -> io::Result<Laws> {
    let mut laws = Laws::default();
    // Direct files in file order; the last declaration replaces the previous
    // object of the same token.
    let names = layers.key_names()?;
    for path in script_files::paths(layers, "common/occupation_laws/", Some(".txt")) {
        match parse(&layers.read(&path)?, &names) {
            Ok(items) => {
                for law in items {
                    laws.definitions.insert(law.token.clone(), law);
                }
            }
            Err(error) => laws
                .warnings
                .push(format!("Occupation laws unavailable in {path}: {error}")),
        }
    }
    Ok(laws)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sprite_pair_and_last_definition() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let laws = parse(br#"@frame=4 custom={icon="mod_sprite:3" icon=@frame icon=other visible={always=yes} ai_will_do={base=1 modifier={factor=2 always=yes}} state_modifier={compliance_gain=0.1}} custom={icon="new:2"}"#, &KeyNames::default()).unwrap();
        assert_eq!((laws[0].sprite.as_str(), laws[0].frame), ("other", 4));
        assert_eq!((laws[1].sprite.as_str(), laws[1].frame), ("new", 2));
        let numeric = parse(
            b"law={icon=001} quoted={icon=\"9\"} signed={icon=-1} invalid={icon=\"sprite:wrong\"}",
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(numeric[0].frame, 1);
        assert_eq!(numeric[1].frame, 9);
        assert_eq!(numeric[2].sprite, "-1");
        assert_eq!(numeric[3].frame, 0);
    }
    #[test]
    fn eligibility_trigger_bodies_are_preserved() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let laws = parse(
            b"custom={visible={always=yes} visible={has_dlc=Example} available={FROM={tag=MOD}}}",
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(laws[0].visible.len(), 2);
        assert!(laws[0].visible[0].keyword_is("always"));
        assert!(laws[0].visible[1].keyword_is("has_dlc"));
        assert_eq!(laws[0].available.len(), 1);
        assert!(laws[0].available[0].keyword_is("FROM"));
    }

    #[test]
    fn unsettled_modifier_is_named_not_assumed() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let error = parse(
            b"extra={state_modifier={modded={a=1}}}",
            &KeyNames::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("modded") && error.contains("line 1"));
    }

    /// Read-only syntax reach audit. Set BATTLE_PLAN_GAME_DIR and BATTLE_PLAN_WORKSHOP_DIR;
    /// every installed folder is inspected, without an assumed mod count.
    #[test]
    #[ignore = "requires the installed vanilla and Workshop corpus"]
    fn installed_corpus() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let game =
            std::path::PathBuf::from(std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).expect("BATTLE_PLAN_GAME_DIR"));
        let workshop = std::path::PathBuf::from(
            std::env::var_os("BATTLE_PLAN_WORKSHOP_DIR").or_else(|| std::env::var_os("HOI4_WORKSHOP_DIR")).expect("BATTLE_PLAN_WORKSHOP_DIR"),
        );
        let mut roots = vec![game];
        roots.extend(
            std::fs::read_dir(workshop)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| path.is_dir()),
        );
        roots.sort();
        let mut files = 0;
        let mut declarations = 0;
        let mut refusals = Vec::new();
        let mut icon_names = std::collections::BTreeSet::new();
        for root in &roots {
            let directory = root.join("common/occupation_laws");
            let Ok(entries) = std::fs::read_dir(directory) else {
                continue;
            };
            let mut paths: Vec<_> = entries
                .map(|entry| entry.unwrap().path())
                .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "txt"))
                .collect();
            paths.sort();
            for path in paths {
                files += 1;
                match parse(&std::fs::read(&path).unwrap(), &KeyNames::default()) {
                    Ok(laws) => {
                        declarations += laws.len();
                        icon_names.extend(laws.iter().map(|law| law.sprite.clone()));
                        println!("{}: {} declarations", path.display(), laws.len());
                    }
                    Err(error) => refusals.push(format!("{}: {error}", path.display())),
                }
            }
        }
        println!(
            "{} roots, {files} files, {declarations} declarations, {} refused files",
            roots.len(),
            refusals.len()
        );
        for refusal in &refusals {
            println!("REFUSED {refusal}");
        }
        assert!(
            refusals.is_empty(),
            "reached content forms must be settled; {} refused files",
            refusals.len()
        );
        fn gfx_files(directory: &std::path::Path, paths: &mut Vec<std::path::PathBuf>) {
            let Ok(entries) = std::fs::read_dir(directory) else {
                return;
            };
            for entry in entries {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    gfx_files(&path, paths);
                } else if path.extension().is_some_and(|ext| ext == "gfx") {
                    paths.push(path);
                }
            }
        }
        let mut icons = 0;
        for root in &roots {
            let mut paths = Vec::new();
            gfx_files(&root.join("interface"), &mut paths);
            paths.sort();
            for path in paths {
                let read =
                    crate::gui::native::sprites::read_sprites(&std::fs::read(&path).unwrap());
                for sprite in read.sprites {
                    let Ok(name) = std::str::from_utf8(&sprite.name) else {
                        continue;
                    };
                    if !icon_names.contains(name) {
                        continue;
                    }
                    icons += 1;
                    println!("ICON {}: {name}: {}", path.display(), sprite.kind.keyword());
                    use crate::gui::native::sprites::{DEFAULT_EFFECT, SpriteKind};
                    let supported = matches!(&sprite.kind, SpriteKind::Plain(plain) if
                        plain.animations.is_empty() && plain.masking_texture.is_empty() && plain.effect() == DEFAULT_EFFECT
                        && matches!(plain.frame_direction, 0 | 1) && plain.stretch == 1.0 && plain.v_scale == 1.0 && !plain.flip_u);
                    if !supported || !sprite.refusals.is_empty() {
                        refusals.push(format!("{}: {name}: {:?}", path.display(), sprite));
                    }
                }
            }
        }
        println!(
            "{icons} occupation-law sprite declarations, {} unsupported forms",
            refusals.len()
        );
        for refusal in &refusals {
            println!("REFUSED {refusal}");
        }
        assert!(
            refusals.is_empty(),
            "reached icon forms must be implemented"
        );
    }
}
