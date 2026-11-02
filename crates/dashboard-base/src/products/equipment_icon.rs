//! An equipment variant's icon as the game's production row picks it, through
//! the recorded content stack:
//!
//! 1. An active entry that is not the auto-design `"."`: a kind-0 sprite that
//!    exists, or a kind-1 texture path whose file exists; otherwise step 3.
//! 2. An active `"."`: the graphic pool for the creator, with its ruling party's
//!    ideology group and its graphical culture; without a result, step 3.
//! 3. The fallback for country L (the origin when set, else the creator, by its
//!    original tag when dynamic), in this order for the variant's type and then
//!    recursively its family: sprites named after the
//!    type and its archetype (`_capital_medium` first for a capital ship, then
//!    `_medium`), then the single technology enabling the type, then the single one
//!    enabling its archetype (by id, then `parent`, each with L's tag, L's 2D
//!    culture and no prefix), then the graphic pool for L alone.
//! 4. Otherwise no icon.
//!
//! Products about equipment (production lines, stockpile) read icons here. The rule's only
//! questions of the artwork are whether a sprite is declared and whether a texture file
//! exists ([`Probe`]); a choice can be made with them left open ([`icon_choice`]) and answered
//! later by whichever presentation shows it ([`IconChoice::shown`]).

use crate::catalogue::Catalogue;
use installed_content_core::{equipment::Database, technology::Enablers};
use installed_content_equipment_graphics::{
    equipment_graphics, equipment_graphics::Context, equipment_graphics::context::Prepared,
    equipment_graphics::countries::ModelCountry,
};
use replay_data::{rows::CountryLinkRow, rows::VariantDefinitionRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};

/// What the icon rule reads of an equipment type: its archetype (the type
/// itself for an archetype, `None` for the native null object) and its `family`.
struct TypeModel<'a> {
    archetype: Option<&'a str>,
    family: Option<&'a str>,
}

/// Why a type the rule needs is not in the recorded content: refused by the reader
/// (named), or not declared at all.
fn missing_type(equipment: &Database, key: &str) -> String {
    match &equipment.lost {
        Some(reason) => format!("equipment {key}'s database is refused: {reason}"),
        None => format!("equipment {key} is not in the recorded content"),
    }
}

/// A type's archetype and family, found by the native database's name lookup.
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
        // The default archetype is the null object: its name is empty and it is invalid.
        archetype: kind.archetype.map(name).transpose()?,
        family: (kind.family != at).then(|| name(kind.family)).transpose()?,
    })
}

/// Where an icon's picture comes from.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum Icon {
    /// A sprite the content's interface files declare.
    Sprite(String),
    /// A texture file of the content stack.
    Path(String),
}

/// One question the icon rule asks of the artwork, in the order it asks.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum Probe {
    /// Whether a sprite is declared (an `Err` when it is refused).
    Sprite(String),
    /// Whether a texture file exists.
    File(String),
    /// A name from a pool: a declared sprite, else a texture file when it reads as a path.
    Existing(String),
}

/// A variant's icon with the artwork questions left open: the rule's questions in the order
/// it asks them, and what it gives when every answer is no. The first question a
/// presentation answers yes gives the icon it shows ([`Self::walk`]).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct IconChoice {
    pub probes: Vec<Probe>,
    /// With every answer no: no icon, or why it cannot be known.
    pub otherwise: Result<(), String>,
}

impl IconChoice {
    /// A choice that cannot be made, with why.
    pub fn unknown(reason: String) -> Self {
        Self {
            probes: Vec::new(),
            otherwise: Err(reason),
        }
    }

    /// The icon the rule chooses where `answer` answers its questions: the first one found,
    /// else none or why; a question `answer` cannot tell stops the walk with why, as it stops
    /// the rule.
    pub fn walk(
        &self,
        answer: impl Fn(&Probe) -> Result<Option<Icon>, String>,
    ) -> Result<Option<Icon>, String> {
        for probe in &self.probes {
            if let Some(icon) = answer(probe)? {
                return Ok(Some(icon));
            }
        }
        self.otherwise.clone().map(|()| None)
    }

