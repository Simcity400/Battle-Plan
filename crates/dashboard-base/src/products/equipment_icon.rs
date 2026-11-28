use content_reader_key_names::key_name_products::KeyNamesExt as _;

use crate::catalogue::Catalogue;
use content_reader_key_names::equipment::Database;
use content_reader_military::technology::Enablers;
use content_reader_equipment_graphics::{
    equipment_graphics, equipment_graphics::Context, equipment_graphics::context::Prepared,
    equipment_graphics::countries::ModelCountry,
};
use replay_data::{
    rows::CountryLinkRow, rows::Recorded, rows::VariantDefinitionRow, rows::tables,
};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};

struct TypeModel<'a> {
    archetype: Option<&'a str>,
    family: Option<&'a str>,
}

fn missing_type(equipment: &Database, key: &str) -> String {
    match &equipment.lost {
        Some(reason) => format!("equipment {key}'s database is refused: {reason}"),
        None => format!("equipment {key} is not in the recorded content"),
    }
}

fn type_model<'a>(equipment: &'a Database, key: &str) -> Result<TypeModel<'a>, String> {
    if equipment.lost.is_some() {
        return Err(missing_type(equipment, key));
    }
    let at = equipment
        .get(key)
        .ok_or_else(|| missing_type(equipment, key))?;
    let kind = &equipment.equipment[at];
    let name = |at: usize| {
        equipment.equipment[at]
            .name_str()
            .map_err(|error| format!("equipment {key}: {error}"))
    };
    Ok(TypeModel {
        archetype: kind.archetype.map(name).transpose()?,
        family: (kind.family != at).then(|| name(kind.family)).transpose()?,
    })
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub enum Icon {
    Sprite(String),
    Path(String),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum Probe {
    Sprite(String),
    File(String),
    Existing(String),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
struct ChoiceBranch {
    assignment: Vec<Option<String>>,
    probes: Vec<Probe>,
    otherwise: Result<(), String>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct IconChoice {
    branches: Vec<ChoiceBranch>,
    reasons: Vec<String>,
}

impl IconChoice {
    pub fn unknown(reason: String) -> Self {
        Self {
            branches: vec![ChoiceBranch {
                assignment: Vec::new(),
                probes: Vec::new(),
                otherwise: Err(reason),
            }],
            reasons: Vec::new(),
        }
    }

    pub fn walk(
        &self,
        answer: impl Fn(&Probe) -> Result<Option<Icon>, String>,
    ) -> Result<Option<Icon>, String> {
        if self.branches.len() == 1 && self.reasons.is_empty() {
            let branch = &self.branches[0];
            for probe in &branch.probes {
                if let Some(icon) = answer(probe)? {
                    return Ok(Some(icon));
                }
            }
            return branch.otherwise.clone().map(|()| None);
        }
        let mut cache: BTreeMap<&Probe, Result<Option<Icon>, String>> = BTreeMap::new();
        let mut outcomes: Vec<Result<Option<Icon>, String>> =
            Vec::with_capacity(self.branches.len());
        for branch in &self.branches {
            let mut outcome = None;
            for probe in &branch.probes {
                let hit = if let Some(cached) = cache.get(probe) {
                    cached.clone()
                } else {
                    let result = answer(probe);
                    cache.insert(probe, result.clone());
                    result
                };
                match hit {
                    Ok(Some(icon)) => {
                        outcome = Some(Ok(Some(icon)));
                        break;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        outcome = Some(Err(error));
                        break;
                    }
                }
            }
            outcomes.push(outcome.unwrap_or_else(|| branch.otherwise.clone().map(|()| None)));
        }
        let mut first_ok: Option<&Option<Icon>> = None;
        let mut disagrees = false;
        let mut first_err: Option<&String> = None;
        for outcome in &outcomes {
            match outcome {
                Ok(icon) => match first_ok {
                    None => first_ok = Some(icon),
                    Some(seen) if seen != icon => {
                        disagrees = true;
                        break;
                    }
                    Some(_) => {}
                },
                Err(error) => {
                    if first_err.is_none() {
                        first_err = Some(error);
                    }
                }
            }
        }
        if !disagrees {
            if let Some(error) = first_err {
                return Err(error.clone());
            }
            return Ok(first_ok.cloned().flatten());
        }
        if self.reasons.len() == 1 {
            return Err(self.reasons[0].clone());
        }
        for index in 0..self.reasons.len() {
            for a in 0..self.branches.len() {
                let Ok(left_icon) = &outcomes[a] else {
                    continue;
                };
                for b in (a + 1)..self.branches.len() {
                    let Ok(right_icon) = &outcomes[b] else {
                        continue;
                    };
                    if left_icon == right_icon {
                        continue;
                    }
                    let same_other = self.branches[a]
                        .assignment
                        .iter()
                        .zip(&self.branches[b].assignment)
                        .enumerate()
                        .all(|(k, (left, right))| k == index || left == right);
                    if same_other {
                        return Err(self.reasons[index].clone());
                    }
                }
            }
        }
        for a in 0..self.branches.len() {
            let Ok(left_icon) = &outcomes[a] else {
                continue;
            };
            for b in (a + 1)..self.branches.len() {
                let Ok(right_icon) = &outcomes[b] else {
                    continue;
                };
                if left_icon == right_icon {
                    continue;
                }
                if let Some(index) = self.branches[a]
                    .assignment
                    .iter()
                    .zip(&self.branches[b].assignment)
                    .position(|(left, right)| left != right)
                    && let Some(reason) = self.reasons.get(index)
                {
                    return Err(reason.clone());
                }
                return Err(self
                    .reasons
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "continent unknown".to_owned()));
            }
        }
        Err(self
            .reasons
            .first()
            .cloned()
            .unwrap_or_else(|| "continent unknown".to_owned()))
    }

    pub fn shown(&self, catalogue: &Catalogue) -> Result<Option<Icon>, String> {
        let layers = catalogue.content.layers.presentation();
        let sprite = |name: &str| match catalogue.sprites.refusal(name) {
            Some(reason) => Err(reason),
            None => Ok(catalogue
                .sprites
                .contains(name)
                .then(|| Icon::Sprite(name.to_owned()))),
        };
        let file = |path: &str| {
            art_file_present(layers, path).map(|found| found.then(|| Icon::Path(path.to_owned())))
        };
        self.walk(|probe| match probe {
            Probe::Sprite(name) => sprite(name),
            Probe::File(path) => file(path),
            Probe::Existing(name) => match sprite(name)? {
                Some(icon) => Ok(Some(icon)),
                None if reads_as_path(name) => file(name),
                None => Ok(None),
            },
        })
    }
}

fn reads_as_path(name: &str) -> bool {
    name.contains('/') || name.contains('.')
}

pub fn resolve_image(
    catalogue: &Catalogue,
    icon: &Icon,
    max_edge: u32,
) -> super::artwork::ArtResolution {
    super::artwork::resolve_equipment_icon(catalogue, icon, max_edge)
}

pub fn image(catalogue: &Catalogue, icon: &Icon, max_edge: u32) -> io::Result<String> {
    resolve_image(catalogue, icon, max_edge)
        .into_io_result()?
        .ok_or_else(|| crate::invalid("equipment icon unavailable"))
}

pub struct AuthoredIcons {
    pub token: String,
    pub icons: Vec<Icon>,
    pub refusals: Vec<String>,
}

fn icon_suffixes(capital: bool) -> &'static [&'static str] {
    if capital {
        &["_capital_medium", "_medium"]
    } else {
        &["_medium"]
    }
}

fn sprite_names(name: &str, suffix: &str, tag: Option<&str>) -> [Option<String>; 2] {
    [
        tag.map(|tag| format!("GFX_{tag}_{name}{suffix}")),
        Some(format!("GFX_{name}{suffix}")),
    ]
}

type Files = Box<dyn Fn(&str) -> Result<bool, String> + Send + Sync>;

pub struct IconContent {
    pub catalogue: Option<Arc<Catalogue>>,
    units: Arc<Database>,
    enablers: Enablers,
    load_warnings: Vec<String>,
    pools: equipment_graphics::Content,
    geography: equipment_graphics::context::Content,
    dlc: BTreeSet<String>,
    sprites: BTreeSet<String>,
    refused_sprites: BTreeMap<String, String>,
    files: Files,
    stamped: bool,
}

fn settled<T>(read: std::thread::Result<T>) -> T {
    read.unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

pub fn inputs_stamped(layers: &content_reader_layers::ResolvedLayers) -> io::Result<bool> {
    if !layers.stamped("map/default.map")? {
        return Ok(false);
    }
    let default_map = content_reader_key_names::terrain::read_default_map(layers)?;
    for name in ["definitions", "provinces", "continent"] {
        let Ok(path) = default_map.file(name) else {
            return Ok(false);
        };
        if !layers.stamped(&path)? {
            return Ok(false);
        }
    }
    Ok(true)
}

impl IconContent {
    pub fn native(
        layers: Arc<content_reader_layers::ResolvedLayers>,
        units: Arc<Database>,
        scripted: &Result<dashboard_text::scripted_text::ScriptedContent, String>,
        registry: &content_reader_gui::gui::native::resources::Registry,
    ) -> io::Result<Self> {
        let default_map = content_reader_key_names::terrain::read_default_map(&layers)?;
        let provinces = content_reader_map::map::load_definitions(&layers)?;
        let defines = crate::catalogue::defines_for_layers(&layers)?;
        let mut warnings = Vec::new();
        let names = equipment_graphics::Names::read(&layers, &default_map, &mut warnings)?;
        let enablers = content_reader_military::technology::read_enablers(&layers, |limit| {
            scripted
                .as_ref()
                .map_err(Clone::clone)?
                .unrecorded_trigger_world()?
                .evaluate(limit, crate::triggers::Scopes::empty())
                .decided()
        })?;
        let pools = equipment_graphics::Content::load(&layers, &names)?;
        let geography = equipment_graphics::context::Content::load(
            &layers,
            &default_map,
            &provinces,
            &defines,
        )?;
        let dlc = layers.dlc_names()?;
        let sprites = registry.visible_sprites()
            .keys()
            .filter_map(|name| std::str::from_utf8(name).ok().map(str::to_owned))
            .collect();
        Ok(Self {
            catalogue: None,
            units,
            enablers,
            pools,
            geography,
            dlc,
            sprites,
            refused_sprites: BTreeMap::new(),
            files: Box::new(move |path| art_file_present(layers.presentation(), path)),
            stamped: false,
            load_warnings: warnings,
        })
    }

    pub fn load(catalogue: Arc<Catalogue>) -> io::Result<Self> {
        Self::with_equipment(catalogue, None)
    }

    pub fn with_equipment(
        catalogue: Arc<Catalogue>,
        units: Option<Arc<Database>>,
    ) -> io::Result<Self> {
        let layers = &catalogue.content.layers;
        let default_map = content_reader_key_names::terrain::read_default_map(layers)?;
        let provinces = content_reader_map::map::load_definitions(layers)?;
        let defines = catalogue.read_defines()?;
        let mut warnings = Vec::new();
        let files = Arc::clone(&catalogue);
        let (names, pools, enablers, units, geography) = std::thread::scope(|threads| {
            use content_reader_layers::parallel::spawn;
            let enablers = spawn(threads, || {
                dashboard_text::catalogue::capture(|| {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let world = std::cell::OnceCell::new();
                        content_reader_military::technology::read_enablers(layers, |limit| {
                            world
                                .get_or_init(|| {
                                    catalogue
                                        .scripted_content()
                                        .and_then(|content| content.unrecorded_trigger_world())
                                })
                                .as_ref()
                                .map_err(Clone::clone)?
                                .evaluate(limit, crate::triggers::Scopes::empty())
                                .decided()
                        })
                    }))
                })
            });
            let units = spawn(threads, || match units {
                Some(units) => Ok(units),
                None => {
                    let names = layers.key_name_products()?;
                    match names.equipment_database() {
                        Some(database) if database.lost.is_none() => {
                            for path in content_reader_key_names::equipment::files(layers)? {
                                path.read(layers)?;
                            }
                            Ok(Arc::clone(database))
                        }
                        _ => Database::shared_complete(layers),
                    }
                }
            });
            let geography = spawn(threads, || {
                equipment_graphics::context::Content::load(
                    layers,
                    &default_map,
                    &provinces,
                    defines,
                )
            });
            let names = equipment_graphics::Names::read(layers, &default_map, &mut warnings);
            let pools = names.as_ref().ok().map(|names| {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    equipment_graphics::Content::load(layers, names)
                }))
            });
            let (enablers, units, geography) = (enablers.join(), units.join(), geography.join());
            (names, pools, enablers, units, geography)
        });
        names?;
        let (enablers, noted) = settled(enablers);
        dashboard_text::catalogue::credit(&noted);
        let enablers = settled(enablers)?;
        let units = settled(units)?;
        let pools = settled(pools.expect("the pools are read once the names are"))?;
        let geography = settled(geography)?;
        Ok(Self {
            units,
            enablers,
            pools,
            geography,
            dlc: layers.dlc_names()?,
            sprites: BTreeSet::new(),
            refused_sprites: BTreeMap::new(),
            files: Box::new(move |path| {
                files
                    .content
                    .layers
                    .has_file(path)
                    .map_err(|error| format!("{path}: {error}"))
            }),
            stamped: inputs_stamped(layers).unwrap_or(false),
            catalogue: Some(catalogue),
            load_warnings: warnings,
        })
    }

    fn file(&self, path: &str) -> Result<bool, String> {
        (self.files)(path)
    }

    pub fn stamped(&self) -> bool {
        self.stamped
    }

    pub fn from_parts(
        units: Database,
        enablers: Enablers,
        pools: equipment_graphics::Content,
        dlc: BTreeSet<String>,
        sprites: BTreeSet<String>,
        files: BTreeSet<String>,
    ) -> Self {
        Self {
            catalogue: None,
            units: Arc::new(units),
            enablers,
            load_warnings: Vec::new(),
            pools,
            geography: Default::default(),
            dlc,
            sprites,
            refused_sprites: BTreeMap::new(),
            files: Box::new(move |path| Ok(files.contains(path))),
            stamped: false,
        }
    }

    pub fn with_geography(mut self, geography: equipment_graphics::context::Content) -> Self {
        self.geography = geography;
        self
    }

    pub fn warnings(&self) -> Vec<String> {
        let mut warnings = self.load_warnings.clone();
        let refused = &self.enablers.refused;
        if !refused.is_empty() {
            warnings.push(format!(
                "{} technologies enable equipment through a script, which is not supported; icons that would use them are unknown: {}",
                refused.len(),
                refused.join("; ")
            ));
        }
        if !self.enablers.unverified.is_empty() {
            warnings.push(format!(
                "technology equipment enablers are withheld because native technology reading failed: {}",
                self.enablers.unverified.join("; ")
            ));
        }
        warnings
    }

    pub fn preparation_warnings(&self) -> Vec<content_reader_parser::Diagnostic> {
        let mut warnings: Vec<_> = self
            .load_warnings
            .iter()
            .cloned()
            .map(|message| (content_reader_parser::DiagnosticKind::Warning, message))
            .collect();
        warnings.extend(self.enablers.refused.iter().map(|definition| {
            (content_reader_parser::DiagnosticKind::Unsupported,
                format!("technology {definition} enables equipment through a script, which is not supported; icons that would use it are unknown"))
        }));
        warnings.extend(self.enablers.unverified.iter().map(|reason| {
            (
                content_reader_parser::DiagnosticKind::Unsettled,
                format!(
                    "technology equipment enablers withheld because native technology reading failed: {reason}"
                ),
            )
        }));
        warnings.extend(self.pools.refusals().into_iter().map(|(subject, reason)| {
            (
                content_reader_parser::DiagnosticKind::Refused,
                format!("{subject}: {reason}"),
            )
        }));
        warnings
    }

    pub fn authored_icons(&self) -> io::Result<Vec<AuthoredIcons>> {
        self.units
            .equipment
            .iter()
            .map(|equipment| {
                let token = equipment
                    .name_str()
                    .map_err(|why| crate::invalid(why.to_string()))?
                    .to_owned();
                let mut current = Some(token.clone());
                let mut visited = BTreeSet::new();
                let mut icons = BTreeSet::new();
                let mut refusals = BTreeSet::new();
                while let Some(key) = current.take() {
                    if !visited.insert(key.clone()) {
                        break;
                    }
                    let model = match type_model(&self.units, &key) {
                        Ok(model) => model,
                        Err(reason) => {
                            refusals.insert(reason);
                            break;
                        }
                    };
                    let mut bases = vec![key.as_str()];
                    bases.extend(model.archetype);
                    for name in [Some(key.as_str()), model.archetype].into_iter().flatten() {
                        match self.enablers.single(name) {
                            Ok(Some(enabler)) => {
                                bases.push(&enabler.id);
                                match &enabler.parent {
                                    Ok(parent) => bases.extend(parent.as_deref()),
                                    Err(reason) => {
                                        refusals.insert(reason.clone());
                                    }
                                }
                            }
                            Ok(None) => {}
                            Err(reason) => {
                                refusals.insert(reason);
                            }
                        }
                    }
                    for base in bases {
                        for suffix in icon_suffixes(true) {
                            for candidate in sprite_names(base, suffix, None).into_iter().flatten()
                            {
                                match self.sprite(&candidate) {
                                    Ok(Some(icon)) => {
                                        icons.insert(icon);
                                    }
                                    Ok(None) => {}
                                    Err(reason) => {
                                        refusals.insert(reason);
                                    }
                                }
                            }
                        }
                    }
                    current = model.family.map(str::to_owned);
                }
                Ok(AuthoredIcons {
                    token,
                    icons: icons.into_iter().collect(),
                    refusals: refusals.into_iter().collect(),
                })
            })
            .collect()
    }

    fn sprite(&self, name: &str) -> Result<Option<Icon>, String> {
        if let Some(catalogue) = &self.catalogue {
            if let Some(reason) = catalogue.sprites.refusal(name) {
                return Err(reason);
            }
            return Ok(matches!(catalogue.sprites.find(name), Ok(Some(_)))
                .then(|| Icon::Sprite(name.to_owned())));
        }
        if let Some(reason) = self.refused_sprites.get(name) {
            return Err(format!("sprite {name} is refused: {reason}"));
        }
        Ok(self
            .sprites
            .contains(name)
            .then(|| Icon::Sprite(name.to_owned())))
    }

    fn existing(&self, name: &str) -> Result<Option<Icon>, String> {
        if let Some(icon) = self.sprite(name)? {
            return Ok(Some(icon));
        }
        if !reads_as_path(name) {
            return Ok(None);
        }
        Ok(self.file(name)?.then(|| Icon::Path(name.to_owned())))
    }

    pub(crate) fn answer(&self, probe: &Probe) -> Result<Option<Icon>, String> {
        match probe {
            Probe::Sprite(name) => self.sprite(name),
            Probe::File(path) => Ok(self.file(path)?.then(|| Icon::Path(path.clone()))),
            Probe::Existing(name) => self.existing(name),
        }
    }
}

