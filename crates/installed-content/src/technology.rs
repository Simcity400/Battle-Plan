//! Equipment localisation links projected from the native technology reader.
//! An equipment-unlocking technology can use its equipment's generic name and
//! description instead of technology-localisation keys. [`read_enablers`] also
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
mod units;
mod descriptions;
pub use descriptions::{Catalog, Definition};
pub(crate) use units::{StatKey, equipment_pool_roster, equipment_pool_shape, stat_key};

/// Each technology's single unconditional equipment link, when known.
pub type EquipmentLinks = BTreeMap<String, Option<String>>;

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
    parse_with_registry(input, &units::Registry::default(), names).map_err(|(_, error)| error)
}

fn parse_with_registry(
    input: &[u8],
    registry: &units::Registry,
    names: &KeyNames,
) -> Result<(EquipmentLinks, Vec<crate::Diagnostic>), (crate::DiagnosticKind, io::Error)> {
    parse_with_tokens(input, registry, names, &mut Vec::new())
}

fn parse_with_tokens(
    input: &[u8],
    registry: &units::Registry,
    names: &KeyNames,
    tokens: &mut Vec<String>,
) -> Result<(EquipmentLinks, Vec<crate::Diagnostic>), (crate::DiagnosticKind, io::Error)> {
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
            let mut unlocks: Vec<String> = Vec::new();
            let mut conditional = false;
            for field in technology.value.items().unwrap_or_default() {
                if field.keyword_is("folder") {
                    for position in field
                        .value
                        .items()
                        .unwrap_or_default()
                        .iter()
                        .filter(|i| i.keyword_is("position"))
                    {
                        let values = position.value.items().unwrap_or_default();
                        // 1402C7A50 has fixed-length recovery, not a list.
                        // These are the two forms for which List is identical.
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
                for equipment in field.value.items().unwrap_or_default() {
                    if equipment.keyword_is("limit") {
                        conditional = true;
                        continue;
                    }
                    // 140AC3C70 has a special, evaluated `limit` branch in
                    // this raw loop. Its conditional early exit is not a list.
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
                }
            }
            result.insert(
                token.to_owned(),
                (!conditional && unlocks.len() == 1).then(|| unlocks.remove(0)),
            );
        }
    }
    Ok((
        result,
        read.misreads
            .iter()
            .map(|note| (note.kind, note.to_string()))
            .collect(),
    ))
}

/// One preparation read supplies both native equipment links and the inventory.
#[derive(Debug)]
pub struct Database {
    pub equipment: BTreeMap<String, String>,
    pub tokens: Vec<String>,
    pub equipment_warnings: Vec<crate::Diagnostic>,
    pub inventory_warnings: Vec<crate::Diagnostic>,
}

impl Database {
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let registry = units::Registry::read(layers)?;
        let names = layers.key_names()?;
        let mut result = BTreeMap::new();
        let mut tokens = Vec::new();
        let mut equipment_warnings = Vec::new();
        let mut inventory_warnings = Vec::new();
        // 140AC1BA0 enumerates this directory without recursion.
        for path in layers.directory_files("common/technologies", ".txt")? {
            match parse_with_tokens(&layers.read(&path)?, &registry, &names, &mut tokens) {
                Ok((definitions, notes)) => {
                    result.extend(definitions);
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
            equipment_warnings,
            inventory_warnings,
        })
    }
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
    // CTechnologyDatabase 140ABCFC0 (EOF loop), CTechnologyTemplate
    // 140AC3960 / 140AC3C70. All dynamic content names come from layers.
    // CTechnologyDatabase::LoadDirectory (140AC1BA0) constructs each file's
    // lexer in mode 0 (1424A6370).
    // CTechnologyTemplate::vf4 140AC3C70 passes country scope at 140AC582B.
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
                // 140AB8A50 / 140A83900 accept limit until the first anonymous
                // statement (140A835F0); other assignments use only the value.
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
                // 140AC3C70 peeks for a leading limit (on_research_complete_limit,
                // +0x470), then CEffect::ReadScript (140534280) reads the body only
                // when the value slot is `{`.
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
                // 140636690 checks subunits, then categories, before CModifier.
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
                } // 140AC3B00
                (Folder, Some(b"name")) => Shape::One, // 140AC39C0
                // 1402C7A50: the lexer's tokens, the value slot unchanged.
                (Folder, Some(b"position")) if field.opens() => Shape::RawList,
                (Folder | Path, _) => Shape::Skip,
                (Scalars | Building, _) => Shape::One, // 1413B04D0 and inline loops
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

/// A technology that enables equipment: its id and its `parent`, the alternate
/// name the game's technology template keeps at `CTechnologyTemplate+0x3b0` (read
/// id for names and equipment icons (`string_reader_140b3d930`).
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
    /// The one technology that enables `key` (`string_source_scan@140b36c60` uses a
    /// technology only when exactly one enables the type): none when no or several
    /// plain entries name it; `Err` when a scripted entry may add one, so how many
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

/// For each equipment type, every technology whose `enable_equipments` names it,
/// in technology id order (the game's icon fallback uses a technology only when
/// exactly one enables the type, `string_source_scan@140b36c60`). Files are read in
/// layer and path order, a later technology of the same id replacing an earlier one.
///
/// Equipment and technology ids are lexer tokens, so both are compared as the lexer
/// identifies them (`ASCII token fold`: only ASCII A–Z fold, `AddDynamicToken@1424A65A0`):
/// the map is keyed by the folded equipment key, `enable_equipments = { TANK_1 }`
/// names the type `tank_1`, and `Tank_Tech` redefines `tank_tech`. A technology keeps
/// the spelling of its last definition. A scripted entry refuses only what that
/// technology could enable ([`Enablers::single`]).
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
    #[test]
    fn tokens_keep_database_order_replacement_and_refused_inventory_notes() {
        struct TempRoot(std::path::PathBuf);
        impl Drop for TempRoot {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let root = TempRoot(std::env::temp_dir().join(format!(
            "technology-tokens-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
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
    fn effect_bodies_follow_the_brace_guard_and_leading_limit() {
        // 140AC3C70 peeks for a leading limit and reads it as a trigger; then
        // 140534280 reads the body only when the value slot, as that left it,
        // is `{`. The one technology's unlock, by case:
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
        // in the slot: LoadBody and ReadScript report it and the fields after
        // it are the technology's.
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
    fn native_keywords_references_and_list_appends() {
        let links = parse_equipment(br#"@e=one TECHNOLOGIES={a={"enable_equipments"=wrong enable_equipments={@e} enable_equipments={ONE}} b={enable_equipments={one} enable_equipments={two}}}"#, &KeyNames::default()).unwrap();
        assert_eq!(links["a"].as_deref(), Some("one"));
        assert_eq!(links["b"], None);
    }

    #[test]
    fn conditional_unlock_preserves_following_fields_and_names_unsafe_tails() {
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
        let (links, _) = parse_with_registry(input, &units, &names).unwrap();
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
