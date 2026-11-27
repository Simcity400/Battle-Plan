
use dashboard_base::{Source, catalogue::{self, Catalogue}, products::at_hour::{Keys, Read}};
use dashboard_base::products::equipment_icon::{self, Icon, IconChoice, IconContent, IconHour};
use dashboard_core::config::LoadContext;
use dashboard_text::variant_names::{self, Tags, Texts};
use replay_recording::columnar_frames::Recording;
use replay_data::rows::{CountryAppearanceRow, CountryRow, LexicalTokenRow, PoliticalPartyRow, PoliticsRow, ReservedCountryNamingRow, TableId, VariantDefinitionRow, mask, tables};
use replay_frame::frame::{DecodedFrame, Metrics, SharedView, TableView, not_selected};
use replay_lanes::{dated::{Dated, Lent}, lanes::Keys as HistoryKeys};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, Mutex, PoisonError},
};

use serde::{Deserialize, Serialize};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;


#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct VariantName {
    pub id: String,
    #[doc = " The hour its names were read; absent in a reply at an hour, whose hour it is."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hour: Option<i32>,
    #[doc = " False where the variant had no definition row at that hour; the other fields are then empty."]
    pub known: bool,
    #[doc = " The equipment type's script key."]
    pub token: String,
    pub name: Option<String>,
    pub type_name: Option<String>,
    #[doc = " Why the names are unknown, when they are."]
    pub name_unknown: Option<String>,
    #[doc = " The icon's key in `icons`; null when it shows none or it is unknown."]
    pub icon: Option<String>,
    pub icon_unknown: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OrganisationName {
    pub id: String,
    #[doc = " The hour its names were read; absent in a reply at an hour, whose hour it is."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hour: Option<i32>,
    #[doc = " The script key; empty where its row was not read."]
    pub token: String,
    pub name: Option<String>,
    pub name_unknown: Option<String>,
    pub icon: Option<String>,
    pub icon_unknown: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EquipmentIcon {
    pub key: String,
    #[doc = " An artwork URL."]
    pub image: String,
}

pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::ReservedCountryNaming,
    TableId::PoliticalParties,
    TableId::Factions,
    TableId::FactionMembers,
    TableId::Politics,
    TableId::States,
    TableId::LexicalTokens,
    TableId::VariantDefinitions,
    TableId::IndustrialOrganisations,
];

pub const ORGANISATION_TABLES: &[TableId] = &[TableId::Countries, TableId::CountryLinks, TableId::CountryAppearance, TableId::LexicalTokens, TableId::IndustrialOrganisations];

pub const WHOLE: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::ReservedCountryNaming,
    TableId::PoliticalParties,
    TableId::Factions,
    TableId::FactionMembers,
    TableId::Politics,
];

pub const ORGANISATION_WHOLE: &[TableId] = &[TableId::Countries, TableId::CountryLinks, TableId::CountryAppearance];

pub fn variant_tokens(held: &DecodedFrame) -> impl Iterator<Item = u32> + '_ {
    held.complete::<tables::VariantDefinitions>().ok().into_iter().flatten().map(|row| row.equipment_token)
}

pub fn capital_state_read(held: &DecodedFrame) -> Read {
    let capitals = held.complete::<tables::Politics>().ok().into_iter().flatten().flat_map(|row| [row.capital, row.original_capital]);
    Read::these(TableId::States, capitals.filter_map(|state| u32::try_from(state).ok().filter(|&state| state > 0)).map(u128::from))
}

pub fn state_continent_tokens(held: &DecodedFrame) -> impl Iterator<Item = u32> + '_ {
    held.complete::<tables::States>().ok().into_iter().flatten().filter_map(|row| match row.continent {
        replay_data::rows::Recorded::Value(Some(token)) if token != 0 => Some(token),
        _ => None,
    })
}

pub fn spelling_reads(held: &DecodedFrame, mut tokens: BTreeSet<u32>, pictured: bool) -> Vec<Read> {
    if pictured {
        tokens.extend(state_continent_tokens(held));
    }
    vec![Read::these(TableId::LexicalTokens, tokens.into_iter().map(u128::from))]
}