fn art_file_present(layers: &content_reader_layers::ResolvedLayers, path: &str) -> Result<bool, String> {
    match layers.art_file_source(path.as_bytes()) {
        Ok(content_reader_layers::content::ArtFileSource::Mounted { .. }) => Ok(true),
        Ok(content_reader_layers::content::ArtFileSource::Unmounted { layer, reason }) => {
            let owner = &layers.layers[layer];
            let name = owner.display_name().unwrap_or(&owner.id);
            Err(format!(
                "{name} is not mounted ({reason}); {path} cannot be resolved without risking a substitute"
            ))
        }
        Ok(content_reader_layers::content::ArtFileSource::Missing) => Ok(false),
        Err(error) => Err(format!("{path}: {error}")),
    }
}

#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct IconHour {
    links: BTreeMap<u32, u32>,
    countries: BTreeMap<u32, ModelCountry>,
    slot_own_handles: BTreeMap<u32, u32>,
    handle_tags: BTreeMap<u32, String>,
    dynamic_tag_boundary: Result<i32, String>,
    original_refs: Result<BTreeMap<u32, (Recorded<Option<i32>>, Recorded<Option<Vec<u8>>>)>, String>,
    culture_2d: Result<BTreeMap<u32, String>, String>,
    prepared: Result<Prepared, String>,
}

