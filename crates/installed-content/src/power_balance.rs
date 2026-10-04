//! Authored balance ranges (140A7F150, 140A7FD10, 140A7FC50, 140A7FC20).
//! Files use lexer mode 0, no recursion; a repeated template is rebuilt.
//! Ranges append, with template ranges before side ranges (140A7F510).

use crate::{
    ResolvedLayers,
    gui::native::FormattedLocalization,
    ideas, modifiers,
    parser::{
        self, Item,
        native::{self, Shape},
    },
};
use std::{collections::BTreeMap, io};

/// CRuleDefinition's executable registry, in order (14062DBC0).
/// Name, localisation key, default (+0x32), fulfilled value (+0x30).
pub const RULES: &[(&str, &str, bool, bool)] = &[
    ("none", "NONE", false, true),
    (
        "can_not_build_buildings",
        "CAN_NOT_BUILD_BUILDINGS",
        false,
        false,
    ),
    ("can_not_declare_war", "CAN_NOT_DECLARE_WAR", false, false),
    (
        "can_declare_war_without_wargoal_when_in_war",
        "CAN_DECLARE_WAR_WITHOUT_WARGOAL_WHEN_IN_WAR",
        false,
        true,
    ),
    (
        "can_declare_war_on_same_ideology",
        "CAN_DECLARE_WAR_ON_SAME_IDEOLOGY",
        true,
        true,
    ),
    ("can_force_government", "CAN_FORCE_GOVERNMENT", true, true),
    (
        "can_guarantee_other_ideologies",
        "CAN_GUARANTEE_OTHER_IDEOLOGIES",
        false,
        true,
    ),
    ("can_send_volunteers", "CAN_SEND_VOLUNTEERS", false, true),
    (
        "can_generate_female_aces",
        "CAN_GENERATE_FAMALE_ACES",
        false,
        true,
    ),
    (
        "can_use_kamikaze_pilots",
        "CAN_USE_KAMIKAZE_PILOTS",
        false,
        true,
    ),
    ("can_lower_tension", "CAN_LOWER_TENSION", false, true),
    (
        "can_join_opposite_factions",
        "CAN_JOIN_OPPOSITE_FACTIONS",
        true,
        true,
    ),
    ("can_create_factions", "CAN_CREATE_FACTIONS", false, true),
    ("can_puppet", "CAN_PUPPET", true, true),
    (
        "can_only_justify_war_on_threat_country",
        "CAN_ONLY_JUSTIFY_WAR_ON_THREAT_COUNTRY",
        false,
        false,
    ),
    (
        "can_boost_other_ideologies",
        "CAN_BOOST_OTHER_IDEOLOGIES",
        false,
        true,
    ),
    ("can_occupy_non_war", "CAN_OCCUPY_NON_WAR", true, true),
    (
        "can_decline_call_to_war",
        "CAN_DECLINE_CALL_TO_WAR",
        true,
        true,
    ),
    (
        "units_deployed_to_overlord",
        "UNITS_DEPLOYED_TO_OVERLORD",
        false,
        false,
    ),
    ("can_join_factions", "CAN_JOIN_FACTIONS", true, true),
    (
        "can_create_collaboration_government",
        "CAN_CREATE_COLLABORATION_GOVERNMENT",
        true,
        true,
    ),
    ("can_be_spymaster", "CAN_BE_SPYMASTER", true, true),
    (
        "contributes_operatives",
        "CONTRIBUTES_OPERATIVES",
        true,
        true,
    ),
    (
        "can_boost_own_ideology",
        "CAN_BOOST_OWN_IDEOLOGY",
        true,
        true,
    ),
    (
        "can_generate_female_unit_leaders",
        "CAN_GENERATE_FEMALE_UNIT_LEADERS",
        false,
        true,
    ),
    (
        "can_generate_female_country_leaders",
        "CAN_GENERATE_FEMALE_COUNTRY_LEADERS",
        false,
        true,
    ),
    ("can_access_market", "CAN_ACCESS_MARKET", true, true),
    (
        "can_use_underway_replenishment",
        "CAN_USE_UNDERWAY_REPLENISHMENT",
        false,
        true,
    ),
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Range {
    pub id: Vec<u8>,
    pub modifiers: modifiers::Modifiers,
    /// One boolean per executable rule, including the defaults (14062BED0).
    pub rules: Vec<bool>,
    pub activation: Vec<Effect>,
    pub deactivation: Vec<Effect>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    Tooltip(FormattedLocalization),
    Group(Vec<Effect>),
    Hidden(Vec<Effect>),
    Unported(Vec<u8>),
}

#[derive(Default)]
pub struct Database {
    balances: BTreeMap<Vec<u8>, Vec<Range>>,
    pub notes: Vec<String>,
}

impl Database {
    /// A recorded id identifies a unique authored range. The active pointer
    /// (+0xB0, 140E43E70) is not recorded, so duplicate ids are ambiguous.
    pub fn range(&self, balance: &[u8], range: &[u8]) -> Result<Option<&Range>, String> {
        let balance = parser::c_string(balance);
        let range = parser::c_string(range);
        let matches: Vec<_> = self
            .balances
            .get(&balance.to_ascii_lowercase())
            .into_iter()
            .flatten()
            .filter(|r| r.id == range)
            .collect();
        match matches.as_slice() {
            [] if range.is_empty() => Ok(None),
            _ if range.is_empty() => Err(format!(
                "balance {}: {} unnamed authored ranges cannot be distinguished from the null range; the recording lacks the active range's pointer or ordinal (+0xB0, 140E43E70)",
                String::from_utf8_lossy(balance),
                matches.len()
            )),
            [found] => Ok(Some(found)),
            [] => Err(format!(
                "balance {}: 1 missing authored range {}",
                String::from_utf8_lossy(balance),
                String::from_utf8_lossy(range)
            )),
            _ => Err(format!(
                "balance {}: {} ranges named {}; the recording lacks the active range's pointer or ordinal (+0xB0, 140E43E70)",
                String::from_utf8_lossy(balance),
                matches.len(),
                String::from_utf8_lossy(range)
            )),
        }
    }
}

#[derive(Clone, Copy)]
enum Node {
    Top,
    Balance,
    Side,
    Range,
    Modifier(modifiers::Node),
    Rule(ideas::RuleNode),
    Effect,
}

fn read_file(input: &[u8], registry: modifiers::Registry<'_>) -> Result<native::Read, String> {
    let mut undecided = Vec::new();
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(Node::Top),
        None,
        &mut |node, field| match node {
            Node::Top => Shape::Object(Node::Balance),
            Node::Balance => match field.keyword().as_deref() {
                Some(b"range") => Shape::Object(Node::Range),
                Some(b"side") => Shape::Object(Node::Side),
                Some(b"left_side" | b"right_side" | b"decision_category") => Shape::Current,
                Some(b"initial_value") => Shape::One,
                Some(b"on_activate" | b"on_deactivate") => Shape::ByKey,
                _ => Shape::Skip,
            },
            Node::Side => match field.keyword().as_deref() {
                Some(b"range") => Shape::Object(Node::Range),
                Some(b"id") => Shape::Current,
                Some(b"icon") => Shape::One,
                Some(b"on_activate" | b"on_deactivate") => Shape::ByKey,
                _ => Shape::Skip,
            },
            // CPowerBalanceRange delegates its progress fields to 140A05820.
            Node::Range => match field.keyword().as_deref() {
                Some(b"id") => Shape::Current,
                Some(b"min" | b"max") => Shape::One,
                Some(b"modifier") => Shape::Object(Node::Modifier(modifiers::Node::Modifier)),
                Some(b"rule") => Shape::Object(Node::Rule(ideas::RuleNode::Rule)),
                Some(b"on_activate" | b"on_deactivate") => Shape::Body(Node::Effect),
                _ => Shape::Skip,
            },
            Node::Modifier(node) => {
                modifiers::shape(node, field, registry, &mut undecided).map(Node::Modifier)
            }
            Node::Rule(node) => ideas::rule_field(node, field).map(Node::Rule),
            // 140534280 requires blocks: 1405306C0 clears the assign flag.
            // 141384F50/142390E40 take tooltip values from the resolved slot.
            Node::Effect => match field.keyword().as_deref() {
                Some(b"effect_tooltip" | b"hidden_effect") => Shape::Body(Node::Effect),
                _ => native::untraced(field),
            },
        },
    );
    if let Some(reason) = undecided
        .first()
        .map(ToString::to_string)
        .or_else(|| read.unsettled.as_ref().map(ToString::to_string))
    {
        return Err(reason);
    }
    Ok(read)
}