fn read_into(recording: &Recording, run: u32, hour: i32, frame: &mut DecodedFrame, reads: &[Read]) -> io::Result<()> {
    let keyed: Vec<replay_lanes::at::Read<'_>> = reads
        .iter()
        .map(|read| replay_lanes::at::Read {
            table: read.table,
            columns: None,
            keys: match &read.keys {
                Keys::All => replay_lanes::at::Keys::All,
                Keys::These(keys) => replay_lanes::at::Keys::These(keys),
                Keys::Range(low, high) => replay_lanes::at::Keys::Range(*low, *high),
                Keys::Ranges(ranges) => replay_lanes::at::Keys::Ranges(ranges),
            },
        })
        .collect();
    let read = dashboard_query::cost::storage(|| replay_lanes::at::frame(recording, run, hour, &keyed))?;
    for asked in reads {
        frame.tables[asked.table.index()] = read.tables[asked.table.index()].clone();
    }
    Ok(())
}

const ICON_EDGE: u32 = 96;
const NO_CONTENT: &str = "Names and pictures need the game folders.";

#[derive(Clone)]
pub struct Content {
    pub catalogue: Arc<Catalogue>,
    pub icons: Result<Arc<IconContent>, String>,
    held: Arc<Mutex<Held>>,
}

impl Content {
    pub fn from_catalogue(catalogue: Arc<Catalogue>) -> Self {
        Self {
            catalogue,
            icons: Err("Equipment icons are unavailable".to_owned()),
            held: Arc::new(Mutex::new(Held::default())),
        }
    }
}

pub type Context = Result<Content, String>;

