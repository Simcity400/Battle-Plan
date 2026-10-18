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
//! Products about equipment (production lines, stockpile) read icons here.

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
    /// Every texture path the rule asked about, with the answer: the files it
    /// found or missed, outside the recorded content's hash.
    probes: std::sync::Mutex<BTreeMap<String, Result<bool, String>>>,
    /// A digest of the rule's visual inputs the recorded content's hash does not
    /// cover: the declared and refused sprites, the graphic pools, the geography
    /// and the DLC names.
    visual: String,
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
        let names = equipment_graphics::Names::read(layers, &default_map, &mut warnings)?;
        let files = Arc::clone(&catalogue);
        Ok(Self {
            units: match units {
                Some(units) => units,
                None => {
                    let names = layers.key_names()?;
                    match names.equipment_database() {
                        Some(database) if database.lost.is_none() => {
                            // Reuse registration only after the ordinary verified reads,
                            // in the order followed by the complete database loader.
                            for path in installed_content_core::equipment::files(layers) {
                                layers.read(&path)?;
                            }
                            Arc::clone(database)
                        }
                        // Preserve the loader's stopped reads and complete-database
                        // refusal, including when only a prefix could be registered.
                        _ => Arc::new(Database::load_complete(layers)?),
                    }
                }
            },
            enablers: installed_content_core::technology::read_enablers(layers)?,
            pools: equipment_graphics::Content::load(layers, &names)?,
            geography: equipment_graphics::context::Content::load(
                layers,
                &default_map,
                &provinces,
                &defines,
            )?,
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
            catalogue: Some(catalogue),
            load_warnings: warnings,
            probes: Default::default(),
            visual: String::new(),
        }
        .fingerprinted())
    }

    /// The content with the digest of its visual inputs.
    fn fingerprinted(mut self) -> Self {
        let inputs = format!(
            "{:?}\0{:?}\0{:?}\0{:?}\0{:?}",
            self.sprites, self.refused_sprites, self.pools, self.geography, self.dlc
        );
        self.visual = blake3::hash(inputs.as_bytes()).to_hex().to_string();
        self
    }

    /// Whether a texture file exists, recorded with its answer ([`Self::file_probes`]).
    fn file(&self, path: &str) -> Result<bool, String> {
        let found = (self.files)(path);
        self.probes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(path.to_owned(), found.clone());
        found
    }

    /// Every texture path asked about so far, with the answer it had.
    pub fn file_probes(&self) -> BTreeMap<String, Result<bool, String>> {
        self.probes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// Whether every one of `probes` has the answer it had when it was asked.
    pub fn probes_hold(&self, probes: &BTreeMap<String, Result<bool, String>>) -> bool {
        probes
            .iter()
            .all(|(path, answer)| self.file(path) == *answer)
    }

    /// The digest of the rule's visual inputs outside the recorded content's
    /// hash; the rest it reads (the equipment database, the technologies'
    /// enablers, the map) is game content the recording pins by hash.
    pub fn visual_inputs(&self) -> &str {
        &self.visual
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
            probes: Default::default(),
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
                            bases.extend(enabler.parent.as_deref());
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
        if !(name.contains('/') || name.contains('.')) {
            return Ok(None);
        }
        Ok(self.file(name)?.then(|| Icon::Path(name.to_owned())))
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

impl IconHour {
    pub fn read(frame: &DecodedFrame, content: &IconContent) -> io::Result<Self> {
        let links: BTreeMap<u32, u32> = frame
            .complete::<tables::CountryLinks>()?
            .iter()
            .map(|row: &CountryLinkRow| (row.id, row.canonical_slot))
            .collect();
        let countries = equipment_graphics::countries::read(frame)?;
        let tags: BTreeMap<u32, String> = frame
            .complete::<tables::Countries>()?
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
            .map_err(|error| format!("the country appearance table is unavailable: {error}"));
        Ok(Self {
            prepared: Prepared::read(frame, &content.geography),
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
                    .ok_or("the creator's ruling party is not recorded at this hour")?;
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
            )? {
                return Ok(Some(icon));
            }
        } else {
            let found = if variant.icon_kind == 0 {
                content.sprite(&entry)?
            } else {
                content
                    .file(&entry)?
                    .then(|| Icon::Path(entry.clone().into_owned()))
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
                if let Some(icon) = content.sprite(&candidate)? {
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
            for technology in [Some(enabler.id.as_str()), enabler.parent.as_deref()]
                .into_iter()
                .flatten()
            {
                // L's (original) tag first, then its 2D culture: each unknown leaves
                // the icon unknown rather than skipping to the next name.
                if let Some(tag) = names.technology_tag.clone()?
                    && let Some(icon) =
                        content.sprite(&format!("GFX_{tag}_{technology}{suffix}"))?
                {
                    return Ok(Some(icon));
                }
                if let Some(culture) = names.culture_2d.clone()?
                    && let Some(icon) =
                        content.sprite(&format!("GFX_{culture}_{technology}{suffix}"))?
                {
                    return Ok(Some(icon));
                }
                if let Some(icon) = content.sprite(&format!("GFX_{technology}{suffix}"))? {
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
        )?
    {
        return Ok(Some(icon));
    }
    pool(content, hour, key, country, None, None)
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
        |name| match content.existing(name) {
            Ok(found) => found.is_some(),
            Err(reason) => {
                failed.borrow_mut().get_or_insert(reason);
                true
            }
        },
    )?;
    if let Some(reason) = failed.into_inner() {
        return Err(reason);
    }
    match found {
        Some(name) => content.existing(&name),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests;