fn effects(items: &[Item]) -> Vec<Effect> {
    items
        .iter()
        .map(|item| match item.keyword().map(|k| k.folded()).as_deref() {
            Some(b"custom_effect_tooltip") => match item.value.scalar() {
                Some(text) => Effect::Tooltip(FormattedLocalization::parse(parser::c_string(text))),
                None => Effect::Unported(b"custom_effect_tooltip bound localisation".to_vec()),
            },
            Some(b"effect_tooltip") => {
                Effect::Group(effects(item.value.items().unwrap_or_default()))
            }
            Some(b"hidden_effect") => {
                Effect::Hidden(effects(item.value.items().unwrap_or_default()))
            }
            _ => Effect::Unported(item.key.clone().unwrap_or_default()),
        })
        .collect()
}

fn range(items: &[Item], notes: &mut Vec<String>, registry: modifiers::Registry<'_>) -> Range {
    let mut range = Range {
        rules: RULES.iter().map(|r| r.2).collect(),
        ..Default::default()
    };
    for item in items {
        match item.keyword().map(|k| k.folded()).as_deref() {
            Some(b"id") => {
                range.id = parser::c_string(item.value.scalar().unwrap_or_default()).to_vec()
            }
            Some(b"modifier") => range
                .modifiers
                .read_all(item.value.items().unwrap_or_default()),
            Some(b"on_activate") => range
                .activation
                .extend(effects(item.value.items().unwrap_or_default())),
            Some(b"on_deactivate") => range
                .deactivation
                .extend(effects(item.value.items().unwrap_or_default())),
            Some(b"rule") => {
                for rule in item.value.items().unwrap_or_default() {
                    if let Some(at) = RULES.iter().position(|r| rule.keyword_is(r.0)) {
                        // 14062F5E0's temporary bool starts false: a failed scan stores false.
                        let value = rule.value.scalar().and_then(parser::read_bool);
                        if value.is_none() {
                            notes.push(format!("range {}: rule {} failed boolean conversion; stored false (14062F5E0)", String::from_utf8_lossy(&range.id), RULES[at].0));
                        }
                        range.rules[at] = value.unwrap_or(false);
                    } else if !rule.keyword_is("icon")
                        && !rule.keyword_is("desc")
                        && !rule.keyword_is("override")
                    {
                        notes.push(format!(
                            "range {}: unknown rule {} ignored (14062EDC0)",
                            String::from_utf8_lossy(&range.id),
                            String::from_utf8_lossy(rule.key.as_deref().unwrap_or_default())
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    range.modifiers.values.retain(|m| {
        if registry(m.name.as_bytes()) == Some(false) {
            notes.push(format!(
                "range {}: unknown modifier {} ignored (1406042A0)",
                String::from_utf8_lossy(&range.id),
                m.key
            ));
            false
        } else {
            true
        }
    });
    let mut scans = modifiers::ScanFailures::default();
    scans.modifiers(&String::from_utf8_lossy(&range.id), &range.modifiers);
    notes.extend(scans.warning());
    range
}

/// Reads the recorded content stack. An unsettled file refuses the database
/// by file and reader: its unread suffix could replace any template.
pub fn read(layers: &ResolvedLayers, registry: modifiers::Registry<'_>) -> io::Result<Database> {
    let mut database = Database::default();
    for path in layers.directory_files("common/bop", ".txt")? {
        let read = read_file(&layers.read_uncovered(&path)?, registry).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{path}: 1 unread balance file: {e}"),
            )
        })?;
        database
            .notes
            .extend(read.misreads.iter().map(|n| format!("{path}: {n}")));
        for balance in read.items {
            let key = parser::c_string(balance.key.as_deref().unwrap_or_default()).to_vec();
            let items = balance.value.items().unwrap_or_default();
            let mut ranges = Vec::new();
            for item in items.iter().filter(|i| i.keyword_is("range")) {
                ranges.push(range(
                    item.value.items().unwrap_or_default(),
                    &mut database.notes,
                    registry,
                ));
            }
            for side in items.iter().filter(|i| i.keyword_is("side")) {
                for item in side
                    .value
                    .items()
                    .unwrap_or_default()
                    .iter()
                    .filter(|i| i.keyword_is("range"))
                {
                    ranges.push(range(
                        item.value.items().unwrap_or_default(),
                        &mut database.notes,
                        registry,
                    ));
                }
            }
            if let Some(cause) = ranges.iter().find_map(|r| r.modifiers.unheld.cause()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{path}: 1 unread balance file: {cause}"),
                ));
            }
            // 140A7B800 hashes without case; 140A7F150 compares with _stricmp.
            if database
                .balances
                .insert(key.to_ascii_lowercase(), ranges)
                .is_some()
            {
                database.notes.push(format!(
                    "{path}: duplicate template {} replaced (140A7F150)",
                    String::from_utf8_lossy(&key)
                ));
            }
        }
    }
    Ok(database)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_range_values_and_rule_defaults() {
        let read = read_file(
            br#"@value = 0.25 @tip = TIP balance = { range = { id = @value
            modifier = { first = @value hidden_modifier = { first = 2 } first = 3 }
            rule = { can_create_factions = yes can_join_factions = invalid }
            on_activate = { add_political_power = 1 }
            on_activate = { custom_effect_tooltip = @tip
                effect_tooltip { custom_effect_tooltip = IGNORED }
                effect_tooltip = { custom_effect_tooltip = @tip } }
            on_deactivate = { hidden_effect = { custom_effect_tooltip = HIDDEN } }
        } }"#,
            &|_| Some(true),
        )
        .unwrap();
        let mut notes = Vec::new();
        let range = range(
            read.items[0].value.items().unwrap()[0]
                .value
                .items()
                .unwrap(),
            &mut notes,
            &|_| Some(true),
        );
        assert_eq!(range.id, b"@value");
        assert_eq!(range.modifiers.values[0].value, 300_000);
        assert!(range.modifiers.listed().next().is_none());
        let rule = |name| range.rules[RULES.iter().position(|r| r.0 == name).unwrap()];
        assert!(rule("can_create_factions"));
        assert!(!rule("can_join_factions"));
        assert!(rule("can_puppet"));
        assert!(notes.iter().any(|note| note.contains("can_join_factions")));
        assert_eq!(
            range.activation,
            [
                Effect::Unported(b"add_political_power".to_vec()),
                Effect::Tooltip(FormattedLocalization::Key(b"TIP".to_vec())),
                Effect::Group(Vec::new()),
                Effect::Group(vec![Effect::Tooltip(FormattedLocalization::Key(
                    b"TIP".to_vec()
                ))]),
            ]
        );
        assert_eq!(
            range.deactivation,
            [Effect::Hidden(vec![Effect::Tooltip(
                FormattedLocalization::Key(b"HIDDEN".to_vec())
            )])]
        );
    }

    #[test]
    fn unsettled_modifier_and_effect_are_named() {
        let error = read_file(
            b"balance={range={modifier={unknown={a=1}}}}",
            modifiers::UNKNOWN,
        )
        .unwrap_err();
        assert!(error.contains("unknown") && error.contains("1406042A0"));
        let error = read_file(
            b"balance={range={on_activate={x @value=1}}}",
            modifiers::UNKNOWN,
        )
        .unwrap_err();
        assert!(error.contains("@value") && error.contains("line 1"));
    }

    #[test]
    fn layered_templates_replace_and_root_ranges_precede_sides() {
        let root = std::env::temp_dir().join(format!(
            "bop-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let base = root.join("base");
        let overlay = root.join("overlay");
        for folder in [&base, &overlay] {
            std::fs::create_dir_all(folder.join("common/bop")).unwrap();
        }
        std::fs::write(
            base.join("common/bop/a.txt"),
            b"\"BALANCE\"={range={id=old}} discarded={range={id=old}}",
        )
        .unwrap();
        std::fs::write(overlay.join("common/bop/b.txt"), b"balance={side={range={id=same modifier={value=2}}} range={id=same modifier={value=1}}} unnamed={range={modifier={value=7}}}").unwrap();
        let layers = ResolvedLayers::from_folders(&[(&base, &[]), (&overlay, &[])]).unwrap();
        let database = read(&layers, &|_| Some(true)).unwrap();
        assert!(
            database
                .range(b"BALANCE", b"old")
                .unwrap_err()
                .contains("missing authored range old")
        );
        assert_eq!(
            database.balances[b"balance".as_slice()]
                .iter()
                .map(|r| r.modifiers.values[0].value)
                .collect::<Vec<_>>(),
            [100_000, 200_000]
        );
        assert!(
            database
                .range(b"BaLaNcE", b"same")
                .unwrap_err()
                .contains("2 ranges named same")
        );
        assert!(
            database
                .range(b"unnamed", b"")
                .unwrap_err()
                .contains("1 unnamed authored ranges")
        );
        let layers =
            ResolvedLayers::from_folders(&[(&base, &[]), (&overlay, &["common/bop"])]).unwrap();
        assert!(
            read(&layers, &|_| Some(true))
                .unwrap()
                .range(b"discarded", b"old")
                .is_err()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
