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
};
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FocusDefinition {
    pub localisation: String,
    pub description: String,
    /// Authored points before saved progress or country-specific modifiers.
    pub cost: Option<f64>,
    /// Conditional icon scripts need game evaluation and remain unavailable.
    pub icon: Option<String>,
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
    /// `common/national_focus` (1402D7F90).
    National,
    /// `common/continuous_focus` (1406B6140).
    Continuous,
}

#[derive(Clone, Copy)]
enum Node {
    /// The national database's tokens (1402D7F90): `focus_tree`,
    /// `shared_focus`, `joint_focus`, `search_filter_prios`, `style`.
    NationalFile,
    /// The continuous database's (1406B6140): `continuous_focus_palette`
    /// (0x36DD) alone.
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

/// National database 1402D7F90, tree 1402D8880, focus 1402D76A0 and
/// joint 140DE1F10; continuous database 1406B6140, palette 1406B62F0 and
/// focus 1406B67D0. The databases and nested objects use CPersistent::Read;
/// CNationalFocus::Read 1402D71D0 only adds source metadata before that loop.
/// Each database reports and skips the other's roots.
pub fn parse(
    input: &[u8],
    database: Database,
    names: &KeyNames,
) -> io::Result<(BTreeMap<String, FocusDefinition>, Vec<String>)> {
    use Node::*;
    let mut undecided = Vec::new();
    let mut refused = Vec::new();
    let top = match database {
        Database::National => NationalFile,
        Database::Continuous => ContinuousFile,
    };
    // The national files' preparation task (1402DC730) constructs the lexer
    // in mode 1, the continuous loader (1406B5860) in mode 0.
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
                // 1402C7A50 and 140A3D660 read the lexer's tokens, leaving the
                // value slot; search_filters (1402C7640) resolves into it.
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
                    Shape::Effect
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
                // ReadIntPoint (140A3D660) reads the value `{` and then seven
                // tokens, `x = n y = n }`, whatever they are; only that exact
                // form reads as a list does (checked below).
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
                    // 141362610 spins forever on an unknown token. Refuse it.
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
        return Err(invalid(format!("focus: {cause}")));
    }
    if let Some(cause) = undecided.first() {
        return Err(invalid(format!("focus: {cause}")));
    }
    if let Some(cause) = refused.first() {
        return Err(invalid(cause));
    }
    // 1402C7A50 accepts the two-number or x/y forms; ReadIntPoint
    // (140A3D660, inlay windows) only `x = n y = n`, as it consumes a fixed
    // seven tokens. Refuse other forms, whose token recovery is not replayed.
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
    vectors(&document.items, false)?;
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
                let icon = children
                    .iter()
                    .rfind(|item| item.keyword_is("icon"))
                    .filter(|item| item.value.scalar().is_some())
                    .map(|item| {
                        item.value
                            .c_str()
                            .map(str::to_owned)
                            .map_err(|e| invalid(format!("focus {id} icon: {e}")))
                    })
                    .transpose()?;
                output.insert(
                    id.to_owned(),
                    FocusDefinition {
                        localisation: localisation.into(),
                        description: format!("{localisation}_desc"),
                        // ReadFixed5 (1424ABB10 -> 1424B0480) leaves the
                        // cost as it was when a conversion fails.
                        cost: children
                            .iter()
                            .rev()
                            .filter(|item| item.keyword_is("cost"))
                            .filter_map(|item| item.value.scalar())
                            .find_map(crate::fixed5::script_fixed5)
                            .filter(|cost| *cost >= 0)
                            .map(|cost| cost as f64 / 100_000.0),
                        icon,
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
    collect(&own, &mut result)?;
    Ok((
        result,
        document.misreads.iter().map(ToString::to_string).collect(),
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

pub fn read(
    layers: &ResolvedLayers,
) -> io::Result<(BTreeMap<String, FocusDefinition>, Vec<String>)> {
    let mut paths = layers
        .paths()
        .filter(|p| {
            (p.starts_with("common/national_focus/") || p.starts_with("common/continuous_focus/"))
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
                warnings.extend(notes.into_iter().map(|note| format!("{path}: {note}")));
            }
            Err(error) => {
                warnings.push(format!("Focus definitions unavailable in {path}: {error}"))
            }
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
        super::parse(input, database, names).map(|(definitions, _)| definitions)
    }

    #[test]
    fn layer_warnings_name_and_count_deferred_effect_readers() -> io::Result<()> {
        let root = std::env::temp_dir().join(format!("focus-effect-gap-{}", std::process::id()));
        std::fs::create_dir_all(root.join("common/national_focus"))?;
        std::fs::write(
            root.join("common/national_focus/gap.txt"),
            b"focus_tree = {
focus = { id = test completion_reward = {
set_capital = { state = 1 } set_capital = { state = 2 }
} } }",
        )?;
        let layers = ResolvedLayers::from_folders(&[(&root, &[])])?;
        let (definitions, warnings) = read(&layers)?;
        assert!(definitions.contains_key("test"));
        let warning = warnings
            .iter()
            .find(|warning| warning.contains("Deferred effect `set_capital`"))
            .unwrap();
        assert!(
            warning.contains("common/national_focus/gap.txt: line 3:"),
            "{warning}"
        );
        assert!(warning.contains("key reader 0x1403B4F40"), "{warning}");
        assert!(warning.contains("count: 2"), "{warning}");
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn native_focus_variables_keywords_and_scripted_key_dispatch() {
        let defs = parse(
            br#"@name=A @sprite=GFX_a focus_tree={FOCUS={id=@name "text"=ignored
            icon={@sprite=yes {ignored={}}} unknown={id=hidden} cost=0x2}}
            shared_focus={id=B icon=@sprite}"#,
            Database::National,
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(defs["A"].localisation, "A");
        assert_eq!(defs["A"].icon, None);
        assert_eq!(defs["A"].cost, Some(2.0));
        assert_eq!(defs["B"].icon.as_deref(), Some("GFX_a"));
    }

    #[test]
    fn authored_text_shared_and_conditional_icons() {
        let defs = parse(br#"focus_tree={focus={id=A icon=GFX_goal_a text=TITLE} focus={id=B icon={value=GFX_goal_b}}} shared_focus={id=C icon=GFX_goal_c}"#, Database::National, &KeyNames::default()).unwrap();
        assert_eq!(defs["A"].localisation, "TITLE");
        assert_eq!(defs["A"].description, "TITLE_desc");
        assert_eq!(defs["A"].icon.as_deref(), Some("GFX_goal_a"));
        assert_eq!(defs["B"].icon, None);
        assert_eq!(defs["C"].localisation, "C");
    }

    #[test]
    fn unknown_description_is_skipped_by_the_native_focus_reader() {
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
        // 140A3D660 would consume `} } focus = { id` here: refused, not read.
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
        let text = br#"focus_tree={focus={id=A}} continuous_focus_palette={focus={id=E}} shared_focus={id=B}"#;
        let national = parse(text, Database::National, &KeyNames::default()).unwrap();
        assert_eq!(national.keys().collect::<Vec<_>>(), ["A", "B"]);
        let continuous = parse(text, Database::Continuous, &KeyNames::default()).unwrap();
        assert_eq!(continuous.keys().collect::<Vec<_>>(), ["E"]);
    }

    #[test]
    fn a_failed_repeated_cost_keeps_the_stored_one() {
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
