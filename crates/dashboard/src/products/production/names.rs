//! Names and pictures of produced variants and of organisations, made from one hour's tables in
//! the reader's language: a variant as `variant_names` names it and as the icon rule pictures it;
//! an organisation by its name key (else its script key) localised preferring `<owner tag>_<key>`,
//! pictured by its stored sprite, else `GFX_<owner tag>_<key>`, `GFX_<key>`, then the game's
//! unknown-organisation sprite, the first the content declares. Pictures are drawn once per
//! source and catalogue.

use super::{EquipmentIcon, OrganisationName, VariantName};
use crate::{
    Source,
    catalogue::{self, Catalogue},
    products::at_hour::{Keys, Read},
};
use dashboard_base::products::equipment_icon::{self, Icon, IconChoice, IconContent, IconHour};
use dashboard_core::config::LoadContext;
use dashboard_text::variant_names::{self, Tags, Texts};
use replay::columnar_frames::Recording;
use replay_data::rows::{LexicalTokenRow, TableId, tables};
use replay_frame::frame::{DecodedFrame, Metrics, not_selected};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, Mutex, PoisonError},
};

/// Every table naming reads.
pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::PoliticalParties,
    TableId::Factions,
    TableId::FactionMembers,
    TableId::Politics,
    TableId::States,
    TableId::LexicalTokens,
    TableId::VariantDefinitions,
    TableId::IndustrialOrganisations,
];

/// The tables an organisation's name and picture read.
pub const ORGANISATION_TABLES: &[TableId] = &[TableId::Countries, TableId::CountryLinks, TableId::CountryAppearance, TableId::LexicalTokens, TableId::IndustrialOrganisations];

/// Read whole: the countries the names and the icon rule read. The tokens they spell and the
/// states the icon rule asks of are read by key after them ([`spelling_reads`]).
pub const WHOLE: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::PoliticalParties,
    TableId::Factions,
    TableId::FactionMembers,
    TableId::Politics,
];

/// Read whole for an organisation's name and picture; its tokens by key after.
pub const ORGANISATION_WHOLE: &[TableId] = &[TableId::Countries, TableId::CountryLinks, TableId::CountryAppearance];

/// The tokens `held`'s variant definitions spell: each one's equipment type.
pub fn variant_tokens(held: &DecodedFrame) -> impl Iterator<Item = u32> + '_ {
    held.complete::<tables::VariantDefinitions>().ok().into_iter().flatten().map(|row| row.equipment_token)
}

/// The reads naming takes after its whole tables and the definitions and organisations it names:
/// `tokens` of LexicalTokens (an hour holds every token the game ever interned, tens of
/// thousands) and, where variants are pictured, the States the icon rule asks of (each
/// country's capital and original capital in `held`'s Politics), both by key.
pub fn spelling_reads(held: &DecodedFrame, tokens: BTreeSet<u32>, pictured: bool) -> Vec<Read> {
    let mut reads = vec![Read::these(TableId::LexicalTokens, tokens.into_iter().map(u128::from))];
    if pictured {
        let capitals = held.complete::<tables::Politics>().ok().into_iter().flatten().flat_map(|row| [row.capital, row.original_capital]);
        reads.push(Read::these(TableId::States, capitals.filter_map(|state| u32::try_from(state).ok().filter(|&state| state > 0)).map(u128::from)));
    }
    reads
}

/// Reads `reads` after `hour` of run `run` into `frame`, each replacing its table there.
fn read_into(recording: &Recording, run: u32, hour: i32, frame: &mut DecodedFrame, reads: &[Read]) -> io::Result<()> {
    let keyed: Vec<replay::at::Read<'_>> = reads
        .iter()
        .map(|read| replay::at::Read {
            table: read.table,
            columns: None,
            keys: match &read.keys {
                Keys::All => replay::at::Keys::All,
                Keys::These(keys) => replay::at::Keys::These(keys),
                Keys::Range(low, high) => replay::at::Keys::Range(*low, *high),
                Keys::Ranges(ranges) => replay::at::Keys::Ranges(ranges),
            },
        })
        .collect();
    let read = dashboard_query::cost::storage(|| replay::at::frame(recording, run, hour, &keyed))?;
    for asked in reads {
        frame.tables[asked.table.index()] = read.tables[asked.table.index()].clone();
    }
    Ok(())
}

