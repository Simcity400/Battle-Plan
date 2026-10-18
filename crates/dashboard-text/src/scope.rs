//! Script scope targets resolved over one recorded hour.
//!
//! [`installed_content_core::scope::parse`] reads a scope as the game's event target
//! reader does; this resolves it against what the recording
//! holds. Whatever the recording cannot reach is refused by name with the
//! missing input, never guessed. The viewer and the dashboard both reach it
//! (the viewer depends on the dashboard), behind [`RecordedScopes::resolve`].

use installed_content_core::{scope::Keyword, scope::Names, scope::Scope, scope::ScopeKind};
use installed_content_core::{tag_aliases::TagAlias, tag_aliases::TagAliases};
use replay_data::{rows::FactionMemberRow, rows::TableSchema, rows::tables};
use replay_frame::{frame::DecodedFrame, frame::SharedView, frame::TableView};
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

mod original_tags;
pub use original_tags::OriginalTagLists;

/// A resolved scope target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// A country by its canonical slot.
    Country(u32),
    /// A state by its id.
    State(u32),
    /// A character by its recorded id.
    Character(u32),
}

impl Target {
    /// The target's kind for [`installed_content_core::scope::TriggerScopes`]. A scope
    /// set with the clearing flag carries only this kind; callers that keep
    /// other carried targets pass them themselves.
    pub fn kind(self) -> ScopeKind {
        match self {
            Self::Country(_) => ScopeKind::Country,
            Self::State(_) => ScopeKind::State,
            Self::Character(_) => ScopeKind::Character,
        }
    }
}

/// Why a scope resolves to no target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unresolved {
    /// The scope names nothing in the recorded facts, as the game's scope is
    /// null there: a state whose owner or controller link is 0, a country
    /// with no overlord or capital, a scope the context does not set, a
    /// zero number or text the event target reader does not accept. Not a
    /// refusal.
    Nothing(String),
    /// A recorded input the scope needs is missing or incomplete (a table
    /// the hour lacks, a link CountryLinks does not hold) or not recorded at
    /// all: a refusal, naming the input.
    Missing(String),
}

impl std::fmt::Display for Unresolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Nothing(what) | Self::Missing(what) => f.write_str(what),
        }
    }
}

/// The scopes a script or text is evaluated in. `None` is a scope the
/// caller's context does not set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Context {
    pub root: Option<Target>,
    pub this: Option<Target>,
    pub prev: Option<Target>,
    pub from: Option<Target>,
}

impl Context {
    /// A country as ROOT and THIS, as a country context sets them.
    pub fn country(slot: u32) -> Self {
        Self {
            root: Some(Target::Country(slot)),
            this: Some(Target::Country(slot)),
            ..Self::default()
        }
    }
}

/// The recorded facts scopes resolve through, read from one hour. Each map is
/// built from the hour's tables when a query first needs it.
#[derive(Clone, Debug, Default)]
pub struct RecordedScopes {
    /// The hour's tables the maps below are built from.
    views: Option<Arc<Views>>,
    /// Raw country link to canonical slot (`CountryLinks`).
    links: OnceLock<BTreeMap<u32, u32>>,
    /// Canonical slot to tag, and back (`Countries`).
    tags: OnceLock<BTreeMap<u32, String>>,
    raw_ids: OnceLock<BTreeMap<u32, u32>>,
    /// Whether `Countries` is complete, so `tags` is every country.
    countries_complete: bool,
    slots: OnceLock<BTreeMap<String, u32>>,
    /// State id to raw owner and controller links (`States`).
    states: OnceLock<BTreeMap<u32, (u32, u32)>>,
    /// Canonical slot to capital state (`Politics.capital`).
    capitals: OnceLock<BTreeMap<u32, i32>>,
    /// Canonical slot to raw overlord link (`CountryAppearance.overlord`).
    overlords: OnceLock<BTreeMap<u32, i32>>,
    faction_members_complete: bool,
    /// Character script token, folded as the lexer folds a token (the
    /// lookup is by token ID), to its ids with their countries' slots
    /// (`Characters.name_token_id` through `LexicalTokens`).
    characters: OnceLock<BTreeMap<String, Vec<(u32, Option<u32>)>>>,
    /// Whether `Characters` and `LexicalTokens` are complete, so a token
    /// no row has names no live character.
    characters_complete: bool,
    /// Canonical slot to owned state count
    /// (`CountryCounterInputs.owned_state_count`).
    owned: OnceLock<BTreeMap<u32, i32>>,
    /// Exact recorded original-tag buckets and physical country-array order.
    original_tags: OriginalTagLists,
    /// The content's tag alias database.
    aliases: Arc<TagAliases>,
    saved_targets_complete: bool,
    /// Why a table's rows may lack an entry, by table name.
    gaps: BTreeMap<&'static str, String>,
}

/// The hour's tables recorded scopes read.
#[derive(Clone, Debug)]
struct Views {
    links: SharedView,
    countries: SharedView,
    states: SharedView,
    politics: SharedView,
    appearance: SharedView,
    faction_members: SharedView,
    counters: SharedView,
    characters: SharedView,
    lexical: SharedView,
    saved_targets: SharedView,
}

/// A view's rows: none when the table is unavailable.
fn view_rows<T: TableSchema>(view: Option<&SharedView>) -> &[T::Row] {
    // Built for a recorded query, whose own result is its input.
    crate::triggers::inputs::unrecorded(|| match view.map(|view| &**view) {
        Some(TableView::Complete(rows) | TableView::Truncated { rows, .. }) => {
            rows.rows::<T>().unwrap_or_default()
        }
        _ => &[],
    })
}

/// A table's rows, and why an entry it lacks is unknown.
fn table<T: TableSchema>(frame: &DecodedFrame) -> (&[T::Row], String) {
    match &*frame.tables[T::ID.index()] {
        TableView::Complete(rows) => (
            rows.rows::<T>().unwrap_or_default(),
            "the complete table has no row for it".into(),
        ),
        TableView::Truncated { rows, missing } => (
            rows.rows::<T>().unwrap_or_default(),
            format!("the table is truncated ({missing} rows missing)"),
        ),
        TableView::Unavailable { reason } => (&[], format!("the table is unavailable: {reason}")),
    }
}

/// Saved-target validity using captured object liveness.
pub(crate) fn valid_saved_target(r: &replay_data::rows::GlobalSavedEventTargetRow) -> bool {
    !r.name.is_empty()
        && (r.state != 0
            || r.country > 0
            || r.live & !replay_data::rows::GlobalSavedEventTargetRow::UNSAVED_NAME != 0)
}

/// A recorded query of the hour's scopes ([`crate::triggers::inputs`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Query {
    SavedTarget(Vec<u8>),
    SavedSelection(Vec<u8>, crate::triggers::saved_event_targets::Reader),
    SavedCountry(i32, Vec<u8>),
    ResolveTag(String),
    RawSlot(u32),
    PositiveCountry(u32),
    OriginalBucket(i32),
    Alias(String),
    AliasEntry(String),
    OwnedStates(u32),
    OriginalTags,
    Tag(u32),
    Country(String),
    Slot(u32),
    Faction(u32),
    FactionFirstRaw(u64),
    FactionLeaderRaw(u32),
    FactionLeader(u32),
    ResolveText(String, bool, Context),
    Resolve(ScopeKey, Context),
    Describe(Target),
    Roster,
}

