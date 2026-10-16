//! Equipment localisation links projected from the native technology reader.
//! An equipment-unlocking technology can use its equipment's name and description;
//! consumers share admission while retaining the distinct native display gates. [`read_enablers`] also
//! projects its id and parent for the production icon's technology fallback.
use crate::key_names::KeyNames;
use crate::{
    ResolvedLayers,
    fields::invalid,
    modifiers,
    parser::{
        self, Item, fold_token,
        native::{self, Shape},
    },
    scope::ScopeKind,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};
mod descriptions;
mod units;
pub use descriptions::{Catalog, Definition};
pub(crate) use units::{StatKey, equipment_pool_roster, equipment_pool_shape, stat_key};

/// Each technology's single unconditional equipment link, when known.
pub type EquipmentLinks = BTreeMap<String, Option<String>>;
type ParsedPresentation = (
    EquipmentLinks,
    BTreeMap<String, Result<PresentationName, String>>,
    Vec<crate::Diagnostic>,
);
type ParseError = (crate::DiagnosticKind, io::Error);

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

/// Its triggers are read against `names`.
pub fn parse_equipment(
    input: &[u8],
    names: &KeyNames,
) -> io::Result<(EquipmentLinks, Vec<crate::Diagnostic>)> {
    parse_with_registry(input, &units::Registry::default(), names)
        .map(|(equipment, _, notes)| (equipment, notes))
        .map_err(|(_, error)| error)
}

fn parse_with_registry(
    input: &[u8],
    registry: &units::Registry,
    names: &KeyNames,
) -> Result<ParsedPresentation, ParseError> {
    parse_with_tokens(
        input,
        registry,
        names,
        &mut Vec::new(),
        &mut BTreeMap::new(),
    )
}

/// `folders` receives every technology this file defines, with the research
/// folders it is placed in, only when the whole file reads.
fn parse_with_tokens(
    input: &[u8],
    registry: &units::Registry,
    names: &KeyNames,
    tokens: &mut Vec<String>,
    folders: &mut BTreeMap<String, Vec<String>>,
) -> Result<ParsedPresentation, ParseError> {
    let (read, undecided, refused) = read_templates_with_tokens(input, registry, names, tokens);
    if let Some(cause) = read.unsettled {
        return Err((cause.kind, invalid(format!("technology: {cause}"))));
    }
    if let Some(cause) = refused.first() {
        return Err((crate::DiagnosticKind::Refused, invalid(cause)));
    }
    if let Some(cause) = undecided.first() {
        return Err((
            crate::DiagnosticKind::Unsettled,
            invalid(format!("technology: {cause}")),
        ));
    }
    let mut result = BTreeMap::new();
    let mut display_keys = BTreeMap::new();
    let mut placements = BTreeMap::new();
    let mut folder_notes = Vec::new();
    for root in read
        .items
        .iter()
        .filter(|item| item.keyword_is("technologies"))
    {
        for technology in root.value.items().unwrap_or_default() {
            let token = std::str::from_utf8(crate::parser::c_string(
                technology.key.as_deref().unwrap_or_default(),
            ))
            .map_err(|_| {
                (
                    crate::DiagnosticKind::Refused,
                    invalid("technology token is not UTF-8"),
                )
            })?;
            // A technology template's parent overrides its display-name key.
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
            // The research folders (tabs) the technology is placed in, in authored
            // order; a folder block's last `name` names it.
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
                                crate::DiagnosticKind::Refused,
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
                        // Fixed-length recovery, not a list. These are the two forms for
                        // which List is identical.
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
                                crate::DiagnosticKind::Unsupported,
                                invalid(format!(
                                    "technology {token} folder position: unsupported vector form"
                                )),
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
                    // This raw loop has a special, evaluated `limit` branch.
                    // Its conditional early exit is not a list.
                    if equipment.value_keyword().as_deref() == Some("limit")
                        || matches!(equipment.kind, native::Kind::Open | native::Kind::Equals)
                    {
                        return Err((
                            crate::DiagnosticKind::Unsupported,
                            invalid(format!(
                                "technology {token} enable_equipments: unsupported conditional or nested unlock"
                            )),
                        ));
                    }
                    let name = equipment
                        .value
                        .read_str()
                        .map_err(|e| invalid(format!("technology {token} enable_equipments: {e}")))
                        .map_err(|error| (crate::DiagnosticKind::Refused, error))?;
                    if !unlocks.iter().any(|old| old.eq_ignore_ascii_case(name)) {
                        unlocks.push(name.into());
                    }
                    candidates.push(EquipmentName {
                        name: name.into(),
                        limits: limits.clone(),
                    });
                }
            }
            let fields = technology.value.items().unwrap_or_default();
            // A failed boolean value keeps the preceding assignment.
            let force_small = parser::read_bool_field(fields, "force_use_small_tech_layout", false);
            display_keys.insert(
                token.to_owned(),
                parent.map(|parent| {
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
                }),
            );
            result.insert(
                token.to_owned(),
                (!conditional && unlocks.len() == 1).then(|| unlocks.remove(0)),
            );
            placements.insert(token.to_owned(), placed);
        }
    }
    folders.extend(placements);
    Ok((
        result,
        display_keys,
        read.misreads
            .iter()
            .map(|note| (note.kind, note.to_string()))
            .chain(folder_notes)
            .collect(),
    ))
}

