use content_reader_key_names::key_name_products::KeyNamesExt as _;
use crate::key_names::KeyNames;
use content_reader_script::native::{self, Shape};
use crate::{
    ResolvedLayers,
    fields::invalid,
    modifiers,
    parser::{
        Item,
    },
    scope::ScopeKind,
};
use content_reader_parser::coverage::{Coverage, Gap};
use content_reader_layers::script_files::ScriptPath;
use std::{collections::BTreeMap, io, sync::Arc};

pub const UNKNOWN_ICON: &str = "GFX_goal_unknown";

pub type Definitions = BTreeMap<String, FocusDefinition>;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Focuses {
    national: Arc<Store>,
    continuous: Arc<Store>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Store {
    definitions: Definitions,
    coverage: Coverage,
}

impl Store {
    fn get(&self, id: &str) -> Result<Option<&FocusDefinition>, &Gap> {
        match self.coverage.unknown(id) {
            Some(gap) => Err(gap),
            None => Ok(self.definitions.get(id)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unknown<'a> {
    Gap(&'a Gap),
    Ambiguous,
}

impl std::fmt::Display for Unknown<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Gap(gap) => write!(f, "{gap}"),
            Self::Ambiguous => f.write_str(
                "the national and continuous focus databases define it differently, and which one is meant is not given",
            ),
        }
    }
}

impl Focuses {
    pub fn get_in(&self, database: Database, id: &str) -> Result<Option<&FocusDefinition>, &Gap> {
        self.store(database).get(id)
    }

    pub fn get(&self, id: &str) -> Result<Option<&FocusDefinition>, Unknown<'_>> {
        let national = self.national.get(id).map_err(Unknown::Gap)?;
        let continuous = self.continuous.get(id).map_err(Unknown::Gap)?;
        match (national, continuous) {
            (Some(national), Some(continuous)) if national != continuous => Err(Unknown::Ambiguous),
            (Some(definition), _) | (None, Some(definition)) => Ok(Some(definition)),
            (None, None) => Ok(None),
        }
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.national
            .definitions
            .keys()
            .chain(self.continuous.definitions.keys())
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
    }

    pub fn ids_in(&self, database: Database) -> impl Iterator<Item = &str> {
        self.store(database).definitions.keys().map(String::as_str)
    }

    pub fn known(&self) -> impl Iterator<Item = &FocusDefinition> {
        [Database::National, Database::Continuous]
            .into_iter()
            .flat_map(|database| self.known_in(database))
    }

    pub fn known_in(&self, database: Database) -> impl Iterator<Item = &FocusDefinition> {
        let store = self.store(database);
        store
            .definitions
            .iter()
            .filter(move |(id, _)| store.coverage.unknown(id).is_none())
            .map(|(_, definition)| definition)
    }

    fn store(&self, database: Database) -> &Store {
        match database {
            Database::National => &self.national,
            Database::Continuous => &self.continuous,
        }
    }

    fn store_mut(&mut self, database: Database) -> &mut Store {
        match database {
            Database::National => Arc::make_mut(&mut self.national),
            Database::Continuous => Arc::make_mut(&mut self.continuous),
        }
    }

    fn read_file(&mut self, database: Database, definitions: Definitions) {
        let store = self.store_mut(database);
        store.coverage.next_file();
        for (id, definition) in definitions {
            store.coverage.replace(&id);
            store.definitions.insert(id, definition);
        }
    }

    fn refuse_file(&mut self, database: Database, path: &str, cause: String) {
        let coverage = &mut self.store_mut(database).coverage;
        coverage.next_file();
        coverage.gap(path, cause);
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FocusDefinition {
    pub localisation: String,
    pub description: String,
    pub cost: Option<f64>,
    pub icon: Option<String>,
    pub scripted_icon: bool,
    pub icon_present: bool,
    pub icons: crate::scripted_keys::ScriptedKey,
    pub alternate_icons: crate::scripted_keys::ScriptedKey,
    pub alternate_icon_present: bool,
}

impl FocusDefinition {
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

    pub fn sprite_overrides(&self, token: &str, tag: Option<&str>) -> Vec<String> {
        let mut sprites = Vec::new();
        if let Some(tag) = tag {
            sprites.push(format!("GFX_{tag}_{token}"));
        }
        sprites.push(format!("GFX_{token}"));
        sprites
    }

    pub fn button_sprite_overrides(&self, token: &str, tag: Option<&str>) -> Vec<String> {
        if self.icon_present { self.sprite_overrides(token, tag) } else { Vec::new() }
    }

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Database {
    National,
    Continuous,
}

#[derive(Clone, Copy)]
enum Node {
    NationalFile,
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

pub fn parse(
    input: &[u8],
    database: Database,
    names: &KeyNames,
) -> Result<(Definitions, Vec<crate::Diagnostic>), (crate::DiagnosticKind, io::Error)> {
    use Node::*;
    let mut undecided = Vec::new();
    let mut refused = Vec::new();
    let top = match database {
        Database::National => NationalFile,
        Database::Continuous => ContinuousFile,
    };
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
                        Shape::EffectIn(0)
                    } else {
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
    fn vectors(items: &[Item], inlay: bool) -> io::Result<()> {
        for item in items {
            if let Some(children) = item.value.items() {
                if item.keyword_is("position") || item.keyword_is("continuous_focus_position") {
                    let valid = !inlay && children.len() == 2 && children.iter().all(Item::number)
                        || children.len() == 6
                            && children[0].value.scalar().is_some_and(|v| {
                                content_reader_parser::parser::c_string(v).eq_ignore_ascii_case(b"x")
                            })
                            && children[1].value.scalar_str() == Some("=")
                            && children[2].number()
                            && children[3].value.scalar().is_some_and(|v| {
                                content_reader_parser::parser::c_string(v).eq_ignore_ascii_case(b"y")
                            })
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
                let mut icon_present = false;
                for field in children {
                    let target = if field.keyword_is("icon") {
                        icon_present = true;
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
                        icon_present,
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

pub fn read(layers: &ResolvedLayers) -> io::Result<(Focuses, Vec<crate::Diagnostic>)> {
    let national = read_store(layers, Database::National)?;
    let continuous = read_store(layers, Database::Continuous)?;
    let mut files: Vec<_> = national.notes.iter().chain(&continuous.notes).collect();
    files.sort_by(|a, b| (a.layer, &a.path).cmp(&(b.layer, &b.path)));
    let warnings = files.into_iter().flat_map(|file| file.notes.iter().cloned()).collect();
    Ok((Focuses { national: Arc::clone(&national.store), continuous: Arc::clone(&continuous.store) }, warnings))
}

pub fn read_national(layers: &ResolvedLayers) -> io::Result<(Focuses, Vec<crate::Diagnostic>)> {
    read_database(layers, Database::National)
}

pub fn read_database(layers: &ResolvedLayers, database: Database) -> io::Result<(Focuses, Vec<crate::Diagnostic>)> {
    let read = read_store(layers, database)?;
    let warnings = read.notes.iter().flat_map(|file| file.notes.iter().cloned()).collect();
    let store = Arc::clone(&read.store);
    let definitions = match database {
        Database::National => Focuses { national: store, ..Default::default() },
        Database::Continuous => Focuses { continuous: store, ..Default::default() },
    };
    Ok((definitions, warnings))
}

fn focus_database(path: &[u8]) -> Option<Database> {
    let starts = |prefix: &[u8]| path.get(..prefix.len()).is_some_and(|part| part.eq_ignore_ascii_case(prefix));
    if starts(b"common/national_focus/") {
        Some(Database::National)
    } else if starts(b"common/continuous_focus/") {
        Some(Database::Continuous)
    } else {
        None
    }
}

fn database_paths(
    layers: &ResolvedLayers,
    database: Option<Database>,
) -> io::Result<Vec<(usize, ScriptPath)>> {
    let mut paths = layers
        .effective_paths()?
        .filter(|(path, _)| {
            path.ends_with(b".txt") && focus_database(path).is_some_and(|found| database.is_none_or(|wanted| wanted == found))
        })
        .map(|(path, layer)| (layer, ScriptPath::new(path.to_vec())))
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

struct FileNotes {
    layer: usize,
    path: ScriptPath,
    notes: Vec<crate::Diagnostic>,
}

struct ReadStore {
    store: Arc<Store>,
    notes: Vec<FileNotes>,
}

fn read_store(layers: &ResolvedLayers, database: Database) -> io::Result<Arc<ReadStore>> {
    let product = match database {
        Database::National => "national focus definitions",
        Database::Continuous => "continuous focus definitions",
    };
    layers.retained_product(product, 1, &[], |layers| {
        let paths = database_paths(layers, Some(database))?;
        let names = layers.key_names()?;
        let workers = std::thread::available_parallelism().map_or(1, usize::from);
        let (definitions, notes) = parse_files(&paths, &names, workers, |path| path.read(layers), parse)?;
        let store = match database {
            Database::National => definitions.national,
            Database::Continuous => definitions.continuous,
        };
        Ok(ReadStore { store, notes })
    })
}

type ParsedFile =
    Result<(Definitions, Vec<crate::Diagnostic>), (crate::DiagnosticKind, io::Error)>;

fn parse_files<B: AsRef<[u8]> + std::ops::Deref<Target = [u8]> + Sync>(
    paths: &[(usize, ScriptPath)],
    names: &KeyNames,
    workers: usize,
    mut read: impl FnMut(&ScriptPath) -> io::Result<B>,
    parser: impl Fn(&[u8], Database, &KeyNames) -> ParsedFile + Sync,
) -> io::Result<(Focuses, Vec<FileNotes>)> {
    let mut result = Focuses::default();
    let mut warnings = Vec::new();
    for wave in paths.chunks(workers.max(1)) {
        let files = wave
            .iter()
            .map(|(_, path)| read(path).map(|bytes| (path, bytes)))
            .collect::<io::Result<Vec<_>>>()?;
        let database_of =
            |path: &ScriptPath| focus_database(path.as_bytes()).unwrap_or(Database::Continuous);
        let parse_one = |(path, bytes): &(&ScriptPath, B)| {
            parser(bytes, database_of(path), names)
        };
        let parsed = if files.len() == 1 {
            vec![parse_one(&files[0])]
        } else {
            std::thread::scope(|scope| {
                let jobs = files
                    .iter()
                    .map(|file| {
                        let priority = job_priority::JobPriority::current();
                        let cpu = job_priority::accounting::Context::capture();
                        let memory = engine_allocation::labels::current();
                        job_priority::spawn_in(scope, move || {
                            let _memory = engine_allocation::labels::resume(memory);
                            let _cpu = cpu.enter("content focus");
                            priority.enter(job_priority::Level::Normal);
                            parse_one(file)
                        })
                    })
                    .collect::<Vec<_>>();
                jobs.into_iter()
                    .map(|job| {
                        job.join()
                            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
                    })
                    .collect::<Vec<_>>()
            })
        };
        for ((layer, path), parsed) in wave.iter().zip(parsed) {
            let notes = match parsed {
                Ok((definitions, notes)) => {
                    result.read_file(database_of(path), definitions);
                    notes.into_iter().map(|(kind, note)| (kind, format!("{path}: {note}"))).collect()
                }
                Err((kind, error)) => {
                    result.refuse_file(database_of(path), &path.to_string(), error.to_string());
                    vec![(kind, format!("Focus definitions unavailable in {path}: {error}"))]
                }
            };
            if !notes.is_empty() {
                warnings.push(FileNotes { layer: *layer, path: path.clone(), notes });
            }
        }
    }
    Ok((result, warnings))
}

#[cfg(test)]
#[path = "focus_read_tests.rs"]
mod read_tests;

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
        content_reader_script::test_game::enter();
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
        assert_eq!(
            definitions.get("SYN_FOCUS").unwrap().unwrap().localisation,
            "SYN_NATIONAL"
        );
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn layer_warnings_name_and_count_deferred_effect_readers() -> io::Result<()> {
        content_reader_script::test_game::enter();
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
        assert!(definitions.get("test").unwrap().is_some());
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
        content_reader_script::test_game::enter();
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
        content_reader_script::test_game::enter();
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
        content_reader_script::test_game::enter();
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
        content_reader_script::test_game::enter();
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
        content_reader_script::test_game::enter();
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
        content_reader_script::test_game::enter();
        let text = br#"focus_tree={focus={id=A}} continuous_focus_palette={focus={id=E}} shared_focus={id=B}"#;
        let national = parse(text, Database::National, &KeyNames::default()).unwrap();
        assert_eq!(national.keys().collect::<Vec<_>>(), ["A", "B"]);
        let continuous = parse(text, Database::Continuous, &KeyNames::default()).unwrap();
        assert_eq!(continuous.keys().collect::<Vec<_>>(), ["E"]);
    }

    #[test]
    fn a_failed_repeated_cost_keeps_the_stored_one() {
        content_reader_script::test_game::enter();
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
        content_reader_script::test_game::enter();
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
        content_reader_script::test_game::enter();
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
        content_reader_script::test_game::enter();
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

#[cfg(test)]
mod sprite_override_tests {
    use super::*;

    fn definition(source: &[u8], id: &str, database: Database) -> FocusDefinition {
        parse(source, database, &KeyNames::default()).unwrap().0[id].clone()
    }

    #[test]
    fn only_a_focus_with_an_icon_field_tries_the_qualified_sprites_on_its_button() {
        content_reader_script::test_game::enter();
        for (source, database) in [
            (&b"focus_tree={focus={id=f_icon icon=GFX_own}}"[..], Database::National),
            (b"continuous_focus_palette={focus={id=f_icon icon=GFX_own}}", Database::Continuous),
        ] {
            assert_eq!(
                definition(source, "f_icon", database).button_sprite_overrides("f_icon", Some("SYN")),
                ["GFX_SYN_f_icon", "GFX_f_icon"]
            );
        }
        let empty = definition(b"focus_tree={focus={id=f_empty icon={}}}", "f_empty", Database::National);
        assert_eq!(empty.button_sprite_overrides("f_empty", Some("SYN")), ["GFX_SYN_f_empty", "GFX_f_empty"]);
        let bare = definition(b"focus_tree={focus={id=f_bare}}", "f_bare", Database::National);
        assert!(bare.button_sprite_overrides("f_bare", Some("SYN")).is_empty());
        assert_eq!(bare.sprite_overrides("f_bare", Some("SYN")), ["GFX_SYN_f_bare", "GFX_f_bare"]);
    }
}