#[derive(Default)]
struct Held {
    catalogue: Option<Arc<Catalogue>>,
    icons: Arc<catalogue::CachedResult<IconContent>>,
    drawn: BTreeMap<Drawing, (Result<Option<(String, String)>, String>, catalogue::OwnRefusals)>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Drawing {
    Variant(IconChoice),
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

fn picture_key(asset: &str, image: &str) -> String {
    format!("{asset}#{}", blake3::hash(image.as_bytes()).to_hex())
}

fn asset_key(icon: &Icon) -> String {
    match icon {
        Icon::Sprite(name) => format!("sprite:{name}"),
        Icon::Path(path) => format!("path:{path}"),
    }
}

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

fn draw(content: &Content, drawing: Drawing, named: &mut Named) -> Result<Option<String>, String> {
    let state = &content.held;
    let kept = state.lock().unwrap_or_else(PoisonError::into_inner).drawn.get(&drawing).cloned();
    let drawn = match kept {
        Some(drawn) => drawn,
        None => {
            let made = catalogue::capture(|| catalogue::draw(|| "production picture".to_owned(), &content.catalogue, |catalogue| render(&drawing, catalogue)).get().clone());
            state.lock().unwrap_or_else(PoisonError::into_inner).drawn.insert(drawing, made.clone());
            made
        }
    };
    let (drawn, noted) = drawn;
    catalogue::credit(&noted);
    Ok(drawn?.map(|(key, image)| {
        named.icons.entry(key.clone()).or_insert(image);
        key
    }))
}

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

pub fn spelled(lexical: &[LexicalTokenRow], token: u32) -> Option<String> {
    let at = lexical.binary_search_by_key(&token, |row| row.token).ok()?;
    std::str::from_utf8(&lexical[at].literal).ok().map(str::to_owned)
}

fn unknown_variant(id: u64, hour: Option<i32>, why: String) -> VariantName {
    VariantName { id: id.to_string(), hour, known: false, token: String::new(), name: None, type_name: None, name_unknown: Some(why.clone()), icon: None, icon_unknown: Some(why) }
}

fn unknown_organisation(id: u64, hour: Option<i32>, why: String) -> OrganisationName {
    OrganisationName { id: id.to_string(), hour, token: String::new(), name: None, name_unknown: Some(why.clone()), icon: None, icon_unknown: Some(why) }
}

fn wanted(variants: &[u64], organisations: &[u64], named: &Named) -> (Vec<u64>, Vec<u64>) {
    (
        variants.iter().copied().filter(|id| !named.variants.get(id).is_some_and(|held| held.known)).collect(),
        organisations.iter().copied().filter(|id| !named.organisations.get(id).is_some_and(|held| held.name_unknown.is_none() && !held.token.is_empty())).collect(),
    )
}

fn icon_hour(frame: &DecodedFrame, context: &Context) -> Result<IconHour, String> {
    match context {
        Ok(Content { icons: Ok(icons), .. }) => IconHour::read(frame, icons).map_err(|error| format!("Equipment icons are unavailable at this hour: {error}")),
        Ok(Content { icons: Err(why), .. }) => Err(why.clone()),
        Err(why) => Err(why.clone()),
    }
}

pub fn name(frame: &DecodedFrame, variants: &[u64], organisations: &[u64], context: &Context, named: &mut Named) {
    let (wanted_variants, wanted_organisations) = wanted(variants, organisations, named);
    name_wanted(frame, (&wanted_variants, &wanted_organisations), context, Some(frame.hour), named);
}

pub fn catalogue_names(frame: &DecodedFrame, variants: &[u64], organisations: &[u64], context: &Context) -> Named {
    let mut named = Named::default();
    name_wanted(frame, (variants, organisations), context, None, &mut named);
    named
}

pub fn undated_table_error(frame: &DecodedFrame, error: &io::Error) -> String {
    let mut reason = error.to_string();
    let prefix = format!("hour {} ", frame.hour);
    if reason.starts_with(&prefix) {
        reason.drain(..prefix.len());
    }
    reason
}

fn name_wanted(frame: &DecodedFrame, ids: (&[u64], &[u64]), context: &Context, hour: Option<i32>, named: &mut Named) {
    if ids.0.is_empty() && ids.1.is_empty() {
        return;
    }
    let icons = if ids.0.is_empty() { Err(String::new()) } else { icon_hour(frame, context) };
    name_in(frame, ids, context, icons.as_ref().map_err(Clone::clone), hour, named);
}

fn name_in(frame: &DecodedFrame, (wanted_variants, wanted_organisations): (&[u64], &[u64]), context: &Context, icon_hour: Result<&IconHour, String>, hour: Option<i32>, named: &mut Named) {
    let table_error = |error: &io::Error| if hour.is_some() { error.to_string() } else { undated_table_error(frame, error) };
    let countries = frame.complete::<tables::Countries>();
    let links = frame.complete::<tables::CountryLinks>();
    let lexical = frame.complete::<tables::LexicalTokens>();
    let (countries, links, lexical) = match (countries, links, lexical) {
        (Ok(countries), Ok(links), Ok(lexical)) => (countries, links, lexical),
        (countries, links, lexical) => {
            let why = [countries.err(), links.err(), lexical.err()].iter().flatten().map(table_error).collect::<Vec<_>>().join("; ");
            for &id in wanted_variants {
                named.variants.insert(id, unknown_variant(id, hour, why.clone()));
            }
            for &id in wanted_organisations {
                named.organisations.insert(id, unknown_organisation(id, hour, why.clone()));
            }
            named.warnings.insert(format!("Production names were not read: {why}"));
            return;
        }
    };
    let tags = Tags::new(countries, links, frame.complete::<tables::CountryAppearance>().ok());
    let texts: Option<&dyn Texts> = context.as_ref().ok().map(|content| content.catalogue.as_ref() as &dyn Texts);
    let no_texts = context.as_ref().err().cloned();
    match frame.complete::<tables::VariantDefinitions>() {
        Err(error) => {
            for &id in wanted_variants {
                named.variants.insert(id, unknown_variant(id, hour, table_error(&error)));
            }
        }
        Ok(definitions) => {
            for &id in wanted_variants {
                let Ok(at) = definitions.binary_search_by_key(&id, |row| row.id) else {
                    named.variants.insert(id, unknown_variant(id, hour, "The variant has no definition at this hour.".to_owned()));
                    continue;
                };
                let definition = &definitions[at];
                let raw = variant_names::raw_variant(definition, frame.hour, &tags, lexical);
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
                named.variants.insert(id, VariantName { id: id.to_string(), hour, known: true, token: raw.token.clone(), name, type_name, name_unknown, icon, icon_unknown });
            }
        }
    }
    match frame.complete::<tables::IndustrialOrganisations>() {
        Err(error) => {
            for &id in wanted_organisations {
                named.organisations.insert(id, unknown_organisation(id, hour, table_error(&error)));
            }
        }
        Ok(rows) => {
            for &id in wanted_organisations {
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
                named.organisations.insert(id, OrganisationName { id: id.to_string(), hour, token: token.unwrap_or_default(), name, name_unknown, icon, icon_unknown });
            }
        }
    }
}

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
    if !wanted_variants.is_empty() {
        let capitals = [capital_state_read(&frame)];
        read_into(recording, run, hour, &mut frame, &capitals)?;
    }
    let mut tokens: BTreeSet<u32> = variant_tokens(&frame).collect();
    tokens.extend(frame.complete::<tables::IndustrialOrganisations>().ok().into_iter().flatten().map(|row| row.token));
    let spelling = spelling_reads(&frame, tokens, !wanted_variants.is_empty());
    read_into(recording, run, hour, &mut frame, &spelling)?;
    name(&frame, &wanted_variants, &wanted_organisations, context, named);
    Ok(())
}

const VARIANT_FIELDS: u64 = mask::<VariantDefinitionRow>(&["equipment_token", "creator_link", "origin_link", "append_variant_name", "classifier_flags", "display_name", "variant_name", "icon_active", "icon_kind", "icon"]);

const WHOLE_FIELDS: [(TableId, u64); 8] = [
    (TableId::Countries, mask::<CountryRow>(&["canonical_slot", "tag"])),
    (TableId::CountryLinks, u64::MAX),
    (TableId::CountryAppearance, mask::<CountryAppearanceRow>(&["cosmetic_tag", "name_tag", "graphical_culture", "graphical_culture_2d", "original_tag", "original_tag_reference"])),
    (TableId::ReservedCountryNaming, mask::<ReservedCountryNamingRow>(&["original_tag", "original_tag_reference", "dynamic_tag_boundary"])),
    (TableId::PoliticalParties, mask::<PoliticalPartyRow>(&["is_ruling", "token"])),
    (TableId::Factions, u64::MAX),
    (TableId::FactionMembers, u64::MAX),
    (TableId::Politics, mask::<PoliticsRow>(&["capital", "original_capital"])),
];

pub fn naming_columns(table: TableId) -> Option<Vec<usize>> {
    let fields = match table {
        TableId::VariantDefinitions => VARIANT_FIELDS,
        TableId::States => mask::<replay_data::rows::StateRow>(&["continent"]),
        _ => WHOLE_FIELDS.iter().find(|(held, _)| *held == table)?.1,
    };
    (fields & table.full_mask() != table.full_mask()).then(|| (0..table.field_count()).filter(|&column| fields & (1 << column) != 0).collect())
}

pub struct Naming {
    run: u32,
    branch: u64,
    last: i32,
    whole: Vec<Dated>,
    variants: Dated,
    spelled: Mutex<Spelled>,
}

pub struct NamingSession<'n> {
    naming: &'n Naming,
    whole: Vec<Lent<'n>>,
    icon: Option<(Vec<i32>, usize, Arc<Result<IconHour, String>>)>,
}