/// Native full-name roots for a recorded technology owner.
#[derive(Debug)]
pub struct PresentationName {
    pub template: String,
    pub equipment: Vec<EquipmentName>,
    pub force_small: bool,
}

/// One authored child, with its load-time admission condition when present.
#[derive(Debug)]
pub struct EquipmentName {
    pub name: String,
    pub limits: Vec<Vec<Item>>,
}

impl PresentationName {
    /// The full localized name uses one admitted child unless the small-layout
    /// flag forces the template key. Limits are checked at content load in a
    /// fresh empty scope, then accepted names are stored. Runtime resolution
    /// applies the validity/category/DLC gate before filling the instance.
    /// Classifier propagation does not recheck these limits.
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

    /// Admission is shared by names and descriptions. Descriptions count
    /// non-duplicate children separately, then select the first child.
    pub fn children(
        &self,
        mut evaluate: impl FnMut(&[Item]) -> Result<bool, String>,
        mut resolve: impl FnMut(&str) -> Result<Option<String>, String>,
    ) -> Result<Vec<String>, String> {
        if self.force_small {
            return Ok(Vec::new());
        }
        let mut children = Vec::new();
        let mut accepted = BTreeSet::new();
        for candidate in &self.equipment {
            // A false limit exits this authored field's whole loop; later
            // checks in that field cannot restore its admission.
            let mut admitted = true;
            for limit in &candidate.limits {
                if !evaluate(limit).map_err(|why| {
                    format!(
                        "enable_equipments limit for {} at content load: {why}",
                        candidate.name
                    )
                })? {
                    admitted = false;
                    break;
                }
            }
            if !admitted {
                continue;
            }
            // Template names deduplicate by folded authored token; distinct
            // aliases may resolve to the same pointer and still occupy two
            // runtime children.
            if accepted.insert(candidate.name.to_ascii_lowercase())
                && let Some(name) = resolve(&candidate.name)?
            {
                children.push(name);
            }
        }
        Ok(children)
    }
}

/// One preparation read supplies both native equipment links and the inventory.
#[derive(Debug)]
pub struct Database {
    pub equipment: BTreeMap<String, String>,
    pub tokens: Vec<String>,
    /// Each technology's research folders in authored order; a technology
    /// redefined by a later file keeps the later definition's folders.
    pub folders: BTreeMap<String, Vec<String>>,
    pub equipment_warnings: Vec<crate::Diagnostic>,
    pub inventory_warnings: Vec<crate::Diagnostic>,
    pub names: BTreeMap<String, Result<PresentationName, String>>,
}

