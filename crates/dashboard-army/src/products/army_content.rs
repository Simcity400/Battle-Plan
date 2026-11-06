//! Names and artwork the army products share: countries at a reached hour,
//! template names and icons, sub-unit names, design names and counted refusals.
use dashboard_base::{catalogue, products::equipment, source::Source};
use dashboard_core::config::LoadContext;
use dashboard_text::{
    catalogue::{Catalogue, View},
    metadata::{TaggedCountry, link_slot, tagged_country},
    template_facts::TemplateNaming,
};
use replay_data::rows::{TableId, tables};
use replay_frame::frame::{DecodedFrame, TableView};
use std::{
    collections::BTreeMap,
    io,
    sync::Arc,
};

/// Each refused name or artwork, counted, for the reply's warnings.
#[derive(Clone, Default)]
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
    /// Note `reason` `times` times.
    pub fn note_times(&mut self, reason: impl Into<String>, times: usize) {
        if times > 0 {
            *self.0.entry(reason.into()).or_default() += times;
        }
    }
    /// Each reason with how many times it was noted, in reason order.
    pub fn counted(self) -> Vec<(String, usize)> {
        self.0.into_iter().collect()
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
        TableView::NotRecorded { mode } => {
            Some(format!("{id:?}: not recorded in a {mode} recording"))
        }
        TableView::Truncated { .. } => Some(format!("{id:?}: incomplete read")),
    }
}

/// A country by its recorded identity: its canonical slot and country id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
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

/// Every listed tag's [`holder`] in one pass; a listed tag without one has no resolved identity.
pub fn holders(frame: &DecodedFrame) -> io::Result<BTreeMap<Vec<u8>, Option<Holder>>> {
    let countries = frame.complete::<tables::Countries>()?;
    let links = frame.complete::<tables::CountryLinks>()?;
    Ok(dashboard_text::metadata::tagged_countries(countries, links)
        .into_iter()
        .map(|(tag, country)| {
            let holder = match country {
                TaggedCountry::Found(row, slot) => Some(Holder { slot, id: row.id }),
                _ => None,
            };
            (tag.to_vec(), holder)
        })
        .collect())
}

/// [`holder`] of `tag` from the hour's [`holders`].
pub fn holder_in(holders: &BTreeMap<Vec<u8>, Option<Holder>>, tag: &str) -> io::Result<Option<Holder>> {
    match holders.get(tag.as_bytes()) {
        None => Ok(None),
        Some(Some(holder)) => Ok(Some(*holder)),
        Some(None) => Err(io::Error::other(format!("{tag} has no resolved country identity at this hour"))),
    }
}

/// Each slot's country id in a read, as [`same_holder`] finds it (the slot's first row).
pub fn read_identities(read: &DecodedFrame) -> Result<BTreeMap<u32, u32>, ()> {
    let rows = read.complete::<tables::Countries>().map_err(|_| ())?;
    let mut ids = BTreeMap::new();
    for row in rows {
        ids.entry(row.canonical_slot).or_insert(row.id);
    }
    Ok(ids)
}

/// [`same_holder`] from the read's [`read_identities`].
pub fn same_holder_in(ids: &Result<BTreeMap<u32, u32>, ()>, holder: Holder) -> Result<(), &'static str> {
    let id = ids
        .as_ref()
        .ok()
        .and_then(|ids| ids.get(&holder.slot))
        .ok_or("its country identity was not recorded")?;
    if *id == holder.id { Ok(()) } else { Err("it belongs to another country identity") }
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
    template_name_of(&template_naming(frame, handle), cat)
}

/// What [`template_name`] names a template from at the frame's hour.
pub fn template_naming(frame: &DecodedFrame, handle: u64) -> Result<TemplateNaming, String> {
    dashboard_text::template_facts::display_naming(frame, handle)
}

/// [`template_naming`] of any frame's tables, a walk's views among them.
pub fn template_naming_in(
    frame: replay_frame::frame::HourFrame<'_>,
    handle: u64,
) -> Result<TemplateNaming, String> {
    dashboard_text::template_facts::display_naming_in(frame, handle)
}