    /// The icon `catalogue` shows: sprites its presentation declares, texture files its
    /// presentation stack holds.
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
            layers
                .has_file(path)
                .map(|found| found.then(|| Icon::Path(path.to_owned())))
                .map_err(|error| format!("{path}: {error}"))
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

/// Whether a name from a pool or an icon entry is a texture path: the save loader classifies
/// a string holding `/` or `.` as one.
fn reads_as_path(name: &str) -> bool {
    name.contains('/') || name.contains('.')
}

/// Every equipment product decodes the same native sprite frame/texture path.
pub fn image(catalogue: &Catalogue, icon: &Icon, max_edge: u32) -> io::Result<String> {
    let layers = &catalogue.content.layers;
    let decode = *catalogue
        .gui_decode
        .as_ref()
        .map_err(|why| crate::invalid(why.clone()))?;
    match icon {
        Icon::Sprite(name) => {
            let sprite = catalogue.sprites.lookup(name).map_err(crate::invalid)?;
            crate::catalogue::sprite_frame_png(layers, sprite, decode, 0, max_edge)
        }
        Icon::Path(path) => crate::catalogue::texture_png(layers, path, decode, 1, max_edge),
    }
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

/// Whether the content stack holds a file the texture loader can read at a
/// relative path; `Err` when that cannot be told.
type Files = Box<dyn Fn(&str) -> Result<bool, String> + Send + Sync>;

/// The content the rule reads, loaded once per catalogue.
pub struct IconContent {
    /// The catalogue it was read from; none for fixtures.
    pub catalogue: Option<Arc<Catalogue>>,
    units: Arc<Database>,
    enablers: Enablers,
    /// What loading the content counted and skipped, for the product's warnings.
    load_warnings: Vec<String>,
    pools: equipment_graphics::Content,
    geography: equipment_graphics::context::Content,
    dlc: BTreeSet<String>,
    /// Every sprite the interface files declare.
    sprites: BTreeSet<String>,
    refused_sprites: BTreeMap<String, String>,
    files: Files,
    /// Whether every file it read is one the store key stamps ([`inputs_stamped`]).
    stamped: bool,
    /// A digest of what a choice ([`icon_choice`]) reads that the recorded content's hash
    /// does not cover: the graphic pools, the geography and the DLC names. The declared
    /// sprites and texture files are not among them: a choice leaves those questions open.
    visual: String,
}

/// A part's result, its panic resumed here once the parts before it succeeded.
fn settled<T>(read: std::thread::Result<T>) -> T {
    read.unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

/// Whether every file icon content reads is one the store key stamps: the folders it lists are,
/// but `default.map`, as a read resolves it, may name its definitions or continents where
/// coverage stops.
pub fn inputs_stamped(layers: &installed_content_core::ResolvedLayers) -> io::Result<bool> {
    if !layers.stamped("map/default.map")? {
        return Ok(false);
    }
    let default_map = installed_content_core::terrain::read_default_map(layers)?;
    for name in ["definitions", "continent"] {
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
    pub fn load(catalogue: Arc<Catalogue>) -> io::Result<Self> {
        Self::with_equipment(catalogue, None)
    }

    pub fn with_equipment(
        catalogue: Arc<Catalogue>,
        units: Option<Arc<Database>>,
    ) -> io::Result<Self> {
        let layers = &catalogue.content.layers;
        let default_map = installed_content_core::terrain::read_default_map(layers)?;
        let provinces = installed_content_core::map::parse_definitions(
            &layers.read(&default_map.file("definitions")?)?,
        )?;
        let defines = installed_content_core::defines::read(layers)?;
        let mut warnings = Vec::new();
        let files = Arc::clone(&catalogue);
        // The enablers, the equipment and the geography read on helper threads while this one
        // reads the graphics' names and pools; each is taken in the order they were written.
        let (names, pools, enablers, units, geography) = std::thread::scope(|threads| {
            use installed_content_core::parallel::spawn;
            let enablers = spawn(threads, || {
                // Scripted content's refusal notes reach this thread's consumers when joined, a
                // panic's too.
                dashboard_text::catalogue::capture(|| {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        // The game checks each equipment limit at content load in a fresh empty
                        // scope.
                        let world = std::cell::OnceCell::new();
                        installed_content_core::technology::read_enablers(layers, |limit| {
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
                    let names = layers.key_names()?;
                    match names.equipment_database() {
                        Some(database) if database.lost.is_none() => {
                            // Reuse registration only after the ordinary verified reads,
                            // in the order followed by the complete database loader.
                            for path in installed_content_core::equipment::files(layers) {
                                path.read(layers)?;
                            }
                            Ok(Arc::clone(database))
                        }
                        // Preserve the loader's stopped reads and complete-database
                        // refusal, including when only a prefix could be registered.
                        _ => Database::load_complete(layers).map(Arc::new),
                    }
                }
            });
            // The defines' Lua state synchronizes each read, so this thread shares it.
            let geography = spawn(threads, || {
                equipment_graphics::context::Content::load(
                    layers,
                    &default_map,
                    &provinces,
                    &defines,
                )
            });
            let names = equipment_graphics::Names::read(layers, &default_map, &mut warnings);
            // Taken after the enablers and the equipment, a panic included.
            let pools = names.as_ref().ok().map(|names| {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    equipment_graphics::Content::load(layers, names)
                }))
            });
            let (enablers, units, geography) = (enablers.join(), units.join(), geography.join());
            (names, pools, enablers, units, geography)
        });
        // The first part to fail in the order they were written decides, by error or panic.
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
            sprites: catalogue
                .sprites
                .iter()
                .map(|(key, _)| key.clone())
                .collect(),
            refused_sprites: catalogue
                .sprites
                .refused_names()
                .map(|(key, why)| (key.clone(), why.clone()))
                .collect(),
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
            visual: String::new(),
        }
        .fingerprinted())
    }

    /// The content with the digest of its visual inputs.
    fn fingerprinted(mut self) -> Self {
        let inputs = format!("{:?}\0{:?}\0{:?}", self.pools, self.geography, self.dlc);
        self.visual = blake3::hash(inputs.as_bytes()).to_hex().to_string();
        self
    }

    /// Whether a texture file exists.
    fn file(&self, path: &str) -> Result<bool, String> {
        (self.files)(path)
    }

    /// The digest of a choice's inputs outside the recorded content's hash; the rest it
    /// reads (the equipment database, the technologies' enablers, the map) is game content
    /// the recording pins by hash.
    pub fn visual_inputs(&self) -> &str {
        &self.visual
    }

    /// Whether every file it read is one the store key stamps ([`inputs_stamped`]).
    pub fn stamped(&self) -> bool {
        self.stamped
    }

    /// Content from its parts, for fixtures: no geography, the given sprites and files.
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
            visual: String::new(),
        }
        .fingerprinted()
    }