fn undated(frame: &DecodedFrame, why: impl ToString) -> String {
    why.to_string()
        .replace(&format!("hour {} ", frame.hour), "")
}

impl IconHour {
    pub fn heap(&self) -> usize {
        let text = |text: &String| std::mem::size_of::<String>() + text.len();
        let optional = |text: &Option<String>| text.as_ref().map_or(0, String::len);
        self.links.len() * 2 * std::mem::size_of::<u32>()
            + self
                .countries
                .values()
                .map(|country| {
                    std::mem::size_of::<(u32, ModelCountry)>()
                        + country.tag.len()
                        + country.cosmetic.len()
                        + optional(&country.graphical_culture)
                        + optional(&country.original_tag)
                        + optional(&country.original_graphical_culture)
                        + country
                            .ruling_ideology
                            .as_ref()
                            .and_then(Option::as_ref)
                            .map_or(0, String::len)
                })
                .sum::<usize>()
            + self.slot_own_handles.len() * 2 * std::mem::size_of::<u32>()
            + self.handle_tags.values().map(text).sum::<usize>()
            + match &self.dynamic_tag_boundary {
                Ok(_) => 0,
                Err(why) => why.len(),
            }
            + match &self.original_refs {
                Ok(refs) => refs
                    .values()
                    .map(|(_, bytes)| match bytes {
                        Recorded::Value(Some(bytes)) => bytes.len(),
                        _ => 0,
                    })
                    .sum(),
                Err(why) => why.len(),
            }
            + match &self.culture_2d {
                Ok(cultures) => cultures.values().map(text).sum(),
                Err(why) => why.len(),
            }
            + match &self.prepared {
                Ok(prepared) => prepared.heap(),
                Err(why) => why.len(),
            }
    }