/// [`template_naming_hourly`] of any frame's tables, a walk's views among them.
pub fn template_naming_hourly_in(frame: replay_frame::frame::HourFrame<'_>, handle: u64) -> bool {
    if let Some(decoded) = frame.decoded() {
        return template_naming_hourly(decoded.decoded_frame(), handle);
    }
    const NAME: u64 =
        replay_data::rows::mask::<replay_data::rows::DivisionTemplateIconRow>(&["display_name"]);
    frame
        .complete::<tables::DivisionTemplateIcons>(NAME)
        .ok()
        .and_then(|rows| rows.get(u128::from(handle)).ok().flatten())
        .is_some_and(|row| {
            matches!(&row.display_name, replay_data::rows::Recorded::Value(Some(name)) if name.sample_hour.stamp().is_some())
        })
}

/// [`template_artwork`] of any frame's tables, a walk's views among them.
pub fn template_artwork_in(
    frame: replay_frame::frame::HourFrame<'_>,
    handle: u64,
) -> Result<TemplateArtwork, String> {
    if let Some(decoded) = frame.decoded() {
        return template_artwork(decoded.decoded_frame(), handle);
    }
    const ARTWORK: u64 = replay_data::rows::mask::<replay_data::rows::DivisionTemplateIconRow>(&[
        "priority_unit",
        "custom_icon",
        "priority_unit_name",
    ]);
    let rows = frame
        .complete::<tables::DivisionTemplateIcons>(ARTWORK)
        .map_err(|why| format!("{ARTWORK_UNAVAILABLE}{why}"))?;
    let row = rows
        .get(u128::from(handle))
        .ok()
        .flatten()
        .ok_or("Template artwork operands not recorded")?;
    Ok(TemplateArtwork {
        priority_unit: row.priority_unit,
        custom_icon: row.custom_icon,
        unit: row.priority_unit_name.clone(),
    })
}

/// Whether a template's naming operands were sampled for this hour only.
pub fn template_naming_hourly(frame: &DecodedFrame, handle: u64) -> bool {
    frame
        .complete::<tables::DivisionTemplateIcons>()
        .ok()
        .and_then(|rows| {
            let at = rows.binary_search_by_key(&handle, |row| row.template).ok()?;
            rows.get(at)
        })
        .is_some_and(|row| {
            matches!(&row.display_name, replay_data::rows::Recorded::Value(Some(name)) if name.sample_hour.stamp().is_some())
        })
}

/// [`template_name`] from its operands.
pub fn template_name_of(naming: &Result<TemplateNaming, String>, cat: Option<&Arc<Catalogue>>) -> Result<String, String> {
    let cat = cat.ok_or("Template name needs the recorded content stack")?;
    naming
        .clone()
        .and_then(|naming| naming.localise(&cat.localisation))
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
    template_icon_of(source, &template_artwork(frame, handle), cat)
}

/// How an artwork refused for its unreadable icons table starts.
pub const ARTWORK_UNAVAILABLE: &str = "Template artwork unavailable: ";

/// What [`template_icon`] draws a template from at the frame's hour.
pub fn template_artwork(frame: &DecodedFrame, handle: u64) -> Result<TemplateArtwork, String> {
    let rows = frame
        .complete::<tables::DivisionTemplateIcons>()
        .map_err(|why| format!("{ARTWORK_UNAVAILABLE}{why}"))?;
    let row = rows
        .binary_search_by_key(&handle, |row| row.template)
        .ok()
        .map(|at| &rows[at])
        .ok_or("Template artwork operands not recorded")?;
    Ok(TemplateArtwork {
        priority_unit: row.priority_unit,
        custom_icon: row.custom_icon,
        unit: row.priority_unit_name.clone(),
    })
}

/// A template icon's drawing operands ([`template_artwork`]).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[derive(replay_spool::Spooled)]
pub struct TemplateArtwork {
    pub priority_unit: u32,
    pub custom_icon: i32,
    pub unit: Vec<u8>,
}