impl Database {
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let registry = units::Registry::read(layers)?;
        let names = layers.key_names()?;
        let mut result = BTreeMap::new();
        let mut tokens = Vec::new();
        let mut folders = BTreeMap::new();
        let mut presentation_names = BTreeMap::new();
        let mut equipment_warnings = Vec::new();
        let mut inventory_warnings = Vec::new();
        // Enumerate this directory without recursion.
        for path in layers.directory_files("common/technologies", ".txt")? {
            match parse_with_tokens(
                &layers.read(&path)?,
                &registry,
                &names,
                &mut tokens,
                &mut folders,
            ) {
                Ok((definitions, display_names, notes)) => {
                    result.extend(definitions);
                    presentation_names.extend(display_names);
                    let notes: Vec<_> = notes
                        .into_iter()
                        .map(|(kind, note)| (kind, format!("{path}: {note}")))
                        .collect();
                    equipment_warnings.extend(notes.iter().cloned());
                    inventory_warnings.extend(notes);
                }
                Err((kind, error)) => {
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
            equipment: result
                .into_iter()
                .filter_map(|(token, equipment)| equipment.map(|equipment| (token, equipment)))
                .collect(),
            tokens,
            folders,
            equipment_warnings,
            inventory_warnings,
            names: presentation_names,
        })
    }
}

/// A research folder (a tab of the technology screen), in content order.
#[derive(Clone, Debug)]
pub struct FolderDefinition {
    pub key: String,
    /// The tab's `available` trigger, read in country scope; none means the
    /// tab is always available. A folder's last `available` is kept.
    pub available: Option<Vec<Item>>,
}

/// Every research folder `common/technology_tags` defines, in file order, and
/// the notes from reading them.
#[derive(Clone, Debug, Default)]
pub struct Folders {
    pub folders: Vec<FolderDefinition>,
    pub warnings: Vec<crate::Diagnostic>,
}