#[derive(Default)]
struct Spelled {
    rows: BTreeMap<u32, Option<LexicalTokenRow>>,
    refused: bool,
}

fn joined<T>(handle: std::thread::ScopedJoinHandle<'_, io::Result<T>>) -> io::Result<T> {
    handle.join().unwrap_or_else(|_| Err(io::Error::other("A design-name history read panicked")))
}

impl Naming {
    fn read(recording: &Recording, run: u32) -> io::Result<Self> {
        let (whole, variants) = std::thread::scope(|scope| {
            let whole = scope.spawn(|| WHOLE_FIELDS.iter().map(|&(table, fields)| Dated::read(recording, run, table, fields, HistoryKeys::All)).collect::<io::Result<Vec<_>>>());
            let variants = Dated::read(recording, run, TableId::VariantDefinitions, VARIANT_FIELDS, HistoryKeys::All);
            (joined(whole), variants)
        });
        let held = recording.run(run)?;
        Ok(Self {
            run,
            branch: held.branch,
            last: held.first_hour + held.ordinals as i32 - 1,
            whole: whole?,
            variants: variants?,
            spelled: Mutex::default(),
        })
    }

    fn keyed_at(&self, recording: &Recording, at: i32, tokens: &BTreeSet<u32>) -> io::Result<SharedView> {
        let mut frame = DecodedFrame { branch: self.branch, hour: at, tables: std::array::from_fn(|_| not_selected()), metrics: Metrics::default() };
        read_into(recording, self.run, at, &mut frame, &[Read::these(TableId::LexicalTokens, tokens.iter().map(|&token| u128::from(token)))])?;
        Ok(frame.tables[TableId::LexicalTokens.index()].clone())
    }