impl Query {
    /// What this query's answer is a function of besides the content
    /// ([`crate::triggers::inputs::Key::dependence`]); None when undeclared.
    pub(crate) fn dependence(&self) -> Option<Vec<crate::triggers::inputs::TableRead>> {
        use crate::triggers::inputs::TableRead;
        use replay_data::rows::TableId;
        let link = |raw: u32| {
            TableRead::fields(TableId::CountryLinks, &["canonical_slot"]).row(raw.into())
        };
        let tags = || TableRead::fields(TableId::Countries, &["canonical_slot", "tag"]);
        Some(match self {
            &Query::Slot(raw) => vec![link(raw)],
            &Query::RawSlot(raw) => {
                if raw == 0 {
                    Vec::new()
                } else {
                    vec![link(raw)]
                }
            }
            &Query::PositiveCountry(raw) => vec![
                link(raw),
                TableRead::fields(TableId::Countries, &["canonical_slot"]),
            ],
            Query::Tag(_) | Query::Country(_) => vec![tags()],
            // Parsing and alias lookup read the recorded tags; alias bodies
            // themselves belong to the fixed content stack.
            Query::Alias(_) | Query::AliasEntry(_) => vec![tags()],
            Query::Roster => vec![TableRead::fields(TableId::Countries, &["canonical_slot"])],
            Query::Describe(Target::Country(_)) => vec![tags()],
            Query::Describe(Target::State(_) | Target::Character(_)) => Vec::new(),
            Query::ResolveTag(_) => Self::tag_dependence(),
            Query::Resolve(ScopeKey(Scope::Country(_)), _) => Self::tag_dependence(),
            _ => return None,
        })
    }

    /// Tag substitution can visit every member of the original-tag bucket,
    /// resolving its raw reference and testing whether it owns a state.
    fn tag_dependence() -> Vec<crate::triggers::inputs::TableRead> {
        use crate::triggers::inputs::TableRead;
        use replay_data::rows::TableId;
        vec![
            TableRead::fields(TableId::Countries, &["canonical_slot", "tag"]),
            TableRead::fields(TableId::CountryLinks, &["canonical_slot"]),
            TableRead::fields(TableId::CountryCounterInputs, &["owned_state_count"]),
            TableRead::fields(
                TableId::ReservedCountryNaming,
                &["owned_state_count", "original_tag_buckets"],
            ),
        ]
    }
}

/// A parsed scope as a query argument, compared and hashed by its full
/// `Debug` form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeKey(Scope);

impl std::hash::Hash for ScopeKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        format!("{:?}", self.0).hash(state);
    }
}

impl RecordedScopes {
    /// The digest `query` reads in this hour's scopes.
    pub fn reread(&self, query: &Query) -> u64 {
        match query {
            Query::SavedTarget(name) => {
                crate::triggers::inputs::digest_debug(&self.saved_target_unrecorded(name))
            }
            Query::SavedSelection(name, reader) => crate::triggers::inputs::digest_debug(
                &self.saved_selection_unrecorded(name, *reader),
            ),
            Query::SavedCountry(raw, name) => {
                crate::triggers::inputs::digest(&self.saved_country_unrecorded(*raw, name))
            }
            Query::ResolveTag(tag) => {
                crate::triggers::inputs::digest(&self.resolve_tag_unrecorded(tag))
            }
            Query::RawSlot(raw) => crate::triggers::inputs::digest(&self.raw_slot_unrecorded(*raw)),
            Query::PositiveCountry(raw) => {
                crate::triggers::inputs::digest(&self.positive_country_unrecorded(*raw))
            }
            Query::OriginalBucket(selected) => {
                crate::triggers::inputs::digest(&self.original_bucket_unrecorded(*selected))
            }
            Query::Alias(tag) => crate::triggers::inputs::digest(&self.alias_unrecorded(tag)),
            Query::AliasEntry(tag) => {
                crate::triggers::inputs::digest_debug(&self.alias_entry_unrecorded(tag))
            }
            Query::OwnedStates(slot) => {
                crate::triggers::inputs::digest(&self.owned_states_unrecorded(*slot))
            }
            Query::OriginalTags => {
                crate::triggers::inputs::digest_debug(&self.original_tags_unrecorded())
            }
            Query::Tag(slot) => crate::triggers::inputs::digest(&self.tag_unrecorded(*slot)),
            Query::Country(tag) => crate::triggers::inputs::digest(&self.country_unrecorded(tag)),
            Query::Slot(link) => crate::triggers::inputs::digest(&self.slot_unrecorded(*link)),
            Query::Faction(slot) => {
                crate::triggers::inputs::digest(&self.faction_unrecorded(*slot))
            }
            Query::FactionFirstRaw(faction) => {
                crate::triggers::inputs::digest(&self.faction_first_raw_unrecorded(*faction))
            }
            Query::FactionLeaderRaw(slot) => {
                crate::triggers::inputs::digest(&self.faction_leader_raw_unrecorded(*slot))
            }
            Query::FactionLeader(slot) => {
                crate::triggers::inputs::digest(&self.faction_leader_unrecorded(*slot))
            }
            Query::ResolveText(text, quoted, context) => crate::triggers::inputs::digest_debug(
                &self.resolve_text_unrecorded(text, *quoted, context),
            ),
            Query::Resolve(scope, context) => {
                crate::triggers::inputs::digest_debug(&self.resolve_unrecorded(&scope.0, context))
            }
            Query::Describe(target) => {
                crate::triggers::inputs::digest(&self.describe_unrecorded(*target))
            }
            Query::Roster => crate::triggers::inputs::digest(
                &self.roster_unrecorded().map(Iterator::collect::<Vec<u32>>),
            ),
        }
    }
}

impl RecordedScopes {
    /// Reads scope facts and native country-list snapshots at this hour.
    pub fn read(frame: &DecodedFrame, aliases: Arc<TagAliases>) -> Self {
        Self::read_with(frame, OriginalTagLists::read(frame), aliases)
    }

    /// Uses a selectively read native-list snapshot for this same hour.
    pub fn read_with(
        frame: &DecodedFrame,
        original_tags: OriginalTagLists,
        aliases: Arc<TagAliases>,
    ) -> Self {
        let complete = |id: replay_data::rows::TableId| {
            matches!(&*frame.tables[id.index()], TableView::Complete(_))
        };
        let mut gaps = BTreeMap::new();
        gaps.insert("CountryLinks", table::<tables::CountryLinks>(frame).1);
        gaps.insert("Countries", table::<tables::Countries>(frame).1);
        gaps.insert("States", table::<tables::States>(frame).1);
        gaps.insert("Politics", table::<tables::Politics>(frame).1);
        gaps.insert(
            "CountryAppearance",
            table::<tables::CountryAppearance>(frame).1,
        );
        gaps.insert("FactionMembers", table::<tables::FactionMembers>(frame).1);
        gaps.insert(
            "CountryCounterInputs",
            table::<tables::CountryCounterInputs>(frame).1,
        );
        gaps.insert(
            "Characters",
            format!(
                "{} (LexicalTokens: {})",
                table::<tables::Characters>(frame).1,
                table::<tables::LexicalTokens>(frame).1
            ),
        );
        gaps.insert(
            "GlobalSavedEventTargets",
            table::<tables::GlobalSavedEventTargets>(frame).1,
        );
        let view = |id: replay_data::rows::TableId| frame.tables[id.index()].clone();
        Self {
            views: Some(Arc::new(Views {
                links: view(tables::CountryLinks::ID),
                countries: view(tables::Countries::ID),
                states: view(tables::States::ID),
                politics: view(tables::Politics::ID),
                appearance: view(tables::CountryAppearance::ID),
                faction_members: view(tables::FactionMembers::ID),
                counters: view(tables::CountryCounterInputs::ID),
                characters: view(tables::Characters::ID),
                lexical: view(tables::LexicalTokens::ID),
                saved_targets: view(tables::GlobalSavedEventTargets::ID),
            })),
            countries_complete: complete(tables::Countries::ID),
            faction_members_complete: complete(tables::FactionMembers::ID),
            // A character's script token is its name token id spelled by the
            // hour's LexicalTokens (`Characters.token` is its saved name).
            characters_complete: complete(tables::Characters::ID)
                && complete(tables::LexicalTokens::ID),
            saved_targets_complete: complete(tables::GlobalSavedEventTargets::ID),
            original_tags,
            aliases,
            gaps,
            ..Self::default()
        }
    }

