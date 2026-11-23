use content_reader_key_names::key_name_products::KeyNamesExt as _;
use content_reader_layers::ResolvedLayers;
use content_reader_parser::{fields::invalid, fixed5::decimal_i32, parser::{self, Item}};
use content_reader_script::{key_names::KeyNames, modifiers, native::{self, Shape}, scope::ScopeKind};
use std::{collections::BTreeMap, io};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    CoreResistance = 0,
    StateResistance = 1,
    CoreCompliance = 2,
    StateCompliance = 3,
    #[default]
    Invalid = 4,
}

impl Kind {
    pub fn is_core(self) -> bool {
        matches!(self, Self::CoreResistance | Self::CoreCompliance)
    }

    fn read(value: &[u8]) -> Option<Self> {
        [
            (b"core_resistance_modifier".as_slice(), Self::CoreResistance),
            (
                b"state_resistance_modifier".as_slice(),
                Self::StateResistance,
            ),
            (b"core_compliance_modifier".as_slice(), Self::CoreCompliance),
            (
                b"state_compliance_modifier".as_slice(),
                Self::StateCompliance,
            ),
        ]
        .into_iter()
        .find_map(|(name, kind)| name.eq_ignore_ascii_case(value).then_some(kind))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    pub token: String,
    pub kind: Kind,
    pub threshold: i64,
    pub margin: i64,
    pub sprite: String,
    pub frame: i32,
    pub tooltip: String,
    pub state_modifier: Vec<Item>,
    pub on_enable: Vec<Item>,
    pub on_disable: Vec<Item>,
    pub visible: Vec<Item>,
    pub enabled: Vec<Item>,
}

impl Default for Definition {
    fn default() -> Self {
        Self {
            token: String::new(),
            kind: Kind::Invalid,
            threshold: 0,
            margin: 0,
            sprite: "GFX_occupation_modifier_strip".into(),
            frame: 1,
            tooltip: String::new(),
            state_modifier: Vec::new(),
            on_enable: Vec::new(),
            on_disable: Vec::new(),
            visible: Vec::new(),
            enabled: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Definitions {
    pub definitions: BTreeMap<String, Definition>,
    pub notes: Vec<String>,
    pub refusals: Vec<String>,
}

impl Definitions {
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let names = layers.key_names()?;
        let mut out = Self::default();
        for path in layers.directory_files("common/resistance_compliance_modifiers", ".txt")? {
            match parse_with_notes(&path.read(layers)?, &names) {
                Ok((definitions, notes)) => {
                    out.notes
                        .extend(notes.into_iter().map(|why| format!("{path}: {why}")));
                    for definition in definitions {
                        let token = definition.token.clone();
                        if out.definitions.insert(token.clone(), definition).is_some() {
                            out.notes.push(format!("{path}: duplicate occupation modifier {token} replaced (1 definition)"));
                        }
                    }
                }
                Err(why) => {
                    out.definitions.clear();
                    out.refusals.push(format!("{path}: {why} (1 refused file)"));
                }
            }
        }
        Ok(out)
    }

    pub fn find(&self, token: &str) -> Result<Option<&Definition>, String> {
        if let Some(definition) = self.definitions.get(token) {
            return Ok(Some(definition));
        }
        if !self.refusals.is_empty() {
            return Err(format!(
                "occupation modifier {token}: {} refused files: {}",
                self.refusals.len(),
                self.refusals.join("; ")
            ));
        }
        Ok(None)
    }
}

pub fn read(layers: &ResolvedLayers) -> io::Result<Definitions> {
    Definitions::read(layers)
}

pub fn parse(input: &[u8], names: &KeyNames) -> io::Result<Vec<Definition>> {
    parse_with_notes(input, names).map(|(definitions, _)| definitions)
}

fn parse_with_notes(input: &[u8], names: &KeyNames) -> io::Result<(Vec<Definition>, Vec<String>)> {
    #[derive(Clone, Copy)]
    enum Node {
        File,
        Definition,
        Modifier(modifiers::Node),
    }
    let mut unsettled = Vec::new();
    let registry = |key: &[u8]| modifiers::registered(key, Some(names));
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(Node::File),
        Some(names),
        &mut |node, field| match node {
            Node::File => Shape::Object(Node::Definition),
            Node::Definition => match field.keyword().as_deref() {
                Some(
                    b"type" | b"tooltip" | b"icon" | b"small_icon" | b"threshold" | b"margin"
                    | b"is_dynamic" | b"alert_level" | b"alert_margin",
                ) => Shape::One,
                Some(b"visible" | b"enabled") => Shape::Trigger,
                Some(b"on_enable" | b"on_disable") => {
                    Shape::EffectIn(ScopeKind::Country.bit() | ScopeKind::State.bit())
                }
                Some(b"state_modifier") => Shape::Object(Node::Modifier(modifiers::Node::Modifier)),
                _ => Shape::Skip,
            },
            Node::Modifier(node) => {
                modifiers::shape(node, field, &registry, &mut unsettled).map(Node::Modifier)
            }
        },
    );
    if let Some(why) = read.unsettled {
        return Err(invalid(format!("occupation modifiers: {why}")));
    }
    if !unsettled.is_empty() {
        return Err(invalid(format!(
            "occupation modifiers: {} unsupported forms: {}",
            unsettled.len(),
            unsettled
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        )));
    }
    let mut notes: Vec<_> = read.misreads.iter().map(ToString::to_string).collect();
    let mut definitions = Vec::new();
    for entry in read.items {
        let mut definition = Definition {
            token: parser::text(parser::c_string(entry.key.as_deref().unwrap_or_default()))
                .map_err(|why| invalid(format!("occupation modifier name: {why}")))?
                .into(),
            ..Default::default()
        };
        for field in entry.value.items().unwrap_or_default() {
            let bytes = parser::c_string(field.value.scalar().unwrap_or_default());
            match field.keyword().map(|keyword| keyword.folded()).as_deref() {
                Some(b"type") => {
                    if field.kind == native::Kind::Bare
                        && let Some(kind) = Kind::read(bytes)
                    {
                        definition.kind = kind;
                    }
                }
                Some(b"threshold" | b"margin") => {
                    if let Some(value) = field.value.scalar().and_then(content_reader_parser::fixed5::script_fixed5)
                    {
                        if field.keyword_is("threshold") {
                            definition.threshold = value;
                        } else {
                            definition.margin = value;
                        }
                    }
                }
                Some(b"tooltip") => {
                    definition.tooltip = parser::text(parser::read_string(bytes))
                        .map_err(|why| invalid(format!("{} tooltip: {why}", definition.token)))?
                        .into()
                }
                Some(b"icon") => read_icon(bytes, &mut definition)?,
                Some(
                    b"state_modifier" | b"on_enable" | b"on_disable" | b"visible" | b"enabled",
                ) => {
                    let destination = if field.keyword_is("state_modifier") {
                        &mut definition.state_modifier
                    } else if field.keyword_is("on_enable") {
                        &mut definition.on_enable
                    } else if field.keyword_is("on_disable") {
                        &mut definition.on_disable
                    } else if field.keyword_is("visible") {
                        &mut definition.visible
                    } else {
                        &mut definition.enabled
                    };
                    destination.extend_from_slice(field.value.items().unwrap_or_default());
                }
                _ => {}
            }
        }
        if definition.kind == Kind::Invalid {
            notes.push(format!(
                "occupation modifier {} has invalid type and is not registered (1 definition)",
                definition.token
            ));
        } else {
            definitions.push(definition);
        }
    }
    Ok((definitions, notes))
}

fn read_icon(bytes: &[u8], definition: &mut Definition) -> io::Result<()> {
    if !bytes.is_empty() && bytes.iter().all(u8::is_ascii_digit) {
        definition.frame = decimal_i32(bytes);
    } else if let Some(colon) = bytes.iter().position(|byte| *byte == b':') {
        definition.sprite = parser::text(&bytes[..colon])
            .map_err(|why| invalid(format!("{} icon: {why}", definition.token)))?
            .into();
        definition.frame = decimal_i32(&bytes[colon + 1..]);
    } else {
        definition.sprite = parser::text(bytes)
            .map_err(|why| invalid(format!("{} icon: {why}", definition.token)))?
            .into();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn initialize() {
        content_reader_script::test_game::enter();
    }

    #[test]
    fn occupation_definitions_keep_native_kinds_defaults_and_authored_bodies() {
        initialize();
        let definitions = parse(br#"@frame=7 mod_state={type=state_resistance_modifier icon="custom:3" icon=@frame threshold=31.99 margin=2 tooltip=MOD_TIP state_modifier={custom_modifier_tooltip=MOD_RAW} on_disable={custom_effect_tooltip=MOD_DISABLE} on_enable={custom_effect_tooltip=MOD_ENABLE} visible={always=yes} enabled={always=no}} core={type=core_resistance_modifier} compliance={type=state_compliance_modifier} core_compliance={type=core_compliance_modifier} invalid={threshold=1}"#, &KeyNames::default()).unwrap();
        assert_eq!(definitions.len(), 4);
        let state = &definitions[0];
        assert_eq!(
            (state.kind, state.sprite.as_str(), state.frame),
            (Kind::StateResistance, "custom", 7)
        );
        assert_eq!(
            (state.threshold, state.margin, state.tooltip.as_str()),
            (3_199_000, 200_000, "MOD_TIP")
        );
        for body in [
            &state.state_modifier,
            &state.on_disable,
            &state.on_enable,
            &state.visible,
            &state.enabled,
        ] {
            assert_eq!(body.len(), 1);
        }
        assert_eq!(definitions[1].kind as u8, 0);
        assert_eq!(definitions[2].kind as u8, 3);
        assert_eq!(definitions[3].kind as u8, 2);
        assert_eq!(
            (
                definitions[1].sprite.as_str(),
                definitions[1].frame,
                definitions[1].threshold
            ),
            ("GFX_occupation_modifier_strip", 1, 0)
        );
    }

    #[test]
    fn occupation_definitions_use_stack_replacement_and_exact_name_identity() {
        initialize();
        let stack = content_reader_layers::test_support::Stack::new("occupation_definitions");
        stack.write(
            &stack.base(),
            "common/resistance_compliance_modifiers/a.txt",
            "mod={type=state_resistance_modifier threshold=1}",
        );
        stack.write(
            &stack.addon(),
            "common/resistance_compliance_modifiers/a.txt",
            "mod={type=core_resistance_modifier threshold=2}",
        );
        stack.write(
            &stack.addon(),
            "common/resistance_compliance_modifiers/b.txt",
            "mod={type=state_compliance_modifier threshold=3} MOD={type=core_compliance_modifier}",
        );
        stack.write(
            &stack.addon(),
            "common/resistance_compliance_modifiers/nested/ignored.txt",
            "mod={type=core_compliance_modifier threshold=99}",
        );
        let definitions = read(&stack.layers(&[])).unwrap();
        assert_eq!(definitions.find("mod").unwrap().unwrap().threshold, 300_000);
        assert_eq!(
            definitions.find("MOD").unwrap().unwrap().kind,
            Kind::CoreCompliance
        );
        assert!(definitions.find("Mod").unwrap().is_none());
        assert_eq!(definitions.notes.len(), 1);
        assert_eq!(
            read(&stack.layers(&["common/resistance_compliance_modifiers"]))
                .unwrap()
                .definitions
                .len(),
            2
        );
    }

    #[test]
    fn occupation_unreadable_definition_refuses_lookup_by_name_and_count() {
        initialize();
        let stack = content_reader_layers::test_support::Stack::new("occupation_refused");
        stack.write(
            &stack.base(),
            "common/resistance_compliance_modifiers/a.txt",
            "",
        );
        std::fs::write(
            stack
                .base()
                .join("common/resistance_compliance_modifiers/a.txt"),
            b"mod={type=state_resistance_modifier icon=\"\xfe\"}",
        )
        .unwrap();
        let definitions = read(&stack.layers(&[])).unwrap();
        let why = definitions.find("mod").unwrap_err();
        assert!(why.contains("mod icon") && why.contains("1 refused files"));
        assert!(definitions.definitions.is_empty());
    }

    #[test]
    fn occupation_unsettled_modifier_names_each_unsupported_form() {
        initialize();
        let mut names = KeyNames::default();
        names.effects.modifiers_gap = Some("synthetic unread registry".into());
        let why = parse(
            b"mod={type=state_resistance_modifier state_modifier={mod_unsettled={a=1}}}",
            &names,
        )
        .unwrap_err()
        .to_string();
        assert!(why.contains("mod_unsettled") && why.contains("1 unsupported forms"));
        assert!(
            parse(
                b"mod={type=state_resistance_modifier state_modifier={mod_unsettled={a=1}}}",
                &KeyNames::default()
            )
            .is_ok()
        );
    }

    #[test]
    fn occupation_later_definition_establishes_identity_after_an_unread_file() {
        initialize();
        let stack = content_reader_layers::test_support::Stack::new("occupation_reestablished");
        stack.write(
            &stack.base(),
            "common/resistance_compliance_modifiers/a.txt",
            "before={type=state_resistance_modifier}",
        );
        stack.write(
            &stack.base(),
            "common/resistance_compliance_modifiers/b.txt",
            "",
        );
        std::fs::write(
            stack
                .base()
                .join("common/resistance_compliance_modifiers/b.txt"),
            b"bad={type=state_resistance_modifier icon=\"\xfe\"}",
        )
        .unwrap();
        stack.write(
            &stack.base(),
            "common/resistance_compliance_modifiers/c.txt",
            "after={type=core_compliance_modifier}",
        );
        let definitions = read(&stack.layers(&[])).unwrap();
        assert_eq!(
            definitions.find("after").unwrap().unwrap().kind,
            Kind::CoreCompliance
        );
        assert!(
            definitions
                .find("before")
                .unwrap_err()
                .contains("1 refused files")
        );
    }

    #[test]
    fn occupation_type_requires_a_keyword_and_failed_numbers_retain_previous_values() {
        initialize();
        let definitions = parse(br#"quoted={type="state_resistance_modifier"} kept={type=state_resistance_modifier type=bad threshold=8 threshold=no margin=3 margin=no}"#, &KeyNames::default()).unwrap();
        assert_eq!(definitions.len(), 1);
        assert_eq!(
            (
                definitions[0].token.as_str(),
                definitions[0].kind,
                definitions[0].threshold,
                definitions[0].margin
            ),
            ("kept", Kind::StateResistance, 800_000, 300_000)
        );
    }
}
