//! Focus presentation definitions, using the shared Clausewitz and layer readers.
use crate::key_names::KeyNames;
use crate::{
    ResolvedLayers,
    fields::invalid,
    modifiers,
    parser::{
        Item,
        native::{self, Shape},
    },
    scope::ScopeKind,
};
use std::{collections::BTreeMap, io};

/// The default focus icon resolved through content.
pub const UNKNOWN_ICON: &str = "GFX_goal_unknown";

/// Focus definitions by id.
pub type Focuses = BTreeMap<String, FocusDefinition>;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FocusDefinition {
    pub localisation: String,
    pub description: String,
    /// Authored points before saved progress or country-specific modifiers.
    pub cost: Option<f64>,
    /// First unconditional icon case, for consumers without an event scope.
    pub icon: Option<String>,
    /// An authored trigger-chosen icon block, distinct from no icon field.
    pub scripted_icon: bool,
    pub icons: crate::scripted_keys::ScriptedKey,
    pub alternate_icons: crate::scripted_keys::ScriptedKey,
    /// Parsed alternate icon definitions, including an empty block.
    pub alternate_icon_present: bool,
}

impl FocusDefinition {
    /// Alternate icon cases are read only when present and selected by
    /// window mode. Without that operand, equal normalized choices remain
    /// exact; differing choices retain a named refusal.
    pub fn icon(
        &self,
        alternate: Option<bool>,
        mut evaluate: impl FnMut(&[Item]) -> Result<bool, String>,
    ) -> Result<Option<&str>, String> {
        if !self.alternate_icon_present {
            return self.icons.select(evaluate);
        }
        match alternate {
            Some(true) => self.alternate_icons.select(evaluate),
            Some(false) => self.icons.select(evaluate),
            None => {
                let normal = self.icons.select(&mut evaluate)?;
                let alternate = self.alternate_icons.select(evaluate)?;
                if normal.unwrap_or(UNKNOWN_ICON) == alternate.unwrap_or(UNKNOWN_ICON) {
                    Ok(normal)
                } else {
                    Err("focus alternate icon: focus-window mode is not recorded".into())
                }
            }
        }
    }

    /// Try country-qualified and token-qualified sprites before icon
    /// selection. Notification thumbnails use an empty suffix.
    pub fn sprite_overrides(&self, token: &str, tag: Option<&str>) -> Vec<String> {
        let mut sprites = Vec::new();
        if let Some(tag) = tag {
            sprites.push(format!("GFX_{tag}_{token}"));
        }
        sprites.push(format!("GFX_{token}"));
        sprites
    }

    /// Normal icon selection is independent of the unrecorded alternate-mode
    /// flag and retains the default icon fallback.
    pub fn select_normal_icon(
        &self,
        evaluate: impl FnMut(&[Item]) -> Result<bool, String>,
    ) -> Result<&str, String> {
        Ok(self.icons.select(evaluate)?.unwrap_or(UNKNOWN_ICON))
    }
}

fn scalar<'a>(items: &'a [Item], key: &str) -> io::Result<Option<&'a str>> {
    items
        .iter()
        .rfind(|item| item.keyword_is(key))
        .map(|item| {
            item.value
                .read_str()
                .map_err(|e| invalid(format!("focus {key}: {e}")))
        })
        .transpose()
}

/// The database a file is read into, by its directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Database {
    /// `common/national_focus`.
    National,
    /// `common/continuous_focus`.
    Continuous,
}

#[derive(Clone, Copy)]
enum Node {
    /// The national database's tokens: `focus_tree`,
    /// `shared_focus`, `joint_focus`, `search_filter_prios`, `style`.
    NationalFile,
    /// The continuous database reads only `continuous_focus_palette` roots.
    ContinuousFile,
    Tree,
    Palette,
    Focus,
    Joint,
    Continuous,
    Dependency,
    Offset,
    Position,
    Shortcut,
    Inlay,
    Priorities,
    Style,
    Mtth,
    Modifier(modifiers::Node),
}

