use content_reader_key_names::key_name_products::KeyNamesExt as _;
use content_reader_layers::{ResolvedLayers, script_files};
use content_reader_parser::{fields::invalid, fixed5::decimal_i32};
use content_reader_script::{key_names::KeyNames, modifiers, native::{self, Shape}};
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Law {
    pub token: String,
    pub sprite: String,
    pub frame: i32,
    pub visible: Vec<content_reader_parser::parser::Item>,
    pub available: Vec<content_reader_parser::parser::Item>,
    pub tooltip: String,
    pub state_modifier: Vec<content_reader_parser::parser::Item>,
    pub suppressed_state_modifier: Option<Vec<content_reader_parser::parser::Item>>,
}

const DEFAULT_SPRITE: &str = "GFX_occupation_policy_icon_strip";

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
            tooltip: String::new(),
            state_modifier: Vec::new(),
            suppressed_state_modifier: None,
        };
        for field in item.value.items().unwrap_or_default() {
            if field.keyword_is("state_modifier") {
                law.state_modifier
                    .extend_from_slice(field.value.items().unwrap_or_default());
                continue;
            }
            if field.keyword_is("suppressed_state_modifier") {
                law.suppressed_state_modifier
                    .get_or_insert_with(Vec::new)
                    .extend_from_slice(field.value.items().unwrap_or_default());
                continue;
            }
            if field.keyword_is("tooltip") {
                law.tooltip = std::str::from_utf8(content_reader_parser::parser::c_string(field.value.scalar().unwrap_or_default()))
                    .map_err(|e| invalid(format!("{} tooltip: {e}", law.token)))?
                    .into();
                continue;
            }
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
            let bytes = content_reader_parser::parser::c_string(field.value.scalar().unwrap_or_default());
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
    let names = layers.key_names()?;
    for path in script_files::paths(layers, "common/occupation_laws/", Some(".txt"))? {
        match parse(&path.read(layers)?, &names) {
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
        content_reader_script::test_game::enter();
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
        content_reader_script::test_game::enter();
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
    fn the_tooltip_key_and_both_modifier_blocks_are_kept() {
        content_reader_script::test_game::enter();
        let laws = parse(
            b"strict={tooltip=first tooltip=strict_tt state_modifier={compliance_gain=0.1} state_modifier={resistance_growth=-0.2} suppressed_state_modifier={local_manpower=0.5}} plain={}",
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(laws[0].tooltip, "strict_tt");
        assert_eq!(laws[0].state_modifier.len(), 2);
        assert!(laws[0].state_modifier[1].keyword_is("resistance_growth"));
        assert_eq!(laws[0].suppressed_state_modifier.as_ref().map(Vec::len), Some(1));
        assert_eq!(laws[1].tooltip, "");
        assert!(laws[1].state_modifier.is_empty());
        assert_eq!(laws[1].suppressed_state_modifier, None);
    }

    #[test]
    fn unsettled_modifier_is_named_not_assumed() {
        content_reader_script::test_game::enter();
        let error = parse(
            b"extra={state_modifier={modded={a=1}}}",
            &KeyNames::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("modded") && error.contains("line 1"));
    }

}
