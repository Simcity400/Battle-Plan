//! Names and artwork the army products share: countries at a reached hour,
//! template names and icons, sub-unit names, design names and counted refusals.
use dashboard_base::{catalogue, products::equipment, source::Source};
use dashboard_core::config::LoadContext;
use dashboard_text::{
    catalogue::{Catalogue, View},
    metadata::{TaggedCountry, link_slot, tagged_country},
};
use replay_data::rows::{TableId, tables};
use replay_frame::frame::{DecodedFrame, TableView};
use std::{
    collections::BTreeMap,
    io,
    sync::Arc,
};

/// Each refused name or artwork, counted, for the reply's warnings.
#[derive(Default)]
pub struct Refusals(BTreeMap<String, usize>);
impl Refusals {
    pub fn note(&mut self, reason: impl Into<String>) {
        *self.0.entry(reason.into()).or_default() += 1;
    }
    /// Keep a value, or note why it is missing and keep that reason beside it.
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
        self.0
            .into_iter()
            .map(|(reason, count)| {
                if count == 1 {
                    reason
                } else {
                    format!("{reason} ({count})")
                }
            })
            .collect()
    }
}

/// The recording's content catalogue, or why names are unavailable.
pub fn content(source: &Source, context: &LoadContext) -> Result<Arc<Catalogue>, String> {
    context
        .content()
        .and_then(|options| catalogue::catalogue(source, options))
        .map_err(|why| format!("Names unavailable: {why}"))
}

/// Why a table could not be read at this hour, or none when it was.
pub fn unavailable(frame: &DecodedFrame, id: TableId) -> Option<String> {
    match &*frame.tables[id.index()] {
        TableView::Complete(_) => None,
        TableView::Unavailable { reason } => Some(format!("{id:?}: {reason}")),
        TableView::Truncated { .. } => Some(format!("{id:?}: incomplete read")),
    }
}

/// A country by its recorded identity: its canonical slot and country id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Holder {
    pub slot: u32,
    pub id: u32,
}

/// The country holding `tag` at the frame's hour; none when no country holds it.
pub fn holder(frame: &DecodedFrame, tag: &str) -> io::Result<Option<Holder>> {
    let countries = frame.complete::<tables::Countries>()?;
    let links = frame.complete::<tables::CountryLinks>()?;
    match tagged_country(countries, links, tag) {
        TaggedCountry::Found(row, slot) => Ok(Some(Holder { slot, id: row.id })),
        TaggedCountry::Absent => Ok(None),
        TaggedCountry::Unresolved => Err(io::Error::other(format!(
            "{tag} has no resolved country identity at this hour"
        ))),
    }
}

/// The canonical slot of the country holding `tag` at the frame's hour; none
/// when no country holds it.
pub fn country_slot(frame: &DecodedFrame, tag: &str) -> io::Result<Option<u32>> {
    Ok(holder(frame, tag)?.map(|holder| holder.slot))
}

/// Whether a supplying read is of the same country: its row at the holder's
/// slot carries the holder's id, so a reused tag or slot never lends another
/// country's facts.
pub fn same_holder(read: &DecodedFrame, holder: Holder) -> Result<(), &'static str> {
    let row = read
        .complete::<tables::Countries>()
        .ok()
        .and_then(|rows| rows.iter().find(|row| row.canonical_slot == holder.slot))
        .ok_or("its country identity was not recorded")?;
    if row.id == holder.id { Ok(()) } else { Err("it belongs to another country identity") }
}

/// Raw country references resolved to their canonical slot and current tag.
pub struct Countries<'a> {
    links: &'a [replay_data::rows::CountryLinkRow],
    tags: BTreeMap<u32, String>,
}
impl<'a> Countries<'a> {
    pub fn read(frame: &'a DecodedFrame) -> io::Result<Self> {
        let links = frame.complete::<tables::CountryLinks>()?;
        let tags = frame
            .complete::<tables::Countries>()?
            .iter()
            .filter_map(|row| Some((row.canonical_slot, String::from_utf8(row.tag.clone()).ok()?)))
            .collect();
        Ok(Self { links, tags })
    }
    pub fn slot(&self, raw: u32) -> Option<u32> {
        link_slot(self.links, raw)
    }
    pub fn tag(&self, raw: u32) -> Option<String> {
        self.tags.get(&self.slot(raw)?).cloned()
    }
}