/// National files read trees, shared focuses, and joint focuses; continuous
/// files read palettes and continuous focuses. Both use persistent object
/// semantics, and focus reads add source metadata. Each database reports
/// and skips roots belonging to the other.
pub fn parse(
    input: &[u8],
    database: Database,
    names: &KeyNames,
) -> Result<(Focuses, Vec<crate::Diagnostic>), (crate::DiagnosticKind, io::Error)> {
    use Node::*;
    let mut undecided = Vec::new();
    let mut refused = Vec::new();
    let top = match database {
        Database::National => NationalFile,
        Database::Continuous => ContinuousFile,
    };
    // National files use lexer mode 1; continuous files use mode 0.
    let mode = match database {
        Database::National => native::Mode::SkipMark,
        Database::Continuous => native::Mode::KeepMark,
    };
    let document = native::read(
        input,
        mode,
        native::Top::Persistent(top),
        Some(names),
        &mut |node, field| {
            match (node, field.keyword().as_deref()) {
                (NationalFile, Some(b"focus_tree")) => Shape::Object(Tree),
                (ContinuousFile, Some(b"continuous_focus_palette")) => Shape::Object(Palette),
                (NationalFile, Some(b"shared_focus")) | (Tree, Some(b"focus")) => {
                    Shape::Object(Focus)
                }
                (NationalFile, Some(b"joint_focus")) => Shape::Object(Joint),
                (Palette, Some(b"focus")) => Shape::Object(Continuous),
                (NationalFile | Tree | Focus | Joint, Some(b"search_filter_prios")) => {
                    Shape::Object(Priorities)
                }
                (NationalFile, Some(b"style")) => Shape::Object(Style),
                (Tree | Palette, Some(b"country")) => Shape::Object(Mtth),
                (Tree, Some(b"shortcut")) => Shape::Object(Shortcut),
                (Tree, Some(b"inlay_window")) => Shape::Object(Inlay),
                (Tree, Some(b"initial_show_position")) => Shape::Object(Position),
                // Some lists read lexer tokens while retaining the value slot;
                // `search_filters` resolves entries into that slot.
                (Tree, Some(b"continuous_focus_position")) | (Palette, Some(b"position")) => {
                    if field.opens() {
                        Shape::RawList
                    } else {
                        Shape::Skip
                    }
                }
                (
                    Tree,
                    Some(
                        b"id"
                        | b"default"
                        | b"reset_on_civilwar"
                        | b"shared_focus"
                        | b"joint_focus"
                        | b"alternate_icon_set",
                    ),
                )
                | (Palette, Some(b"id" | b"default" | b"reset_on_civilwar")) => Shape::One,
                (Focus | Joint, Some(b"prerequisite" | b"mutually_exclusive")) => {
                    Shape::Object(Dependency)
                }
                (Focus | Joint | Position, Some(b"offset"))
                | (Inlay, Some(b"override_position")) => Shape::Object(Offset),
                (Focus | Joint | Continuous, Some(b"icon"))
                | (Focus | Joint, Some(b"alternate_icon")) => Shape::ScriptedKey,
                (Focus | Joint | Continuous, Some(b"ai_will_do")) => Shape::Object(Mtth),
                (
                    Focus | Joint,
                    Some(b"cancel" | b"available" | b"bypass" | b"historical_ai" | b"allow_branch"),
                )
                | (Joint, Some(b"joint_trigger"))
                | (Continuous, Some(b"available" | b"enabled" | b"enable"))
                | (Offset | Shortcut, Some(b"trigger")) => Shape::Trigger,
                (
                    Focus | Joint,
                    Some(
                        b"completion_reward" | b"bypass_effect" | b"select_effect"
                        | b"complete_tooltip" | b"on_uncomplete",
                    ),
                )
                | (
                    Joint,
                    Some(b"completion_reward_joint_originator" | b"completion_reward_joint_member"),
                )
                | (Continuous, Some(b"select_effect" | b"cancel_effect" | b"daily_reward")) => {
                    if field.keyword_is("complete_tooltip") {
                        // Complete-tooltip effects use a zero scope mask.
                        Shape::EffectIn(0)
                    } else {
                        // Other effects use country scope.
                        Shape::EffectIn(ScopeKind::Country.bit())
                    }
                }
                (Continuous, Some(b"modifier")) => {
                    Shape::Object(Modifier(modifiers::Node::Modifier))
                }
                (Focus | Joint, Some(b"search_filters")) => {
                    if field.opens() {
                        Shape::List
                    } else {
                        Shape::One
                    }
                }
                (Focus | Joint, Some(b"will_lead_to_war_with")) => {
                    if field.opens() {
                        Shape::Skip
                    } else {
                        Shape::One
                    }
                }
                (
                    Focus | Joint,
                    Some(
                        b"id"
                        | b"x"
                        | b"y"
                        | b"text"
                        | b"overlay"
                        | b"cost"
                        | b"historical"
                        | b"dynamic"
                        | b"cancelable"
                        | b"cancel_if_invalid"
                        | b"continue_if_invalid"
                        | b"available_if_capitulated"
                        | b"relative_position_id"
                        | b"enable_automatic_bypass"
                        | b"text_icon"
                        | b"bypass_if_unavailable",
                    ),
                )
                | (
                    Continuous,
                    Some(
                        b"id"
                        | b"text"
                        | b"idea"
                        | b"daily_cost"
                        | b"available_if_capitulated"
                        | b"supports_ai_strategy",
                    ),
                )
                | (Dependency, Some(b"or" | b"focus"))
                | (Offset, Some(b"x" | b"y"))
                | (Position, Some(b"x" | b"y" | b"focus"))
                | (Shortcut, Some(b"name" | b"target" | b"scroll_wheel_factor"))
                | (Inlay, Some(b"id"))
                | (
                    Style,
                    Some(
                        b"name" | b"default" | b"available" | b"unavailable" | b"completed"
                        | b"current",
                    ),
                ) => Shape::One,
                // Point reads consume `{` followed by seven tokens, `x = n y = n }`.
                // Only that exact form matches list consumption, as checked below.
                (Inlay, Some(b"position")) => {
                    if !field.opens() {
                        refused.push(format!(
                            "focus inlay_window position at line {}: not a block",
                            field.key.line
                        ));
                    }
                    Shape::RawList
                }
                (Priorities, _) => {
                    if field.opens() {
                        refused.push(format!(
                            "focus search_filter_prios {}: block value",
                            String::from_utf8_lossy(&field.key.text)
                        ));
                    }
                    Shape::One
                }
                (Style, _) => {
                    // An unknown token would stall the game reader; refuse it by name.
                    refused.push(format!(
                        "focus style field {}: unsupported native nonterminating dispatch",
                        String::from_utf8_lossy(&field.key.text)
                    ));
                    Shape::Skip
                }
                (Mtth, _) => native::mean_time_to_happen(field),
                (Modifier(node), _) => {
                    modifiers::shape(node, field, modifiers::UNKNOWN, &mut undecided).map(Modifier)
                }
                _ => Shape::Skip,
            }
        },
    );
    if let Some(cause) = document.unsettled {
        return Err((cause.kind, invalid(format!("focus: {cause}"))));
    }
    if let Some(cause) = undecided.first() {
        return Err((
            crate::DiagnosticKind::Unsettled,
            invalid(format!("focus: {cause}")),
        ));
    }
    if let Some(cause) = refused.first() {
        return Err((crate::DiagnosticKind::Refused, invalid(cause)));
    }
    // Offsets accept two-number or x/y forms. Fixed-length point reads accept
    // only `x = n y = n`; other recovery forms remain refused.
    fn vectors(items: &[Item], inlay: bool) -> io::Result<()> {
        for item in items {
            if let Some(children) = item.value.items() {
                if item.keyword_is("position") || item.keyword_is("continuous_focus_position") {
                    let valid = !inlay && children.len() == 2 && children.iter().all(Item::number)
                        || children.len() == 6
                            && children[0]
                                .value
                                .scalar()
                                .is_some_and(|v| v.eq_ignore_ascii_case(b"x"))
                            && children[1].value.scalar_str() == Some("=")
                            && children[2].number()
                            && children[3]
                                .value
                                .scalar()
                                .is_some_and(|v| v.eq_ignore_ascii_case(b"y"))
                            && children[4].value.scalar_str() == Some("=")
                            && children[5].number();
                    if !valid {
                        return Err(invalid(format!(
                            "focus {}: unsupported vector form",
                            item.key_str().unwrap_or_default()
                        )));
                    }
                } else if item.keyword_is("focus_tree")
                    || item.keyword_is("continuous_focus_palette")
                {
                    vectors(children, false)?;
                } else if item.keyword_is("inlay_window") {
                    vectors(children, true)?;
                }
            }
        }
        Ok(())
    }
    vectors(&document.items, false).map_err(|error| (crate::DiagnosticKind::Unsupported, error))?;
    fn collect(items: &[Item], output: &mut BTreeMap<String, FocusDefinition>) -> io::Result<()> {
        for item in items {
            let Some(children) = item.value.items() else {
                continue;
            };
            if item.keyword_is("focus")
                || item.keyword_is("shared_focus")
                || item.keyword_is("joint_focus")
            {
                let Some(id) = scalar(children, "id")? else {
                    return Err(invalid("focus definition has no id"));
                };
                let localisation = scalar(children, "text")?.unwrap_or(id);
                let mut icons = crate::scripted_keys::ScriptedKey::default();
                let mut alternate_icons = crate::scripted_keys::ScriptedKey::default();
                let mut alternate_icon_present = false;
                for field in children {
                    let target = if field.keyword_is("icon") {
                        &mut icons
                    } else if field.keyword_is("alternate_icon") {
                        alternate_icon_present = true;
                        &mut alternate_icons
                    } else {
                        continue;
                    };
                    target
                        .append(&field.value)
                        .map_err(|why| invalid(format!("focus {id} icon: {why}")))?;
                }
                let icon = icons
                    .cases
                    .first()
                    .filter(|case| case.trigger.is_none())
                    .map(|case| case.key.clone());
                output.insert(
                    id.to_owned(),
                    FocusDefinition {
                        localisation: localisation.into(),
                        description: format!("{localisation}_desc"),
                        // Failed fixed5 scans retain the previous cost.
                        cost: children
                            .iter()
                            .rev()
                            .filter(|item| item.keyword_is("cost"))
                            .filter_map(|item| item.value.scalar())
                            .find_map(crate::fixed5::script_fixed5)
                            .filter(|cost| *cost >= 0)
                            .map(|cost| cost as f64 / 100_000.0),
                        icon,
                        scripted_icon: icons.cases.iter().any(|case| case.trigger.is_some()),
                        icons,
                        alternate_icons,
                        alternate_icon_present,
                    },
                );
            } else if item.keyword_is("focus_tree") || item.keyword_is("continuous_focus_palette") {
                collect(children, output)?;
            }
        }
        Ok(())
    }
    let mut result = BTreeMap::new();
    // A root the database does not read was reported and skipped.
    let roots: &[&str] = match database {
        Database::National => &["focus_tree", "shared_focus", "joint_focus"],
        Database::Continuous => &["continuous_focus_palette"],
    };
    let own: Vec<Item> = document
        .items
        .into_iter()
        .filter(|item| roots.iter().any(|root| item.keyword_is(root)))
        .collect();
    collect(&own, &mut result).map_err(|error| (crate::DiagnosticKind::Refused, error))?;
    Ok((
        result,
        document
            .misreads
            .iter()
            .map(|note| (note.kind, note.to_string()))
            .collect(),
    ))
}