    pub fn read(frame: &DecodedFrame, content: &IconContent) -> io::Result<Self> {
        let failed = |error: io::Error| io::Error::new(error.kind(), undated(frame, &error));
        let links: BTreeMap<u32, u32> = frame
            .complete::<tables::CountryLinks>()
            .map_err(failed)?
            .iter()
            .map(|row: &CountryLinkRow| (row.id, row.canonical_slot))
            .collect();
        let countries = equipment_graphics::countries::read(frame).map_err(failed)?;
        let country_rows = frame.complete::<tables::Countries>().map_err(failed)?;
        let mut slot_own_handles: BTreeMap<u32, u32> = BTreeMap::new();
        let mut handle_tags: BTreeMap<u32, String> = BTreeMap::new();
        for row in country_rows {
            slot_own_handles.insert(row.canonical_slot, row.id);
            if let Ok(text) = std::str::from_utf8(&row.tag) {
                handle_tags.insert(row.id, text.to_owned());
            }
        }
        let appearance = frame.complete::<tables::CountryAppearance>();
        if let Ok(rows) = &appearance {
            for row in *rows {
                if let (Recorded::Value(Some(orig)), Recorded::Value(Some(bytes))) =
                    (row.original_tag_reference, &row.original_tag)
                    && orig > 0
                    && let Ok(text) = std::str::from_utf8(bytes)
                {
                    handle_tags
                        .entry(orig as u32)
                        .or_insert_with(|| text.to_owned());
                }
            }
        }
        let reserved = frame.complete::<tables::ReservedCountryNaming>();
        let dynamic_tag_boundary = match &reserved {
            Ok(rows) => {
                for row in *rows {
                    if let (Recorded::Value(Some(orig)), Recorded::Value(Some(bytes))) =
                        (row.original_tag_reference, &row.original_tag)
                        && orig > 0
                        && let Ok(text) = std::str::from_utf8(bytes)
                    {
                        handle_tags
                            .entry(orig as u32)
                            .or_insert_with(|| text.to_owned());
                    }
                }
                match rows.first().map(|row| row.dynamic_tag_boundary) {
                    Some(Recorded::Value(Some(boundary))) => Ok(boundary),
                    Some(Recorded::Value(None)) => Err(
                        "the country database's dynamic-tag boundary capture refused".to_owned(),
                    ),
                    Some(Recorded::NotRecorded) => Err(
                        "the country database's dynamic-tag boundary is not recorded".to_owned(),
                    ),
                    None => Err(
                        "reserved country naming has no row, so the dynamic-tag boundary is unknown"
                            .to_owned(),
                    ),
                }
            }
            Err(error) => Err(format!(
                "reserved country naming is unavailable, so the dynamic-tag boundary is unknown: {}",
                undated(frame, error)
            )),
        };
        let original_refs = appearance
            .as_ref()
            .map(|rows| {
                rows.iter()
                    .filter_map(|row| {
                        Some((
                            *links.get(&row.country)?,
                            (row.original_tag_reference, row.original_tag.clone()),
                        ))
                    })
                    .collect()
            })
            .map_err(|error| {
                format!(
                    "the country appearance table is unavailable, so the original tag is unknown: {}",
                    undated(frame, error)
                )
            });
        let culture_2d = appearance
            .as_ref()
            .map(|rows| {
                rows.iter()
                    .filter_map(|row| {
                        Some((
                            *links.get(&row.country)?,
                            String::from_utf8_lossy(&row.graphical_culture_2d).into_owned(),
                        ))
                    })
                    .collect()
            })
            .map_err(|error| {
                format!(
                    "the country appearance table is unavailable: {}",
                    undated(frame, error)
                )
            });
        Ok(Self {
            prepared: Prepared::read(frame, &content.geography).map_err(|why| undated(frame, why)),
            links,
            countries,
            slot_own_handles,
            handle_tags,
            dynamic_tag_boundary,
            original_refs,
            culture_2d,
        })
    }