/// A template's in-game name: its literal, else its key localised for its
/// composition country.
pub fn template_name(frame: &DecodedFrame, handle: u64, cat: Option<&Arc<Catalogue>>) -> Result<String, String> {
    let cat = cat.ok_or("Template name needs the recorded content stack")?;
    dashboard_text::template_facts::display_name(frame, handle, &cat.localisation)
        .and_then(|name| catalogue::plain_text(&name))
        .map_err(|why| format!("Template name unavailable: {why}"))
}

/// A template's artwork as the division log draws it.
pub fn template_icon(
    source: &Source,
    frame: &DecodedFrame,
    handle: u64,
    cat: Option<&Arc<Catalogue>>,
) -> Result<String, String> {
    let cat = cat.ok_or("Template artwork needs the recorded content stack")?;
    let rows = frame
        .complete::<tables::DivisionTemplateIcons>()
        .map_err(|why| format!("Template artwork unavailable: {why}"))?;
    let row = rows
        .binary_search_by_key(&handle, |row| row.template)
        .ok()
        .map(|at| &rows[at])
        .ok_or("Template artwork operands not recorded")?;
    dashboard_land_battles::products::division_log::template_icon(source, cat, row)
}

/// A lexical token's literal: the key of the content definition it names.
pub fn token_key(frame: &DecodedFrame, token: u32) -> Result<String, String> {
    let rows = frame
        .complete::<tables::LexicalTokens>()
        .map_err(|why| format!("Sub-unit names unavailable: {why}"))?;
    let at = rows
        .binary_search_by_key(&token, |row| row.token)
        .map_err(|_| format!("Definition token {token} has no recorded spelling"))?;
    std::str::from_utf8(installed_content_core::parser::c_string(&rows[at].literal))
        .map(str::to_owned)
        .map_err(|_| format!("Definition token {token} is not text"))
}

/// A content key's in-game name in the plain view.
pub fn localised(cat: Option<&Arc<Catalogue>>, key: &str) -> Result<String, String> {
    let cat = cat.ok_or("Names need the recorded content stack")?;
    cat.localised(key, View::Plain)
        .ok_or_else(|| format!("{key} has no localised name"))
        .and_then(|text| catalogue::plain_text(&text))
}

/// Design names from the supplying read of their definitions.
pub struct Designs<'a> {
    read: Option<(
        &'a DecodedFrame,
        equipment::Tags<'a>,
        &'a [replay_data::rows::VariantDefinitionRow],
        &'a [replay_data::rows::LexicalTokenRow],
    )>,
    cat: Option<&'a Arc<Catalogue>>,
    types: Result<Arc<installed_content_core::equipment::Database>, String>,
}

/// One design's name and its archetype's key and name.
#[derive(Clone, Debug, Default)]
pub struct Design {
    pub name: Option<String>,
    pub archetype: Option<String>,
    pub archetype_name: Option<String>,
}

impl<'a> Designs<'a> {
    pub fn new(read: Option<&'a DecodedFrame>, cat: Option<&'a Arc<Catalogue>>) -> Self {
        Self {
            read: read.and_then(|read| {
                Some((
                    read,
                    equipment::Tags::new(
                        read.complete::<tables::Countries>().ok()?,
                        read.complete::<tables::CountryLinks>().ok()?,
                        read.complete::<tables::CountryAppearance>().ok(),
                    ),
                    read.complete::<tables::VariantDefinitions>().ok()?,
                    read.complete::<tables::LexicalTokens>().ok()?,
                ))
            }),
            cat,
            types: cat
                .ok_or_else(|| "Equipment classification needs the recorded content stack".to_owned())
                .and_then(|cat| equipment::units_of(cat)),
        }
    }
    pub fn design(&self, variant: u64, refusals: &mut Refusals) -> Design {
        let Some((read, tags, definitions, lexical)) = &self.read else {
            refusals.note("Equipment designs were not read at or before this hour");
            return Design::default();
        };
        let Ok(at) = definitions.binary_search_by_key(&variant, |row| row.id) else {
            refusals.note("An equipment design has no recorded definition");
            return Design::default();
        };
        let raw = equipment::raw_variant(&definitions[at], read.hour, tags, lexical);
        let (name, _) = equipment::variant_names(&raw, self.cat.map(|c| c.as_ref() as &dyn equipment::Texts));
        if name.is_none() {
            refusals.note("An equipment design has no resolved name");
        }
        let archetype = match self
            .types
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|types| equipment::archetype_of(types, &raw.token))
        {
            Ok((key, _, _)) => Some(key),
            Err(why) => {
                refusals.note(format!("Equipment design archetype unavailable: {why}"));
                None
            }
        };
        let archetype_name = archetype.as_ref().and_then(|key| localised(self.cat, key).ok());
        Design {
            name,
            archetype,
            archetype_name,
        }
    }
}