    fn rows<T: TableSchema>(&self, view: impl Fn(&Views) -> &SharedView) -> &[T::Row] {
        view_rows::<T>(self.views.as_deref().map(view))
    }

    fn links(&self) -> &BTreeMap<u32, u32> {
        self.links.get_or_init(|| {
            self.rows::<tables::CountryLinks>(|views| &views.links)
                .iter()
                .map(|link| (link.id, link.canonical_slot))
                .collect()
        })
    }

    fn tags(&self) -> &BTreeMap<u32, String> {
        self.tags.get_or_init(|| {
            self.rows::<tables::Countries>(|views| &views.countries)
                .iter()
                .map(|country| {
                    (
                        country.canonical_slot,
                        String::from_utf8_lossy(&country.tag).into_owned(),
                    )
                })
                .collect()
        })
    }

    fn raw_ids(&self) -> &BTreeMap<u32, u32> {
        self.raw_ids.get_or_init(|| {
            self.rows::<tables::Countries>(|views| &views.countries)
                .iter()
                .map(|row| (row.canonical_slot, row.id))
                .collect()
        })
    }

    fn slots(&self) -> &BTreeMap<String, u32> {
        self.slots.get_or_init(|| {
            self.tags()
                .iter()
                .map(|(slot, tag)| (tag.clone(), *slot))
                .collect()
        })
    }

    fn states(&self) -> &BTreeMap<u32, (u32, u32)> {
        self.states.get_or_init(|| {
            self.rows::<tables::States>(|views| &views.states)
                .iter()
                .map(|state| (state.id, (state.owner, state.controller)))
                .collect()
        })
    }

    fn capitals(&self) -> &BTreeMap<u32, i32> {
        self.capitals.get_or_init(|| {
            let links = self.links();
            self.rows::<tables::Politics>(|views| &views.politics)
                .iter()
                .filter_map(|row| Some((*links.get(&row.country)?, row.capital)))
                .collect()
        })
    }

    fn overlords(&self) -> &BTreeMap<u32, i32> {
        self.overlords.get_or_init(|| {
            let links = self.links();
            self.rows::<tables::CountryAppearance>(|views| &views.appearance)
                .iter()
                .filter_map(|row| Some((*links.get(&row.country)?, row.overlord)))
                .collect()
        })
    }

    fn owned(&self) -> &BTreeMap<u32, i32> {
        self.owned.get_or_init(|| {
            let links = self.links();
            self.rows::<tables::CountryCounterInputs>(|views| &views.counters)
                .iter()
                .filter_map(|row| Some((*links.get(&row.country)?, row.owned_state_count)))
                .collect()
        })
    }

    fn characters(&self) -> &BTreeMap<String, Vec<(u32, Option<u32>)>> {
        self.characters.get_or_init(|| {
            let links = self.links();
            let lexical = self.rows::<tables::LexicalTokens>(|views| &views.lexical);
            let mut by_token: BTreeMap<String, Vec<(u32, Option<u32>)>> = BTreeMap::new();
            for row in self.rows::<tables::Characters>(|views| &views.characters) {
                if let Some(token) = crate::identity::character_script_token(row, lexical) {
                    by_token
                        .entry(token.to_ascii_lowercase())
                        .or_default()
                        .push((row.id, links.get(&row.country).copied()));
                }
            }
            by_token
        })
    }

    fn faction_members(&self) -> &[FactionMemberRow] {
        self.rows::<tables::FactionMembers>(|views| &views.faction_members)
    }

    fn saved_targets(&self) -> &[replay_data::rows::GlobalSavedEventTargetRow] {
        self.rows::<tables::GlobalSavedEventTargets>(|views| &views.saved_targets)
    }

    /// Return the first name match, even when that record is invalid;
    /// no later duplicate wins.
    pub(crate) fn saved_target(
        &self,
        name: &[u8],
    ) -> Result<Option<&replay_data::rows::GlobalSavedEventTargetRow>, String> {
        crate::triggers::inputs::query_debug(
            || crate::triggers::inputs::Key::Scopes(Query::SavedTarget(name.to_vec())),
            || self.saved_target_unrecorded(name),
        )
    }

    fn saved_target_unrecorded(
        &self,
        name: &[u8],
    ) -> Result<Option<&replay_data::rows::GlobalSavedEventTargetRow>, String> {
        if !self.saved_targets_complete {
            return Err(format!(
                "saved event target {}: GlobalSavedEventTargets {}",
                String::from_utf8_lossy(name),
                self.gaps
                    .get("GlobalSavedEventTargets")
                    .map(String::as_str)
                    .unwrap_or("is not recorded")
            ));
        }
        Ok(self.saved_targets().iter().find(|r| r.name == name))
    }

    pub(crate) fn saved_selection(
        &self,
        name: &[u8],
        reader: crate::triggers::saved_event_targets::Reader,
    ) -> Result<crate::triggers::saved_event_targets::Selected, String> {
        crate::triggers::inputs::query_debug(
            || crate::triggers::inputs::Key::Scopes(Query::SavedSelection(name.to_vec(), reader)),
            || self.saved_selection_unrecorded(name, reader),
        )
    }

    fn saved_selection_unrecorded(
        &self,
        name: &[u8],
        reader: crate::triggers::saved_event_targets::Reader,
    ) -> Result<crate::triggers::saved_event_targets::Selected, String> {
        use crate::triggers::saved_event_targets::{Selected, select};
        Ok(self
            .saved_target(name)?
            .map_or(Selected::Missing, |r| select(r, reader)))
    }