const ICON_EDGE: u32 = 96;
const UNKNOWN_ORGANISATION_ICON: &str = "GFX_idea_unknown";
const NO_CONTENT: &str = "Names and pictures need the game folders.";

/// What naming reads of the recorded content: the reader's catalogue and the icon rule's content.
#[derive(Clone)]
pub struct Content {
    pub catalogue: Arc<Catalogue>,
    pub icons: Result<Arc<IconContent>, String>,
    /// What the source keeps of drawing through this catalogue.
    held: Arc<Mutex<Held>>,
}

/// The content, or why there is none.
pub type Context = Result<Content, String>;

/// What a source keeps of drawing: the catalogue it drew through, its icon content (loaded once,
/// every request asking while it loads waiting for that load) and every picture drawn through it,
/// by what was asked.
#[derive(Default)]
struct Held {
    catalogue: Option<Arc<Catalogue>>,
    icons: Arc<catalogue::CachedResult<IconContent>>,
    drawn: BTreeMap<Drawing, Result<Option<(String, String)>, String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Drawing {
    Variant(IconChoice),
    /// Sprite candidates in order: the first the content declares is drawn.
    Sprites(Vec<String>),
}

fn held(source: &Source, catalogue: &Arc<Catalogue>) -> Arc<Mutex<Held>> {
    let state = source.state::<Held>();
    {
        let mut held = state.lock().unwrap_or_else(PoisonError::into_inner);
        if !held.catalogue.as_ref().is_some_and(|kept| Arc::ptr_eq(kept, catalogue)) {
            *held = Held { catalogue: Some(Arc::clone(catalogue)), ..Held::default() };
        }
    }
    state
}

/// The content the reader's request binds, or why there is none.
pub fn context(source: &Source, load: &LoadContext) -> io::Result<Context> {
    let Some(options) = load.content.as_ref() else { return Ok(Err(NO_CONTENT.to_owned())) };
    let catalogue = catalogue::catalogue(source, options)?;
    let state = held(source, &catalogue);
    let once = Arc::clone(&state.lock().unwrap_or_else(PoisonError::into_inner).icons);
    let icons = match once.get_or_init(|| IconContent::load(Arc::clone(&catalogue))) {
        Ok(icons) => Ok(icons),
        Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
        Err(error) => Err(format!("Equipment icons are unavailable: {error}")),
    };
    Ok(Ok(Content { catalogue, icons, held: state }))
}

/// The key a picture is shared under: its asset and a digest of its pixels.
fn picture_key(asset: &str, image: &str) -> String {
    format!("{asset}#{}", blake3::hash(image.as_bytes()).to_hex())
}

fn asset_key(icon: &Icon) -> String {
    match icon {
        Icon::Sprite(name) => format!("sprite:{name}"),
        Icon::Path(path) => format!("path:{path}"),
    }
}

/// Whether `catalogue` declares sprite `name`, refused declarations included.
fn declared(catalogue: &Catalogue, name: &str) -> bool {
    catalogue.sprites.contains(name) || catalogue.sprites.refusal(name).is_some()
}

fn render(drawing: &Drawing, catalogue: &Catalogue) -> Result<Option<(String, String)>, String> {
    let icon = match drawing {
        Drawing::Variant(choice) => choice.shown(catalogue)?,
        Drawing::Sprites(names) => names.iter().find(|name| declared(catalogue, name)).map(|name| Icon::Sprite(name.clone())),
    };
    let Some(icon) = icon else { return Ok(None) };
    let asset = asset_key(&icon);
    let image = equipment_icon::image(catalogue, &icon, ICON_EDGE).map_err(|error| format!("{asset}: picture unavailable: {error}"))?;
    Ok(Some((picture_key(&asset, &image), image)))
}

/// A picture's key, its image kept in `named.icons`; none when it shows none.
fn draw(content: &Content, drawing: Drawing, named: &mut Named) -> Result<Option<String>, String> {
    let state = &content.held;
    let kept = state.lock().unwrap_or_else(PoisonError::into_inner).drawn.get(&drawing).cloned();
    let drawn = match kept {
        Some(drawn) => drawn,
        None => {
            let made = catalogue::draw(|| "production picture".to_owned(), &content.catalogue, |catalogue| render(&drawing, catalogue)).get().clone();
            state.lock().unwrap_or_else(PoisonError::into_inner).drawn.insert(drawing, made.clone());
            made
        }
    };
    Ok(drawn?.map(|(key, image)| {
        named.icons.entry(key.clone()).or_insert(image);
        key
    }))
}

/// Names, pictures and what could not be read, gathered over one or more hours.
#[derive(Default)]
pub struct Named {
    pub variants: BTreeMap<u64, VariantName>,
    pub organisations: BTreeMap<u64, OrganisationName>,
    pub icons: BTreeMap<String, String>,
    pub warnings: BTreeSet<String>,
}

impl Named {
    pub fn icons(&self) -> Vec<EquipmentIcon> {
        self.icons.iter().map(|(key, image)| EquipmentIcon { key: key.clone(), image: image.clone() }).collect()
    }
}

pub(super) fn spelled(lexical: &[LexicalTokenRow], token: u32) -> Option<String> {
    let at = lexical.binary_search_by_key(&token, |row| row.token).ok()?;
    std::str::from_utf8(&lexical[at].literal).ok().map(str::to_owned)
}

fn unknown_variant(id: u64, hour: i32, why: String) -> VariantName {
    VariantName { id: id.to_string(), hour: Some(hour), known: false, token: String::new(), name: None, type_name: None, name_unknown: Some(why.clone()), icon: None, icon_unknown: Some(why) }
}

fn unknown_organisation(id: u64, hour: i32, why: String) -> OrganisationName {
    OrganisationName { id: id.to_string(), hour: Some(hour), token: String::new(), name: None, name_unknown: Some(why.clone()), icon: None, icon_unknown: Some(why) }
}

/// Of `variants` and `organisations`, those `named` does not know yet.
fn wanted(variants: &[u64], organisations: &[u64], named: &Named) -> (Vec<u64>, Vec<u64>) {
    (
        variants.iter().copied().filter(|id| !named.variants.get(id).is_some_and(|held| held.known)).collect(),
        organisations.iter().copied().filter(|id| !named.organisations.get(id).is_some_and(|held| held.name_unknown.is_none() && !held.token.is_empty())).collect(),
    )
}

/// Names `variants` and `organisations` from `frame` into `named`; an id already named is named
/// again only where it was not known.
pub fn name(frame: &DecodedFrame, variants: &[u64], organisations: &[u64], context: &Context, named: &mut Named) {
    let hour = frame.hour;
    let (wanted_variants, wanted_organisations) = wanted(variants, organisations, named);
    if wanted_variants.is_empty() && wanted_organisations.is_empty() {
        return;
    }
    let countries = frame.complete::<tables::Countries>();
    let links = frame.complete::<tables::CountryLinks>();
    let lexical = frame.complete::<tables::LexicalTokens>();
    let (countries, links, lexical) = match (countries, links, lexical) {
        (Ok(countries), Ok(links), Ok(lexical)) => (countries, links, lexical),
        (countries, links, lexical) => {
            let why = [countries.err(), links.err(), lexical.err()].into_iter().flatten().map(|error| error.to_string()).collect::<Vec<_>>().join("; ");
            for id in wanted_variants {
                named.variants.insert(id, unknown_variant(id, hour, why.clone()));
            }
            for id in wanted_organisations {
                named.organisations.insert(id, unknown_organisation(id, hour, why.clone()));
            }
            named.warnings.insert(format!("Production names were not read: {why}"));
            return;
        }
    };
    let tags = Tags::new(countries, links, frame.complete::<tables::CountryAppearance>().ok());
    let texts: Option<&dyn Texts> = context.as_ref().ok().map(|content| content.catalogue.as_ref() as &dyn Texts);
    let no_texts = context.as_ref().err().cloned();
    let icon_hour = match context {
        _ if wanted_variants.is_empty() => Err(String::new()),
        Ok(Content { icons: Ok(icons), .. }) => IconHour::read(frame, icons).map_err(|error| format!("Equipment icons are unavailable at this hour: {error}")),
        Ok(Content { icons: Err(why), .. }) => Err(why.clone()),
        Err(why) => Err(why.clone()),
    };
    match frame.complete::<tables::VariantDefinitions>() {
        Err(error) => {
            for id in wanted_variants {
                named.variants.insert(id, unknown_variant(id, hour, error.to_string()));
            }
        }
        Ok(definitions) => {
            for id in wanted_variants {
                let Ok(at) = definitions.binary_search_by_key(&id, |row| row.id) else {
                    named.variants.insert(id, unknown_variant(id, hour, "The variant has no definition at this hour.".to_owned()));
                    continue;
                };
                let definition = &definitions[at];
                let raw = variant_names::raw_variant(definition, hour, &tags, lexical);
                let (name, type_name) = variant_names::variant_names(&raw, texts);
                let name_unknown = match (&no_texts, &name) {
                    (Some(why), _) => Some(why.clone()),
                    (None, None) => Some("The content does not name this variant.".to_owned()),
                    (None, Some(_)) => None,
                };
                let (icon, icon_unknown) = match (context, &icon_hour) {
                    (Ok(content), Ok(icon_hour)) => {
                        let Ok(icons) = &content.icons else { unreachable!("an icon hour is read only from icon content") };
                        let choice = equipment_icon::icon_choice(icons, icon_hour, definition, &raw.token);
                        match draw(content, Drawing::Variant(choice), named) {
                            Ok(icon) => (icon, None),
                            Err(why) => {
                                named.warnings.insert(why.clone());
                                (None, Some(why))
                            }
                        }
                    }
                    (_, Err(why)) => (None, Some(why.clone())),
                    (Err(why), _) => (None, Some(why.clone())),
                };
                named.variants.insert(id, VariantName { id: id.to_string(), hour: Some(hour), known: true, token: raw.token.clone(), name, type_name, name_unknown, icon, icon_unknown });
            }
        }
    }
    match frame.complete::<tables::IndustrialOrganisations>() {
        Err(error) => {
            for id in wanted_organisations {
                named.organisations.insert(id, unknown_organisation(id, hour, error.to_string()));
            }
        }
        Ok(rows) => {
            for id in wanted_organisations {
                let Ok(at) = rows.binary_search_by_key(&id, |row| row.organisation) else {
                    named.organisations.insert(id, unknown_organisation(id, hour, "The organisation is not recorded at this hour.".to_owned()));
                    continue;
                };
                let row = &rows[at];
                let token = spelled(lexical, row.token);
                let owner_tag = tags.tag_of(row.owner);
                let owner_unknown = (row.owner as i32) > 0 && owner_tag.is_none();
                let name_key = String::from_utf8_lossy(&row.name).into_owned();
                let icon_name = String::from_utf8_lossy(&row.icon).into_owned();
                let (name, name_unknown) = match (&token, owner_unknown, texts) {
                    (_, _, None) => (None, no_texts.clone()),
                    (_, true, _) => (None, Some("The organisation's owner is not recorded at this hour.".to_owned())),
                    (None, _, _) if name_key.is_empty() => (None, Some("lexical tokens does not spell the organisation's key.".to_owned())),
                    (token, false, Some(texts)) => {
                        let key = if name_key.is_empty() { token.as_deref().unwrap_or_default() } else { name_key.as_str() };
                        match variant_names::localise_for(texts, owner_tag.as_deref(), key) {
                            Some(name) => (Some(name), None),
                            None => (None, Some("The content does not name this organisation.".to_owned())),
                        }
                    }
                };
                let mut candidates = Vec::new();
                if !icon_name.is_empty() {
                    candidates.push(icon_name);
                }
                let fallback_known = !owner_unknown && token.is_some();
                if let (true, Some(token)) = (fallback_known, &token) {
                    candidates.extend(owner_tag.as_ref().map(|tag| format!("GFX_{tag}_{token}")));
                    candidates.push(format!("GFX_{token}"));
                    candidates.push(UNKNOWN_ORGANISATION_ICON.to_owned());
                }
                let (icon, icon_unknown) = match context {
                    Err(why) => (None, Some(why.clone())),
                    Ok(content) => match draw(content, Drawing::Sprites(candidates), named) {
                        Ok(Some(icon)) => (Some(icon), None),
                        Ok(None) if !fallback_known => (None, Some("The organisation's owner or key is not recorded, so its fallback icon cannot be chosen.".to_owned())),
                        Ok(None) => (None, None),
                        Err(why) => {
                            named.warnings.insert(why.clone());
                            (None, Some(why))
                        }
                    },
                };
                named.organisations.insert(id, OrganisationName { id: id.to_string(), hour: Some(hour), token: token.unwrap_or_default(), name, name_unknown, icon, icon_unknown });
            }
        }
    }
}

/// Names `variants` and `organisations` from the tables of run `run` after `hour`: the countries
/// whole, then the definitions and organisations named, then the tokens they spell and the states
/// the icon rule asks of, each by key. Nothing is read where every id is named already.
pub fn name_at(recording: &Recording, run: u32, hour: i32, variants: &[u64], organisations: &[u64], context: &Context, named: &mut Named) -> io::Result<()> {
    let (wanted_variants, wanted_organisations) = wanted(variants, organisations, named);
    if wanted_variants.is_empty() && wanted_organisations.is_empty() {
        return Ok(());
    }
    let mut frame = DecodedFrame { branch: recording.run(run)?.branch, hour, tables: std::array::from_fn(|_| not_selected()), metrics: Metrics::default() };
    let mut reads: Vec<Read> = WHOLE.iter().map(|&table| Read::all(table)).collect();
    reads.push(Read::these(TableId::VariantDefinitions, wanted_variants.iter().map(|&id| u128::from(id))));
    reads.push(Read::these(TableId::IndustrialOrganisations, wanted_organisations.iter().map(|&id| u128::from(id))));
    read_into(recording, run, hour, &mut frame, &reads)?;
    let mut tokens: BTreeSet<u32> = variant_tokens(&frame).collect();
    tokens.extend(frame.complete::<tables::IndustrialOrganisations>().ok().into_iter().flatten().map(|row| row.token));
    let spelling = spelling_reads(&frame, tokens, !wanted_variants.is_empty());
    read_into(recording, run, hour, &mut frame, &spelling)?;
    name(&frame, &wanted_variants, &wanted_organisations, context, named);
    Ok(())
}

/// Each of `variants`' names at `frame`'s hour, without pictures: none where it has no definition,
/// a table is not read or the content does not name it.
pub fn plain_names(frame: &DecodedFrame, variants: &[u64], context: &Context) -> BTreeMap<u64, Option<String>> {
    let tables = (frame.complete::<tables::Countries>(), frame.complete::<tables::CountryLinks>(), frame.complete::<tables::LexicalTokens>(), frame.complete::<tables::VariantDefinitions>());
    let (Ok(countries), Ok(links), Ok(lexical), Ok(definitions)) = tables else { return variants.iter().map(|&id| (id, None)).collect() };
    let tags = Tags::new(countries, links, frame.complete::<tables::CountryAppearance>().ok());
    let texts: Option<&dyn Texts> = context.as_ref().ok().map(|content| content.catalogue.as_ref() as &dyn Texts);
    variants
        .iter()
        .map(|&id| {
            let name = definitions.binary_search_by_key(&id, |row| row.id).ok().and_then(|at| variant_names::variant_names(&variant_names::raw_variant(&definitions[at], frame.hour, &tags, lexical), texts).0);
            (id, name)
        })
        .collect()
}