    /// What loading counted and skipped (country tag keys the game declares no tag
    /// for), and the technologies whose `enable_equipments` holds a script, each named
    /// in a counted warning: only the types those may enable lose the technology step.
    pub fn warnings(&self) -> Vec<String> {
        let mut warnings = self.load_warnings.clone();
        let refused = &self.enablers.refused;
        if refused.is_empty() {
            return warnings;
        }
        warnings.push(format!(
            "{} technologies enable equipment through a script, which is not supported; icons that would use them are unknown: {}",
            refused.len(),
            refused.join("; ")
        ));
        warnings
    }

    pub fn preparation_warnings(&self) -> Vec<installed_content_core::Diagnostic> {
        let mut warnings: Vec<_> = self
            .load_warnings
            .iter()
            .cloned()
            .map(|message| (installed_content_core::DiagnosticKind::Warning, message))
            .collect();
        warnings.extend(self.enablers.refused.iter().map(|definition| {
            (installed_content_core::DiagnosticKind::Unsupported,
                format!("technology {definition} enables equipment through a script, which is not supported; icons that would use it are unknown"))
        }));
        warnings.extend(self.pools.refusals().into_iter().map(|(subject, reason)| {
            (
                installed_content_core::DiagnosticKind::Refused,
                format!("{subject}: {reason}"),
            )
        }));
        warnings
    }