    pub(crate) fn saved_country(&self, raw: i32, name: &[u8]) -> Result<Option<u32>, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Scopes(Query::SavedCountry(raw, name.to_vec())),
            || self.saved_country_unrecorded(raw, name),
        )
    }

    fn saved_country_unrecorded(&self, raw: i32, name: &[u8]) -> Result<Option<u32>, String> {
        if raw <= 0 {
            return Ok(None);
        }
        self.slot(raw as u32).map(Some).ok_or_else(|| {
            format!(
                "saved event target {}: country reference {raw} is not in CountryLinks",
                String::from_utf8_lossy(name)
            )
        })
    }

    /// An owning own country wins; otherwise use the first owning bucket
    /// member, or the first member when none owns a state. The caller accepts
    /// only an owning result, so a nonowning member or an empty bucket keeps
    /// the own country.
    pub fn resolve_tag(&self, tag: &str) -> Result<u32, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Scopes(Query::ResolveTag(tag.to_owned())),
            || self.resolve_tag_unrecorded(tag),
        )
    }

    fn resolve_tag_unrecorded(&self, tag: &str) -> Result<u32, String> {
        if let Some(refusal) = self.alias(tag) {
            return Err(refusal);
        }
        let slot = self
            .country(tag)
            .ok_or_else(|| format!("country {tag} has no recorded Countries row"))?;
        if self.owned_states(slot)? >= 1 {
            return Ok(slot);
        }
        let raw = *self
            .raw_ids()
            .get(&slot)
            .ok_or_else(|| format!("country slot {slot} has no own raw tag"))?;
        let canonical = self.raw_slot(raw)?;
        for &member in self.original_tags.bucket(canonical)? {
            let member = self.raw_slot(member)?;
            if self.owned_states(member)? >= 1 {
                // Stop at this owning member and accept only a positive
                // returned own word; otherwise keep the own country.
                let raw = if member == 0 {
                    0
                } else {
                    *self
                        .raw_ids()
                        .get(&member)
                        .ok_or_else(|| format!("country slot {member} has no own raw tag"))?
                        as i32
                };
                return Ok(if raw > 0 { member } else { slot });
            }
        }
        Ok(slot)
    }

    /// A known zero reference resolves to the separately authenticated reserved
    /// country. Missing positive links cannot be inferred to map to zero.
    pub(crate) fn raw_slot(&self, raw: u32) -> Result<u32, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Scopes(Query::RawSlot(raw)),
            || self.raw_slot_unrecorded(raw),
        )
    }

    fn raw_slot_unrecorded(&self, raw: u32) -> Result<u32, String> {
        if raw == 0 {
            return Ok(0);
        }
        self.slot(raw)
            .ok_or_else(|| format!("country raw reference {raw} is not in CountryLinks"))
    }

    /// Country iterators retain only signed-positive own tag words
    /// after resolving a raw reference.
    pub(crate) fn positive_country(&self, raw: u32) -> Result<Option<u32>, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Scopes(Query::PositiveCountry(raw)),
            || self.positive_country_unrecorded(raw),
        )
    }

    fn positive_country_unrecorded(&self, raw: u32) -> Result<Option<u32>, String> {
        let slot = self.raw_slot(raw)?;
        let own = if slot == 0 {
            0
        } else {
            *self.raw_ids().get(&slot).ok_or_else(|| {
                format!("country slot {slot} has no Countries row for its own raw tag")
            })?
        };
        Ok(((own as i32) > 0).then_some(slot))
    }

    /// Original-tag bucket selected by a positive tag word, before appending
    /// that word. Nonpositive words do not demand the snapshot.
    pub(crate) fn original_bucket(&self, selected: i32) -> Result<&[u32], String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Scopes(Query::OriginalBucket(selected)),
            || self.original_bucket_unrecorded(selected),
        )
    }

    fn original_bucket_unrecorded(&self, selected: i32) -> Result<&[u32], String> {
        if selected <= 0 {
            return Ok(&[]);
        }
        self.original_tags.bucket(self.raw_slot(selected as u32)?)
    }

    /// Why a three-byte name that is no recorded country tag is refused
    /// when it is, or may be, a tag alias: the alias database is checked
    /// after the country tags, and the country an alias names is chosen
    /// at run time by its targets and
    /// triggers, which are not evaluated yet.
    pub fn alias(&self, tag: &str) -> Option<String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Scopes(Query::Alias(tag.to_owned())),
            || self.alias_unrecorded(tag),
        )
    }

    fn alias_unrecorded(&self, tag: &str) -> Option<String> {
        if self.slots().contains_key(tag) {
            return None;
        }
        if self.aliases.get(tag).is_some() {
            return Some(format!(
                "tag alias {tag}: its triggers run in the trigger evaluator (crate::triggers), not in a scope read alone"
            ));
        }
        (tag.len() == 3 && !self.aliases.unknown.is_empty()).then(|| {
            format!(
                "{tag}: whether it is a tag alias is unknown: {}",
                self.aliases.unknown.join("; ")
            )
        })
    }

    /// The body of the tag alias a three-byte name is, when it is no
    /// recorded country tag: the alias database is checked after the
    /// country tags.
    pub fn alias_entry(&self, tag: &str) -> Option<&TagAlias> {
        crate::triggers::inputs::query_debug(
            || crate::triggers::inputs::Key::Scopes(Query::AliasEntry(tag.to_owned())),
            || self.alias_entry_unrecorded(tag),
        )
    }

    fn alias_entry_unrecorded(&self, tag: &str) -> Option<&TagAlias> {
        if self.slots().contains_key(tag) {
            return None;
        }
        self.aliases.get(tag)
    }

    /// Every captured canonical country, in slot order. This canonical
    /// roster is distinct from the physical native array with alias occurrences.
    pub fn roster(&self) -> Result<impl Iterator<Item = u32> + '_, String> {
        let roster = crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Scopes(Query::Roster),
            || self.roster_unrecorded().map(Iterator::collect::<Vec<u32>>),
        );
        roster.map(Vec::into_iter)
    }

    fn roster_unrecorded(&self) -> Result<impl Iterator<Item = u32> + '_, String> {
        if !self.countries_complete {
            return Err(format!(
                "the country roster is not complete: {}",
                self.gaps.get("Countries").cloned().unwrap_or_default()
            ));
        }
        Ok(self.tags().keys().copied())
    }

    /// A country's owned state count, or why it is not
    /// recorded.
    pub fn owned_states(&self, slot: u32) -> Result<i32, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Scopes(Query::OwnedStates(slot)),
            || self.owned_states_unrecorded(slot),
        )
    }

    fn owned_states_unrecorded(&self, slot: u32) -> Result<i32, String> {
        if slot == 0 {
            return self.original_tags.reserved_owned_states();
        }
        self.owned().get(&slot).copied().ok_or_else(|| {
            format!(
                "the owned state count (CountryCounterInputs.owned_state_count) of {} is not recorded: {}",
                self.describe(Target::Country(slot)),
                self.gaps.get("CountryCounterInputs").cloned().unwrap_or_default()
            )
        })
    }

    /// Exact recorded original-tag buckets and physical country-array order.
    pub fn original_tags(&self) -> &OriginalTagLists {
        crate::triggers::inputs::query_debug(
            || crate::triggers::inputs::Key::Scopes(Query::OriginalTags),
            || self.original_tags_unrecorded(),
        )
    }

    fn original_tags_unrecorded(&self) -> &OriginalTagLists {
        &self.original_tags
    }

    /// A country's tag.
    pub fn tag(&self, slot: u32) -> Option<&str> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Scopes(Query::Tag(slot)),
            || self.tag_unrecorded(slot),
        )
    }

    fn tag_unrecorded(&self, slot: u32) -> Option<&str> {
        self.tags().get(&slot).map(String::as_str)
    }

    /// A country's canonical slot by tag.
    pub fn country(&self, tag: &str) -> Option<u32> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Scopes(Query::Country(tag.to_owned())),
            || self.country_unrecorded(tag),
        )
    }

    fn country_unrecorded(&self, tag: &str) -> Option<u32> {
        self.slots().get(tag).copied()
    }

    /// A raw country link's canonical slot.
    pub fn slot(&self, link: u32) -> Option<u32> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Scopes(Query::Slot(link)),
            || self.slot_unrecorded(link),
        )
    }

    fn slot_unrecorded(&self, link: u32) -> Option<u32> {
        self.links().get(&link).copied()
    }

    /// The actual country's faction. Country0 uses its recorded faction field.
    pub(crate) fn faction(&self, slot: u32) -> Result<Option<u64>, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Scopes(Query::Faction(slot)),
            || self.faction_unrecorded(slot),
        )
    }

    fn faction_unrecorded(&self, slot: u32) -> Result<Option<u64>, String> {
        if slot == 0 {
            return self.original_tags.reserved_faction();
        }
        if let Some(member) = self
            .faction_members()
            .iter()
            .find(|row| row.country != 0 && self.slot(row.country) == Some(slot))
        {
            return Ok(Some(member.faction));
        }
        if !self.faction_members_complete {
            return Err(format!(
                "the faction of country slot {slot} is unknown: FactionMembers {}",
                self.gaps["FactionMembers"]
            ));
        }
        if let Some(row) = self
            .faction_members()
            .iter()
            .find(|row| row.country != 0 && self.slot(row.country).is_none())
        {
            return Err(format!(
                "the faction of country slot {slot} is unknown: a FactionMembers row's country link {} resolves to no country: CountryLinks {}",
                row.country, self.gaps["CountryLinks"]
            ));
        }
        Ok(None)
    }

    /// The exact first member's own raw country word (ordinal zero).
    /// Literal raw0 is an authenticated Country0 member, never a null placeholder.
    pub(crate) fn faction_first_raw(&self, faction: u64) -> Result<u32, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Scopes(Query::FactionFirstRaw(faction)),
            || self.faction_first_raw_unrecorded(faction),
        )
    }

    fn faction_first_raw_unrecorded(&self, faction: u64) -> Result<u32, String> {
        self.faction_members()
            .iter()
            .find(|row| row.faction == faction && row.ordinal == 0)
            .map(|row| row.country)
            .ok_or_else(|| {
                format!(
                    "faction {faction} has no first member: FactionMembers {}",
                    self.gaps["FactionMembers"]
                )
            })
    }

    pub(crate) fn faction_leader_raw(&self, slot: u32) -> Result<Option<u32>, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Scopes(Query::FactionLeaderRaw(slot)),
            || self.faction_leader_raw_unrecorded(slot),
        )
    }

    fn faction_leader_raw_unrecorded(&self, slot: u32) -> Result<Option<u32>, String> {
        self.faction(slot)?
            .map(|faction| self.faction_first_raw(faction))
            .transpose()
    }

    /// Existing scalar API: retain its ordinary link resolution. Scope and text
    /// select their distinct literal-zero behavior through faction_leader_raw.
    pub fn faction_leader(&self, slot: u32) -> Result<Option<u32>, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Scopes(Query::FactionLeader(slot)),
            || self.faction_leader_unrecorded(slot),
        )
    }

    fn faction_leader_unrecorded(&self, slot: u32) -> Result<Option<u32>, String> {
        let Some(raw) = self.faction_leader_raw(slot)? else {
            return Ok(None);
        };
        self.slot(raw).map(Some).ok_or_else(|| {
            format!(
                "the faction leader link {raw} resolves to no country: CountryLinks {}",
                self.gaps["CountryLinks"]
            )
        })
    }

    /// Parses `text` as the game's event target reader does
    /// ([`installed_content_core::scope::parse`]) with this hour's countries and
    /// characters, and resolves it in `context`.
    pub fn resolve_text(
        &self,
        text: &str,
        quoted: bool,
        context: &Context,
    ) -> Result<Target, Unresolved> {
        crate::triggers::inputs::query_debug(
            || {
                crate::triggers::inputs::Key::Scopes(Query::ResolveText(
                    text.to_owned(),
                    quoted,
                    *context,
                ))
            },
            || self.resolve_text_unrecorded(text, quoted, context),
        )
    }

    fn resolve_text_unrecorded(
        &self,
        text: &str,
        quoted: bool,
        context: &Context,
    ) -> Result<Target, Unresolved> {
        self.resolve(&installed_content_core::scope::parse(text, quoted, self), context)
    }

    /// Resolves a parsed scope in `context`: the target, a scope that names
    /// nothing ([`Unresolved::Nothing`]), or a refusal naming the recorded
    /// input it would need ([`Unresolved::Missing`]).
    ///
    /// Not yet implemented: the `occupied` promotion, which
    /// `StateResistance.country` (the occupied country's original tag) would
    /// answer; it is refused as unrecorded until a consumer needs it.
    pub fn resolve(&self, scope: &Scope, context: &Context) -> Result<Target, Unresolved> {
        crate::triggers::inputs::query_debug(
            || {
                crate::triggers::inputs::Key::Scopes(Query::Resolve(
                    ScopeKey(scope.clone()),
                    *context,
                ))
            },
            || self.resolve_unrecorded(scope, context),
        )
    }

    fn resolve_unrecorded(&self, scope: &Scope, context: &Context) -> Result<Target, Unresolved> {
        use Unresolved::{Missing, Nothing};
        let gap = |table: &str| self.gaps.get(table).cloned().unwrap_or_default();
        match scope {
            Scope::State(id) => {
                let id = u32::try_from(*id)
                    .map_err(|_| Nothing(format!("{scope}: no state has a negative id")))?;
                if self.states().contains_key(&id) {
                    Ok(Target::State(id))
                } else {
                    Err(Missing(format!(
                        "state {id} has no recorded States row: {}",
                        gap("States")
                    )))
                }
            }
            Scope::Country(tag) => self.resolve_tag(tag).map(Target::Country).map_err(Missing),
            // Look the token up among the scope country's characters.
            // Missing country or character lookups are logged by the game;
            // a missing character leaves the scope's character invalid.
            Scope::Character(token) => {
                let Some(Target::Country(country)) = context.this else {
                    return Err(Nothing(format!(
                        "character {token}: the scope has no country, which the game requires"
                    )));
                };
                let found = self
                    .characters()
                    .get(&token.to_ascii_lowercase())
                    .and_then(|found| {
                        found
                            .iter()
                            .find(|(_, owner)| *owner == Some(country))
                            .map(|(id, _)| Target::Character(*id))
                    });
                found.ok_or_else(|| {
                    let why = format!(
                        "character {token} is not a character of {}",
                        self.describe(Target::Country(country))
                    );
                    if self.characters_complete {
                        Nothing(why)
                    } else {
                        Missing(format!("{why} in the recording: {}", gap("Characters")))
                    }
                })
            }
            Scope::Keyword(keyword) => self.keyword(*keyword, context),
            Scope::Chain { head, then } => {
                let head = self.keyword(*head, context)?;
                let context = Context {
                    this: Some(head),
                    prev: context.this,
                    ..*context
                };
                self.resolve(then, &context)
            }
            Scope::Variable(name) => Err(Missing(format!(
                "var:{name}: runtime script variables are not recorded"
            ))),
            Scope::EventTarget { name, .. } => Err(Missing(format!(
                "event_target:{name}: saved targets resolve in the trigger evaluator"
            ))),
            // Generic prefix accepted by the parser, with no matching resolver
            // case: only script variables handle the global_event_target: prefix.
            Scope::Prefixed { prefix, value }
                if prefix.eq_ignore_ascii_case("global_event_target") =>
            {
                Err(Nothing(format!("{prefix}:{value}: invalid context scope")))
            }
            Scope::Prefixed { prefix, value } => Err(Missing(format!(
                "{prefix}:{value}: {prefix} scopes are not recorded"
            ))),
            Scope::ZeroNumber(text) => Err(Nothing(format!(
                "{text}: the event target reader keeps a zero number as a scope that names nothing"
            ))),
            Scope::Unknown(text) => Err(Nothing(format!(
                "{text} is no scope the event target reader accepts"
            ))),
        }
    }

    fn keyword(&self, keyword: Keyword, context: &Context) -> Result<Target, Unresolved> {
        use Unresolved::{Missing, Nothing};
        let name = keyword.name();
        let unset = || Nothing(format!("{name} is not set in this context"));
        let this = || context.this.ok_or_else(unset);
        // A link of 0 or below is no country; a positive one CountryLinks
        // does not hold is a missing input, not an absence.
        let country = |link: i64, what: &str, of: &str| -> Result<Target, Unresolved> {
            let Some(link) = u32::try_from(link).ok().filter(|link| *link > 0) else {
                return Err(Nothing(format!("{name}: {of} has no {what}")));
            };
            self.slot(link).map(Target::Country).ok_or_else(|| {
                Missing(format!(
                    "{name}: {of}'s {what} link {link} has no CountryLinks row: {}",
                    self.gaps.get("CountryLinks").cloned().unwrap_or_default()
                ))
            })
        };
        match keyword {
            Keyword::Root => context.root.ok_or_else(unset),
            Keyword::This => this(),
            Keyword::Prev => context.prev.ok_or_else(unset),
            Keyword::From => context.from.ok_or_else(unset),
            Keyword::Owner | Keyword::Controller => match this()? {
                Target::State(id) => {
                    let (owner, controller) = self.states().get(&id).copied().ok_or_else(|| {
                        Missing(format!(
                            "{name}: state {id} has no recorded States row: {}",
                            self.gaps["States"]
                        ))
                    })?;
                    let link = if keyword == Keyword::Owner {
                        owner
                    } else {
                        controller
                    };
                    country(link.into(), "country in States", &format!("state {id}"))
                }
                other => Err(Missing(format!(
                    "{name} of {}: only a state's owner and controller are recorded (States)",
                    self.describe(other)
                ))),
            },
            Keyword::CapitalScope => match this()? {
                Target::Country(slot) => {
                    let capital = self.capitals().get(&slot).copied().ok_or_else(|| {
                        Missing(format!(
                            "{name}: {} has no recorded Politics row: {}",
                            self.describe(Target::Country(slot)),
                            self.gaps["Politics"]
                        ))
                    })?;
                    u32::try_from(capital)
                        .ok()
                        .filter(|capital| *capital > 0)
                        .map(Target::State)
                        .ok_or_else(|| {
                            Nothing(format!(
                                "{name}: {} has no capital",
                                self.describe(Target::Country(slot))
                            ))
                        })
                }
                other => Err(Missing(format!(
                    "{name} of {}: only a country's capital is recorded (Politics.capital)",
                    self.describe(other)
                ))),
            },
            Keyword::Overlord => match this()? {
                Target::Country(slot) => {
                    let overlord = self.overlords().get(&slot).copied().ok_or_else(|| {
                        Missing(format!(
                            "{name}: {} has no recorded CountryAppearance row: {}",
                            self.describe(Target::Country(slot)),
                            self.gaps["CountryAppearance"]
                        ))
                    })?;
                    country(
                        overlord.into(),
                        "overlord",
                        &self.describe(Target::Country(slot)),
                    )
                }
                other => Err(Missing(format!(
                    "{name} of {}: only a country's overlord is recorded (CountryAppearance.overlord)",
                    self.describe(other)
                ))),
            },
            // This fallback has no country-presence gate. Known
            // clearing state/character/empty scopes ask actual Country0's faction.
            // A directly carried faction remains outside these supported carriers.
            Keyword::FactionLeader => {
                let slot = match context.this {
                    Some(Target::Country(slot)) => slot,
                    Some(Target::State(_) | Target::Character(_)) | None => 0,
                };
                let Some(raw) = self.faction_leader_raw(slot).map_err(Missing)? else {
                    return Err(Nothing(format!(
                        "{name}: the resolved country has no faction"
                    )));
                };
                if raw == 0 {
                    // set_country(0, clear=1) leaves validity, but no carried kinds.
                    Ok(Target::State(crate::triggers::NULL_STATE))
                } else {
                    // Positive raw aliases remain country carriers, even at slot0.
                    self.slot(raw).map(Target::Country).ok_or_else(|| {
                        Missing(format!(
                            "{name}: faction leader link {raw} has no CountryLinks row: {}",
                            self.gaps["CountryLinks"]
                        ))
                    })
                }
            }
            Keyword::Occupied | Keyword::Character | Keyword::Operation | Keyword::Opponent => {
                Err(Missing(format!(
                    "{name}: this scope is set by the running script and is not recorded"
                )))
            }
        }
    }

    /// A target as a message names it.
    pub fn describe(&self, target: Target) -> String {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Scopes(Query::Describe(target)),
            || self.describe_unrecorded(target),
        )
    }

    fn describe_unrecorded(&self, target: Target) -> String {
        match target {
            Target::Country(slot) => match self.tag(slot) {
                Some(tag) => format!("country {tag}"),
                None => format!("country slot {slot}"),
            },
            Target::State(id) => format!("state {id}"),
            Target::Character(id) => format!("character {id}"),
        }
    }
}

