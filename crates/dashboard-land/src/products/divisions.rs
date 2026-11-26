
pub mod careers;
pub mod equipment;
pub mod templates;

use dashboard_base::{Source, catalogue::{self, Catalogue, View}};
use dashboard_core::config::LoadContext;
use dashboard_query::identity::{Holder, HolderRun, Holders};
use dashboard_text::metadata::link_slot;
use content_reader_division_names::division_names::Database;
use replay_data::rows::{CountryLinkRow, DivisionRow, DivisionStatusRow, DivisionTemplateIconRow, LexicalTokenRow, NameOperands, Recorded, TableId, VariantDefinitionRow, tables};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex, MutexGuard, OnceLock, Weak},
};

pub use super::army::ArmyAttribution;

pub(super) fn naming_columns(table: TableId) -> Option<Vec<usize>> {
    let fields: &[&str] = match table {
        TableId::Countries => &["tag", "canonical_slot"],
        TableId::CountryLinks => &["canonical_slot"],
        TableId::CountryAppearance => &["name_tag"],
        TableId::DivisionTemplates => &["name", "name_key"],
        TableId::DivisionTemplateIcons => &["priority_unit", "priority_unit_name", "custom_icon", "country", "display_name"],
        TableId::VariantDefinitions => &["equipment_token", "display_name", "variant_name", "append_variant_name", "origin_link", "creator_link"],
        _ => return None,
    };
    Some(dashboard_base::products::at_hour::field_columns(table, fields))
}

pub(super) fn names_through<'a>(frame: &DecodedFrame, handles: impl Iterator<Item = &'a str>, content: &Content) -> i32 {
    if content.cat.is_err() { return i32::MAX; }
    template_names_through(frame, handles.filter_map(|handle| handle.parse().ok()))
}

pub(super) fn template_names_through(frame: &DecodedFrame, handles: impl Iterator<Item = u64>) -> i32 {
    let Ok(icons) = frame.complete::<tables::DivisionTemplateIcons>() else { return i32::MAX };
    handles.filter_map(|handle| {
        let icon = find(Some(icons), handle, |row| row.template)?;
        let stamp = icon.display_name.require("template display name").ok()?.as_ref()?.sample_hour.stamp()?;
        if stamp == frame.hour { Some(stamp) } else if stamp > frame.hour { Some(stamp - 1) } else { None }
    }).min().unwrap_or(i32::MAX)
}

pub fn side(attribution: ArmyAttribution) -> u8 {
    match attribution {
        ArmyAttribution::Owner => replay_data::rows::DivisionEquipmentCountRow::OWNER,
        ArmyAttribution::Host => replay_data::rows::DivisionEquipmentCountRow::HOST,
    }
}

pub fn attributed(row: &DivisionRow, attribution: ArmyAttribution) -> u32 {
    match attribution {
        ArmyAttribution::Owner => replay_data::military_aid::owner(row),
        ArmyAttribution::Host => row.roster_owner,
    }
}

#[derive(Clone, Default)]
pub struct Refusals(BTreeMap<String, usize>);

impl Refusals {
    pub fn note(&mut self, reason: impl Into<String>) {
        *self.0.entry(reason.into()).or_default() += 1;
    }

    pub fn note_times(&mut self, reason: impl Into<String>, times: usize) {
        if times > 0 {
            *self.0.entry(reason.into()).or_default() += times;
        }
    }

    pub fn keep<T>(&mut self, value: Result<T, String>) -> (Option<T>, Option<String>) {
        match value {
            Ok(value) => (Some(value), None),
            Err(reason) => {
                self.note(reason.clone());
                (None, Some(reason))
            }
        }
    }

    pub fn lines(self) -> Vec<String> {
        self.0.into_iter().map(|(reason, count)| if count == 1 { reason } else { format!("{reason} ({count})") }).collect()
    }
}

pub fn unavailable(frame: &DecodedFrame, id: TableId) -> Option<String> {
    dashboard_text::scope::unread(id, &frame.tables[id.index()])
}