/// `NDefines.NFocus.FOCUS_POINT_DAYS` from the evaluated defines. Content
/// the defines reader refuses, or a missing or non-positive value, leaves the
/// days unavailable; file errors still fail.
pub fn point_days(layers: &ResolvedLayers) -> io::Result<Option<f64>> {
    let defines = match crate::defines::read(layers) {
        Ok(defines) => defines,
        Err(error) if error.kind() == io::ErrorKind::InvalidData => return Ok(None),
        Err(error) => return Err(error),
    };
    Ok(defines
        .number("NFocus", "FOCUS_POINT_DAYS")
        .ok()
        .filter(|days| days.is_finite() && *days > 0.0))
}

/// Reader diagnostics with severity retained from their creation sites.
pub fn read(layers: &ResolvedLayers) -> io::Result<(Focuses, Vec<crate::Diagnostic>)> {
    read_databases(layers, true)
}

/// National-focus lookup does not read the separate continuous database.
pub fn read_national(layers: &ResolvedLayers) -> io::Result<(Focuses, Vec<crate::Diagnostic>)> {
    read_databases(layers, false)
}

fn read_databases(
    layers: &ResolvedLayers,
    continuous: bool,
) -> io::Result<(Focuses, Vec<crate::Diagnostic>)> {
    let mut paths = layers
        .paths()
        .filter(|p| {
            (p.starts_with("common/national_focus/")
                || continuous && p.starts_with("common/continuous_focus/"))
                && p.ends_with(".txt")
        })
        .map(|p| (layers.layer_index(p).unwrap_or(0), p.to_owned()))
        .collect::<Vec<_>>();
    paths.sort();
    let mut result = BTreeMap::new();
    let mut warnings = Vec::new();
    let names = layers.key_names()?;
    for (_, path) in paths {
        let database = if path.starts_with("common/national_focus/") {
            Database::National
        } else {
            Database::Continuous
        };
        match parse(&layers.read(&path)?, database, &names) {
            Ok((definitions, notes)) => {
                result.extend(definitions);
                warnings.extend(
                    notes
                        .into_iter()
                        .map(|(kind, note)| (kind, format!("{path}: {note}"))),
                );
            }
            Err((kind, error)) => warnings.push((
                kind,
                format!("Focus definitions unavailable in {path}: {error}"),
            )),
        }
    }
    Ok((result, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(
        input: &[u8],
        database: Database,
        names: &KeyNames,
    ) -> io::Result<BTreeMap<String, FocusDefinition>> {
        super::parse(input, database, names)
            .map(|(definitions, _)| definitions)
            .map_err(|(_, error)| error)
    }

    #[test]
    fn national_lookup_excludes_continuous_database() -> io::Result<()> {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let root =
            std::env::temp_dir().join(format!("national-focus-names-{}", std::process::id()));
        std::fs::create_dir_all(root.join("common/national_focus"))?;
        std::fs::create_dir_all(root.join("common/continuous_focus"))?;
        std::fs::write(
            root.join("common/national_focus/names.txt"),
            b"focus_tree={focus={id=SYN_FOCUS text=SYN_NATIONAL}}",
        )?;
        std::fs::write(
            root.join("common/continuous_focus/names.txt"),
            b"continuous_focus_palette={focus={id=SYN_FOCUS text=SYN_CONTINUOUS}}",
        )?;
        let layers = ResolvedLayers::from_folders(&[(&root, &[])])?;
        let (definitions, warnings) = read_national(&layers)?;
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(definitions["SYN_FOCUS"].localisation, "SYN_NATIONAL");
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn layer_warnings_name_and_count_deferred_effect_readers() -> io::Result<()> {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let root = std::env::temp_dir().join(format!("focus-effect-gap-{}", std::process::id()));
        std::fs::create_dir_all(root.join("common/national_focus"))?;
        std::fs::write(
            root.join("common/national_focus/gap.txt"),
            b"focus_tree = {
focus = { id = test completion_reward = {
free_operative = { operative = a } free_operative = { operative = b }
} } }",
        )?;
        let layers = ResolvedLayers::from_folders(&[(&root, &[])])?;
        let (definitions, warnings) = read(&layers)?;
        assert!(definitions.contains_key("test"));
        let warning = warnings
            .iter()
            .find(|(_, warning)| warning.contains("Deferred effect `free_operative`"))
            .unwrap();
        let warning = &warning.1;
        assert!(
            warning.contains("common/national_focus/gap.txt: line 3:"),
            "{warning}"
        );
        assert!(
            warning.contains("its value reader is unsupported; by-key reading retained"),
            "{warning}"
        );
        assert!(warning.contains("count: 2"), "{warning}");
        assert!(read(&layers)?.1.iter().any(|(kind, message)| *kind
            == crate::DiagnosticKind::Unsupported
            && message == warning));
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn focus_variables_keywords_and_scripted_key_dispatch() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let defs = parse(
            br#"@name=A @sprite=GFX_a focus_tree={FOCUS={id=@name "text"=ignored
            icon={@sprite=yes {ignored={}}} unknown={id=hidden} cost=0x2}}
            shared_focus={id=B icon=@sprite}"#,
            Database::National,
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(defs["A"].localisation, "A");
        assert_eq!(defs["A"].icon.as_deref(), Some("GFX_a"));
        assert_eq!(defs["A"].cost, Some(2.0));
        assert_eq!(defs["B"].icon.as_deref(), Some("GFX_a"));
    }

    #[test]
    fn focus_scripted_icons_preserve_repeated_cases_and_refuse_unknown_conditions() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let defs = parse(
            br#"focus_tree={focus={id=A
            icon={CONDITIONAL={always=yes}}
            icon={FALLBACK=yes}
            alternate_icon={ALTERNATE=yes}
        }}"#,
            Database::National,
            &KeyNames::default(),
        )
        .unwrap();
        let focus = &defs["A"];
        assert_eq!(
            focus.select_normal_icon(|_| Ok(true)).unwrap(),
            "CONDITIONAL"
        );
        assert_eq!(focus.select_normal_icon(|_| Ok(false)).unwrap(), "FALLBACK");
        assert_eq!(
            focus
                .select_normal_icon(|_| Err("unknown native operand".into()))
                .unwrap_err(),
            "unknown native operand"
        );
        assert!(focus.alternate_icon_present);
        assert_eq!(
            focus.alternate_icons.select(|_| Ok(false)).unwrap(),
            Some("ALTERNATE")
        );
        assert!(focus.scripted_icon);
    }

    #[test]
    fn authored_text_shared_and_conditional_icons() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let defs = parse(br#"focus_tree={focus={id=A icon=GFX_goal_a text=TITLE} focus={id=B icon={value=GFX_goal_b}}} shared_focus={id=C icon=GFX_goal_c}"#, Database::National, &KeyNames::default()).unwrap();
        assert_eq!(defs["A"].localisation, "TITLE");
        assert_eq!(defs["A"].description, "TITLE_desc");
        assert_eq!(defs["A"].icon.as_deref(), Some("GFX_goal_a"));
        assert_eq!(defs["B"].icon, None);
        assert!(defs["B"].scripted_icon);
        assert!(!defs["A"].scripted_icon);
        assert_eq!(defs["C"].localisation, "C");
    }

    #[test]
    fn unknown_description_is_skipped_by_the_focus_reader() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let defs = parse(
            br#"focus_tree={focus={id=A text=TITLE desc=DESCRIPTION}}"#,
            Database::National,
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(defs["A"].description, "TITLE_desc");
    }

    #[test]
    fn inlay_positions_take_only_the_read_int_point_form() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        // This malformed point would consume closing braces and the next
        // focus identifier, so it remains refused.
        assert!(
            parse(
                br#"focus_tree={inlay_window={id=w position={1 2}} focus={id=A}}"#,
                Database::National,
                &KeyNames::default()
            )
            .unwrap_err()
            .to_string()
            .contains("vector form")
        );
        let defs = parse(
            br#"focus_tree={inlay_window={id=w position={X=1 y=2}} focus={id=A position={1 2}}}"#,
            Database::National,
            &KeyNames::default(),
        )
        .unwrap();
        assert!(defs.contains_key("A"));
    }

    #[test]
    fn each_database_reads_only_its_own_roots() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let text = br#"focus_tree={focus={id=A}} continuous_focus_palette={focus={id=E}} shared_focus={id=B}"#;
        let national = parse(text, Database::National, &KeyNames::default()).unwrap();
        assert_eq!(national.keys().collect::<Vec<_>>(), ["A", "B"]);
        let continuous = parse(text, Database::Continuous, &KeyNames::default()).unwrap();
        assert_eq!(continuous.keys().collect::<Vec<_>>(), ["E"]);
    }

    #[test]
    fn a_failed_repeated_cost_keeps_the_stored_one() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let defs = parse(
            br#"focus_tree={focus={id=A cost=2 cost=bad} focus={id=B cost=bad}}"#,
            Database::National,
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!((defs["A"].cost, defs["B"].cost), (Some(2.0), None));
    }

    #[test]
    fn authored_costs_and_scripted_costs() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let defs = parse(br#"focus_tree={focus={id=A cost=10} focus={id=B cost=2.5} focus={id=C cost=@scripted} focus={id=D cost=-1}}"#, Database::National, &KeyNames::default()).unwrap();
        let continuous = parse(
            br#"continuous_focus_palette={focus={id=E}}"#,
            Database::Continuous,
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(defs["A"].cost, Some(10.0));
        assert_eq!(defs["B"].cost, Some(2.5));
        for id in ["C", "D"] {
            assert_eq!(defs[id].cost, None);
        }
        assert_eq!(continuous["E"].cost, None);
    }
}