    /// Authored generic candidates only. No missing country operand is treated as
    /// known absence, and no variant entry or graphic-pool choice is invented.
    /// Both native suffixes form a candidate inventory; they do not select a
    /// capital-ship classifier for any variant.
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
                    // The null archetype has no direct sprites. Its technology
                    // lookup cannot match: the name's sentinel hash is -1.
                    bases.extend(model.archetype);
                    for name in [Some(key.as_str()), model.archetype].into_iter().flatten() {
                        // A scripted-enabler Err is already named by preparation_warnings.
                        if let Ok(Some(enabler)) = self.enablers.single(name) {
                            bases.push(&enabler.id);
                            match &enabler.parent {
                                Ok(parent) => bases.extend(parent.as_deref()),
                                Err(reason) => {
                                    refusals.insert(reason.clone());
                                }
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

    /// A declared sprite by its exact name: the sprite registry compares names byte
    /// for byte (see `installed_content_gfx::gfx::SpriteSet`),
    /// unlike the lexer tokens a name is built from.
    fn sprite(&self, name: &str) -> Result<Option<Icon>, String> {
        if let Some(reason) = self.refused_sprites.get(name) {
            return Err(format!("sprite {name} is refused: {reason}"));
        }
        Ok(self
            .sprites
            .contains(name)
            .then(|| Icon::Sprite(name.to_owned())))
    }

    /// A name from a pool or an icon entry: a sprite, else a texture file (the save
    /// loader classifies a string holding `/` or `.` as a path).
    fn existing(&self, name: &str) -> Result<Option<Icon>, String> {
        if let Some(icon) = self.sprite(name)? {
            return Ok(Some(icon));
        }
        if !reads_as_path(name) {
            return Ok(None);
        }
        Ok(self.file(name)?.then(|| Icon::Path(name.to_owned())))
    }

    /// The answer to one of the rule's questions in this content.
    pub(crate) fn answer(&self, probe: &Probe) -> Result<Option<Icon>, String> {
        match probe {
            Probe::Sprite(name) => self.sprite(name),
            Probe::File(path) => Ok(self.file(path)?.then(|| Icon::Path(path.clone()))),
            Probe::Existing(name) => self.existing(name),
        }
    }
}

/// One hour's country operands the rule reads.
pub struct IconHour {
    links: BTreeMap<u32, u32>,
    countries: BTreeMap<u32, ModelCountry>,
    /// Each country's tag, from Countries (recorded with every link).
    tags: BTreeMap<u32, String>,
    slots_by_tag: BTreeMap<String, u32>,
    /// Each country's 2D graphical culture (empty when it has none), or why this
    /// hour's appearance table is unavailable.
    culture_2d: Result<BTreeMap<u32, String>, String>,
    /// Faction and capital operands, or why this hour lacks them.
    prepared: Result<Prepared, String>,
}

/// A table's reason without the frame's hour, so the same gap read again is the same choice.
fn undated(frame: &DecodedFrame, why: impl ToString) -> String {
    why.to_string().replace(&format!("hour {} ", frame.hour), "")
}

impl IconHour {
    /// The bytes it holds on the heap, about.
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
            + self.tags.values().map(text).sum::<usize>()
            + self.slots_by_tag.keys().map(text).sum::<usize>()
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
        let tags: BTreeMap<u32, String> = frame
            .complete::<tables::Countries>()
            .map_err(failed)?
            .iter()
            .filter_map(|row| {
                Some((
                    row.canonical_slot,
                    std::str::from_utf8(&row.tag).ok()?.to_owned(),
                ))
            })
            .collect();
        let slots_by_tag = tags
            .iter()
            .map(|(slot, tag)| (tag.clone(), *slot))
            .collect();
        let culture_2d = frame
            .complete::<tables::CountryAppearance>()
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
            tags,
            slots_by_tag,
            culture_2d,
        })
    }
}

/// The variant's icon at this hour; `Err` says why it cannot be known (a table the
/// rule reads is missing, or content the rule cannot decide).
pub fn variant_icon(
    content: &IconContent,
    hour: &IconHour,
    variant: &VariantDefinitionRow,
    spelling: &str,
) -> Result<Option<Icon>, String> {
    rule(content, hour, variant, spelling, &|probe| content.answer(&probe))
}

/// The variant's icon at this hour with the artwork questions left open, for any
/// presentation to answer ([`IconChoice::shown`]): what [`variant_icon`] chooses wherever the
/// answers are this content's.
///
/// The questions are those the rule asks when every answer is no, recorded in order. They
/// do not depend on the answers: the rule's only inputs a presentation can change are these
/// answers, and it uses each only to return the icon found (or, where one cannot be told, to
/// stop with why); no answer names a later candidate, reaches a recorded fact or decides a
/// later branch (the pools are tried as one list, first found wins). So up to the first yes,
/// the questions asked with any answers are these, and walking them makes the rule's choice.
pub fn icon_choice(
    content: &IconContent,
    hour: &IconHour,
    variant: &VariantDefinitionRow,
    spelling: &str,
) -> IconChoice {
    let probes = std::cell::RefCell::new(Vec::new());
    let otherwise = rule(content, hour, variant, spelling, &|probe| {
        probes.borrow_mut().push(probe);
        Ok(None)
    });
    IconChoice {
        probes: probes.into_inner(),
        // Nothing was found, so the rule ended with no icon or why.
        otherwise: otherwise.map(|_| ()),
    }
}

/// The answer to one of the rule's questions.
type Ask<'a> = dyn Fn(Probe) -> Result<Option<Icon>, String> + 'a;