pub fn find<R>(rows: Option<&[R]>, id: u64, key: impl FnMut(&R) -> u64) -> Option<&R> {
    let rows = rows?;
    rows.binary_search_by_key(&id, key).ok().map(|at| &rows[at])
}

pub struct Countries<'a> {
    links: &'a [CountryLinkRow],
    tags: HashMap<u32, String>,
}

impl<'a> Countries<'a> {
    pub fn read(frame: &'a DecodedFrame) -> std::io::Result<Self> {
        Ok(Self { links: frame.complete::<tables::CountryLinks>()?, tags: HashMap::new() })
    }

    pub fn witnessed(frame: &'a DecodedFrame) -> (Self, Option<String>) {
        let links = dashboard_query::coverage::witnessed_rows::<tables::CountryLinks>(frame);
        (Self { links: links.rows, tags: HashMap::new() }, links.missing)
    }

    pub fn named(mut self, holders: &Holders, hour: i32) -> Self {
        for (tag, runs) in &holders.tags {
            let at = runs.partition_point(|run| run.last < hour);
            if let Some(HolderRun { holder: Holder::Found { slot, .. }, first, .. }) = runs.get(at)
                && *first <= hour
            {
                self.tags.insert(*slot, tag.clone());
            }
        }
        self
    }

    pub fn slot(&self, raw: u32) -> Option<u32> {
        link_slot(self.links, raw)
    }

    pub fn tag(&self, raw: u32) -> Option<String> {
        self.tags.get(&self.slot(raw)?).cloned()
    }
}

type NameGroups = (Result<Arc<Database>, String>, Vec<String>);

#[derive(Default)]
struct Cache {
    binding: Weak<Catalogue>,
    groups: Arc<OnceLock<NameGroups>>,
    icons: BTreeMap<(u32, i32, Vec<u8>), Result<String, String>>,
}

impl Cache {
    fn select(&mut self, cat: &Arc<Catalogue>) {
        if !Weak::ptr_eq(&self.binding, &Arc::downgrade(cat)) {
            *self = Self { binding: Arc::downgrade(cat), ..Self::default() };
        }
    }
}

pub struct Content {
    pub cat: Result<Arc<Catalogue>, String>,
    cache: Arc<Mutex<Cache>>,
}

impl Content {
    pub fn load(source: &Source, load: &LoadContext) -> Self {
        Self {
            cat: load.content().and_then(|options| catalogue::catalogue(source, options)).map_err(|why| format!("Names unavailable: {why}")),
            cache: source.state::<Cache>(),
        }
    }

    fn cat(&self, what: &str) -> Result<&Arc<Catalogue>, String> {
        self.cat.as_ref().map_err(|_| format!("{what} need the recorded content stack"))
    }

    pub fn unavailable(&self) -> Option<&str> {
        self.cat.as_ref().err().map(String::as_str)
    }

    pub fn group_note_lines(&self) -> Vec<String> {
        self.groups().map(|(_, notes)| notes).unwrap_or_default()
    }

    pub fn localised(&self, key: &str) -> Result<String, String> {
        let cat = self.cat("Names")?;
        cat.localised(key, View::Plain).ok_or_else(|| format!("{key} has no localised name")).and_then(|text| catalogue::plain_text(&text))
    }

    pub fn template_name(&self, frame: &DecodedFrame, handle: u64) -> Result<String, String> {
        let cat = self.cat("Template names")?;
        dashboard_text::template_facts::display_naming(frame, handle)
            .and_then(|naming| naming.localise(&cat.localisation))
            .and_then(|name| catalogue::plain_text(&name))
            .map_err(|why| format!("Template name unavailable: {why}"))
    }

    pub fn template_icon(&self, frame: &DecodedFrame, handle: u64) -> Result<String, String> {
        let rows = frame.complete::<tables::DivisionTemplateIcons>().map_err(|why| format!("Template artwork unavailable: {why}"))?;
        let row = find(Some(rows), handle, |row| row.template).ok_or("Template artwork operands not recorded")?;
        self.icon_of(row)
    }

    pub fn icon_of(&self, row: &DivisionTemplateIconRow) -> Result<String, String> {
        self.icon(row.priority_unit, row.custom_icon, &row.priority_unit_name)
    }

