
use crate::catalogue::Catalogue;
use content_reader_core::{equipment::Database, technology::Enablers};
use content_reader_equipment_graphics::{
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

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
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
pub struct IconChoice {
    pub probes: Vec<Probe>,
    pub otherwise: Result<(), String>,
}

impl IconChoice {
    pub fn unknown(reason: String) -> Self {
        Self {
            probes: Vec::new(),
            otherwise: Err(reason),
        }
    }

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

fn reads_as_path(name: &str) -> bool {
    name.contains('/') || name.contains('.')
}

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
    visual: String,
}

fn settled<T>(read: std::thread::Result<T>) -> T {
    read.unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

pub fn inputs_stamped(layers: &content_reader_core::ResolvedLayers) -> io::Result<bool> {
    if !layers.stamped("map/default.map")? {
        return Ok(false);
    }
    let default_map = content_reader_core::terrain::read_default_map(layers)?;
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
    pub fn native(
        layers: Arc<content_reader_core::ResolvedLayers>,
        units: Arc<Database>,
        scripted: &Result<dashboard_text::scripted_text::ScriptedContent, String>,
        registry: &content_reader_core::gui::native::resources::Registry,
    ) -> io::Result<Self> {
        let default_map = content_reader_core::terrain::read_default_map(&layers)?;
        let provinces = content_reader_core::map::parse_definitions(
            &layers.read(&default_map.file("definitions")?)?,
        )?;
        let defines = content_reader_core::defines::read(&layers)?;
        let mut warnings = Vec::new();
        let names = equipment_graphics::Names::read(&layers, &default_map, &mut warnings)?;
        let enablers = content_reader_core::technology::read_enablers(&layers, |limit| {
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
        let sprites = content_reader_core::gui::native::sprites::registry(&registry.files)
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
            files: Box::new(move |path| layers.has_file(path).map_err(|e| e.to_string())),
            stamped: false,
            load_warnings: warnings,
            visual: String::new(),
        }
        .fingerprinted())
    }

    pub fn load(catalogue: Arc<Catalogue>) -> io::Result<Self> {
        Self::with_equipment(catalogue, None)
    }

    pub fn with_equipment(
        catalogue: Arc<Catalogue>,
        units: Option<Arc<Database>>,
    ) -> io::Result<Self> {
        let layers = &catalogue.content.layers;
        let default_map = content_reader_core::terrain::read_default_map(layers)?;
        let provinces = content_reader_core::map::parse_definitions(
            &layers.read(&default_map.file("definitions")?)?,
        )?;
        let defines = content_reader_core::defines::read(layers)?;
        let mut warnings = Vec::new();
        let files = Arc::clone(&catalogue);
        let (names, pools, enablers, units, geography) = std::thread::scope(|threads| {
            use content_reader_core::parallel::spawn;
            let enablers = spawn(threads, || {
                dashboard_text::catalogue::capture(|| {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let world = std::cell::OnceCell::new();
                        content_reader_core::technology::read_enablers(layers, |limit| {
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
                            for path in content_reader_core::equipment::files(layers)? {
                                path.read(layers)?;
                            }
                            Ok(Arc::clone(database))
                        }
                        _ => Database::load_complete(layers).map(Arc::new),
                    }
                }
            });
            let geography = spawn(threads, || {
                equipment_graphics::context::Content::load(
                    layers,
                    &default_map,
                    &provinces,
                    &defines,
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

    fn fingerprinted(mut self) -> Self {
        let inputs = format!("{:?}\0{:?}\0{:?}", self.pools, self.geography, self.dlc);
        self.visual = blake3::hash(inputs.as_bytes()).to_hex().to_string();
        self
    }

    fn file(&self, path: &str) -> Result<bool, String> {
        (self.files)(path)
    }

    pub fn visual_inputs(&self) -> &str {
        &self.visual
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
            visual: String::new(),
        }
        .fingerprinted()
    }

    pub fn with_geography(mut self, geography: equipment_graphics::context::Content) -> Self {
        self.geography = geography;
        self.fingerprinted()
    }

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

    pub fn preparation_warnings(&self) -> Vec<content_reader_core::Diagnostic> {
        let mut warnings: Vec<_> = self
            .load_warnings
            .iter()
            .cloned()
            .map(|message| (content_reader_core::DiagnosticKind::Warning, message))
            .collect();
        warnings.extend(self.enablers.refused.iter().map(|definition| {
            (content_reader_core::DiagnosticKind::Unsupported,
                format!("technology {definition} enables equipment through a script, which is not supported; icons that would use it are unknown"))
        }));
        warnings.extend(self.pools.refusals().into_iter().map(|(subject, reason)| {
            (
                content_reader_core::DiagnosticKind::Refused,
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

    fn sprite(&self, name: &str) -> Result<Option<Icon>, String> {
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

#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct IconHour {
    links: BTreeMap<u32, u32>,
    countries: BTreeMap<u32, ModelCountry>,
    tags: BTreeMap<u32, String>,
    slots_by_tag: BTreeMap<String, u32>,
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

pub fn variant_icon(
    content: &IconContent,
    hour: &IconHour,
    variant: &VariantDefinitionRow,
    spelling: &str,
) -> Result<Option<Icon>, String> {
    rule(content, hour, variant, spelling, &|probe| {
        content.answer(&probe)
    })
}

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
        otherwise: otherwise.map(|_| ()),
    }
}

type Ask<'a> = dyn Fn(Probe) -> Result<Option<Icon>, String> + 'a;

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
    if variant.icon_kind > 1 {
        return Err(format!(
            "variant {} has an icon entry of unknown kind {}",
            variant.id, variant.icon_kind
        ));
    }
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
    } else if variant.icon_active != 0 {
        let found = if variant.icon_kind == 0 {
            ask(Probe::Sprite(entry.clone().into_owned()))?
        } else {
            ask(Probe::File(entry.clone().into_owned()))?
        };
        if found.is_some() {
            return Ok(found);
        }
    }
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
    let l_link = if (variant.origin_link as i32) > 0 {
        Some(variant.origin_link)
    } else if (variant.creator_link as i32) > 0
        && l_slot == hour.links.get(&variant.creator_link).copied()
    {
        Some(variant.creator_link)
    } else {
        l_slot.and_then(|slot| {
            hour.links
                .iter()
                .find(|(_, s)| **s == slot)
                .map(|(raw, _)| *raw)
        })
    };
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

#[derive(Clone)]
struct Names<'a> {
    tag: Option<&'a str>,
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
    ask: &Ask,
) -> Result<Option<Icon>, String> {
    let model = type_model(&content.units, key)?;
    if !visited.insert(key.to_owned()) {
        return Ok(None);
    }
    let suffixes = icon_suffixes(capital);
    for suffix in suffixes {
        for name in [Some(key), model.archetype].into_iter().flatten() {
            for candidate in sprite_names(name, suffix, names.tag).into_iter().flatten() {
                if let Some(icon) = ask(Probe::Sprite(candidate))? {
                    return Ok(Some(icon));
                }
            }
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
        |name| match ask(Probe::Existing(name.to_owned())) {
            Ok(Some(icon)) => {
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