/// [`variant_icon`]'s rule, asking `ask` each artwork question.
fn rule(
    content: &IconContent,
    hour: &IconHour,
    variant: &VariantDefinitionRow,
    spelling: &str,
    ask: &Ask,
) -> Result<Option<Icon>, String> {
    let key = content
        .units
        .named(spelling)
        .ok_or_else(|| missing_type(&content.units, spelling))?
        .name_str()
        .map_err(|error| error.to_string())?;
    let entry = String::from_utf8_lossy(&variant.icon);
    // Step 1: an active entry that exists; the auto-design "." is step 2.
    if variant.icon_active != 0 {
        if variant.icon_kind > 1 {
            return Err(format!(
                "variant {} has an icon entry of unknown kind {}",
                variant.id, variant.icon_kind
            ));
        }
        if variant.icon_kind == 1 && entry == "." {
            // Step 2: the creator's pool, with its ideology group and 3D culture.
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
            if let Some(icon) = pool(
                content,
                hour,
                key,
                country,
                ideology.as_deref(),
                culture.as_deref(),
                ask,
            )? {
                return Ok(Some(icon));
            }
        } else {
            let found = if variant.icon_kind == 0 {
                ask(Probe::Sprite(entry.clone().into_owned()))?
            } else {
                ask(Probe::File(entry.clone().into_owned()))?
            };
            if found.is_some() {
                return Ok(found);
            }
        }
    }
    // Step 3: the country L the fallback names icons after.
    let l_slot =
        if (variant.origin_link as i32) > 0 {
            Some(*hour.links.get(&variant.origin_link).ok_or_else(|| {
                format!("variant origin {} has no country link", variant.origin_link)
            })?)
        } else if (variant.creator_link as i32) > 0 {
            let creator = *hour.links.get(&variant.creator_link).ok_or_else(|| {
                format!(
                    "variant creator {} has no country link",
                    variant.creator_link
                )
            })?;
            let model = hour
                .countries
                .get(&creator)
                .ok_or("the variant's creator has no recorded appearance")?;
            // A dynamic creator is named by its original tag's country.
            Some(match &model.original_tag {
                Some(tag) if *tag != model.tag => *hour
                    .slots_by_tag
                    .get(tag)
                    .ok_or_else(|| format!("original tag {tag} holds no country"))?,
                Some(_) => creator,
                None => return Err("the creator's original tag is not recorded".to_owned()),
            })
        } else {
            None
        };
    // L's tag from Countries; its 2D culture from its appearance, unknown (not none)
    // where this hour lacks it.
    let tag = match l_slot {
        Some(slot) => Some(
            hour.tags
                .get(&slot)
                .cloned()
                .ok_or_else(|| format!("country slot {slot} has no Countries row"))?,
        ),
        None => None,
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
                        tag.as_deref().unwrap_or("the country")
                    )
                }),
        },
    };
    // Technology sprites name L by its original tag when L is dynamic: an origin
    // D01 named after AAA tries `GFX_AAA_<technology>_medium`. Unknown, not skipped,
    // where L's original tag is not recorded.
    let technology_tag: Result<Option<String>, String> = match l_slot {
        None => Ok(None),
        Some(slot) => hour
            .countries
            .get(&slot)
            .ok_or_else(|| {
                format!(
                    "{} has no recorded appearance, so its original tag is unknown",
                    tag.as_deref().unwrap_or("the country")
                )
            })
            .and_then(|model| {
                model
                    .original_tag
                    .clone()
                    .map(Some)
                    .ok_or_else(|| format!("{}'s original tag is not recorded", model.tag))
            }),
    };
    let capital = variant.classifier_flags & (1 << 6) != 0;
    let l_link = l_slot.and_then(|slot| {
        hour.links
            .iter()
            .find(|(_, s)| **s == slot)
            .map(|(raw, _)| *raw)
    });
    fallback(
        content,
        hour,
        key,
        Names {
            tag: tag.as_deref(),
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
        ask,
    )
}

/// What the fallback names sprites after for country L: its tag (type sprites), the
/// tag technology sprites use (its original tag when dynamic) and its 2D culture;
/// an `Err` is unknown, needed only when the rule reaches it.
#[derive(Clone)]
struct Names<'a> {
    tag: Option<&'a str>,
    technology_tag: Result<Option<&'a str>, String>,
    culture_2d: Result<Option<&'a str>, String>,
}