    fn is_dynamic_slot(&self, slot: u32) -> Result<bool, String> {
        self.dynamic_tag_boundary
            .as_ref()
            .map(|&boundary| i64::from(slot) >= i64::from(boundary))
            .map_err(Clone::clone)
    }

    fn witnessed_tag(&self, raw: u32) -> Result<&str, String> {
        self.handle_tags
            .get(&raw)
            .map(String::as_str)
            .ok_or_else(|| format!("country reference {raw} has no witnessed tag spelling"))
    }

    fn original_reference_of(&self, slot: u32) -> Result<Option<u32>, String> {
        let refs = self.original_refs.as_ref().map_err(Clone::clone)?;
        let (reference, _) = refs
            .get(&slot)
            .ok_or_else(|| format!("country slot {slot} has no recorded appearance"))?;
        let raw = reference
            .require("original tag reference of country appearance")?
            .ok_or_else(|| {
                "original tag reference of country appearance capture refused".to_owned()
            })?;
        Ok((raw > 0).then_some(raw as u32))
    }

    fn dynamic_technology_tag_of(&self, slot: u32, own_handle: u32) -> Result<String, String> {
        let refs = self.original_refs.as_ref().map_err(Clone::clone)?;
        let (reference, original_tag) = refs.get(&slot).ok_or_else(|| {
            format!(
                "{} has no recorded appearance, so its original tag is unknown",
                self.handle_tags
                    .get(&own_handle)
                    .map_or("the country", String::as_str)
            )
        })?;
        let raw = reference
            .require("original tag reference of country appearance")?
            .ok_or_else(|| {
                "original tag reference of country appearance capture refused".to_owned()
            })?;
        if raw > 0 {
            if let Recorded::Value(Some(bytes)) = original_tag {
                return std::str::from_utf8(bytes)
                    .map(str::to_owned)
                    .map_err(|_| format!("original tag of country slot {slot} is not UTF-8"));
            }
            return self.witnessed_tag(raw as u32).map(str::to_owned);
        }
        self.witnessed_tag(own_handle).map(str::to_owned)
    }