#[cfg(test)]
mod icon_selection_tests {
    use super::*;

    fn focus() -> FocusDefinition {
        let (definitions, _) = parse(
            br#"focus_tree={focus={id=synthetic
                icon={GFX_first={always=no} GFX_second={always=yes} GFX_final=yes}
                alternate_icon=GFX_alternate}}"#,
            Database::National,
            &KeyNames::default(),
        )
        .unwrap();
        definitions["synthetic"].clone()
    }

    #[test]
    fn first_true_case_wins_and_an_earlier_unknown_is_not_skipped() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let definition = focus();
        let mut visited = 0;
        assert_eq!(
            definition
                .icon(Some(false), |_| {
                    visited += 1;
                    Ok(visited == 2)
                })
                .unwrap(),
            Some("GFX_second")
        );
        assert_eq!(visited, 2);
        assert_eq!(
            definition
                .icon(Some(false), |_| Err("missing recorded predicate".into()))
                .unwrap_err(),
            "missing recorded predicate"
        );
        assert_eq!(
            definition
                .icon(Some(true), |_| panic!("unconditional alternate"))
                .unwrap(),
            Some("GFX_alternate")
        );
        assert!(
            definition
                .icon(None, |_| Ok(false))
                .unwrap_err()
                .contains("focus-window mode")
        );
    }

    #[test]
    fn fallback_agreement_needs_no_window_mode() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (definitions, _) = parse(
            br#"focus_tree={
                focus={id=fallback alternate_icon=GFX_goal_unknown}
                focus={id=same icon=GFX_same alternate_icon={GFX_same=yes}}
                focus={id=absent icon={GFX_conditional={always=no}}}
                focus={id=empty icon=GFX_same alternate_icon={}}
            }"#,
            Database::National,
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(
            definitions["fallback"]
                .icon(None, |_| panic!("no predicate"))
                .unwrap(),
            None
        );
        assert_eq!(
            definitions["same"]
                .icon(None, |_| panic!("no predicate"))
                .unwrap(),
            Some("GFX_same")
        );
        assert_eq!(
            definitions["absent"].icon(None, |_| Ok(false)).unwrap(),
            None
        );
        let empty = &definitions["empty"];
        assert!(empty.alternate_icon_present);
        assert_eq!(
            empty.icon(Some(true), |_| panic!("no predicate")).unwrap(),
            None
        );
        assert!(
            empty
                .icon(None, |_| panic!("no predicate"))
                .unwrap_err()
                .contains("focus-window mode")
        );
    }
}