impl Names for RecordedScopes {
    /// A recorded country's tag, or a tag alias, which then
    /// resolves to a named refusal ([`Self::alias`]).
    fn is_country(&self, tag: &str) -> bool {
        self.slots().contains_key(tag) || self.alias(tag).is_some()
    }

    /// A live character's token; load-time classification reads the
    /// content's templates instead.
    fn is_character(&self, token: &str) -> bool {
        self.characters().contains_key(&token.to_ascii_lowercase())
    }

    /// Recognized keyword prefixes; any other keyword prefix parses
    /// as well but resolves to the same refusal.
    fn is_keyword(&self, text: &str) -> bool {
        // global_event_target is recognized by the lexer.
        ["mio", "sp", "scope", "global_event_target"]
            .iter()
            .any(|prefix| text.eq_ignore_ascii_case(prefix))
    }
}

#[cfg(test)]
#[path = "scope/faction_zero_tests.rs"]
mod faction_zero_tests;

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use super::*;
    use replay_data::{
        rows::CharacterRow, rows::CountryAppearanceRow, rows::CountryCounterInputRow,
        rows::CountryLinkRow, rows::CountryRow, rows::LexicalTokenRow, rows::PoliticsRow,
        rows::Recorded, rows::StateRow, rows::TableId, rows::TableRows,
    };
    use replay_frame::frame::Metrics;

    /// GER (slot 0, link 100) with capital state 64, SOV (slot 1, link 200)
    /// a subject of GER, state 64 owned by GER and controlled by SOV.
    pub fn frame() -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            tables: std::array::from_fn(|_| TableView::Unavailable {
                reason: "not sampled".into(),
            })
            .map(Into::into),
            metrics: Metrics::default(),
        };
        let mut set = |id: TableId, rows: TableRows| {
            *frame.tables[id.index()] = TableView::Complete(rows);
        };
        set(
            TableId::CountryLinks,
            TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 100,
                    canonical_slot: 0,
                },
                CountryLinkRow {
                    id: 200,
                    canonical_slot: 1,
                },
            ]),
        );
        set(
            TableId::Countries,
            TableRows::Countries(
                [(100, 0, "GER"), (200, 1, "SOV")]
                    .map(|(id, canonical_slot, tag)| CountryRow {
                        id,
                        canonical_slot,
                        tag: tag.into(),
                        ..Default::default()
                    })
                    .into(),
            ),
        );
        set(
            TableId::States,
            TableRows::States(vec![StateRow {
                id: 64,
                owner: 100,
                controller: 200,
                ..Default::default()
            }]),
        );
        set(
            TableId::Politics,
            TableRows::Politics(vec![PoliticsRow {
                country: 100,
                capital: 64,
                ..Default::default()
            }]),
        );
        set(
            TableId::CountryAppearance,
            TableRows::CountryAppearance(vec![
                CountryAppearanceRow {
                    country: 100,
                    original_tag_reference: Recorded::Value(Some(0)),
                    original_tag: Recorded::Value(Some(Vec::new())),
                    ..Default::default()
                },
                CountryAppearanceRow {
                    country: 200,
                    original_tag_reference: Recorded::Value(Some(0)),
                    original_tag: Recorded::Value(Some(Vec::new())),
                    overlord: 100,
                    ..Default::default()
                },
            ]),
        );
        set(
            TableId::CountryCounterInputs,
            TableRows::CountryCounterInputs(vec![
                CountryCounterInputRow {
                    country: 100,
                    owned_state_count: 1,
                    ..Default::default()
                },
                CountryCounterInputRow {
                    country: 200,
                    owned_state_count: 3,
                    ..Default::default()
                },
            ]),
        );
        set(
            TableId::Characters,
            TableRows::Characters(vec![CharacterRow {
                id: 7,
                country: 100,
                name_token_id: 9000,
                token: b"Erwin Rommel".to_vec(),
                ..Default::default()
            }]),
        );
        set(
            TableId::LexicalTokens,
            TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 9000,
                literal: b"GER_erwin_rommel".to_vec(),
            }]),
        );
        frame
    }

    /// The current collector's positive-only country roster. Native slot
    /// zero is a separate ReservedCountryNaming singleton, not GER's link.
    pub fn positive_frame() -> DecodedFrame {
        let mut frame = frame();
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            for row in rows {
                row.canonical_slot += 1;
            }
        }
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *frame.tables[TableId::CountryLinks.index()]
        {
            for row in rows {
                row.canonical_slot += 1;
            }
        }
        frame
    }

    /// Original-list fixtures also know the separately authenticated zero
    /// object's original word is nonpositive. Ordinary fixtures keep it absent.
    #[cfg(test)]
    fn original_frame() -> DecodedFrame {
        let mut frame = positive_frame();
        *frame.tables[TableId::ReservedCountryNaming.index()] =
            TableView::Complete(TableRows::ReservedCountryNaming(vec![
                replay_data::rows::ReservedCountryNamingRow {
                    tag: b"ZZ0".to_vec(),
                    original_tag_reference: Recorded::Value(Some(0)),
                    original_tag: Recorded::Value(Some(Vec::new())),
                    original_tag_buckets: Recorded::Value(Some(vec![vec![], vec![200], vec![]])),
                    country_array: Recorded::Value(Some(vec![0, 100, 200])),
                    ..Default::default()
                },
            ]));
        frame
    }

    /// A tag owning no state stands for the first country of its
    /// original-tag list that owns one.
    #[test]
    fn a_tag_without_states_resolves_as_the_game_substitutes_it() {
        let counts = |ger: i32, sov: i32| {
            let mut frame = original_frame();
            *frame.tables[TableId::CountryCounterInputs.index()] =
                TableView::Complete(TableRows::CountryCounterInputs(vec![
                    CountryCounterInputRow {
                        country: 100,
                        owned_state_count: ger,
                        ..Default::default()
                    },
                    CountryCounterInputRow {
                        country: 200,
                        owned_state_count: sov,
                        ..Default::default()
                    },
                ]));
            if let TableView::Complete(TableRows::CountryAppearance(rows)) =
                &mut *frame.tables[TableId::CountryAppearance.index()]
            {
                // SOV is a dynamic country named after GER.
                rows[1].name_tag = b"GER".to_vec();
                rows[1].original_tag_reference = Recorded::Value(Some(100));
                rows[1].original_tag = Recorded::Value(Some(b"GER".to_vec()));
            }
            RecordedScopes::read(&frame, Arc::default())
        };
        assert_eq!(counts(1, 3).resolve_tag("GER"), Ok(1));
        assert_eq!(counts(0, 3).resolve_tag("GER"), Ok(2));
        // No candidate owns a state: the tag's own country.
        assert_eq!(counts(0, 0).resolve_tag("GER"), Ok(1));
        let mut frame = positive_frame();
        *frame.tables[TableId::CountryCounterInputs.index()] = TableView::Unavailable {
            reason: "not sampled".into(),
        };
        assert!(
            RecordedScopes::read(&frame, Arc::default())
                .resolve_tag("GER")
                .unwrap_err()
                .contains("CountryCounterInputs.owned_state_count")
        );
    }

    /// GER (link 100, slot 1) and three dynamic countries D01..D03 (links
    /// 201..203, slots 2..4), with each country's exact original presence and
    /// every country's owned state count.
    #[cfg(test)]
    pub(crate) fn dynamic_frame(hour: i32, names: [&str; 3], owned: [i32; 4]) -> DecodedFrame {
        let mut frame = original_frame();
        frame.hour = hour;
        let links = [(100, 1), (201, 2), (202, 3), (203, 4)];
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(
                links
                    .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
                    .into(),
            ));
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            [
                (100, 1, "GER"),
                (201, 2, "D01"),
                (202, 3, "D02"),
                (203, 4, "D03"),
            ]
            .map(|(id, canonical_slot, tag)| CountryRow {
                id,
                canonical_slot,
                tag: tag.into(),
                ..Default::default()
            })
            .into(),
        ));
        let mut appearance = vec![CountryAppearanceRow {
            country: 100,
            original_tag_reference: Recorded::Value(Some(0)),
            original_tag: Recorded::Value(Some(Vec::new())),
            ..Default::default()
        }];
        for (index, name) in names.iter().enumerate() {
            appearance.push(CountryAppearanceRow {
                country: 201 + index as u32,
                name_tag: name.as_bytes().to_vec(),
                original_tag_reference: Recorded::Value(Some(if name.is_empty() {
                    0
                } else {
                    100
                })),
                original_tag: Recorded::Value(Some(name.as_bytes().to_vec())),
                ..Default::default()
            });
        }
        *frame.tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(appearance));
        *frame.tables[TableId::CountryCounterInputs.index()] =
            TableView::Complete(TableRows::CountryCounterInputs(
                links
                    .iter()
                    .zip(owned)
                    .map(|((country, _), owned_state_count)| CountryCounterInputRow {
                        country: *country,
                        owned_state_count,
                        ..Default::default()
                    })
                    .collect(),
            ));
        let members = names
            .iter()
            .enumerate()
            .filter(|(_, name)| **name == "GER")
            .map(|(i, _)| 201 + i as u32)
            .collect();
        if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
            &mut *frame.tables[TableId::ReservedCountryNaming.index()]
        {
            rows[0].original_tag_buckets =
                Recorded::Value(Some(vec![vec![], members, vec![], vec![], vec![]]));
            rows[0].country_array = Recorded::Value(Some(vec![0, 100, 201, 202, 203]));
        }
        frame
    }

    #[test]
    fn original_tag_resolution_uses_the_captured_bucket_order() {
        let mut frame = dynamic_frame(10, ["GER", "GER", ""], [0, 1, 1, 0]);
        assert_eq!(
            RecordedScopes::read(&frame, Arc::default()).resolve_tag("GER"),
            Ok(2)
        );
        // An original-tag change can remove/reappend D01 between samples while every
        // final original operand remains identical. The stored order wins.
        if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
            &mut *frame.tables[TableId::ReservedCountryNaming.index()]
        {
            rows[0].original_tag_buckets = Recorded::Value(Some(vec![
                vec![],
                vec![202, 201, 202],
                vec![],
                vec![],
                vec![],
            ]));
        }
        assert_eq!(
            RecordedScopes::read(&frame, Arc::default()).resolve_tag("GER"),
            Ok(3)
        );
        if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
            &mut *frame.tables[TableId::CountryCounterInputs.index()]
        {
            rows[1].owned_state_count = 0;
            rows[2].owned_state_count = 0;
        }
        // The first nonowning member is rejected for its zero state count,
        // retaining the own country.
        assert_eq!(
            RecordedScopes::read(&frame, Arc::default()).resolve_tag("GER"),
            Ok(1)
        );
        if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
            &mut *frame.tables[TableId::ReservedCountryNaming.index()]
        {
            rows[0].original_tag_buckets = Recorded::NotRecorded;
        }
        assert!(
            RecordedScopes::read(&frame, Arc::default())
                .resolve_tag("GER")
                .unwrap_err()
                .contains("original_tag_buckets not recorded")
        );
        if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
            &mut *frame.tables[TableId::CountryCounterInputs.index()]
        {
            rows[0].owned_state_count = 1;
        }
        assert_eq!(
            RecordedScopes::read(&frame, Arc::default()).resolve_tag("GER"),
            Ok(1)
        );
    }

    #[test]
    fn reserved_owned_count_is_lazy_and_controls_native_substitution() {
        let mut frame = dynamic_frame(10, ["GER", "", ""], [0, 1, 0, 0]);
        let counts = [
            Recorded::Value(Some(4)),
            Recorded::Value(Some(0)),
            Recorded::Value(Some(-7)),
            Recorded::Value(None),
            Recorded::NotRecorded,
        ];
        for count in counts {
            if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
                &mut *frame.tables[TableId::ReservedCountryNaming.index()]
            {
                rows[0].original_tag_reference = Recorded::Value(Some(100));
                rows[0].original_tag_buckets =
                    Recorded::Value(Some(vec![vec![], vec![0, 201], vec![], vec![], vec![]]));
                rows[0].owned_state_count = count;
            }
            let scopes = RecordedScopes::read(&frame, Arc::default());
            let result = scopes.resolve_tag("GER");
            match count {
                Recorded::Value(Some(raw)) => {
                    assert_eq!(scopes.owned_states(0), Ok(raw));
                    // Reserved owns states: selection stops and returns zero;
                    // the caller rejects that zero result and keeps GER.
                    assert_eq!(result, Ok(if raw > 0 { 1 } else { 2 }));
                }
                _ => assert!(
                    result
                        .unwrap_err()
                        .contains("ReservedCountryNaming.owned_state_count")
                ),
            }
        }
        // Unknown zero count is irrelevant when the own country already owns.
        if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
            &mut *frame.tables[TableId::CountryCounterInputs.index()]
        {
            rows[0].owned_state_count = 1;
        }
        assert_eq!(
            RecordedScopes::read(&frame, Arc::default()).resolve_tag("GER"),
            Ok(1)
        );
    }

    #[test]
    fn a_target_is_its_scope_kind_for_the_trigger_check() {
        use installed_content_core::scope::TriggerScopes;
        let country = Target::Country(0);
        assert!(TriggerScopes(0x4).accepts(|kind| kind == country.kind()));
        assert!(!TriggerScopes(0x2).accepts(|kind| kind == country.kind()));
        assert!(TriggerScopes(0x2).accepts(|kind| kind == Target::State(64).kind()));
    }

    #[test]
    fn scopes_resolve_through_the_recorded_tables_and_refuse_the_rest() {
        let scopes = RecordedScopes::read(&positive_frame(), Arc::default());
        let sov = Context::country(2);
        let resolve = |text: &str, context: &Context| {
            scopes
                .resolve_text(text, false, context)
                .map_err(|unresolved| unresolved.to_string())
        };
        assert_eq!(resolve("ROOT", &sov), Ok(Target::Country(2)));
        assert_eq!(resolve("GER", &sov), Ok(Target::Country(1)));
        assert_eq!(resolve("64", &sov), Ok(Target::State(64)));
        assert_eq!(resolve("overlord", &sov), Ok(Target::Country(1)));
        assert_eq!(
            resolve("ROOT.overlord.capital_scope", &sov),
            Ok(Target::State(64))
        );
        // A character of the scope country only, found by its
        // token, so without case; another country's names nothing there.
        for token in ["GER_erwin_rommel", "ger_ERWIN_rommel"] {
            assert_eq!(
                resolve(token, &Context::country(1)),
                Ok(Target::Character(7))
            );
        }
        assert!(matches!(
            scopes.resolve_text("GER_erwin_rommel", false, &sov),
            Err(Unresolved::Nothing(why))
                if why == "character GER_erwin_rommel is not a character of country SOV"
        ));
        assert!(
            resolve("64.GER_erwin_rommel", &sov).is_ok_and(|target| target == Target::State(64))
        );
        assert!(resolve("ROOT.capital_scope", &sov).is_err());
        assert_eq!(
            resolve("ROOT.overlord.capital_scope.controller", &sov),
            Ok(Target::Country(2))
        );
        // PREV inside a chain is the scope before the step.
        assert_eq!(resolve("overlord.PREV", &sov), Ok(Target::Country(2)));
        for (text, reason) in [
            ("FROM", "from is not set in this context"),
            ("var:x", "var:x: runtime script variables are not recorded"),
            (
                "event_target:hq",
                "event_target:hq: saved targets resolve in the trigger evaluator",
            ),
            ("mio:GER_tank", "mio:GER_tank: mio scopes are not recorded"),
            (
                "65",
                "state 65 has no recorded States row: the complete table has no row for it",
            ),
            (
                "GER.owner",
                "GER.owner is no scope the event target reader accepts",
            ),
            (
                "owner",
                "owner of country SOV: only a state's owner and controller are recorded (States)",
            ),
            ("overlord.overlord", "overlord: country GER has no overlord"),
        ] {
            let error = resolve(text, &sov).unwrap_err();
            assert!(error.starts_with(reason), "{text}: {error}");
        }
        // A table the hour lacks is named with its state.
        let mut frame = positive_frame();
        *frame.tables[TableId::Politics.index()] = TableView::Unavailable {
            reason: "not sampled".into(),
        };
        let scopes = RecordedScopes::read(&frame, Arc::default());
        assert_eq!(
            scopes.resolve_text("capital_scope", false, &Context::country(1)),
            Err(Unresolved::Missing("capital_scope: country GER has no recorded Politics row: the table is unavailable: not sampled".into()))
        );
    }

    /// The event-target selector takes the faction's first
    /// member; without a faction it clears the target's validity.
    #[test]
    fn faction_leader_scope_uses_the_first_member_and_clears_an_absent_target() {
        use replay_data::rows::FactionMemberRow;

        let mut frame = frame();
        *frame.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(vec![
                FactionMemberRow {
                    country: 200,
                    faction: 5,
                    ordinal: 1,
                },
                FactionMemberRow {
                    country: 100,
                    faction: 5,
                    ordinal: 0,
                },
            ]));
        let scopes = RecordedScopes::read(&frame, Arc::default());
        assert_eq!(scopes.faction_leader(1), Ok(Some(0)));
        assert_eq!(
            scopes.resolve_text("faction_leader", false, &Context::country(1)),
            Ok(Target::Country(0))
        );
        assert_eq!(
            scopes.resolve_text("faction_leader.PREV", false, &Context::country(1)),
            Ok(Target::Country(1))
        );

        *frame.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(Vec::new()));
        let scopes = RecordedScopes::read(&frame, Arc::default());
        assert_eq!(scopes.faction_leader(1), Ok(None));
        assert!(matches!(
            scopes.resolve_text("faction_leader", false, &Context::country(1)),
            Err(Unresolved::Nothing(_))
        ));
    }

    /// A three-byte name that is no country tag but a tag alias parses as
    /// a country scope and is refused by name; while an alias file stops
    /// short, any unknown three-byte name is.
    #[test]
    fn a_tag_alias_scope_is_refused_by_name() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let aliases = installed_content_core::tag_aliases::parse(
            &[("a.txt", b"SPA = { }")],
            &installed_content_core::key_names::KeyNames::default(),
        );
        let scopes = RecordedScopes::read(&positive_frame(), Arc::new(aliases));
        let sov = Context::country(2);
        assert_eq!(
            scopes.resolve_text("SPA", false, &sov),
            Err(Unresolved::Missing(
                "tag alias SPA: its triggers run in the trigger evaluator (crate::triggers), not in a scope read alone"
                    .into()
            ))
        );
        assert_eq!(
            scopes.resolve_text("GER", false, &sov),
            Ok(Target::Country(1))
        );
        let short = TagAliases {
            unknown: vec!["common/country_tag_aliases/a.txt: line 2: x".into()],
            ..Default::default()
        };
        let scopes = RecordedScopes::read(&positive_frame(), Arc::new(short));
        assert!(matches!(
            scopes.resolve_text("XYZ", false, &sov),
            Err(Unresolved::Missing(reason)) if reason.contains("whether it is a tag alias")
        ));
    }

    /// A state's owner or controller link of 0 names no country (the game's
    /// scope is null); a positive link CountryLinks does not hold is a
    /// missing input, refused by name.
    #[test]
    fn an_owner_link_of_zero_is_nothing_and_an_unlinked_one_is_missing() {
        let mut frame = frame();
        *frame.tables[TableId::States.index()] =
            TableView::Complete(TableRows::States(vec![StateRow {
                id: 64,
                owner: 0,
                controller: 300,
                ..Default::default()
            }]));
        let scopes = RecordedScopes::read(&frame, Arc::default());
        let state = Context {
            this: Some(Target::State(64)),
            ..Context::default()
        };
        assert_eq!(
            scopes.resolve_text("owner", false, &state),
            Err(Unresolved::Nothing(
                "owner: state 64 has no country in States".into()
            ))
        );
        assert!(matches!(
            scopes.resolve_text("controller", false, &state),
            Err(Unresolved::Missing(reason))
                if reason.starts_with("controller: state 64's country in States link 300 has no CountryLinks row")
        ));
    }
}