    fn technology_tag_of(&self, slot: u32, own_handle: u32) -> Result<String, String> {
        let static_tag = || self.witnessed_tag(own_handle).map(str::to_owned);
        match self.is_dynamic_slot(slot) {
            Ok(false) => static_tag(),
            Ok(true) => self.dynamic_technology_tag_of(slot, own_handle),
            Err(boundary_err) => match (static_tag(), self.dynamic_technology_tag_of(slot, own_handle)) {
                (Ok(static_tag), Ok(dynamic_tag)) if static_tag == dynamic_tag => Ok(static_tag),
                _ => Err(boundary_err),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ContinentFact {
    Token(u32),
    State(u32),
    Slot(u32),
}

enum PoolMode<'a> {
    Discover {
        unknowns: &'a mut Vec<(ContinentFact, String, BTreeSet<String>)>,
        probes: &'a mut Vec<Probe>,
    },
    Assume {
        continents: &'a BTreeMap<ContinentFact, Option<String>>,
        probes: &'a mut Vec<Probe>,
    },
}

impl PoolMode<'_> {
    fn push(&mut self, probe: Probe) {
        match self {
            PoolMode::Discover { probes, .. } | PoolMode::Assume { probes, .. } => {
                probes.push(probe);
            }
        }
    }
}

pub fn variant_icon(
    content: &IconContent,
    hour: &IconHour,
    variant: &VariantDefinitionRow,
    spelling: &str,
) -> Result<Option<Icon>, String> {
    icon_choice(content, hour, variant, spelling).walk(|probe| content.answer(probe))
}

pub fn icon_choice(
    content: &IconContent,
    hour: &IconHour,
    variant: &VariantDefinitionRow,
    spelling: &str,
) -> IconChoice {
    let mut unknowns = Vec::new();
    let mut probes = Vec::new();
    let otherwise = rule(
        content,
        hour,
        variant,
        spelling,
        &mut PoolMode::Discover {
            unknowns: &mut unknowns,
            probes: &mut probes,
        },
    );
    if unknowns.is_empty() {
        return IconChoice {
            branches: vec![ChoiceBranch {
                assignment: Vec::new(),
                probes,
                otherwise,
            }],
            reasons: Vec::new(),
        };
    }
    let mut assignments: Vec<Vec<Option<String>>> = vec![Vec::new()];
    for (_, _, relevant) in &unknowns {
        let mut next = Vec::with_capacity(assignments.len() * (1 + relevant.len()));
        for base in &assignments {
            let mut none_vec = base.clone();
            none_vec.push(None);
            next.push(none_vec);
            for continent in relevant {
                let mut some_vec = base.clone();
                some_vec.push(Some(continent.clone()));
                next.push(some_vec);
            }
        }
        assignments = next;
    }
    let reasons: Vec<String> = unknowns.iter().map(|(_, why, _)| why.clone()).collect();
    let mut branches = Vec::with_capacity(assignments.len());
    for assignment in assignments {
        let continents: BTreeMap<ContinentFact, Option<String>> = unknowns
            .iter()
            .zip(&assignment)
            .map(|((fact, _, _), continent)| (*fact, continent.clone()))
            .collect();
        let mut branch_probes = Vec::new();
        let branch_otherwise = rule(
            content,
            hour,
            variant,
            spelling,
            &mut PoolMode::Assume {
                continents: &continents,
                probes: &mut branch_probes,
            },
        );
        branches.push(ChoiceBranch {
            assignment,
            probes: branch_probes,
            otherwise: branch_otherwise,
        });
    }
    if let Some(first) = branches.first()
        && branches
            .iter()
            .all(|branch| branch.probes == first.probes && branch.otherwise == first.otherwise)
    {
        return IconChoice {
            branches: vec![ChoiceBranch {
                assignment: Vec::new(),
                probes: first.probes.clone(),
                otherwise: first.otherwise.clone(),
            }],
            reasons: Vec::new(),
        };
    }
    IconChoice { branches, reasons }
}

fn rule(
    content: &IconContent,
    hour: &IconHour,
    variant: &VariantDefinitionRow,
    spelling: &str,
    mode: &mut PoolMode<'_>,
) -> Result<(), String> {
    let key = content
        .units
        .named(spelling)
        .ok_or_else(|| missing_type(&content.units, spelling))?
        .name_str()
        .map_err(|error| error.to_string())?;
    let entry = String::from_utf8_lossy(&variant.icon);
    if variant.icon_kind == 1 && entry == "." {
        let creator = variant.creator_link;
        let (country, ideology, culture) = if (creator as i32) <= 0 {
            (None, None, None)
        } else {
            let slot = hour
                .links
                .get(&creator)
                .ok_or_else(|| format!("variant creator {creator} has no country link"))?;
            let model = hour.countries.get(slot).ok_or_else(|| {
                format!("variant creator {creator} has no recorded appearance")
            })?;
            let ideology = model
                .ruling_ideology
                .clone()
                .ok_or("the creator's ruling party is unknown at this hour: not recorded, or its token is not text")?;
            let culture = model
                .graphical_culture
                .clone()
                .ok_or("the creator's graphical culture is not recorded")?;
            (
                Some(creator),
                ideology.filter(|token| !token.is_empty()),
                Some(culture),
            )
        };
        pool(
            content,
            hour,
            key,
            country,
            ideology.as_deref(),
            culture.as_deref(),
            mode,
        )?;
    } else if variant.icon_active != 0 {
        if variant.icon_kind > 1 {
            return Err(format!(
                "variant {} has an icon entry of unknown kind {}",
                variant.id, variant.icon_kind
            ));
        }
        let probe = if variant.icon_kind == 0 {
            Probe::Sprite(entry.clone().into_owned())
        } else {
            Probe::File(entry.clone().into_owned())
        };
        mode.push(probe);
    }
    let (l_link, l_slot) = if (variant.origin_link as i32) > 0 {
        let slot = *hour.links.get(&variant.origin_link).ok_or_else(|| {
            format!("variant origin {} has no country link", variant.origin_link)
        })?;
        (Some(variant.origin_link), Some(slot))
    } else if (variant.creator_link as i32) > 0 {
        let creator_slot = *hour.links.get(&variant.creator_link).ok_or_else(|| {
            format!(
                "variant creator {} has no country link",
                variant.creator_link
            )
        })?;
        let static_creator = (Some(variant.creator_link), Some(creator_slot));
        let dynamic_creator = || -> Result<(Option<u32>, Option<u32>), String> {
            match hour.original_reference_of(creator_slot)? {
                Some(original) => {
                    let slot = *hour.links.get(&original).ok_or_else(|| {
                        format!("original country reference {original} has no country link")
                    })?;
                    Ok((Some(original), Some(slot)))
                }
                None => {
                    let own = *hour.slot_own_handles.get(&creator_slot).ok_or_else(|| {
                        format!("country slot {creator_slot} has no Countries row")
                    })?;
                    Ok((Some(own), Some(creator_slot)))
                }
            }
        };
        match hour.is_dynamic_slot(creator_slot) {
            Ok(false) => static_creator,
            Ok(true) => dynamic_creator()?,
            Err(boundary_err) => match dynamic_creator() {
                Ok(dynamic) if dynamic == static_creator => dynamic,
                _ => return Err(boundary_err),
            },
        }
    } else {
        (None, None)
    };
    let tag: Result<Option<&str>, String> = match l_link {
        Some(raw) => hour.witnessed_tag(raw).map(Some),
        None => Ok(None),
    };
    let culture_2d: Result<Option<String>, String> = match l_slot {
        None => Ok(None),
        Some(slot) => match &hour.culture_2d {
            Err(reason) => Err(reason.clone()),
            Ok(cultures) => cultures
                .get(&slot)
                .map(|culture| (!culture.is_empty()).then(|| culture.clone()))
                .ok_or_else(|| {
                    format!(
                        "{} has no recorded appearance, so its 2D culture is unknown",
                        tag.as_ref()
                            .ok()
                            .and_then(|tag| *tag)
                            .unwrap_or("the country")
                    )
                }),
        },
    };
    let technology_tag: Result<Option<String>, String> = match (l_slot, l_link) {
        (Some(slot), Some(raw)) => hour.technology_tag_of(slot, raw).map(Some),
        _ => Ok(None),
    };
    let capital = variant.classifier_flags & (1 << 6) != 0;
    fallback(
        content,
        hour,
        key,
        Names {
            tag,
            technology_tag: technology_tag
                .as_ref()
                .map(Option::as_deref)
                .map_err(Clone::clone),
            culture_2d: culture_2d
                .as_ref()
                .map(Option::as_deref)
                .map_err(Clone::clone),
        },
        capital,
        l_link,
        &mut BTreeSet::new(),
        mode,
    )
}

#[derive(Clone)]
struct Names<'a> {
    tag: Result<Option<&'a str>, String>,
    technology_tag: Result<Option<&'a str>, String>,
    culture_2d: Result<Option<&'a str>, String>,
}

fn fallback(
    content: &IconContent,
    hour: &IconHour,
    key: &str,
    names: Names,
    capital: bool,
    country: Option<u32>,
    visited: &mut BTreeSet<String>,
    mode: &mut PoolMode<'_>,
) -> Result<(), String> {
    let model = type_model(&content.units, key)?;
    if !visited.insert(key.to_owned()) {
        return Ok(());
    }
    let suffixes = icon_suffixes(capital);
    for suffix in suffixes {
        for name in [Some(key), model.archetype].into_iter().flatten() {
            if let Some(tag) = names.tag.clone()? {
                mode.push(Probe::Sprite(format!("GFX_{tag}_{name}{suffix}")));
            }
            mode.push(Probe::Sprite(format!("GFX_{name}{suffix}")));
        }
    }
    for name in [Some(key), model.archetype].into_iter().flatten() {
        let Some(enabler) = content.enablers.single(name)? else {
            continue;
        };
        for suffix in suffixes {
            for technology in [
                Ok(Some(enabler.id.as_str())),
                enabler.parent.as_ref().map(Option::as_deref),
            ] {
                let Some(technology) = technology.map_err(Clone::clone)? else {
                    continue;
                };
                if let Some(tag) = names.technology_tag.clone()? {
                    mode.push(Probe::Sprite(format!("GFX_{tag}_{technology}{suffix}")));
                }
                if let Some(culture) = names.culture_2d.clone()? {
                    mode.push(Probe::Sprite(format!("GFX_{culture}_{technology}{suffix}")));
                }
                mode.push(Probe::Sprite(format!("GFX_{technology}{suffix}")));
            }
        }
    }
    if let Some(family) = model.family {
        fallback(
            content,
            hour,
            family,
            names.clone(),
            capital,
            country,
            visited,
            mode,
        )?;
    }
    pool(content, hour, key, country, None, None, mode)
}

fn pool(
    content: &IconContent,
    hour: &IconHour,
    key: &str,
    country: Option<u32>,
    ideology: Option<&str>,
    culture: Option<&str>,
    mode: &mut PoolMode<'_>,
) -> Result<(), String> {
    let model = type_model(&content.units, key)?;
    let family = model
        .family
        .map(|name| type_model(&content.units, name).map(|family| (name, family.archetype)))
        .transpose()?;
    let (place, continent_fact, country_tag, capital_continent) = match country {
        Some(raw) => {
            let prepared = hour.prepared.as_ref().map_err(Clone::clone)?;
            let place = prepared.context(raw)?;
            let continent_fact = prepared
                .capital_continent_token(raw)
                .map(ContinentFact::Token)
                .or_else(|| prepared.capital_state(raw).map(ContinentFact::State))
                .or_else(|| prepared.slot(raw).map(ContinentFact::Slot));
            let country_tag = if place.is_some() {
                Some(hour.witnessed_tag(raw)?)
            } else {
                None
            };
            let capital_continent = match place {
                Some(place) => place
                    .capital_continent
                    .as_ref()
                    .map(Option::as_deref)
                    .map_err(String::as_str),
                None => Ok(None),
            };
            (place, continent_fact, country_tag, capital_continent)
        }
        None => (None, None, None, Ok(None)),
    };
    let context = Context {
        equipment: key,
        archetype: model.archetype,
        family,
        subunit: None,
        ideology,
        culture,
        country: country_tag,
        faction_tags: place.map_or(&[], |place| place.faction_tags.as_slice()),
        capital_continent,
    };
    let has_dlc = |name: &str| Some(content.dlc.contains(name));
    match (continent_fact, capital_continent, mode) {
        (Some(fact), Err(why), PoolMode::Discover { unknowns, probes }) => {
            let relevant = content.pools.relevant_icon_continents(&context, has_dlc);
            if relevant.is_empty() {
                let branch = content
                    .pools
                    .lookup_icon_branch(&context, None, has_dlc)?;
                for candidate in branch.items {
                    probes.push(Probe::Existing(candidate));
                }
            } else if let Some((_, _, set)) = unknowns.iter_mut().find(|(id, _, _)| *id == fact) {
                set.extend(relevant);
            } else {
                unknowns.push((fact, why.to_owned(), relevant.into_iter().collect()));
            }
        }
        (Some(fact), Err(why), PoolMode::Assume { continents, probes }) => {
            let assumed = continents.get(&fact).and_then(|c| c.as_deref());
            let branch = content
                .pools
                .lookup_icon_branch(&context, assumed, has_dlc)?;
            for candidate in branch.items {
                probes.push(Probe::Existing(candidate));
            }
            if branch.truncated {
                return Err(why.to_owned());
            }
        }
        (_, _, mode) => {
            let plan = content.pools.lookup_icon_plan(&context, has_dlc)?;
            if let Some(branch) = plan.branches.into_iter().next() {
                for candidate in branch.items {
                    mode.push(Probe::Existing(candidate));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
