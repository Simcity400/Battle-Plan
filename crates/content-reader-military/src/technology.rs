use content_reader_key_names::key_name_products::KeyNamesExt as _;
use content_reader_script::key_names::KeyNames;
use content_reader_layers::ResolvedLayers;
use content_reader_parser::{fields::invalid, parser::{self, Item, fold_token}};
use content_reader_script::{modifiers, native::{self, Shape}, scope::ScopeKind};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};
mod descriptions;
pub(crate) mod units;
pub use descriptions::{Catalog, Definition};


pub type EquipmentLinks = BTreeMap<String, Option<String>>;
type ParsedPresentation = (Vec<Template>, Vec<content_reader_parser::Diagnostic>);
type ParseError = (content_reader_parser::DiagnosticKind, io::Error, bool);

#[derive(Debug)]
struct NativeEnabler {
    candidate: EquipmentName,
    unresolved: bool,
}

#[derive(Debug)]
pub struct Template {
    pub token: String,
    pub database_index: Option<u32>,
    pub presentation: Result<PresentationName, String>,
    pub parent: Result<Option<String>, String>,
    pub equipment: Option<String>,
    pub folders: Vec<String>,
    enablers: Vec<NativeEnabler>,
}

fn equipment_links(definitions: &[Template]) -> EquipmentLinks {
    let mut links = EquipmentLinks::new();
    let mut seen = BTreeSet::new();
    for definition in definitions {
        if seen.insert(definition.token.to_ascii_lowercase()) {
            links.insert(definition.token.clone(), definition.equipment.clone());
        }
    }
    links
}

#[derive(Clone, Copy)]
enum Node {
    File,
    Technologies,
    Technology,
    Path,
    Folder,
    Scalars,
    Building,
    Mtth,
    Uniform,
    Effect,
    Unit(units::Node),
    Modifier(modifiers::Node),
}

pub fn parse_equipment(
    input: &[u8],
    names: &KeyNames,
) -> io::Result<(EquipmentLinks, Vec<content_reader_parser::Diagnostic>)> {
    parse_with_registry(input, &units::Registry::default(), names)
        .map(|(definitions, notes)| (equipment_links(&definitions), notes))
        .map_err(|(_, error, _)| error)
}

fn parse_with_registry(
    input: &[u8],
    registry: &units::Registry,
    names: &KeyNames,
) -> Result<ParsedPresentation, ParseError> {
    parse_with_tokens(input, registry, names, &mut Vec::new())
}

fn parse_with_tokens(
    input: &[u8],
    registry: &units::Registry,
    names: &KeyNames,
    tokens: &mut Vec<String>,
) -> Result<ParsedPresentation, ParseError> {
    let (read, undecided, refused) = read_templates_with_tokens(input, registry, names, tokens);
    if let Some(cause) = read.unsettled {
        return Err((cause.kind, invalid(format!("technology: {cause}")), true));
    }
    if let Some(cause) = refused.first() {
        return Err((content_reader_parser::DiagnosticKind::Refused, invalid(cause), false));
    }
    if let Some(cause) = undecided.first() {
        return Err((
            content_reader_parser::DiagnosticKind::Unsettled,
            invalid(format!("technology: {cause}")),
            false,
        ));
    }
    let mut definitions = Vec::new();
    let mut folder_notes = Vec::new();
    for root in read
        .items
        .iter()
        .filter(|item| item.keyword_is("technologies"))
    {
        for technology in root.value.items().unwrap_or_default() {
            let token = std::str::from_utf8(content_reader_parser::parser::c_string(
                technology.key.as_deref().unwrap_or_default(),
            ))
            .map_err(|_| {
                (
                    content_reader_parser::DiagnosticKind::Refused,
                    invalid("technology token is not UTF-8"),
                    false,
                )
            })?;
            let parent = technology
                .value
                .items()
                .unwrap_or_default()
                .iter()
                .rfind(|field| field.keyword_is("parent"))
                .map(|parent| {
                    parent
                        .value
                        .c_str()
                        .map_err(|error| format!("technology {token} parent: {error}"))
                        .map(str::to_owned)
                })
                .transpose();
            let mut unlocks: Vec<String> = Vec::new();
            let mut conditional = false;
            let mut candidates = Vec::new();
            let mut native_enablers = Vec::new();
            let mut placed: Vec<String> = Vec::new();
            for field in technology.value.items().unwrap_or_default() {
                if field.keyword_is("folder") {
                    if let Some(name) = field
                        .value
                        .items()
                        .unwrap_or_default()
                        .iter()
                        .rfind(|item| item.keyword_is("name"))
                    {
                        match name.value.read_str() {
                            Ok(name) => {
                                if !placed.iter().any(|old| old == name) {
                                    placed.push(name.to_owned());
                                }
                            }
                            Err(error) => folder_notes.push((
                                content_reader_parser::DiagnosticKind::Refused,
                                format!("technology {token} folder name: {error}"),
                            )),
                        }
                    }
                    for position in field
                        .value
                        .items()
                        .unwrap_or_default()
                        .iter()
                        .filter(|i| i.keyword_is("position"))
                    {
                        let values = position.value.items().unwrap_or_default();
                        let valid = values.len() == 2 && values.iter().all(Item::number)
                            || values.len() == 6
                                && values[0].value_keyword().as_deref() == Some("x")
                                && values[1].value.scalar() == Some(b"=")
                                && values[2].number()
                                && values[3].value_keyword().as_deref() == Some("y")
                                && values[4].value.scalar() == Some(b"=")
                                && values[5].number();
                        if !valid {
                            return Err((
                                content_reader_parser::DiagnosticKind::Unsupported,
                                invalid(format!(
                                    "technology {token} folder position: unsupported vector form"
                                )),
                                false,
                            ));
                        }
                    }
                }
                if !field.keyword_is("enable_equipments") {
                    continue;
                }
                let mut limits = Vec::new();
                for equipment in field.value.items().unwrap_or_default() {
                    if equipment.keyword_is("limit") {
                        conditional = true;
                        limits.push(equipment.value.items().unwrap_or_default().to_vec());
                        continue;
                    }
                    if equipment.value_keyword().as_deref() == Some("limit")
                        || matches!(equipment.kind, native::Kind::Open | native::Kind::Equals)
                    {
                        return Err((
                            content_reader_parser::DiagnosticKind::Unsupported,
                            invalid(format!(
                                "technology {token} enable_equipments: unsupported conditional or nested unlock"
                            )),
                            false,
                        ));
                    }
                    let name = equipment
                        .value
                        .read_str()
                        .map_err(|e| invalid(format!("technology {token} enable_equipments: {e}")))
                        .map_err(|error| (content_reader_parser::DiagnosticKind::Refused, error, false))?;
                    if !unlocks.iter().any(|old| old.eq_ignore_ascii_case(name)) {
                        unlocks.push(name.into());
                    }
                    let candidate = EquipmentName {
                        name: name.into(),
                        limits: limits.clone(),
                    };
                    native_enablers.push(NativeEnabler {
                        candidate: candidate.clone(),
                        unresolved: !equipment.quoted() && name.starts_with('@'),
                    });
                    candidates.push(candidate);
                }
            }
            let fields = technology.value.items().unwrap_or_default();
            let force_small = parser::read_bool_field(fields, "force_use_small_tech_layout", false);
            let presentation = parent.clone().map(|parent| {
                let template_key = parent.unwrap_or_else(|| token.into());
                let template_key = if template_key.is_empty() {
                    token.to_owned()
                } else {
                    template_key
                };
                PresentationName {
                    template: template_key,
                    equipment: candidates,
                    force_small,
                }
            });
            let equipment = (!conditional && unlocks.len() == 1).then(|| unlocks.remove(0));
            definitions.push(Template {
                token: token.to_owned(),
                database_index: None,
                presentation,
                parent,
                equipment,
                folders: placed,
                enablers: native_enablers,
            });
        }
    }
    Ok((
        definitions,
        read.misreads
            .iter()
            .map(|note| (note.kind, note.to_string()))
            .chain(folder_notes)
            .collect(),
    ))
}