/// Icon fallback for one equipment type, then its family.
fn fallback(
    content: &IconContent,
    hour: &IconHour,
    key: &str,
    names: Names,
    capital: bool,
    country: Option<u32>,
    visited: &mut BTreeSet<String>,
    ask: &Ask,
) -> Result<Option<Icon>, String> {
    let model = type_model(&content.units, key)?;
    if !visited.insert(key.to_owned()) {
        return Ok(None);
    }
    let suffixes = icon_suffixes(capital);
    // The type then its valid archetype, tag-prefixed then plain.
    // A null archetype skips only its own direct sprites.
    for suffix in suffixes {
        for name in [Some(key), model.archetype].into_iter().flatten() {
            for candidate in sprite_names(name, suffix, names.tag).into_iter().flatten() {
                if let Some(icon) = ask(Probe::Sprite(candidate))? {
                    return Ok(Some(icon));
                }
            }
        }
    }
    // The single enabling technology's id, then its parent,
    // with L's tag, L's 2D culture and no prefix; the type's, then its archetype's.
    // The null object's name has a -1 cached hash and cannot match an authored key.
    for name in [Some(key), model.archetype].into_iter().flatten() {
        // Found as the lexer identifies the key; unknown where a scripted unlock
        // may enable it, since whether exactly one technology does is then not known.
        let Some(enabler) = content.enablers.single(name)? else {
            continue;
        };
        for suffix in suffixes {
            // A parent whose text cannot be held is unknown once the id has missed.
            for technology in [
                Ok(Some(enabler.id.as_str())),
                enabler.parent.as_ref().map(Option::as_deref),
            ] {
                let Some(technology) = technology.map_err(Clone::clone)? else {
                    continue;
                };
                // L's (original) tag first, then its 2D culture: each unknown leaves
                // the icon unknown rather than skipping to the next name.
                if let Some(tag) = names.technology_tag.clone()?
                    && let Some(icon) =
                        ask(Probe::Sprite(format!("GFX_{tag}_{technology}{suffix}")))?
                {
                    return Ok(Some(icon));
                }
                if let Some(culture) = names.culture_2d.clone()?
                    && let Some(icon) =
                        ask(Probe::Sprite(format!("GFX_{culture}_{technology}{suffix}")))?
                {
                    return Ok(Some(icon));
                }
                if let Some(icon) = ask(Probe::Sprite(format!("GFX_{technology}{suffix}")))? {
                    return Ok(Some(icon));
                }
            }
        }
    }
    if let Some(family) = model.family
        && let Some(icon) = fallback(
            content,
            hour,
            family,
            names.clone(),
            capital,
            country,
            visited,
            ask,
        )?
    {
        return Ok(Some(icon));
    }
    pool(content, hour, key, country, None, None, ask)
}

/// The graphic pool for a type and a country (or only the default
/// pools without one).
fn pool(
    content: &IconContent,
    hour: &IconHour,
    key: &str,
    country: Option<u32>,
    ideology: Option<&str>,
    culture: Option<&str>,
    ask: &Ask,
) -> Result<Option<Icon>, String> {
    let model = type_model(&content.units, key)?;
    let family = model
        .family
        .map(|name| type_model(&content.units, name).map(|family| (name, family.archetype)))
        .transpose()?;
    let place = match country {
        Some(raw) => hour.prepared.as_ref().map_err(Clone::clone)?.context(raw)?,
        None => None,
    };
    let failed = std::cell::RefCell::new(None::<String>);
    let shown = std::cell::RefCell::new(None::<Icon>);
    let found = content.pools.lookup_icon(
        &Context {
            equipment: key,
            // The null archetype's empty name has a -1 hash and cannot match an
            // authored pool.
            archetype: model.archetype,
            family,
            subunit: None,
            ideology,
            culture,
            country: place.map(|place| place.country_tag.as_str()),
            faction_tags: place.map_or(&[], |place| place.faction_tags.as_slice()),
            capital_continent: place.and_then(|place| place.capital_continent.as_deref()),
        },
        |name| Some(content.dlc.contains(name)),
        // A candidate whose existence cannot be told stops the lookup: whatever a
        // later candidate would give is not known to be what the game shows.
        |name| match ask(Probe::Existing(name.to_owned())) {
            Ok(Some(icon)) => {
                // The first found is the icon, kept so its question is asked once.
                shown.borrow_mut().get_or_insert(icon);
                true
            }
            Ok(None) => false,
            Err(reason) => {
                failed.borrow_mut().get_or_insert(reason);
                true
            }
        },
    )?;
    if let Some(reason) = failed.into_inner() {
        return Err(reason);
    }
    Ok(found.and(shown.into_inner()))
}

#[cfg(test)]
mod tests;