/// [`template_icon`] from its operands.
pub fn template_icon_of(
    source: &Source,
    artwork: &Result<TemplateArtwork, String>,
    cat: Option<&Arc<Catalogue>>,
) -> Result<String, String> {
    let cat = cat.ok_or("Template artwork needs the recorded content stack")?;
    let artwork = artwork.as_ref().map_err(Clone::clone)?;
    dashboard_land_battles::products::division_log::template_icon_of(
        source,
        cat,
        artwork.priority_unit,
        artwork.custom_icon,
        &artwork.unit,
    )
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

/// A read's design definitions kept by design, not as the read's rows.
pub trait DefinitionsOf {
    fn definition(
        &self,
        variant: u64,
    ) -> Option<std::borrow::Cow<'_, replay_data::rows::VariantDefinitionRow>>;
}

/// A read's design definitions.
#[derive(Clone, Copy)]
pub enum Definitions<'a> {
    Rows(&'a [replay_data::rows::VariantDefinitionRow]),
    Kept(&'a dyn DefinitionsOf),
}

impl<'a> Definitions<'a> {
    fn definition(
        self,
        variant: u64,
    ) -> Option<std::borrow::Cow<'a, replay_data::rows::VariantDefinitionRow>> {
        match self {
            Self::Rows(rows) => {
                let at = rows.binary_search_by_key(&variant, |row| row.id).ok()?;
                rows.get(at).map(std::borrow::Cow::Borrowed)
            }
            Self::Kept(kept) => kept.definition(variant),
        }
    }
}

/// A supplying read of the designs by its parts: its hour, its countries' tags, its
/// definitions, and the token rows its definitions spell.
pub type DesignsRead<'a> = (
    i32,
    equipment::Tags<'a>,
    Definitions<'a>,
    &'a [replay_data::rows::LexicalTokenRow],
);

/// Design names from the supplying read of their definitions.
pub struct Designs<'a> {
    read: Option<DesignsRead<'a>>,
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
        let read = read.and_then(|read| -> Option<DesignsRead<'a>> {
            Some((
                read.hour,
                equipment::Tags::new(
                    read.complete::<tables::Countries>().ok()?,
                    read.complete::<tables::CountryLinks>().ok()?,
                    read.complete::<tables::CountryAppearance>().ok(),
                ),
                Definitions::Rows(read.complete::<tables::VariantDefinitions>().ok()?),
                read.complete::<tables::LexicalTokens>().ok()?,
            ))
        });
        Self::of(read, cat)
    }

    /// Designs of a read given by its parts.
    pub fn of(read: Option<DesignsRead<'a>>, cat: Option<&'a Arc<Catalogue>>) -> Self {
        Self {
            read,
            cat,
            types: cat
                .ok_or_else(|| "Equipment classification needs the recorded content stack".to_owned())
                .and_then(|cat| equipment::units_of(cat)),
        }
    }
    pub fn design(&self, variant: u64, refusals: &mut Refusals) -> Design {
        let Some(operand) = self.operand(variant, refusals) else {
            return Design::default();
        };
        let name = self.name(&operand.raw);
        if name.is_none() {
            refusals.note(UNNAMED_DESIGN);
        }
        let archetype_name = operand.archetype.as_ref().and_then(|key| localised(self.cat, key).ok());
        Design {
            name,
            archetype: operand.archetype,
            archetype_name,
        }
    }

    /// A design's naming operands and archetype; none, noted, when its definition was not read.
    /// the reason noted, when its definitions were not read or do not hold it.
    pub fn operand(&self, variant: u64, refusals: &mut Refusals) -> Option<DesignOperand> {
        let Some((hour, tags, definitions, lexical)) = &self.read else {
            refusals.note("Equipment designs were not read at or before this hour");
            return None;
        };
        let Some(definition) = definitions.definition(variant) else {
            refusals.note("An equipment design has no recorded definition");
            return None;
        };
        let raw = equipment::raw_variant(&definition, *hour, tags, *lexical);
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
        Some(DesignOperand {
            raw: Arc::new(raw),
            archetype,
        })
    }

    /// A design's name from its naming operands.
    pub fn name(&self, raw: &equipment::RawVariant) -> Option<String> {
        design_name(raw, self.cat)
    }
}

/// Why a design read with its operands is shown without a name.
pub const UNNAMED_DESIGN: &str = "An equipment design has no resolved name";

/// A design's name from its naming operands, in the catalogue's language.
pub fn design_name(raw: &equipment::RawVariant, cat: Option<&Arc<Catalogue>>) -> Option<String> {
    equipment::variant_names(raw, cat.map(|c| c.as_ref() as &dyn equipment::Texts)).0
}

/// A design's naming operands and archetype key, as its read of the definitions records them.
#[derive(Clone, Debug)]
pub struct DesignOperand {
    pub raw: Arc<equipment::RawVariant>,
    pub archetype: Option<String>,
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