    pub fn icon(&self, priority_unit: u32, custom: i32, unit: &[u8]) -> Result<String, String> {
        let cat = self.cat("Template artwork")?;
        let operands = (priority_unit, custom, unit.to_vec());
        if let Some(icon) = self.cache_of(cat).icons.get(&operands) {
            return icon.clone();
        }
        let drawn = catalogue::draw(|| format!("division template icon ({})", String::from_utf8_lossy(unit)), cat, |cat| template_sprite(cat, priority_unit, custom, unit));
        let icon = drawn.get().clone();
        self.cache_of(cat).icons.entry(operands).or_insert(icon).clone()
    }

    fn cache_of(&self, cat: &Arc<Catalogue>) -> MutexGuard<'_, Cache> {
        let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
        cache.select(cat);
        cache
    }

    fn groups(&self) -> Result<(Arc<Database>, Vec<String>), String> {
        let cat = self.cat("Division names")?;
        let once = Arc::clone(&self.cache_of(cat).groups);
        let (groups, notes) = once.get_or_init(|| {
            match content_reader_division_names::division_names::read_directories::<content_reader_parser::Diagnostic>(&cat.content.layers, &content_reader_division_names::division_names::DIRECTORIES) {
                Ok((database, notes)) => (Ok(Arc::new(database)), notes.into_iter().map(|(_, note)| note).collect()),
                Err(error) => (Err(format!("Division name groups unavailable: {error}")), Vec::new()),
            }
        });
        Ok((groups.clone()?, notes.clone()))
    }

    pub fn group_notes(&self, refusals: &mut Refusals) {
        if let Ok((_, notes)) = self.groups() {
            for note in notes {
                refusals.note(note);
            }
        }
    }

    pub fn division_name(&self, status: Option<&DivisionStatusRow>) -> Result<String, String> {
        let status = status.ok_or("The division's status was not read")?;
        let cat = self.cat("Division names")?;
        let groups = std::cell::OnceCell::new();
        dashboard_text::unit_names::army_name(status, &cat.localisation, &cat.recorded_language, || {
            groups.get_or_init(|| self.groups().map(|(groups, _)| groups)).as_ref().map(|groups| groups.as_ref()).map_err(Clone::clone)
        })
        .and_then(|name| catalogue::plain_text(&name))
    }

    pub fn division_name_of(&self, division: u64, name: &Recorded<Option<Vec<u8>>>, naming: &Recorded<Option<NameOperands>>) -> Result<String, String> {
        let cat = self.cat("Division names")?;
        let groups = std::cell::OnceCell::new();
        let status = replay_frame::frame::DivisionName { division, name, naming, country_ordinal: &Recorded::NotRecorded };
        dashboard_text::unit_names::army_name(status, &cat.localisation, &cat.recorded_language, || {
            groups.get_or_init(|| self.groups().map(|(groups, _)| groups)).as_ref().map(|groups| groups.as_ref()).map_err(Clone::clone)
        })
        .and_then(|name| catalogue::plain_text(&name))
    }

    pub fn template_name_of(&self, literal: &[u8], key: Option<&[u8]>, tag: Option<&str>) -> Result<String, String> {
        let cat = self.cat("Template names")?;
        dashboard_text::unit_names::template_name_with(
            replay_data::rows::TemplateDisplayNameParts { literal, fallback_key: key },
            0,
            dashboard_text::template_facts::TemplateNaming::Text,
            |_, key| {
                let tag = tag.ok_or("template country not held at this hour")?;
                let key = std::str::from_utf8(key).map_err(|_| "template name key is not UTF-8")?;
                Ok(dashboard_text::template_facts::TemplateNaming::Localised { tag: tag.to_owned(), key: key.to_owned() })
            },
        )
        .and_then(|naming| naming.localise(&cat.localisation))
        .and_then(|name| catalogue::plain_text(&name))
        .map_err(|why| format!("Template name unavailable: {why}"))
    }

    pub fn next_name(&self, custom: &[u8], programmatic: bool, group: Option<&[u8]>, order: i32, historical: bool) -> Result<String, String> {
        if !custom.is_empty() {
            return if programmatic {
                let cat = self.cat("Division names")?;
                dashboard_text::scripted_text::stored_text(&cat.localisation, &cat.recorded_language, custom, "Programmatic division name")
            } else {
                String::from_utf8(custom.to_vec()).map_err(|_| "Division name is not text".into())
            };
        }
        let group = group.ok_or("The template's name group was not read")?;
        let (groups, _) = self.groups()?;
        groups.resolve(group, order, historical).map_err(|why| format!("Division name unavailable: {why}")).and_then(|name| catalogue::plain_text(&name))
    }

    pub fn design_name(&self, frame: &DecodedFrame, definition: &VariantDefinitionRow) -> Result<String, String> {
        let cat = self.cat("Design names")?;
        let lexical = frame.complete::<tables::LexicalTokens>().map_err(|why| format!("Design names unavailable: {why}"))?;
        let tags = dashboard_text::variant_names::Tags::new(
            frame.complete::<tables::Countries>().map_err(|why| format!("Design names unavailable: {why}"))?,
            frame.complete::<tables::CountryLinks>().map_err(|why| format!("Design names unavailable: {why}"))?,
            frame.complete::<tables::CountryAppearance>().ok(),
        );
        let raw = dashboard_text::variant_names::raw_variant(definition, frame.hour, &tags, lexical);
        dashboard_text::variant_names::variant_names(&raw, Some(cat.as_ref() as &dyn dashboard_text::variant_names::Texts)).0.ok_or_else(|| UNNAMED_DESIGN.to_owned())
    }
}