/// The game's name for a template's stored priority: the designer's heading for
/// each of the stages its defines allow that the native designer labels. A stage
/// outside those has no proved name and is refused by value.
pub fn template_priority(cat: Option<&Arc<Catalogue>>, priority: i32) -> Result<String, String> {
    let cat = cat.ok_or("Priority names need the recorded content stack")?;
    let stages = cat
        .defines()?
        .int32("NCountry", "REINFORCEMENT_DIVISION_PRIORITY_COUNT")
        .map_err(|why| format!("Template priority stages unavailable: {why}"))?;
    if !(0..=2).contains(&priority) || priority >= stages {
        return Err(format!("Template priority {priority} has no proved name"));
    }
    localised(Some(cat), &format!("TEMPLATE_PRIO_{priority}"))
}

/// The deployment view's priority buttons, in stored priority order.
const CONVEYOR_VIEW: &str = "military_deployment_conveyor_view";
const PRIORITY_BUTTONS: [&str; 3] = ["low_prio", "medium_prio", "high_prio"];

/// The game's text for each stored deployment priority: the localised tooltip
/// of its button in the effective interface files, read once per reply. The
/// deployment view is the country's own, so a scoped expression in the text
/// (such as `[ROOT.GetName]`) is evaluated for that country with the operands
/// `frame` witnessed, or refused; it is never shown as written.
pub struct DeploymentPriorities(Vec<Result<String, String>>);
impl DeploymentPriorities {
    pub fn read(cat: Option<&Arc<Catalogue>>, frame: &DecodedFrame, root: u32, refusals: &mut Refusals) -> Self {
        let Some(cat) = cat else {
            return Self(vec![Err("Names need the recorded content stack".to_owned()); PRIORITY_BUTTONS.len()]);
        };
        let guis = match cat.guis() {
            Ok(guis) => guis,
            Err(why) => {
                return Self(vec![Err(format!("Deployment priority names unavailable: {why}")); PRIORITY_BUTTONS.len()]);
            }
        };
        // Without the scripted content a bracketed expression is refused.
        let scripted = cat
            .scripted_content()
            .ok()
            .map(|content| dashboard_text::scripted_text::ScriptedText::new(frame, &cat.localisation, content, root));
        let text = |key: &str| {
            if cat.localisation.get(key).is_none() {
                return Err(format!("{key} has no localised name"));
            }
            let text = match &scripted {
                Some(scripted) => dashboard_text::catalogue::localised_in(&cat.localisation, key, &[], scripted),
                None => dashboard_text::catalogue::localised(&cat.localisation, key, View::Scripted),
            };
            text.map_err(|why| format!("Deployment priority name refused: {why}"))
        };
        let mut notes = std::collections::BTreeSet::new();
        let names = PRIORITY_BUTTONS
            .iter()
            .map(|button| {
                let mut warnings = Vec::new();
                let key = installed_content_core::gui::native::window_button_tooltip(guis, CONVEYOR_VIEW, button, &mut warnings);
                notes.extend(warnings);
                key.map_err(|why| format!("Deployment priority name unavailable: {why}"))
                    .and_then(|key| text(&key))
            })
            .collect();
        for note in notes {
            refusals.note(note);
        }
        Self(names)
    }
    pub fn name(&self, priority: u32) -> Result<String, String> {
        usize::try_from(priority)
            .ok()
            .and_then(|at| self.0.get(at).cloned())
            .unwrap_or_else(|| Err(format!("Deployment priority {priority} has no proved name")))
    }
}

/// A deployment order's location as the game shows it: the display name of the
/// state owning its province, or the game's text for no location.
pub fn deployment_location(frame: &DecodedFrame, cat: Option<&Arc<Catalogue>>, province: u32) -> Result<String, String> {
    if province == 0 {
        return localised(cat, "CONVEYOR_SET_LOCATION_NOT_SET");
    }
    let cat = cat.ok_or("Location names need the recorded content stack")?;
    let provinces = frame
        .complete::<tables::Provinces>()
        .map_err(|why| format!("Provinces were not read: {why}"))?;
    let row = provinces
        .binary_search_by_key(&province, |row| row.id)
        .ok()
        .map(|at| &provinces[at])
        .ok_or_else(|| format!("Province {province} was not read"))?;
    let text = dashboard_text::scripted_text::ScriptedText::new(frame, &cat.localisation, cat.scripted_content()?, 0);
    text.plain_state_name(row.state)
        .and_then(|name| catalogue::plain_text(&name))
}