/// Reads `technology_folders` from every `common/technology_tags` file, in
/// directory order. A folder defined again keeps its first definition and the
/// repeat is refused by name. A file that cannot be read whole adds nothing.
pub fn read_folders(layers: &ResolvedLayers) -> io::Result<Folders> {
    let names = layers.key_names()?;
    let mut read = Folders::default();
    for path in layers.directory_files("common/technology_tags", ".txt")? {
        match parse_folders(&layers.read(&path)?, &names) {
            Ok((folders, notes)) => {
                read.warnings.extend(
                    notes
                        .into_iter()
                        .map(|(kind, note)| (kind, format!("{path}: {note}"))),
                );
                for folder in folders {
                    if read.folders.iter().any(|old| old.key == folder.key) {
                        read.warnings.push((
                            crate::DiagnosticKind::Refused,
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
                crate::DiagnosticKind::Unsettled,
                format!("{path}: research folders unavailable: {error}"),
            )),
        }
    }
    Ok(read)
}

/// One `common/technology_tags` file's research folders, in authored order.
pub fn parse_folders(
    input: &[u8],
    names: &KeyNames,
) -> io::Result<(Vec<FolderDefinition>, Vec<crate::Diagnostic>)> {
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
    let mut notes: Vec<crate::Diagnostic> = read
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
            let Ok(key) = std::str::from_utf8(crate::parser::c_string(
                folder.key.as_deref().unwrap_or_default(),
            )) else {
                notes.push((
                    crate::DiagnosticKind::Refused,
                    "research folder key is not UTF-8".into(),
                ));
                continue;
            };
            if key.is_empty() {
                continue;
            }
            if folders.iter().any(|old| old.key == key) {
                notes.push((
                    crate::DiagnosticKind::Refused,
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
) -> io::Result<(BTreeMap<String, String>, Vec<crate::Diagnostic>)> {
    let read = Database::read(layers)?;
    Ok((read.equipment, read.equipment_warnings))
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
    // Read the database to EOF. All dynamic content names come from layers.
    // Each file constructs its lexer in mode 0. Technology fields pass
    // country scope to their trigger and effect readers.
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
                    if let Ok(token) = std::str::from_utf8(crate::parser::c_string(&field.key.text))
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
                // Accept limit until the first anonymous statement; other
                // assignments use only the value.
                (Uniform, Some(b"limit")) if leading_limit => Shape::Trigger,
                (Uniform, _) => {
                    if field.key.text.is_empty() {
                        leading_limit = false;
                    }
                    if registry
                        .units
                        .contains(&crate::parser::c_string(&field.value.text).to_ascii_lowercase())
                    {
                        Shape::One
                    } else {
                        Shape::Skip
                    }
                }
                // Peek for a leading on_research_complete limit, then read the
                // effect body only when the value slot is `{`.
                (Technology, Some(b"on_research_complete")) => Shape::LimitedBody(Effect),
                (Effect, _) => crate::effect_keys::shape(Some(names), field).map(|()| Effect),
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
                // Check subunits, then categories, before modifiers.
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
                // The lexer's tokens, the value slot unchanged.
                (Folder, Some(b"position")) if field.opens() => Shape::RawList,
                (Folder | Path, _) => Shape::Skip,
                (Scalars | Building, _) => Shape::One, // Scalar fields read one value.
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

/// A technology that enables equipment: its id and its `parent`, the
/// alternate name read from the scalar `parent` and used instead of the
/// id for names and equipment icons.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enabler {
    pub id: String,
    pub parent: Option<String>,
}

/// A technology's `enable_equipments` and `parent` as one definition declares them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Enables {
    /// The id as written.
    pub id: String,
    /// The types its plain entries name.
    pub equipment: Vec<String>,
    pub parent: Option<String>,
    /// A scripted entry (a block, a keyed entry or an unresolved `@variable`): which
    /// equipment it enables is not known.
    pub scripted: Option<Scripted>,
}

/// What a refused entry could enable: the names it mentions, or any type at all.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scripted {
    pub names: Vec<String>,
    /// It holds an `@variable` this reader does not resolve, so it could name any type.
    pub any: bool,
}

/// Each technology's definition, by its id as the lexer identifies it (`ASCII token fold`).
pub type EnabledBy = BTreeMap<String, Enables>;

/// Every name a scripted entry mentions (keys and scalars), and whether it holds an
/// unresolved `@variable`.
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

/// Every technology's `enable_equipments` list and `parent`, by technology id; a
/// later definition of an id (in any ASCII case: one lexer token) replaces an
/// earlier one. A keyed, scripted or `@variable` entry is not read as a type: the
/// technology is marked [`Scripted`] with the names it mentions, so only the types
/// it could enable are left unknown.
pub fn parse_enablers(input: &[u8]) -> io::Result<EnabledBy> {
    let document = parser::parse(input)?;
    let mut result = BTreeMap::new();
    // Keywords are lexer tokens, matched in any ASCII case (`parser::is_keyword`).
    for root in document.get_all_keyword("technologies") {
        for technology in root.items().unwrap_or_default() {
            let (Some(key), Some(fields)) = (&technology.key, technology.value.items()) else {
                continue;
            };
            let Ok(token) = std::str::from_utf8(key) else {
                continue;
            };
            let mut enables = Enables {
                id: token.to_owned(),
                ..Enables::default()
            };
            for field in fields {
                let keyword = |name: &str| parser::is_keyword(field, name);
                match () {
                    () if keyword("enable_equipments") => {
                        for item in field.value.items().unwrap_or_default() {
                            match (item.key.is_none(), item.value.scalar_str()) {
                                (true, Some(name)) if !name.starts_with('@') => {
                                    enables.equipment.push(name.to_owned())
                                }
                                _ => mentioned(
                                    item,
                                    enables.scripted.get_or_insert_with(Scripted::default),
                                ),
                            }
                        }
                    }
                    () if keyword("parent") => {
                        enables.parent = field.value.scalar_str().map(str::to_owned)
                    }
                    () => {}
                }
            }
            result.insert(fold_token(token), enables);
        }
    }
    Ok(result)
}

/// Which technologies enable each equipment type, as the icon fallback asks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Enablers {
    /// Each type's technologies named by a plain entry, by its folded key, in
    /// technology id order.
    pub plain: BTreeMap<String, Vec<Enabler>>,
    /// Types (folded) a scripted entry mentions, with those technologies' ids.
    pub uncertain: BTreeMap<String, BTreeSet<String>>,
    /// Technologies with an entry that could name any type.
    pub any: BTreeSet<String>,
    /// Every technology with a scripted entry, as `<path>: <id>`, for a warning.
    pub refused: Vec<String>,
}

impl Enablers {
    /// The one technology that enables `key`: none when no or several plain
    /// entries name it; `Err` when a scripted entry may add one, so how many
    /// enable it is not known.
    pub fn single(&self, key: &str) -> Result<Option<&Enabler>, String> {
        let key = fold_token(key);
        let plain = self.plain.get(&key).map(Vec::as_slice).unwrap_or_default();
        if plain.len() > 1 {
            return Ok(None);
        }
        let uncertain: Vec<&str> = self
            .uncertain
            .get(&key)
            .into_iter()
            .flatten()
            .chain(&self.any)
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
}

/// For each equipment type, every technology whose `enable_equipments` names
/// it, in technology id order. The game's icon fallback uses a technology
/// only when exactly one enables the type. Files are read in layer and path
/// order, a later technology of the same id replacing an earlier one.
///
/// Equipment and technology ids are lexer tokens, compared with ASCII A-Z
/// folding. The map is keyed by the folded equipment key: an uppercase type
/// entry matches its lowercase spelling, and a mixed-case technology id
/// redefines its lowercase spelling. A technology keeps the spelling of its
/// last definition. A scripted entry refuses only what that technology could
/// enable ([`Enablers::single`]).
pub fn read_enablers(layers: &ResolvedLayers) -> io::Result<Enablers> {
    let paths = layers.directory_files("common/technologies", ".txt")?;
    let mut technologies: BTreeMap<String, (String, Enables)> = BTreeMap::new();
    for path in paths {
        let parsed = parse_enablers(&layers.read(&path)?)
            .map_err(|error| io::Error::new(error.kind(), format!("{path}: {error}")))?;
        for (id, enables) in parsed {
            technologies.insert(id, (path.clone(), enables));
        }
    }
    let mut result = Enablers::default();
    for (path, enables) in technologies.into_values() {
        let id = enables.id;
        for name in enables.equipment {
            let list = result.plain.entry(fold_token(&name)).or_default();
            if !list.iter().any(|enabler| enabler.id == id) {
                list.push(Enabler {
                    id: id.clone(),
                    parent: enables.parent.clone(),
                });
            }
        }
        if let Some(scripted) = enables.scripted {
            for name in scripted.names {
                result
                    .uncertain
                    .entry(fold_token(&name))
                    .or_default()
                    .insert(id.clone());
            }
            if scripted.any {
                result.any.insert(id.clone());
            }
            result.refused.push(format!("{path}: {id}"));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "test-support")]
    use std::sync::atomic::{AtomicU64, Ordering};

    #[cfg(feature = "test-support")]
    static NEXT: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn technologies_keep_their_folders_in_authored_order_only_when_the_file_reads() {
        let mut folders = BTreeMap::new();
        parse_with_tokens(
            b"technologies = { two = { folder = { name = infantry_folder position = { x = 0 y = 0 } } folder = { name = support_folder position = { x = 1 y = 2 } } folder = { name = infantry_folder } } none = { research_cost = 1 } renamed = { folder = { name = first name = second } } }",
            &units::Registry::default(),
            &KeyNames::default(),
            &mut Vec::new(),
            &mut folders,
        )
        .unwrap();
        assert_eq!(folders["two"], ["infantry_folder", "support_folder"]);
        assert!(folders["none"].is_empty());
        assert_eq!(folders["renamed"], ["second"]);
        // A file that cannot be read whole contributes no placements.
        let before = folders.clone();
        assert!(
            parse_with_tokens(
                b"technologies = { added = { folder = { name = armour_folder } } broken = { folder = { position = { 1 } } } }",
                &units::Registry::default(),
                &KeyNames::default(),
                &mut Vec::new(),
                &mut folders,
            )
            .is_err()
        );
        assert_eq!(folders, before);
    }

    #[test]
    fn research_folders_keep_file_order_triggers_and_refuse_repeats_by_name() {
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
        assert!(notes.iter().any(|(kind, note)| *kind == crate::DiagnosticKind::Refused
            && note == "research folder infantry_folder is defined again; its first definition is used"));
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn a_mod_adds_research_folders_after_the_base_game_and_cannot_redefine_one() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
                crate::DiagnosticKind::Refused,
                "common/technology_tags/zz_mod.txt: research folder infantry_folder is defined again; its first definition is used".into(),
            )]
        );
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn tokens_keep_database_order_replacement_and_refused_inventory_notes() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        assert!(read.equipment.is_empty());
        assert_eq!(read.equipment_warnings.len(), 1);
        assert_eq!(
            read.names.keys().map(String::as_str).collect::<Vec<_>>(),
            ["a_second", "middle", "repeated", "z_first"]
        );
        assert_eq!(
            read.names["repeated"].as_ref().unwrap().template,
            "repeated"
        );
        let (tokens, notes) = (read.tokens, read.inventory_warnings);
        assert_eq!(
            tokens,
            ["z_first", "a_second", "repeated", "middle", "refused_form"]
        );
        assert_eq!(
            notes,
            [(
                crate::DiagnosticKind::Unsupported,
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
                    |_| unreachable!()
                )
                .unwrap_err()
                .contains("at content load")
        );
        assert_eq!(links["equipment"].as_deref(), Some("mod_equipment"));
    }

    #[test]
    fn invalid_parent_refuses_only_its_presentation_and_keeps_equipment_links() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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

        // Shape::Current stores `{`; its nested fields are technology fields
        // and their `}` closes the technology.
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    fn completion_name_limit_prefix_short_circuits_and_resets_between_fields() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        // Peek for a leading limit and read it as a trigger; then read the
        // body only when the value slot, as that left it, is `{`. The one
        // technology's unlock, by case:
        let link = |body: &str| {
            let text = format!("technologies={{t={{{body}}}}}");
            parse_equipment(text.as_bytes(), &KeyNames::default())
                .unwrap()
                .remove("t")
                .flatten()
        };
        // A scalar body reads nothing: the unlock after it is the technology's.
        assert_eq!(
            link("on_research_complete=no enable_equipments={one}").as_deref(),
            Some("one")
        );
        // An empty leading limit leaves its `{`: the body is read, and a later
        // `limit` in it is an effect.
        assert_eq!(
            link("on_research_complete={limit={} add_ideas=x limit=y} enable_equipments={two}")
                .as_deref(),
            Some("two")
        );
        // `limit = no`, or a limit whose last value is `yes`, leaves that value
        // in the slot: trigger and effect body readers report it and the fields
        // after it are the technology's.
        assert_eq!(
            link("on_research_complete={limit=no enable_equipments={custom_equipment}}").as_deref(),
            Some("custom_equipment")
        );
        assert_eq!(
            link("on_research_complete={limit={always=yes} enable_equipments={three}}").as_deref(),
            Some("three")
        );
        // A scalar body and then a limit block: that `{` opens the body over
        // the technology's remaining fields, the unlock among them.
        assert_eq!(
            link("on_research_complete=no limit={} enable_equipments={lost}"),
            None
        );
        // Forms whose token recovery is not followed are refused by name.
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let links = parse_equipment(br#"@e=one TECHNOLOGIES={a={"enable_equipments"=wrong enable_equipments={@e} enable_equipments={ONE}} b={enable_equipments={one} enable_equipments={two}}}"#, &KeyNames::default()).unwrap();
        assert_eq!(links["a"].as_deref(), Some("one"));
        assert_eq!(links["b"], None);
    }

    #[test]
    fn conditional_unlock_preserves_following_fields_and_names_unsafe_tails() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        assert_eq!(kind, crate::DiagnosticKind::Unsettled);
    }

    #[test]
    fn dynamic_reader_dispatch_uses_the_recorded_registry() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        let overrides = parse_equipment(
            br#"technologies = { a = { enable_equipments = { one } } a = {} }"#,
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(overrides["a"], None);
    }
    #[test]
    fn enablers_list_each_technology_once_with_its_parent_and_mark_scripts() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
            parent: parent.map(str::to_owned),
            scripted: None,
        };
        assert_eq!(
            parsed["infantry_weapons1"],
            plain("infantry_weapons1", &["infantry_equipment_1"], None)
        );
        assert_eq!(
            parsed["renamed"],
            plain("renamed", &["tank_1", "tank_2"], Some("infantry_weapons"))
        );
        // `Other` is the same lexer token as `other`: the later definition replaces it.
        assert_eq!(parsed["other"], plain("Other", &["TANK_3"], None));
        // A script keeps the plain entries and names what it mentions; a variable could be anything.
        assert_eq!(parsed["scripted"].equipment, ["gun_1"]);
        let scripted = parsed["scripted"].scripted.as_ref().unwrap();
        assert!(scripted.names.contains(&"gun_2".to_owned()) && !scripted.any);
        assert!(parsed["variable"].scripted.as_ref().unwrap().any);
        // Keywords are lexer tokens too: in any ASCII case they are read, not ignored.
        let shouted =
            parse_enablers(b"TECHNOLOGIES = { tank_tech = { ENABLE_EQUIPMENTS = { tank_1 } } }")
                .unwrap();
        assert_eq!(shouted["tank_tech"].equipment, ["tank_1"]);
        assert_eq!(
            parse_equipment(
                b"Technologies = { tank_tech = { Enable_Equipments = { tank_1 } } }",
                &KeyNames::default(),
            )
            .unwrap()["tank_tech"],
            Some("tank_1".to_owned())
        );
    }

    /// A scripted entry leaves unknown only whether one technology enables the types
    /// it mentions; every other type keeps its single enabler.
    #[test]
    fn a_scripted_unlock_leaves_only_the_types_it_mentions_unknown() {
        let enablers = |text: &[u8]| {
            let mut result = Enablers::default();
            for enables in parse_enablers(text).unwrap().into_values() {
                for name in &enables.equipment {
                    result
                        .plain
                        .entry(fold_token(name))
                        .or_default()
                        .push(Enabler {
                            id: enables.id.clone(),
                            parent: None,
                        });
                }
                if let Some(scripted) = &enables.scripted {
                    for name in &scripted.names {
                        result
                            .uncertain
                            .entry(fold_token(name))
                            .or_default()
                            .insert(enables.id.clone());
                    }
                    if scripted.any {
                        result.any.insert(enables.id.clone());
                    }
                }
            }
            result
        };
        let found = enablers(
            b"technologies = { rifles = { enable_equipments = { rifle_1 } } guns = { enable_equipments = { if = { limit = { always = yes } GUN_1 } } } both = { enable_equipments = { tank_1 } } tanks = { enable_equipments = { tank_1 } } }",
        );
        assert_eq!(found.single("rifle_1").unwrap().unwrap().id, "rifles");
        let unknown = found.single("gun_1").unwrap_err();
        assert!(unknown.contains("guns"), "{unknown}");
        // Two plain enablers already rule the technology step out, script or not.
        assert_eq!(found.single("tank_1"), Ok(None));
        assert_eq!(found.single("nothing"), Ok(None));
        let any = enablers(b"technologies = { rifles = { enable_equipments = { rifle_1 } } modded = { enable_equipments = { @unknown } } }");
        assert!(any.single("rifle_1").unwrap_err().contains("modded"));
    }

    /// A mod redefining a vanilla technology in another ASCII case replaces it, and
    /// its equipment is found in any case: both are one lexer token.
    #[cfg(feature = "test-support")]
    #[test]
    fn a_mod_technology_replaces_vanilla_under_the_lexer_fold() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let root = std::env::temp_dir().join(format!(
            "hoi4_installed_content_enablers_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let (vanilla, modded) = (root.join("game"), root.join("mod"));
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
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&root);
        // The mod's file replaces the vanilla file of the same path; a mod
        // technology in another file would replace `tank_tech` by its folded id.
        assert!(!enablers.plain.contains_key("tank_1"));
        assert!(enablers.refused.is_empty());
        assert_eq!(
            enablers.plain[&fold_token("mod_tank_1")],
            [Enabler {
                id: "Tank_Tech".to_owned(),
                parent: Some("tanks".to_owned())
            }]
        );
    }

    /// Technologies in two files of one id in different cases are one token: the
    /// later file's definition is the only one.
    #[cfg(feature = "test-support")]
    #[test]
    fn a_technology_redefined_in_another_case_is_one_technology() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let root = std::env::temp_dir().join(format!(
            "hoi4_installed_content_enablers_case_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
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
        let enablers =
            read_enablers(&ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(
            enablers.plain[&fold_token("Tank_1")],
            [Enabler {
                id: "TANK_TECH".to_owned(),
                parent: None
            }]
        );
    }
}