    fn spelled_last(&self, recording: &Recording, tokens: &BTreeSet<u32>) -> io::Result<BTreeSet<u32>> {
        let mut held = self.spelled.lock().unwrap_or_else(PoisonError::into_inner);
        if !held.refused {
            let missing: BTreeSet<u32> = tokens.iter().copied().filter(|token| !held.rows.contains_key(token)).collect();
            if !missing.is_empty() {
                match self.keyed_at(recording, self.last, &missing)?.held() {
                    TableView::Complete(rows) => {
                        held.rows.extend(missing.iter().map(|&token| (token, None)));
                        held.rows.extend(rows.rows::<tables::LexicalTokens>().into_iter().flatten().map(|row| (row.token, Some(row.clone()))));
                    }
                    _ => held.refused = true,
                }
            }
        }
        Ok(if held.refused { BTreeSet::new() } else { tokens.iter().copied().filter(|token| held.rows.get(token).is_some_and(Option::is_some)).collect() })
    }

    fn spellings(&self, recording: &Recording, hour: i32, tokens: &BTreeSet<u32>) -> io::Result<SharedView> {
        if self.spelled_last(recording, tokens)?.len() < tokens.len() {
            return self.keyed_at(recording, hour, tokens);
        }
        let held = self.spelled.lock().unwrap_or_else(PoisonError::into_inner);
        let rows = tokens.iter().filter_map(|token| held.rows.get(token).cloned().flatten()).collect();
        Ok(TableView::Complete(replay_data::rows::TableRows::LexicalTokens(rows)).into())
    }

    pub fn session(&self) -> NamingSession<'_> {
        NamingSession { naming: self, whole: self.whole.iter().map(Lent::new).collect(), icon: None }
    }
}