#[derive(Debug, PartialEq)]
pub struct PresentationName {
    pub template: String,
    pub equipment: Vec<EquipmentName>,
    pub force_small: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EquipmentName {
    pub name: String,
    pub limits: Vec<Vec<Item>>,
}

impl EquipmentName {
    pub fn admitted(
        &self,
        evaluate: &mut impl FnMut(&[Item]) -> Result<bool, String>,
    ) -> Result<bool, String> {
        for limit in &self.limits {
            if !evaluate(limit).map_err(|why| {
                format!(
                    "enable_equipments limit for {} at content load: {why}",
                    self.name
                )
            })? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

impl PresentationName {
    pub fn name(
        &self,
        evaluate: impl FnMut(&[Item]) -> Result<bool, String>,
        resolve: impl FnMut(&str) -> Result<Option<String>, String>,
    ) -> Result<String, String> {
        let mut children = self.children(evaluate, resolve)?;
        Ok(if children.len() == 1 {
            children.remove(0)
        } else {
            self.template.clone()
        })
    }

    pub fn children(
        &self,
        mut evaluate: impl FnMut(&[Item]) -> Result<bool, String>,
        mut resolve: impl FnMut(&str) -> Result<Option<String>, String>,
    ) -> Result<Vec<String>, String> {
        if self.force_small {
            return Ok(Vec::new());
        }
        let mut decided: Vec<(&[Item], Result<bool, usize>)> = Vec::new();
        let mut undecided: Vec<String> = Vec::new();
        let mut entries: Vec<(String, &str, BTreeSet<usize>)> = Vec::new();
        'entries: for candidate in &self.equipment {
            let mut waits = BTreeSet::new();
            for limit in &candidate.limits {
                let outcome = match decided.iter().find(|(seen, _)| *seen == limit.as_slice()) {
                    Some((_, outcome)) => *outcome,
                    None => {
                        let outcome = evaluate(limit).map_err(|why| {
                            undecided.push(format!(
                                "enable_equipments limit for {} at content load: {why}",
                                candidate.name
                            ));
                            undecided.len() - 1
                        });
                        decided.push((limit.as_slice(), outcome));
                        outcome
                    }
                };
                match outcome {
                    Ok(false) => continue 'entries,
                    Ok(true) => {}
                    Err(variable) => {
                        waits.insert(variable);
                    }
                }
            }
            entries.push((
                candidate.name.to_ascii_lowercase(),
                candidate.name.as_str(),
                waits,
            ));
        }
        let mut names: Vec<(&str, Vec<usize>)> = Vec::new();
        for (at, (folded, _, _)) in entries.iter().enumerate() {
            match names.iter_mut().find(|(name, _)| name == folded) {
                Some((_, places)) => places.push(at),
                None => names.push((folded.as_str(), vec![at])),
            }
        }
        let why = |places: &mut dyn Iterator<Item = usize>| {
            places
                .filter_map(|at| entries[at].2.first().copied())
                .min()
                .map_or_else(String::new, |variable| undecided[variable].clone())
        };
        let first: Vec<Vec<usize>> = names
            .iter()
            .map(|(_, places)| {
                places
                    .iter()
                    .enumerate()
                    .filter(|&(index, &at)| {
                        !places[..index]
                            .iter()
                            .any(|&earlier| entries[earlier].2.is_subset(&entries[at].2))
                    })
                    .map(|(_, &at)| at)
                    .collect()
            })
            .collect();
        let absent: Vec<bool> = names
            .iter()
            .map(|(_, places)| places.iter().all(|&at| !entries[at].2.is_empty()))
            .collect();
        let can_precede = |later: &[usize], first: &[usize]| {
            later.iter().any(|&at| {
                first
                    .iter()
                    .filter(|&&earlier| earlier < at)
                    .all(|&earlier| !entries[earlier].2.is_subset(&entries[at].2))
            })
        };
        let mut swaps = Vec::new();
        for a in 0..names.len() {
            for b in a + 1..names.len() {
                let (one, other) = (names[a].1.as_slice(), names[b].1.as_slice());
                if can_precede(one, other) && can_precede(other, one) {
                    swaps.push((a, b));
                }
            }
        }
        let settled = swaps.is_empty()
            && (0..names.len()).all(|name| {
                !absent[name]
                    && first[name]
                        .iter()
                        .all(|&at| entries[at].1 == entries[first[name][0]].1)
            });
        let mut child: Vec<Option<String>> = Vec::with_capacity(names.len());
        if !settled {
            let mut resolved: BTreeMap<&str, Option<String>> = BTreeMap::new();
            let variables: BTreeSet<usize> = entries
                .iter()
                .flat_map(|(_, _, waits)| waits.iter().copied())
                .collect();
            if let (1, Some(&variable)) = (variables.len(), variables.first()) {
                let mut outcomes: Vec<Vec<String>> = Vec::with_capacity(2);
                for holds in [false, true] {
                    let mut seen: Vec<&str> = Vec::new();
                    let mut shown = Vec::new();
                    for (folded, spelling, waits) in &entries {
                        if !(holds || waits.is_empty()) || seen.contains(&folded.as_str()) {
                            continue;
                        }
                        seen.push(folded.as_str());
                        let spelling: &str = *spelling;
                        let value = match resolved.get(spelling) {
                            Some(value) => value.clone(),
                            None => {
                                let value = resolve(spelling)?;
                                resolved.insert(spelling, value.clone());
                                value
                            }
                        };
                        shown.extend(value);
                    }
                    outcomes.push(shown);
                }
                return if outcomes[0] == outcomes[1] {
                    Ok(outcomes.swap_remove(1))
                } else {
                    Err(undecided[variable].clone())
                };
            }
            for (name, (_, places)) in names.iter().enumerate() {
                let mut shown: Vec<Option<String>> = Vec::new();
                if absent[name] {
                    shown.push(None);
                }
                for &at in &first[name] {
                    let spelling = entries[at].1;
                    let value = match resolved.get(spelling) {
                        Some(value) => value.clone(),
                        None => {
                            let value = resolve(spelling)?;
                            resolved.insert(spelling, value.clone());
                            value
                        }
                    };
                    if !shown.contains(&value) {
                        shown.push(value);
                    }
                }
                if shown.len() > 1 {
                    return Err(why(&mut places.iter().copied()));
                }
                child.push(shown.pop().flatten());
            }
            for &(a, b) in &swaps {
                if child[a].is_some() && child[b].is_some() && child[a] != child[b] {
                    return Err(why(&mut names[a].1.iter().chain(&names[b].1).copied()));
                }
            }
        }
        let mut order: Vec<(usize, usize)> = (0..names.len())
            .filter_map(|name| {
                names[name]
                    .1
                    .iter()
                    .copied()
                    .find(|&entry| entries[entry].2.is_empty())
                    .map(|entry| (entry, name))
            })
            .collect();
        order.sort();
        let mut children = Vec::with_capacity(order.len());
        for (entry, name) in order {
            let shown = if settled {
                resolve(entries[entry].1)?
            } else {
                child[name].take()
            };
            children.extend(shown);
        }
        Ok(children)
    }
}

#[derive(Debug)]
pub struct Database {
    pub definitions: Vec<Template>,
    pub tokens: Vec<String>,
    pub equipment_warnings: Vec<content_reader_parser::Diagnostic>,
    pub inventory_warnings: Vec<content_reader_parser::Diagnostic>,
    pub unread: Vec<UnreadFile>,
}

#[derive(Debug)]
pub struct UnreadFile {
    pub why: String,
    pub tokens: Option<BTreeSet<String>>,
}

impl Database {
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let registry = units::Registry::read(layers)?;
        let names = layers.key_names()?;
        let mut definitions = Vec::new();
        let mut tokens = Vec::new();
        let mut equipment_warnings = Vec::new();
        let mut inventory_warnings = Vec::new();
        let mut unread = Vec::new();
        let mut placed = true;
        for path in layers.directory_files("common/technologies", ".txt")? {
            let before = tokens.len();
            match parse_with_tokens(&path.read(layers)?, &registry, &names, &mut tokens) {
                Ok((read, notes)) => {
                    placed &= read.len() == tokens.len() - before;
                    for mut definition in read {
                        definition.database_index = placed
                            .then(|| u32::try_from(definitions.len() + 1).ok())
                            .flatten();
                        definitions.push(definition);
                    }
                    let notes: Vec<_> = notes
                        .into_iter()
                        .map(|(kind, note)| (kind, format!("{path}: {note}")))
                        .collect();
                    equipment_warnings.extend(notes.iter().cloned());
                    inventory_warnings.extend(notes);
                }
                Err((kind, error, stopped)) => {
                    placed = false;
                    unread.push(UnreadFile {
                        why: format!("{path}: {error}"),
                        tokens: (!stopped).then(|| {
                            tokens[before..]
                                .iter()
                                .map(|token| token.to_ascii_lowercase())
                                .collect()
                        }),
                    });
                    equipment_warnings.push((
                        kind,
                        format!("Technology equipment links unavailable in {path}: {error}"),
                    ));
                    inventory_warnings.push((
                        kind,
                        format!("{path}: technology inventory may be incomplete: {error}"),
                    ));
                }
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        tokens.retain(|token| seen.insert(token.clone()));
        Ok(Self {
            definitions,
            tokens,
            equipment_warnings,
            inventory_warnings,
            unread,
        })
    }

    pub fn equipment(&self) -> BTreeMap<String, String> {
        equipment_links(&self.definitions)
            .into_iter()
            .filter_map(|(token, equipment)| equipment.map(|equipment| (token, equipment)))
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct FolderDefinition {
    pub key: String,
    pub available: Option<Vec<Item>>,
}

#[derive(Clone, Debug, Default)]
pub struct Folders {
    pub folders: Vec<FolderDefinition>,
    pub warnings: Vec<content_reader_parser::Diagnostic>,
}

pub fn read_folders(layers: &ResolvedLayers) -> io::Result<Folders> {
    let names = layers.key_names()?;
    let mut read = Folders::default();
    for path in layers.directory_files("common/technology_tags", ".txt")? {
        match parse_folders(&path.read(layers)?, &names) {
            Ok((folders, notes)) => {
                read.warnings.extend(
                    notes
                        .into_iter()
                        .map(|(kind, note)| (kind, format!("{path}: {note}"))),
                );
                for folder in folders {
                    if read.folders.iter().any(|old| old.key == folder.key) {
                        read.warnings.push((
                            content_reader_parser::DiagnosticKind::Refused,
                            format!(
                                "{path}: research folder {} is defined again; its first definition is used",
                                folder.key
                            ),
                        ));
                    } else {
                        read.folders.push(folder);
                    }
                }
            }
            Err(error) => read.warnings.push((
                content_reader_parser::DiagnosticKind::Unsettled,
                format!("{path}: research folders unavailable: {error}"),
            )),
        }
    }
    Ok(read)
}

pub fn parse_folders(
    input: &[u8],
    names: &KeyNames,
) -> io::Result<(Vec<FolderDefinition>, Vec<content_reader_parser::Diagnostic>)> {
    #[derive(Clone, Copy)]
    enum Tag {
        File,
        Folders,
        Folder,
    }
    let read = native::read_in_effect_scope(
        input,
        native::Mode::KeepMark,
        native::Top::File(Tag::File),
        Some(names),
        ScopeKind::Country.bit(),
        &mut |node, field| match (node, field.keyword().as_deref()) {
            (Tag::File, Some(b"technology_folders")) => Shape::Object(Tag::Folders),
            (Tag::File, Some(b"technology_categories")) => Shape::RawList,
            (Tag::File, _) => Shape::Skip,
            (Tag::Folders, _) => Shape::Object(Tag::Folder),
            (Tag::Folder, Some(b"available")) => Shape::Trigger,
            (Tag::Folder, Some(b"ledger")) => Shape::One,
            (Tag::Folder, _) => Shape::Skip,
        },
    );
    if let Some(cause) = read.unsettled {
        return Err(invalid(format!("technology folders: {cause}")));
    }
    let mut folders: Vec<FolderDefinition> = Vec::new();
    let mut notes: Vec<content_reader_parser::Diagnostic> = read
        .misreads
        .iter()
        .map(|note| (note.kind, note.to_string()))
        .collect();
    for root in read
        .items
        .iter()
        .filter(|item| item.keyword_is("technology_folders"))
    {
        for folder in root.value.items().unwrap_or_default() {
            let Ok(key) = std::str::from_utf8(content_reader_parser::parser::c_string(
                folder.key.as_deref().unwrap_or_default(),
            )) else {
                notes.push((
                    content_reader_parser::DiagnosticKind::Refused,
                    "research folder key is not UTF-8".into(),
                ));
                continue;
            };
            if key.is_empty() {
                continue;
            }
            if folders.iter().any(|old| old.key == key) {
                notes.push((
                    content_reader_parser::DiagnosticKind::Refused,
                    format!("research folder {key} is defined again; its first definition is used"),
                ));
                continue;
            }
            let available = folder
                .value
                .items()
                .unwrap_or_default()
                .iter()
                .rfind(|field| field.keyword_is("available"))
                .map(|field| field.value.items().unwrap_or_default().to_vec());
            folders.push(FolderDefinition {
                key: key.to_owned(),
                available,
            });
        }
    }
    Ok((folders, notes))
}

pub fn read_equipment(
    layers: &ResolvedLayers,
) -> io::Result<(BTreeMap<String, String>, Vec<content_reader_parser::Diagnostic>)> {
    let read = Database::read(layers)?;
    Ok((read.equipment(), read.equipment_warnings))
}

fn read_templates(
    input: &[u8],
    registry: &units::Registry,
    names: &KeyNames,
) -> (native::Read, Vec<modifiers::Undecided>, Vec<String>) {
    read_templates_with_tokens(input, registry, names, &mut Vec::new())
}

fn read_templates_with_tokens(
    input: &[u8],
    registry: &units::Registry,
    names: &KeyNames,
    tokens: &mut Vec<String>,
) -> (native::Read, Vec<modifiers::Undecided>, Vec<String>) {
    use Node::*;
    let mut undecided = Vec::new();
    let mut refused = Vec::new();
    let mut leading_limit = true;
    let modifier_registry = &|key: &[u8]| modifiers::registered(key, Some(names));
    let read = native::read_in_effect_scope(
        input,
        native::Mode::KeepMark,
        native::Top::File(File),
        Some(names),
        ScopeKind::Country.bit(),
        &mut |node, field| {
            match (node, field.keyword().as_deref()) {
                (File, Some(b"technologies")) => Shape::Object(Technologies),
                (File, Some(b"technology_categories")) => Shape::RawList,
                (File, Some(b"technology_folders")) => {
                    refused.push("technology_folders: unsupported folder database reader".into());
                    Shape::Skip
                }
                (File, _) => Shape::Skip,
                (Technologies, _) => {
                    if let Ok(token) = std::str::from_utf8(content_reader_parser::parser::c_string(&field.key.text))
                    {
                        tokens.push(token.to_owned());
                    } else {
                        refused.push("technology token is not UTF-8".into());
                    }
                    Shape::Object(Technology)
                }
                (
                    Technology,
                    Some(b"allow" | b"unlock" | b"allow_branch" | b"on_research_complete_limit"),
                ) => Shape::Trigger,
                (Technology, Some(b"ai_will_do")) => Shape::Object(Mtth),
                (Technology, Some(b"modifier")) => {
                    Shape::Object(Modifier(modifiers::Node::Modifier))
                }
                (Technology, Some(b"path")) => Shape::Object(Path),
                (Technology, Some(b"folder")) => Shape::Object(Folder),
                (Technology, Some(b"dependencies" | b"ai_research_weights")) => {
                    Shape::Object(Scalars)
                }
                (Technology, Some(b"enable_building")) => Shape::Object(Building),
                (
                    Technology,
                    Some(
                        b"categories"
                        | b"xor"
                        | b"sub_technologies"
                        | b"enable_equipment_modules"
                        | b"special_project_specialization",
                    ),
                ) => Shape::RawList,
                (Technology, Some(b"enable_equipments")) => Shape::TechnologyEquipment,
                (Technology, Some(b"enable_subunits")) if field.opens() => {
                    leading_limit = true;
                    Shape::Statements(Uniform)
                }
                (Technology, Some(b"enable_subunits")) => Shape::One,
                (Uniform, Some(b"limit")) if leading_limit => Shape::Trigger,
                (Uniform, _) => {
                    if field.key.text.is_empty() {
                        leading_limit = false;
                    }
                    if registry
                        .units
                        .contains(&content_reader_parser::parser::c_string(&field.value.text).to_ascii_lowercase())
                    {
                        Shape::One
                    } else {
                        Shape::Skip
                    }
                }
                (Technology, Some(b"on_research_complete")) => Shape::LimitedBody(Effect),
                (Effect, _) => content_reader_script::effect_keys::shape(Some(names), field).map(|()| Effect),
                (Technology, Some(b"parent" | b"enable_tactic")) => Shape::Current,
                (
                    Technology,
                    Some(
                        b"is_special_project_tech"
                        | b"desc"
                        | b"research_cost"
                        | b"hidden"
                        | b"start_year"
                        | b"max_level"
                        | b"xp_research_bonus"
                        | b"doctrine"
                        | b"show_equipment_icon"
                        | b"show_effect_as_desc"
                        | b"force_use_small_tech_layout"
                        | b"xp_research_cost"
                        | b"xp_research_type"
                        | b"doctrine_name"
                        | b"sub_tech_index"
                        | b"xp_unlock_cost"
                        | b"xp_boost_cost",
                    ),
                ) => Shape::One,
                (Technology, Some(key))
                    if registry.units.contains(key) || registry.categories.contains(key) =>
                {
                    Shape::Object(Unit(units::Node::Stats))
                }
                (Technology, _) => modifiers::shape(
                    modifiers::Node::Modifier,
                    field,
                    modifier_registry,
                    &mut undecided,
                )
                .map(Modifier),
                (Path, Some(b"leads_to_tech" | b"research_cost_coeff" | b"ignore_for_layout")) => {
                    Shape::One
                }
                (Folder, Some(b"name")) => Shape::One,
                (Folder, Some(b"position")) if field.opens() => Shape::RawList,
                (Folder | Path, _) => Shape::Skip,
                (Scalars | Building, _) => Shape::One,
                (Mtth, _) => native::mean_time_to_happen(field),
                (Modifier(node), _) => {
                    modifiers::shape(node, field, modifier_registry, &mut undecided).map(Modifier)
                }
                (Unit(node), _) => registry
                    .shape(node, field, &mut undecided, &mut refused)
                    .map(Unit),
            }
        },
    );
    (read, undecided, refused)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enabler {
    pub id: String,
    pub parent: Result<Option<String>, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enables {
    pub id: String,
    pub equipment: Vec<String>,
    pub parent: Result<Option<String>, String>,
    pub scripted: Option<Scripted>,
    pub limited: Vec<EquipmentName>,
}

impl Enables {
    fn from_template(
        native: Template,
        evaluate: &mut impl FnMut(&[Item]) -> Result<bool, String>,
    ) -> Self {
        let mut equipment = Vec::new();
        let mut limited = Vec::new();
        let mut scripted: Option<Scripted> = None;
        for entry in native.enablers {
            if entry.unresolved {
                let admitted = entry.candidate.limits.is_empty()
                    || !matches!(entry.candidate.admitted(&mut *evaluate), Ok(false));
                if admitted {
                    scripted.get_or_insert_with(Scripted::default).any = true;
                }
            } else if entry.candidate.limits.is_empty() {
                equipment.push(entry.candidate.name);
            } else {
                limited.push(entry.candidate);
            }
        }
        Self {
            id: native.token,
            equipment,
            parent: native.parent,
            scripted,
            limited,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scripted {
    pub names: Vec<String>,
    pub any: bool,
}

pub type EnabledBy = Vec<Enables>;

fn mentioned(item: &parser::Item, scripted: &mut Scripted) {
    let mut note = |bytes: &[u8]| {
        if bytes.starts_with(b"@") {
            scripted.any = true;
        } else if let Ok(name) = std::str::from_utf8(bytes) {
            scripted.names.push(name.to_owned());
        }
    };
    if let Some(key) = &item.key {
        note(key);
    }
    match &item.value {
        parser::Value::Scalar(bytes) => note(bytes),
        parser::Value::Block(items) | parser::Value::Tagged { items, .. } => {
            for item in items {
                mentioned(item, scripted);
            }
        }
    }
}

pub fn parse_enablers(input: &[u8]) -> io::Result<EnabledBy> {
    let document = parser::parse(input)?;
    let mut result = Vec::new();
    for root in document
        .items
        .iter()
        .filter(|item| item.keyword_is("technologies"))
    {
        for technology in root.value.items().unwrap_or_default() {
            let (Some(key), Some(fields)) = (&technology.key, technology.value.items()) else {
                continue;
            };
            let Ok(token) = std::str::from_utf8(key) else {
                continue;
            };
            let mut enables = Enables {
                id: token.to_owned(),
                equipment: Vec::new(),
                parent: Ok(None),
                scripted: None,
                limited: Vec::new(),
            };
            for field in fields {
                if field.keyword_is("enable_equipments") {
                    let mut limited = false;
                    for item in field.value.items().unwrap_or_default() {
                        if item.keyword_is("limit") {
                            limited = true;
                            continue;
                        }
                        match (item.key.is_none(), item.value.scalar_str()) {
                            (true, Some(name)) if !name.starts_with('@') && !limited => {
                                enables.equipment.push(name.to_owned())
                            }
                            _ => mentioned(
                                item,
                                enables.scripted.get_or_insert_with(Scripted::default),
                            ),
                        }
                    }
                } else if field.keyword_is("parent") {
                    enables.parent = field
                        .value
                        .c_str()
                        .map(|parent| Some(parent.to_owned()))
                        .map_err(|error| format!("technology {token} parent: {error}"));
                }
            }
            result.push(enables);
        }
    }
    Ok(result)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Enablers {
    pub plain: BTreeMap<String, Vec<Enabler>>,
    pub uncertain: BTreeMap<String, BTreeSet<String>>,
    pub any: Vec<(String, BTreeSet<String>)>,
    pub refused: Vec<String>,
    pub unverified: Vec<String>,
}

impl Enablers {
    pub fn single(&self, key: &str) -> Result<Option<&Enabler>, String> {
        let key = fold_token(key);
        let plain = self.plain.get(&key).map(Vec::as_slice).unwrap_or_default();
        if plain.len() > 1 {
            return Ok(None);
        }
        if !self.unverified.is_empty() {
            return Err(format!(
                "technology enablers for {key} are withheld because native technology reading failed: {}",
                self.unverified.join("; ")
            ));
        }
        let uncertain: Vec<&str> = self
            .uncertain
            .get(&key)
            .into_iter()
            .flatten()
            .chain(
                self.any
                    .iter()
                    .filter(|(_, certain)| !certain.contains(&key))
                    .map(|(id, _)| id),
            )
            .map(String::as_str)
            .collect();
        if !uncertain.is_empty() {
            return Err(format!(
                "technology {} may enable {key} through a script, which is not supported",
                uncertain.join(", ")
            ));
        }
        Ok(plain.first())
    }

    fn add(
        &mut self,
        source: &str,
        enables: Enables,
        evaluate: &mut impl FnMut(&[Item]) -> Result<bool, String>,
    ) {
        let Enables {
            id,
            mut equipment,
            parent,
            mut scripted,
            limited,
        } = enables;
        for candidate in &limited {
            match candidate.admitted(&mut *evaluate) {
                Ok(true) => equipment.push(candidate.name.clone()),
                Ok(false) => {}
                Err(_) => scripted
                    .get_or_insert_with(Scripted::default)
                    .names
                    .push(candidate.name.clone()),
            }
        }
        let mut certain = BTreeSet::new();
        for name in equipment {
            let key = fold_token(&name);
            if certain.insert(key.clone()) {
                self.plain.entry(key).or_default().push(Enabler {
                    id: id.clone(),
                    parent: parent.clone(),
                });
            }
        }
        let Some(scripted) = scripted else {
            return;
        };
        let mut unknown = false;
        for name in scripted.names {
            let key = fold_token(&name);
            if !certain.contains(&key) {
                self.uncertain.entry(key).or_default().insert(id.clone());
                unknown = true;
            }
        }
        if scripted.any {
            self.any.push((id.clone(), certain));
            unknown = true;
        }
        if unknown {
            self.refused.push(format!("{source}: {id}"));
        }
    }
}

pub fn read_enablers(
    layers: &ResolvedLayers,
    mut evaluate: impl FnMut(&[Item]) -> Result<bool, String>,
) -> io::Result<Enablers> {
    let paths = layers.directory_files("common/technologies", ".txt")?;
    let mut result = Enablers::default();
    if paths.is_empty() {
        return Ok(result);
    }
    let registry = match units::Registry::read(layers) {
        Ok(registry) => registry,
        Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
        Err(error) => {
            result
                .unverified
                .push(format!("native technology unit registry unavailable: {error}"));
            return Ok(result);
        }
    };
    let names = match layers.key_names() {
        Ok(names) => names,
        Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
        Err(error) => {
            result
                .unverified
                .push(format!("native technology key names unavailable: {error}"));
            return Ok(result);
        }
    };
    for path in paths {
        let bytes = match path.read(layers) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
            Err(error) => {
                result.unverified.push(format!("{path}: {error}"));
                continue;
            }
        };
        match parse_with_registry(&bytes, &registry, &names) {
            Ok((definitions, _)) => {
                let source = path.to_string();
                for definition in definitions {
                    let enables = Enables::from_template(definition, &mut evaluate);
                    result.add(&source, enables, &mut evaluate);
                }
            }
            Err((_, error, _)) => {
                result.unverified.push(format!("{path}: {error}"));
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn parse_with_registry(
        input: &[u8],
        registry: &units::Registry,
        names: &KeyNames,
    ) -> Result<
        (
            EquipmentLinks,
            BTreeMap<String, Result<PresentationName, String>>,
            Vec<content_reader_parser::Diagnostic>,
        ),
        ParseError,
    > {
        super::parse_with_registry(input, registry, names).map(|(definitions, notes)| {
            let links = equipment_links(&definitions);
            let mut presentation = BTreeMap::new();
            for definition in definitions {
                presentation
                    .entry(definition.token)
                    .or_insert(definition.presentation);
            }
            (links, presentation, notes)
        })
    }

    #[test]
    fn technologies_keep_their_folders_in_authored_order_only_when_the_file_reads() {
        let (definitions, _) = parse_with_tokens(
            b"technologies = { two = { folder = { name = infantry_folder position = { x = 0 y = 0 } } folder = { name = support_folder position = { x = 1 y = 2 } } folder = { name = infantry_folder } } none = { research_cost = 1 } renamed = { folder = { name = first name = second } } }",
            &units::Registry::default(),
            &KeyNames::default(),
            &mut Vec::new(),
        )
        .unwrap();
        let folders = |token: &str| {
            definitions
                .iter()
                .find(|definition| definition.token == token)
                .unwrap()
                .folders
                .clone()
        };
        assert_eq!(folders("two"), ["infantry_folder", "support_folder"]);
        assert!(folders("none").is_empty());
        assert_eq!(folders("renamed"), ["second"]);
        assert!(
            parse_with_tokens(
                b"technologies = { added = { folder = { name = armour_folder } } broken = { folder = { position = { 1 } } } }",
                &units::Registry::default(),
                &KeyNames::default(),
                &mut Vec::new(),
            )
            .is_err()
        );
    }

    #[test]
    fn a_repeated_technology_is_its_own_template_and_a_name_finds_the_first() {
        content_reader_script::test_game::enter();
        let (definitions, _) = super::parse_with_registry(
            b"technologies = { repeat = { parent = first enable_equipments = { one } } repeat = { parent = second } }",
            &units::Registry::default(),
            &KeyNames::default(),
        )
        .unwrap();
        let templates: Vec<_> = definitions
            .iter()
            .map(|definition| {
                (
                    definition.token.as_str(),
                    definition.presentation.as_ref().unwrap().template.as_str(),
                    definition.equipment.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            templates,
            [
                ("repeat", "first", Some("one")),
                ("repeat", "second", None)
            ]
        );
        assert_eq!(
            equipment_links(&definitions)["repeat"].as_deref(),
            Some("one")
        );
    }

    #[test]
    fn research_folders_keep_file_order_triggers_and_refuse_repeats_by_name() {
        content_reader_script::test_game::enter();
        let (folders, notes) = parse_folders(
            b"technology_categories = { infantry_weapons }\ntechnology_folders = { infantry_folder = { ledger = army } armour_folder = { ledger = army available = { always = no } } nsb_armour_folder = { available = { always = yes } } infantry_folder = { ledger = navy } }",
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(
            folders.iter().map(|f| f.key.as_str()).collect::<Vec<_>>(),
            ["infantry_folder", "armour_folder", "nsb_armour_folder"]
        );
        assert!(folders[0].available.is_none());
        assert!(folders[1].available.as_ref().is_some_and(|t| !t.is_empty()));
        assert!(folders[2].available.is_some());
        assert!(notes.iter().any(|(kind, note)| *kind == content_reader_parser::DiagnosticKind::Refused
            && note == "research folder infantry_folder is defined again; its first definition is used"));
    }

    #[test]
    fn a_mod_adds_research_folders_after_the_base_game_and_cannot_redefine_one() {
        content_reader_script::test_game::enter();
        struct TempRoot(std::path::PathBuf);
        impl Drop for TempRoot {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let root = TempRoot(std::env::temp_dir().join(format!(
            "technology-folders-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )));
        let (base, addon) = (root.0.join("base"), root.0.join("mod"));
        for (layer, path, text) in [
            (
                &base,
                "common/technology_tags/00_technology.txt",
                "technology_folders = { infantry_folder = { ledger = army } air_folder = { ledger = air } }",
            ),
            (
                &addon,
                "common/technology_tags/zz_mod.txt",
                "technology_folders = { mod_folder = { ledger = army } infantry_folder = { ledger = navy } }",
            ),
        ] {
            let file = layer.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        }
        let layers = ResolvedLayers::from_folders(&[(&base, &[]), (&addon, &[])]).unwrap();
        let read = read_folders(&layers).unwrap();
        assert_eq!(
            read.folders
                .iter()
                .map(|f| f.key.as_str())
                .collect::<Vec<_>>(),
            ["infantry_folder", "air_folder", "mod_folder"]
        );
        assert_eq!(
            read.warnings,
            [(
                content_reader_parser::DiagnosticKind::Refused,
                "common/technology_tags/zz_mod.txt: research folder infantry_folder is defined again; its first definition is used".into(),
            )]
        );
    }

    #[test]
    fn tokens_keep_database_order_replacement_and_refused_inventory_notes() {
        content_reader_script::test_game::enter();
        struct TempRoot(std::path::PathBuf);
        impl Drop for TempRoot {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let root = TempRoot(std::env::temp_dir().join(format!(
            "technology-tokens-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )));
        let (base, addon) = (root.0.join("base"), root.0.join("mod"));
        for (layer, path, text) in [
            (
                &base,
                "common/terrain/fixture.txt",
                "categories = { fixture = {} }",
            ),
            (
                &base,
                "common/technologies/a.txt",
                "technologies = { removed = {} }",
            ),
            (
                &base,
                "common/technologies/b.txt",
                "technologies = { middle = {} repeated = {} }",
            ),
            (
                &addon,
                "common/technologies/a.txt",
                "technologies = { z_first = {} a_second = {} repeated = {} }",
            ),
            (
                &addon,
                "common/technologies/nested/c.txt",
                "technologies = { not_loaded = {} }",
            ),
            (
                &addon,
                "common/technologies/c.txt",
                "technologies = { refused_form = { folder = { position = { 1 } } } }",
            ),
        ] {
            let file = layer.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        }
        let layers = ResolvedLayers::from_folders(&[(&base, &[]), (&addon, &[])]).unwrap();
        let read = Database::read(&layers).unwrap();
        assert!(read.equipment().is_empty());
        assert_eq!(read.equipment_warnings.len(), 1);
        assert_eq!(
            read.definitions
                .iter()
                .map(|definition| (definition.token.as_str(), definition.database_index))
                .collect::<Vec<_>>(),
            [
                ("z_first", Some(1)),
                ("a_second", Some(2)),
                ("repeated", Some(3)),
                ("middle", Some(4)),
                ("repeated", Some(5))
            ]
        );
        assert!(read.definitions.iter().all(|definition| {
            definition.presentation.as_ref().unwrap().template == definition.token
        }));
        let (tokens, notes) = (read.tokens, read.inventory_warnings);
        assert_eq!(
            tokens,
            ["z_first", "a_second", "repeated", "middle", "refused_form"]
        );
        assert_eq!(
            notes,
            [(
                content_reader_parser::DiagnosticKind::Unsupported,
                "common/technologies/c.txt: technology inventory may be incomplete: technology refused_form folder position: unsupported vector form".into(),
            )]
        );
    }

    fn parse_equipment(
        input: &[u8],
        names: &KeyNames,
    ) -> io::Result<BTreeMap<String, Option<String>>> {
        super::parse_equipment(input, names).map(|(definitions, _)| definitions)
    }

    #[test]
    fn completion_name_projection_uses_template_single_child_and_load_time_limits() {
        content_reader_script::test_game::enter();
        let (links, display, _) = parse_with_registry(br#"technologies={plain={} renamed={parent=mod_parent} equipment={enable_equipments={mod_equipment}} forced={parent=mod_parent enable_equipments={mod_equipment} force_use_small_tech_layout=yes} conditional={enable_equipments={limit={always=yes} mod_equipment}}}"#, &units::Registry::default(), &KeyNames::default()).unwrap();
        let name = |key: &str| {
            display[key]
                .as_ref()
                .unwrap()
                .name(|_| Ok(true), |name| Ok(Some(name.into())))
                .unwrap()
        };
        assert_eq!(name("plain"), "plain");
        assert_eq!(name("renamed"), "mod_parent");
        assert_eq!(name("equipment"), "mod_equipment");
        assert_eq!(name("forced"), "mod_parent");
        assert_eq!(name("conditional"), "mod_equipment");
        assert_eq!(
            display["conditional"]
                .as_ref()
                .unwrap()
                .name(|_| Ok(false), |_| unreachable!())
                .unwrap(),
            "conditional"
        );
        assert!(
            display["conditional"]
                .as_ref()
                .unwrap()
                .name(
                    |_| Err("load-time global unavailable".into()),
                    |name| Ok(Some(name.into()))
                )
                .unwrap_err()
                .contains("at content load")
        );
        assert_eq!(
            display["conditional"]
                .as_ref()
                .unwrap()
                .name(|_| Err("load-time global unavailable".into()), |_| Ok(None))
                .unwrap(),
            "conditional"
        );
        assert_eq!(links["equipment"].as_deref(), Some("mod_equipment"));
    }

    #[test]
    fn invalid_parent_refuses_only_its_presentation_and_keeps_equipment_links() {
        content_reader_script::test_game::enter();
        let input = b"technologies={\
            before={enable_equipments={before_equipment}} \
            invalid={parent=\x80 enable_equipments={invalid_equipment}} \
            after={parent=after_name enable_equipments={after_equipment}}}";
        let (links, names, _) =
            parse_with_registry(input, &units::Registry::default(), &KeyNames::default()).unwrap();
        assert_eq!(links.len(), 3);
        for token in ["before", "invalid", "after"] {
            assert_eq!(links[token], Some(format!("{token}_equipment")));
        }
        assert_eq!(names["before"].as_ref().unwrap().template, "before");
        assert_eq!(names["after"].as_ref().unwrap().template, "after_name");
        let refusal = names["invalid"].as_ref().unwrap_err();
        assert!(refusal.contains("technology invalid parent:"));
        assert!(refusal.contains("not UTF-8"));
        assert_eq!(parse_equipment(input, &KeyNames::default()).unwrap(), links);

        let block = b"technologies={\
            before={enable_equipments={before_equipment}} \
            after={parent=after_name enable_equipments={after_equipment}} \
            invalid={enable_equipments={invalid_equipment} parent={nested=value}}}";
        let (block_links, block_names, notes) =
            parse_with_registry(block, &units::Registry::default(), &KeyNames::default()).unwrap();
        assert_eq!(block_links, links);
        assert_eq!(block_names["before"].as_ref().unwrap().template, "before");
        assert_eq!(
            block_names["after"].as_ref().unwrap().template,
            "after_name"
        );
        assert_eq!(block_names["invalid"].as_ref().unwrap().template, "{");
        assert!(notes.iter().any(|(_, reason)| {
            reason.contains("`parent`") && reason.contains("block where its reader takes one value")
        }));
    }

    #[test]
    fn malformed_small_layout_boolean_keeps_the_last_value() {
        content_reader_script::test_game::enter();
        for (values, expected) in [
            (&["invalid"][..], false),
            (&["yes", "invalid"][..], true),
            (&["no", "invalid"][..], false),
            (&["yes", "no", "invalid"][..], false),
            (&["no", "yes", "invalid"][..], true),
            (&["yes", "{invalid=yes}"][..], true),
            (&["no", "{invalid=yes}"][..], false),
        ] {
            let assignments = values
                .iter()
                .map(|value| format!("force_use_small_tech_layout={value}"))
                .collect::<Vec<_>>()
                .join(" ");
            let input = format!(
                "technologies={{modded={{parent=mod_name enable_equipments={{mod_equipment}} {assignments}}}}}"
            );
            let (links, names, notes) = parse_with_registry(
                input.as_bytes(),
                &units::Registry::default(),
                &KeyNames::default(),
            )
            .unwrap();
            let presentation = names["modded"].as_ref().unwrap();
            assert_eq!(presentation.force_small, expected, "{assignments}");
            assert_eq!(links["modded"].as_deref(), Some("mod_equipment"));
            if values.last() == Some(&"{invalid=yes}") {
                assert!(notes.iter().any(|(_, reason)| {
                    reason.contains("force_use_small_tech_layout")
                        && reason.contains("block where its reader takes one value")
                }));
            }
            assert_eq!(
                presentation
                    .name(|_| unreachable!(), |token| Ok(Some(token.into())))
                    .unwrap(),
                if expected {
                    "mod_name"
                } else {
                    "mod_equipment"
                },
                "{assignments}"
            );
        }
    }

    #[test]
    fn completion_name_deduplicates_authored_tokens_but_preserves_alias_children() {
        content_reader_script::test_game::enter();
        let (_, display, _) = parse_with_registry(
            br#"technologies={same={enable_equipments={one ONE}} aliases={enable_equipments={one alias}}}"#,
            &units::Registry::default(), &KeyNames::default(),
        ).unwrap();
        assert_eq!(
            display["same"]
                .as_ref()
                .unwrap()
                .name(|_| unreachable!(), |_| Ok(Some("child".into())))
                .unwrap(),
            "child"
        );
        assert_eq!(
            display["aliases"]
                .as_ref()
                .unwrap()
                .name(|_| unreachable!(), |_| Ok(Some("child".into())))
                .unwrap(),
            "aliases"
        );
    }

    #[test]
    fn a_repeated_child_keeps_its_admission_whatever_its_limits() {
        let p = PresentationName {
            template: "template".into(),
            force_small: false,
            equipment: vec![
                EquipmentName {
                    name: "tank_1".into(),
                    limits: Vec::new(),
                },
                EquipmentName {
                    name: "TANK_1".into(),
                    limits: vec![Vec::new()],
                },
            ],
        };
        assert_eq!(
            p.children(
                |_| Err("not decided".to_owned()),
                |name| Ok(Some(name.to_owned()))
            ),
            Ok(vec!["tank_1".to_owned()])
        );
        let entry = |name: &str, limited: bool| EquipmentName {
            name: name.into(),
            limits: if limited { vec![Vec::new()] } else { Vec::new() },
        };
        let children = |equipment: Vec<EquipmentName>| {
            PresentationName {
                template: "template".into(),
                force_small: false,
                equipment,
            }
            .children(
                |_| Err("not decided".to_owned()),
                |name| Ok(Some(name.to_owned())),
            )
        };
        assert_eq!(
            children(vec![entry("tank_1", true), entry("tank_1", false)]),
            Ok(vec!["tank_1".to_owned()])
        );
        assert_eq!(
            children(vec![
                entry("a", true),
                entry("b", true),
                entry("a", false),
                entry("b", false)
            ]),
            Ok(vec!["a".to_owned(), "b".to_owned()])
        );
        let refused = children(vec![entry("b", true), entry("a", false), entry("b", false)]);
        assert!(
            refused
                .as_ref()
                .unwrap_err()
                .contains("enable_equipments limit for b at content load: not decided"),
            "{refused:?}"
        );
        assert!(children(vec![entry("a", false), entry("c", true)]).is_err());
        let aliased = |equipment: Vec<EquipmentName>, alias: bool| {
            PresentationName {
                template: "template".into(),
                force_small: false,
                equipment,
            }
            .children(
                |_| Err("not decided".to_owned()),
                |name| Ok(Some(if alias && name == "alt" { "one" } else { name }.to_owned())),
            )
        };
        let swapped = || vec![entry("alt", true), entry("one", false), entry("alt", false)];
        assert_eq!(
            aliased(swapped(), true),
            Ok(vec!["one".to_owned(), "one".to_owned()])
        );
        assert!(aliased(swapped(), false).is_err());
        assert_eq!(
            PresentationName {
                template: "template".into(),
                force_small: false,
                equipment: vec![entry("a", false), entry("c", true)],
            }
            .children(
                |_| Err("not decided".to_owned()),
                |name| Ok((name == "a").then(|| name.to_owned())),
            ),
            Ok(vec!["a".to_owned()])
        );
        let limit = |flag: &str| {
            content_reader_parser::parser::parse(format!("has_global_flag={flag}").as_bytes())
                .unwrap()
                .items
        };
        let behind = |name: &str, flag: &str| EquipmentName {
            name: name.into(),
            limits: vec![limit(flag)],
        };
        let reordered = |flags: [&str; 3]| {
            PresentationName {
                template: "template".into(),
                force_small: false,
                equipment: vec![
                    behind("c", flags[0]),
                    behind("b", flags[1]),
                    behind("a", flags[2]),
                    entry("a", false),
                    entry("b", false),
                    entry("c", false),
                ],
            }
            .children(
                |_| Err("not decided".to_owned()),
                |name| Ok(Some(if name == "c" { "a" } else { name }.to_owned())),
            )
        };
        assert_eq!(
            reordered(["mod_flag"; 3]),
            Ok(vec!["a".to_owned(), "b".to_owned(), "a".to_owned()])
        );
        assert!(reordered(["c_flag", "b_flag", "a_flag"]).is_err());
    }

    #[test]
    fn completion_name_limit_prefix_short_circuits_and_resets_between_fields() {
        content_reader_script::test_game::enter();
        let p = PresentationName {
            template: "template".into(),
            force_small: false,
            equipment: vec![EquipmentName {
                name: "late".into(),
                limits: vec![Vec::new(), Vec::new()],
            }],
        };
        let mut checks = 0;
        assert_eq!(
            p.name(
                |_| {
                    checks += 1;
                    assert_eq!(checks, 1);
                    Ok(false)
                },
                |_| unreachable!()
            )
            .unwrap(),
            "template"
        );
        assert_eq!(checks, 1);
        let (_, parsed, _) = parse_with_registry(
            br#"technologies={separate={enable_equipments={limit={always=no} first} enable_equipments={limit={always=yes} second}}}"#,
            &units::Registry::default(), &KeyNames::default(),
        ).unwrap();
        assert_eq!(
            parsed["separate"]
                .as_ref()
                .unwrap()
                .name(
                    |body| Ok(body[0].value.scalar() == Some(b"yes")),
                    |name| Ok(Some(name.into())),
                )
                .unwrap(),
            "second"
        );
        let error = parse_with_registry(
            br#"technologies={ambiguous={enable_equipments={limit={always=no} limit={always=yes} second}}}"#,
            &units::Registry::default(), &KeyNames::default(),
        ).unwrap_err().1;
        assert!(error.to_string().contains("enable_equipments.limit"));
    }

    #[test]
    fn effect_bodies_follow_the_brace_guard_and_leading_limit() {
        content_reader_script::test_game::enter();
        let link = |body: &str| {
            let text = format!("technologies={{t={{{body}}}}}");
            parse_equipment(text.as_bytes(), &KeyNames::default())
                .unwrap()
                .remove("t")
                .flatten()
        };
        assert_eq!(
            link("on_research_complete=no enable_equipments={one}").as_deref(),
            Some("one")
        );
        assert_eq!(
            link("on_research_complete={limit={} add_ideas=x limit=y} enable_equipments={two}")
                .as_deref(),
            Some("two")
        );
        assert_eq!(
            link("on_research_complete={limit=no enable_equipments={custom_equipment}}").as_deref(),
            Some("custom_equipment")
        );
        assert_eq!(
            link("on_research_complete={limit={always=yes} enable_equipments={three}}").as_deref(),
            Some("three")
        );
        assert_eq!(
            link("on_research_complete=no limit={} enable_equipments={lost}"),
            None
        );
        for body in [
            "on_research_complete=no {ignored=1 enable_equipments={one}}",
            "enable_equipments={limit=no one}",
        ] {
            let text = format!("technologies={{t={{{body}}}}}");
            assert!(
                parse_equipment(text.as_bytes(), &KeyNames::default()).is_err(),
                "{body}"
            );
        }
    }

    #[test]
    fn keywords_references_and_list_appends() {
        content_reader_script::test_game::enter();
        let links = parse_equipment(br#"@e=one TECHNOLOGIES={a={"enable_equipments"=wrong enable_equipments={@e} enable_equipments={ONE}} b={enable_equipments={one} enable_equipments={two}}}"#, &KeyNames::default()).unwrap();
        assert_eq!(links["a"].as_deref(), Some("one"));
        assert_eq!(links["b"], None);
    }

    #[test]
    fn conditional_unlock_preserves_following_fields_and_names_unsafe_tails() {
        content_reader_script::test_game::enter();
        let links = parse_equipment(br#"technologies={a={enable_equipments={limit={always=yes} one} research_cost=1} b={enable_equipments={two}}}"#, &KeyNames::default()).unwrap();
        assert_eq!(links["a"], None);
        assert_eq!(links["b"].as_deref(), Some("two"));
        let error = parse_equipment(
            b"technologies={a={enable_equipments={limit={always=no} one two}}}",
            &KeyNames::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("enable_equipments.limit"));
        let kind = parse_with_registry(
            b"technologies={a={enable_equipments={limit={always=no} one two}}}",
            &units::Registry::default(),
            &KeyNames::default(),
        )
        .unwrap_err()
        .0;
        assert_eq!(kind, content_reader_parser::DiagnosticKind::Unsettled);
    }

    #[test]
    fn dynamic_reader_dispatch_uses_the_recorded_registry() {
        content_reader_script::test_game::enter();
        let mut registry = units::Registry::default();
        registry.units.insert(b"custom_unit".to_vec());
        registry.terrains.insert(b"custom_terrain".to_vec());
        let bytes = b"technologies={a={custom_unit={custom_terrain={attack=1}} enable_subunits={limit={always=yes} custom_unit} enable_equipments={one}}}";
        let mut names = KeyNames::default();
        names.effects.modifiers_gap = Some("unread modifier suffix".into());
        assert!(
            parse_equipment(bytes, &names)
                .unwrap_err()
                .to_string()
                .contains("custom_unit")
        );
        assert_eq!(
            parse_with_registry(bytes, &registry, &names).unwrap().0["a"].as_deref(),
            Some("one")
        );
    }

    #[test]
    fn modifier_registry_decides_unknown_block_recovery() {
        content_reader_script::test_game::enter();
        let input = b"technologies={t={category_leg_infantry={@lost=2} enable_equipments={one}}} lost=@lost";
        let mut names = KeyNames::default();
        let units = units::Registry::default();
        names.effects.modifiers_gap = Some("unread modifier suffix".into());
        assert!(
            parse_with_registry(input, &units, &names)
                .unwrap_err()
                .1
                .to_string()
                .contains("category_leg_infantry")
        );
        names.effects.modifiers_gap = None;
        let (links, _, _) = parse_with_registry(input, &units, &names).unwrap();
        assert_eq!(links["t"].as_deref(), Some("one"));
        assert_eq!(
            modifiers::registered(b"category_leg_infantry", Some(&names)),
            Some(false)
        );
        assert_eq!(modifiers::registered(b"IF", Some(&names)), Some(false));
        assert_eq!(
            modifiers::registered(b"ARMY_ATTACK_FACTOR", Some(&names)),
            Some(true)
        );
    }

    #[test]
    fn equipment_localisation_follows_only_an_unambiguous_authored_unlock() {
        content_reader_script::test_game::enter();
        let links = parse_equipment(
            br#"technologies = {
            @cost = 1
            weapons = { enable_equipments = { infantry_equipment_3 } }
            cars = { enable_equipments = { armored_car_equipment_2 } }
            multiple = { enable_equipments = { a b } }
            modifier_only = { research_cost = 2 }
        }"#,
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(links["weapons"].as_deref(), Some("infantry_equipment_3"));
        assert_eq!(links["cars"].as_deref(), Some("armored_car_equipment_2"));
        for token in ["multiple", "modifier_only"] {
            assert_eq!(links[token], None);
        }
        let repeated = parse_equipment(
            br#"technologies = { a = { enable_equipments = { one } } a = {} }"#,
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(repeated["a"].as_deref(), Some("one"));
    }
    #[test]
    fn enablers_list_each_technology_once_with_its_parent_and_mark_scripts() {
        content_reader_script::test_game::enter();
        let parsed = parse_enablers(
            br#"technologies = {
            infantry_weapons1 = { enable_equipments = { infantry_equipment_1 } }
            renamed = { Parent = infantry_weapons enable_equipments = { tank_1 tank_2 } }
            other = { research_cost = 1 }
            Other = { ENABLE_EQUIPMENTS = { TANK_3 } }
            scripted = { enable_equipments = { gun_1 if = { limit = { always = yes } gun_2 } } }
            variable = { enable_equipments = { @modded } }
        }"#,
        )
        .unwrap();
        let plain = |id: &str, equipment: &[&str], parent: Option<&str>| Enables {
            id: id.to_owned(),
            equipment: equipment.iter().map(|name| (*name).to_owned()).collect(),
            parent: Ok(parent.map(str::to_owned)),
            scripted: None,
            limited: Vec::new(),
        };
        assert_eq!(
            parsed[0],
            plain("infantry_weapons1", &["infantry_equipment_1"], None)
        );
        assert_eq!(
            parsed[1],
            plain("renamed", &["tank_1", "tank_2"], Some("infantry_weapons"))
        );
        assert_eq!(parsed[2], plain("other", &[], None));
        assert_eq!(parsed[3], plain("Other", &["TANK_3"], None));
        assert_eq!(parsed[4].id, "scripted");
        assert_eq!(parsed[4].equipment, ["gun_1"]);
        let scripted = parsed[4].scripted.as_ref().unwrap();
        assert!(scripted.names.contains(&"gun_2".to_owned()) && !scripted.any);
        assert_eq!(parsed[5].id, "variable");
        assert!(parsed[5].scripted.as_ref().unwrap().any);
        assert_eq!(parsed.len(), 6);
        let shouted =
            parse_enablers(b"TECHNOLOGIES = { tank_tech = { ENABLE_EQUIPMENTS = { tank_1 } } }")
                .unwrap();
        assert_eq!(shouted[0].equipment, ["tank_1"]);
        assert_eq!(
            parse_equipment(
                b"Technologies = { tank_tech = { Enable_Equipments = { tank_1 } } }",
                &KeyNames::default(),
            )
            .unwrap()["tank_tech"],
            Some("tank_1".to_owned())
        );
    }

    #[test]
    fn a_quoted_parent_key_does_not_rename_the_enabler() {
        let parsed = parse_enablers(
            b"technologies = { t = { PARENT = right \"parent\" = wrong enable_equipments = { tank_1 } } \
              u = { parent = \xff enable_equipments = { gun_1 } } } \
              \"technologies\" = { quoted = { enable_equipments = { tank_2 } } }",
        )
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].parent, Ok(Some("right".to_owned())));
        let unknown = parsed[1].parent.as_ref().unwrap_err();
        assert!(unknown.contains("technology u parent"), "{unknown}");
    }

    #[test]
    fn native_enablers_keep_the_keyword_parent() {
        content_reader_script::test_game::enter();
        let stack = content_reader_layers::test_support::Stack::new("technology_enabler_parent");
        stack.write(
            &stack.base(),
            "common/terrain/fixture.txt",
            "categories = { fixture = {} }",
        );
        stack.write(
            &stack.addon(),
            "common/technologies/a.txt",
            "technologies = { t = { parent = right enable_equipments = { tank_1 } \"parent\" = 1 } }",
        );
        let enablers =
            read_enablers(&stack.layers(&[]), |_| Err("no limit here".to_owned())).unwrap();
        assert_eq!(
            enablers.single("tank_1").unwrap().unwrap().parent,
            Ok(Some("right".to_owned()))
        );
    }

    #[test]
    fn a_scripted_unlock_leaves_only_the_types_it_mentions_unknown() {
        let enablers = |text: &[u8]| {
            let mut result = Enablers::default();
            for enables in parse_enablers(text).unwrap() {
                result.add(
                    "fixture",
                    enables,
                    &mut |_: &[Item]| -> Result<bool, String> { Err("not decided".to_owned()) },
                );
            }
            result
        };
        let found = enablers(
            b"technologies = { rifles = { enable_equipments = { rifle_1 } } guns = { enable_equipments = { if = { limit = { always = yes } GUN_1 } } } both = { enable_equipments = { tank_1 } } tanks = { enable_equipments = { tank_1 } } }",
        );
        assert_eq!(found.single("rifle_1").unwrap().unwrap().id, "rifles");
        let unknown = found.single("gun_1").unwrap_err();
        assert!(unknown.contains("guns"), "{unknown}");
        assert_eq!(found.single("tank_1"), Ok(None));
        assert_eq!(found.single("nothing"), Ok(None));
        let any = enablers(b"technologies = { rifles = { enable_equipments = { rifle_1 } } modded = { enable_equipments = { @unknown } } }");
        assert!(any.single("rifle_1").unwrap_err().contains("modded"));
        let limited = enablers(
            b"technologies = { guns = { enable_equipments = { gun_0 } enable_equipments = { limit = { always = no } gun_1 } } }",
        );
        assert_eq!(limited.single("gun_0").unwrap().unwrap().id, "guns");
        assert!(limited.single("gun_1").unwrap_err().contains("guns"));
        assert_eq!(limited.single("always"), Ok(None));
        let repeated = enablers(
            b"technologies = { t = { enable_equipments = { tank_1 } enable_equipments = { limit = { has_global_flag = mod_flag } TANK_1 } } }",
        );
        assert_eq!(repeated.single("tank_1").unwrap().unwrap().id, "t");
        assert!(repeated.refused.is_empty(), "{:?}", repeated.refused);
        let variable = enablers(
            b"technologies = { v = { enable_equipments = { rifle_1 @unknown } } }",
        );
        assert_eq!(variable.single("rifle_1").unwrap().unwrap().id, "v");
        assert!(variable.single("tank_1").unwrap_err().contains("technology v "));
        assert_eq!(variable.refused, ["fixture: v"]);
        let other = enablers(
            b"technologies = { t = { enable_equipments = { tank_1 } } u = { enable_equipments = { limit = { has_global_flag = mod_flag } tank_1 } } }",
        );
        assert!(other.single("tank_1").unwrap_err().contains("technology u "));
    }

    #[test]
    fn enablers_resolve_file_variables_through_the_native_reader() {
        content_reader_script::test_game::enter();
        let stack = content_reader_layers::test_support::Stack::new("technology_enablers_native");
        stack.write(
            &stack.base(),
            "common/terrain/fixture.txt",
            "categories = { fixture = {} }",
        );
        stack.write(
            &stack.addon(),
            "common/technologies/a.txt",
            "@equip = tank_1 technologies = { t = { enable_equipments = { @equip } } guns = { enable_equipments = { gun_0 } enable_equipments = { limit = { always = no } gun_1 } } open = { enable_equipments = { limit = { always = yes } gun_2 } } asks = { enable_equipments = { limit = { always = yes always = no } gun_3 } } held = { enable_equipments = { gun_4 } enable_equipments = { limit = { always = yes always = no } GUN_4 } } }",
        );
        let evaluate = |limit: &[Item]| match limit {
            [always] if always.keyword_is("always") => {
                Ok(always.value.scalar() == Some(&b"yes"[..]))
            }
            _ => Err("synthetic: not decided".to_owned()),
        };
        let enablers = read_enablers(&stack.layers(&[]), evaluate).unwrap();
        assert!(enablers.any.is_empty(), "{:?}", enablers.any);
        assert_eq!(enablers.single("tank_1").unwrap().unwrap().id, "t");
        assert_eq!(enablers.single("gun_0").unwrap().unwrap().id, "guns");
        assert_eq!(enablers.single("gun_1"), Ok(None));
        assert_eq!(enablers.single("gun_2").unwrap().unwrap().id, "open");
        assert!(enablers.single("gun_3").unwrap_err().contains("asks"));
        assert_eq!(enablers.single("gun_4").unwrap().unwrap().id, "held");
        assert_eq!(enablers.refused.len(), 1, "{:?}", enablers.refused);
        assert_eq!(enablers.single("rifle_1"), Ok(None));
    }

    #[test]
    fn a_mod_technology_replaces_vanilla_under_the_lexer_fold() {
        content_reader_script::test_game::enter();
        let root = std::env::temp_dir().join(format!(
            "hoi4_content_reader_enablers_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let (vanilla, modded) = (root.join("game"), root.join("mod"));
        let terrain = vanilla.join("common/terrain/fixture.txt");
        std::fs::create_dir_all(terrain.parent().unwrap()).unwrap();
        std::fs::write(terrain, "categories = { fixture = {} }").unwrap();
        for (folder, text) in [
            (
                &vanilla,
                "technologies = { tank_tech = { enable_equipments = { tank_1 } } gun = { enable_equipments = { gun_1 } } }",
            ),
            (
                &modded,
                "technologies = { Tank_Tech = { parent = tanks enable_equipments = { MOD_TANK_1 } } }",
            ),
        ] {
            let path = folder.join("common/technologies/tech.txt");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let enablers = read_enablers(
            &ResolvedLayers::from_folders(&[(&vanilla, &[][..]), (&modded, &[][..])]).unwrap(),
            |_| Err("no limit in this fixture".to_owned()),
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert!(!enablers.plain.contains_key("tank_1"));
        assert!(enablers.refused.is_empty());
        assert_eq!(
            enablers.plain[&fold_token("mod_tank_1")],
            [Enabler {
                id: "Tank_Tech".to_owned(),
                parent: Ok(Some("tanks".to_owned()))
            }]
        );
    }

    #[test]
    fn a_technology_redefined_in_another_file_is_another_template() {
        content_reader_script::test_game::enter();
        let root = std::env::temp_dir().join(format!(
            "hoi4_content_reader_enablers_case_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let terrain = root.join("common/terrain/fixture.txt");
        std::fs::create_dir_all(terrain.parent().unwrap()).unwrap();
        std::fs::write(terrain, "categories = { fixture = {} }").unwrap();
        for (file, text) in [
            (
                "00_a.txt",
                "technologies = { tank_tech = { enable_equipments = { tank_1 } } }",
            ),
            (
                "01_b.txt",
                "technologies = { TANK_TECH = { enable_equipments = { TANK_1 } } }",
            ),
        ] {
            let path = root.join("common/technologies").join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let enablers = read_enablers(
            &ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap(),
            |_| Err("no limit in this fixture".to_owned()),
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(
            enablers.plain[&fold_token("Tank_1")],
            [
                Enabler {
                    id: "tank_tech".to_owned(),
                    parent: Ok(None)
                },
                Enabler {
                    id: "TANK_TECH".to_owned(),
                    parent: Ok(None)
                }
            ]
        );
        assert_eq!(enablers.single("tank_1"), Ok(None));
    }

    #[test]
    fn unverified_native_parse_retains_reason_and_withholds_effect_consumed_enablers() {
        content_reader_script::test_game::enter();
        let stack = content_reader_layers::test_support::Stack::new("technology_enablers_unverified");
        stack.write(
            &stack.base(),
            "common/terrain/fixture.txt",
            "categories = { fixture = {} }",
        );
        stack.write(
            &stack.base(),
            "common/technologies/a.txt",
            "technologies = { first = { enable_equipments = { dual_tank } } second = { enable_equipments = { dual_tank } } solo = { enable_equipments = { solo_tank } } }",
        );
        stack.write(
            &stack.addon(),
            "common/technologies/b.txt",
            "technologies = { broken = { on_research_complete = no { ignored = 1 enable_equipments = { one } } } consumed = { on_research_complete = no limit = {} enable_equipments = { lost } } plain = { enable_equipments = { safe_looking } } }",
        );
        let enablers =
            read_enablers(&stack.layers(&[]), |_| Err("no limit here".to_owned())).unwrap();
        assert!(!enablers.plain.contains_key("lost"), "{:?}", enablers.plain);
        assert!(!enablers.plain.contains_key("one"), "{:?}", enablers.plain);
        assert!(
            !enablers.plain.contains_key("safe_looking"),
            "{:?}",
            enablers.plain
        );
        assert_eq!(enablers.single("dual_tank"), Ok(None));
        for key in ["lost", "one", "safe_looking", "solo_tank"] {
            let reason = enablers.single(key).unwrap_err();
            assert!(
                reason.contains("common/technologies/b.txt")
                    && reason.contains("on_research_complete"),
                "{key}: {reason}"
            );
        }

        let no_terrain = content_reader_layers::test_support::Stack::new("technology_enablers_no_terrain");
        no_terrain.write(
            &no_terrain.base(),
            "common/technologies/a.txt",
            "technologies = { consumed = { on_research_complete = no limit = {} enable_equipments = { lost } } }",
        );
        let fallback =
            read_enablers(&no_terrain.layers(&[]), |_| Err("no limit here".to_owned())).unwrap();
        assert!(fallback.plain.is_empty(), "{:?}", fallback.plain);
        let reason = fallback.single("lost").unwrap_err();
        assert!(reason.contains("common/terrain"), "{reason}");

        let verified = content_reader_layers::test_support::Stack::new("technology_enablers_consumed_ok");
        verified.write(
            &verified.base(),
            "common/terrain/fixture.txt",
            "categories = { fixture = {} }",
        );
        verified.write(
            &verified.base(),
            "common/technologies/a.txt",
            "technologies = { consumed = { on_research_complete = no limit = {} enable_equipments = { lost } } }",
        );
        let ok = read_enablers(&verified.layers(&[]), |_| Err("no limit here".to_owned())).unwrap();
        assert_eq!(ok.single("lost"), Ok(None));
    }

    #[test]
    fn unresolved_variable_in_native_enablers_withholds_single_fallback() {
        content_reader_script::test_game::enter();
        let stack = content_reader_layers::test_support::Stack::new("technology_enablers_unresolved_var");
        stack.write(
            &stack.base(),
            "common/terrain/fixture.txt",
            "categories = { fixture = {} }",
        );
        stack.write(
            &stack.base(),
            "common/technologies/a.txt",
            "technologies = { rifles = { enable_equipments = { rifle_1 } } modded = { enable_equipments = { @missing } } }",
        );
        let enablers =
            read_enablers(&stack.layers(&[]), |_| Err("no limit here".to_owned())).unwrap();
        assert!(!enablers.plain.contains_key("@missing"), "{:?}", enablers.plain);
        let withheld = enablers.single("rifle_1").unwrap_err();
        assert!(withheld.contains("modded"), "{withheld}");

        let dual = content_reader_layers::test_support::Stack::new("technology_enablers_unresolved_var_dual");
        dual.write(
            &dual.base(),
            "common/terrain/fixture.txt",
            "categories = { fixture = {} }",
        );
        dual.write(
            &dual.base(),
            "common/technologies/a.txt",
            "technologies = { rifles = { enable_equipments = { rifle_1 } } rifles_2 = { enable_equipments = { rifle_1 } } modded = { enable_equipments = { @missing } } }",
        );
        let enablers =
            read_enablers(&dual.layers(&[]), |_| Err("no limit here".to_owned())).unwrap();
        assert_eq!(enablers.single("rifle_1"), Ok(None));
    }

    #[test]
    fn unreadable_technology_file_records_unverified_and_preserves_other_files() {
        content_reader_script::test_game::enter();
        let stack = content_reader_layers::test_support::Stack::new("technology_enablers_unreadable_file");
        stack.write(
            &stack.base(),
            "common/terrain/fixture.txt",
            "categories = { fixture = {} }",
        );
        stack.write(
            &stack.base(),
            "common/technologies/a.txt",
            "technologies = { first = { enable_equipments = { dual_tank } } second = { enable_equipments = { dual_tank } } solo = { enable_equipments = { solo_tank } } }",
        );
        stack.write(
            &stack.addon(),
            "common/technologies/missing.txt",
            "technologies = { gone = { enable_equipments = { gone_tank } } }",
        );
        let layers = stack.layers(&[]);
        std::fs::remove_file(stack.addon().join("common/technologies/missing.txt")).unwrap();
        let enablers = read_enablers(&layers, |_| Err("no limit here".to_owned())).unwrap();
        assert!(
            enablers
                .unverified
                .iter()
                .any(|reason| reason.contains("common/technologies/missing.txt")),
            "{:?}",
            enablers.unverified
        );
        assert_eq!(enablers.single("dual_tank"), Ok(None));
        let withheld = enablers.single("solo_tank").unwrap_err();
        assert!(
            withheld.contains("common/technologies/missing.txt"),
            "{withheld}"
        );
    }
}