pub const UNNAMED_DESIGN: &str = "An equipment design has no resolved name";

pub const DESIGN_TABLES: [TableId; 5] = [TableId::Countries, TableId::CountryLinks, TableId::CountryAppearance, TableId::VariantDefinitions, TableId::LexicalTokens];

fn template_sprite(cat: &Catalogue, priority_unit: u32, custom: i32, unit: &[u8]) -> Result<String, String> {
    let key = icon_key(priority_unit, custom, unit, |key| cat.sprites.find(key).map(|sprite| sprite.is_some()))?;
    let sprite = cat.sprites.find(&key)?.ok_or_else(|| format!("Division sprite {key} is unavailable"))?;
    dashboard_base::products::artwork::resolve_sprite_type(cat, sprite, 0, 48)
        .into_result()
        .and_then(|url| url.ok_or_else(|| "artwork unavailable".to_owned()))
        .map_err(|why| format!("Division sprite {key}: {why}"))
}

fn icon_key(priority_unit: u32, custom: i32, unit: &[u8], exists: impl Fn(&str) -> Result<bool, String>) -> Result<String, String> {
    if priority_unit == 0 {
        return Err("Division priority unit is null or invalid; native template artwork is absent".into());
    }
    if custom >= 0 {
        let key = format!("GFX_div_templ_{custom}_large");
        if exists(&key)? {
            return Ok(key);
        }
    }
    let unit = std::str::from_utf8(unit).map_err(|_| "Division priority unit name is not UTF-8")?;
    if unit.is_empty() {
        return Err("Division priority unit name is empty".into());
    }
    let key = dashboard_base::products::artwork::sprites::unit_icon_key(unit);
    if exists(&key)? { Ok(key) } else { Err(format!("Division sprite {key} is unavailable")) }
}

pub fn token_key(lexical: &[LexicalTokenRow], token: u32) -> Result<String, String> {
    let at = lexical.binary_search_by_key(&token, |row| row.token).map_err(|_| format!("Definition token {token} has no recorded spelling"))?;
    std::str::from_utf8(content_reader_parser::parser::c_string(&lexical[at].literal)).map(str::to_owned).map_err(|_| format!("Definition token {token} is not text"))
}

pub fn design_handle(id: u32) -> u64 {
    u64::from(id) << 32 | u64::from(engine_native::cequipment_variant::REFERENCE_TYPE)
}

#[cfg(test)]
mod tests;