impl NamingSession<'_> {
    fn frame(&mut self, recording: &Recording, hour: i32, variants: &[u64]) -> io::Result<DecodedFrame> {
        let naming = self.naming;
        let mut frame = DecodedFrame { branch: naming.branch, hour, tables: std::array::from_fn(|_| not_selected()), metrics: Metrics::default() };
        for lent in &mut self.whole {
            frame.tables[lent.dated().table().index()] = lent.view(recording, hour)?;
        }
        let keys: Vec<u128> = variants.iter().map(|&id| u128::from(id)).collect();
        frame.tables[TableId::VariantDefinitions.index()] = naming.variants.view_of(recording, hour, &keys)?;
        let tokens: BTreeSet<u32> = variant_tokens(&frame).collect();
        if !tokens.is_empty() {
            frame.tables[TableId::LexicalTokens.index()] = naming.spellings(recording, hour, &tokens)?;
        }
        Ok(frame)
    }

    fn icon_hour(&mut self, frame: &DecodedFrame, context: &Content) -> Arc<Result<IconHour, String>> {
        if !self.whole.iter().all(|lent| lent.dated().covers(frame.hour)) {
            return Arc::new(icon_hour(frame, &Ok(context.clone())));
        }
        let epochs: Vec<i32> = self.whole.iter().map(|lent| lent.dated().epoch(frame.hour)).collect();
        let icons = context.icons.as_ref().map_or(0, |icons| Arc::as_ptr(icons) as usize);
        if let Some((at, with, found)) = &self.icon
            && *at == epochs
            && *with == icons
        {
            return Arc::clone(found);
        }
        let made = Arc::new(icon_hour(frame, &Ok(context.clone())));
        self.icon = Some((epochs, icons, Arc::clone(&made)));
        made
    }

    pub fn name_variants(&mut self, recording: &Recording, hour: i32, variants: &[u64], context: &Context, named: &mut Named) -> io::Result<()> {
        let (wanted_variants, _) = wanted(variants, &[], named);
        self.visit_variant_frames(recording, hour, &wanted_variants, |session, frame, group| {
            let icons = match context {
                Ok(content @ Content { icons: Ok(_), .. }) => session.icon_hour(frame, content),
                _ => Arc::new(icon_hour(frame, context)),
            };
            name_in(frame, (group, &[]), context, icons.as_ref().as_ref().map_err(Clone::clone), Some(hour), named);
        })
    }

    pub fn plain_variant_names(&mut self, recording: &Recording, hour: i32, variants: &[u64], context: &Context) -> io::Result<BTreeMap<u64, Option<String>>> {
        let mut names = BTreeMap::new();
        self.visit_variant_frames(recording, hour, variants, |_, frame, group| {
            names.extend(plain_names(frame, group, context));
        })?;
        Ok(names)
    }

    fn visit_variant_frames(&mut self, recording: &Recording, hour: i32, variants: &[u64], mut visit: impl FnMut(&mut Self, &DecodedFrame, &[u64])) -> io::Result<()> {
        let naming = self.naming;
        if variants.is_empty() {
            return Ok(());
        }
        let keys: Vec<u128> = variants.iter().map(|&id| u128::from(id)).collect();
        let tokens: BTreeMap<u64, u32> = match naming.variants.view_of(recording, hour, &keys)?.held() {
            TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.rows::<tables::VariantDefinitions>().into_iter().flatten().map(|row| (row.id, row.equipment_token)).collect(),
            _ => BTreeMap::new(),
        };
        let spelled = naming.spelled_last(recording, &tokens.values().copied().collect())?;
        let (late, unspelled): (Vec<u64>, Vec<u64>) = variants.iter().partition(|id| tokens.get(id).is_none_or(|token| spelled.contains(token)));
        for group in [late, unspelled] {
            if group.is_empty() {
                continue;
            }
            let frame = self.frame(recording, hour, &group)?;
            visit(self, &frame, &group);
        }
        Ok(())
    }
}

#[derive(Default)]
struct KeptNaming(BTreeMap<u32, Arc<Naming>>);

pub fn naming(source: &Source, recording: &Recording, run: u32) -> io::Result<Arc<Naming>> {
    let kept = source.state::<KeptNaming>();
    if let Some(found) = kept.lock().unwrap_or_else(PoisonError::into_inner).0.get(&run) {
        return Ok(Arc::clone(found));
    }
    let made = Arc::new(dashboard_query::cost::storage(|| Naming::read(recording, run))?);
    kept.lock().unwrap_or_else(PoisonError::into_inner).0.insert(run, Arc::clone(&made));
    Ok(made)
}

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

#[cfg(test)]
#[test]
fn cached_equipment_pictures_keep_their_overlay_notes() {
    use dashboard_base::test_fixtures::visual_mods::{BLUE, Machine, RED, first_pixel, png, select};

    let machine = Machine::new("equipment_picture_notes");
    machine.write("interface/fixture.gfx", b"spriteTypes={spriteType={name=GFX_fixture_equipment textureFile=gfx/fixture_equipment.png}}");
    machine.write("gfx/fixture_equipment.png", &png(RED));
    machine.write_mod("fixture_art", "gfx/fixture_equipment.png", &png(BLUE));
    let (source, mut options) = machine.open();
    select(&source, &mut options, &["fixture_art"]);
    let content = Content::from_catalogue(catalogue::catalogue(&source, &options).unwrap());
    let read = || {
        let mut named = Named::default();
        let (key, noted) = catalogue::capture(|| draw(&content, Drawing::Sprites(vec!["GFX_fixture_equipment".into()]), &mut named).unwrap().unwrap());
        let mut warnings = Vec::new();
        noted.diagnostics().append_to(&mut warnings);
        (first_pixel(&named.icons[&key]), warnings)
    };
    let first = read();
    assert_eq!(first.0, BLUE);
    assert!(first.1.iter().any(|line| line.starts_with("Artwork from visual mods") && line.contains("fixture_art")), "{first:?}");
    assert_eq!(read(), first);
}
