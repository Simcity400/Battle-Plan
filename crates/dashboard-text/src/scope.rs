use crate::triggers::Failure;

use content_reader_script::scope::{Keyword, Names, Scope, ScopeKind};
use content_reader_key_names::tag_aliases::{TagAlias, TagAliases};
use replay_data::{
    rows::CountryAppearanceRow, rows::CountryCounterInputRow, rows::CountryLinkRow,
    rows::CountryRow, rows::FactionMemberRow, rows::GlobalSavedEventTargetRow, rows::PoliticsRow,
    rows::StateRow, rows::TableSchema, rows::column, rows::mask, rows::tables,
};
use replay_frame::frame::{HourFrame, OperandStatus, Rowset, TableView};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

mod original_tags;
pub use original_tags::OriginalTagLists;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum Target {
    Country(u32),
    State(u32),
    Character(u32),
}

impl Target {
    pub fn kind(self) -> ScopeKind {
        match self {
            Self::Country(_) => ScopeKind::Country,
            Self::State(_) => ScopeKind::State,
            Self::Character(_) => ScopeKind::Character,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unresolved {
    Nothing(String),
    Missing(Failure),
}

impl std::fmt::Display for Unresolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Nothing(what) => f.write_str(what),
            Self::Missing(what) => std::fmt::Display::fmt(what, f),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct Context {
    pub root: Option<Target>,
    pub this: Option<Target>,
    pub prev: Option<Target>,
    pub from: Option<Target>,
}

impl Context {
    pub fn country(slot: u32) -> Self {
        Self {
            root: Some(Target::Country(slot)),
            this: Some(Target::Country(slot)),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Default)]
struct CountryValues {
    values: BTreeMap<u32, i32>,
    unresolved: Option<Failure>,
}

impl CountryValues {
    fn get(&self, slot: u32) -> Result<Option<i32>, Failure> {
        if let Some(value) = self.values.get(&slot) {
            return Ok(Some(*value));
        }
        if let Some(failure) = &self.unresolved {
            failure.reached();
            return Err(failure.clone());
        }
        Ok(None)
    }
}

type CharactersByToken = BTreeMap<String, Vec<(u32, u32)>>;

#[derive(Clone, Debug, Default)]
pub struct RecordedScopes<'a> {
    frame: Option<HourFrame<'a>>,
    links: OnceLock<BTreeMap<u32, u32>>,
    tags: OnceLock<BTreeMap<u32, String>>,
    raw_ids: OnceLock<BTreeMap<u32, u32>>,
    countries_complete: bool,
    slots: OnceLock<BTreeMap<String, u32>>,
    states: OnceLock<BTreeMap<u32, (u32, u32)>>,
    capitals: OnceLock<crate::triggers::refusal::Resolved<CountryValues>>,
    overlords: OnceLock<crate::triggers::refusal::Resolved<CountryValues>>,
    faction_members_complete: bool,
    characters: OnceLock<crate::triggers::refusal::Resolved<CharactersByToken>>,
    characters_complete: bool,
    owned: OnceLock<crate::triggers::refusal::Resolved<CountryValues>>,
    original_tags: OriginalTagLists<'a>,
    aliases: Arc<TagAliases>,
    saved_targets_complete: bool,
    gaps: BTreeMap<&'static str, String>,
}

const LINK: u64 = mask::<CountryLinkRow>(&["canonical_slot"]);
const SLOT: u64 = mask::<CountryRow>(&["canonical_slot"]);
const TAG: usize = column::<CountryRow>("tag");
const STATE: u64 = mask::<StateRow>(&["owner", "controller"]);
const CAPITAL: u64 = mask::<PoliticsRow>(&["capital"]);
const OVERLORD: u64 = mask::<CountryAppearanceRow>(&["overlord"]);
const OWNED: u64 = mask::<CountryCounterInputRow>(&["owned_state_count"]);
const MEMBER: u64 = mask::<FactionMemberRow>(&["faction", "ordinal"]);
const SAVED: u64 = tables::GlobalSavedEventTargets::ID.full_mask();
const CHARACTER: u64 = crate::identity::CHARACTER_IDENTITY
    | mask::<replay_data::rows::CharacterRow>(&["country"]);

fn whole(status: OperandStatus<'_>) -> bool {
    matches!(status, OperandStatus::Complete | OperandStatus::WrongLayout)
}

pub fn status_gap(status: OperandStatus<'_>) -> String {
    match status {
        OperandStatus::WrongLayout => OperandStatus::Complete.gap(),
        OperandStatus::TruncatedWrongLayout(missing) => OperandStatus::Truncated(missing).gap(),
        status => status.gap(),
    }
}

fn held_rows<'a, T: TableSchema>(frame: Option<HourFrame<'a>>, mask: u64) -> Rowset<'a, T::Row> {
    crate::triggers::inputs::unrecorded(|| match frame {
        Some(frame)
            if !matches!(
                frame.status(T::ID),
                OperandStatus::Unavailable(_) | OperandStatus::NotRecorded(_)
            ) =>
        {
            frame.held::<T>(mask)
        }
        _ => {
            crate::triggers::refusal::missing_reference(T::ID, None).resolution_failed();
            Rowset::Decoded(&[])
        }
    })
}

fn walked<T: TableSchema>(why: std::io::Error) -> Failure {
    crate::triggers::refusal::input_error(T::ID, why)
}

pub(crate) fn table_in<'a, T: TableSchema>(
    frame: HourFrame<'a>,
    mask: u64,
) -> (Rowset<'a, T::Row>, String) {
    (frame.held::<T>(mask), status_gap(frame.status(T::ID)))
}

pub fn gap(view: &TableView) -> String {
    match view {
        TableView::Complete(_) => "the complete table has no row for it".into(),
        TableView::Truncated { missing, .. } => {
            format!("the table is truncated ({missing} rows missing)")
        }
        TableView::Unavailable { reason } => format!("the table is unavailable: {reason}"),
        TableView::NotRecorded { mode } => {
            format!("the table is not recorded in a {mode} recording")
        }
    }
}

pub fn missing(table: replay_data::rows::TableId, view: &TableView) -> String {
    let what = table.words();
    match view {
        TableView::Complete(_) => format!("{what}: no row for it"),
        TableView::Truncated { missing, .. } => format!("{what}: read only in part ({missing} rows not read)"),
        TableView::Unavailable { reason } if reason.is_empty() => format!("{what}: not available"),
        TableView::Unavailable { reason } => format!("{what}: not available ({reason})"),
        TableView::NotRecorded { mode } => format!("{what}: not recorded in a {mode} recording"),
    }
}

pub fn unread(table: replay_data::rows::TableId, view: &TableView) -> Option<String> {
    (!matches!(view, TableView::Complete(_))).then(|| missing(table, view))
}
pub fn valid_saved_target(r: &replay_data::rows::GlobalSavedEventTargetRow) -> bool {
    !r.name.is_empty()
        && (r.state != 0
            || r.country > 0
            || r.live & !replay_data::rows::GlobalSavedEventTargetRow::UNSAVED_NAME != 0)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
            &Query::OriginalBucket(selected) => {
                if selected <= 0 {
                    Vec::new()
                } else {
                    vec![
                        link(selected as u32),
                        TableRead::fields(
                            TableId::ReservedCountryNaming,
                            &["original_tag_buckets"],
                        ),
                    ]
                }
            }
            Query::Tag(_) | Query::Country(_) => vec![tags()],
            Query::Alias(_) | Query::AliasEntry(_) => vec![tags()],
            Query::Roster => vec![TableRead::fields(TableId::Countries, &["canonical_slot"])],
            Query::Describe(Target::Country(_)) => vec![tags()],
            Query::Describe(Target::State(_) | Target::Character(_)) => Vec::new(),
            Query::ResolveTag(_) => Self::tag_dependence(),
            Query::Resolve(ScopeKey(Scope::Country(_)), _) => Self::tag_dependence(),
            Query::Resolve(ScopeKey(Scope::Keyword(Keyword::Overlord)), _) => vec![
                TableRead::fields(TableId::CountryAppearance, &["overlord"]),
                TableRead::fields(TableId::CountryLinks, &["canonical_slot"]),
                tags(),
            ],
            Query::Resolve(ScopeKey(Scope::Keyword(Keyword::CapitalScope)), _) => vec![
                TableRead::fields(TableId::Politics, &["capital"]),
                TableRead::fields(TableId::CountryLinks, &["canonical_slot"]),
                tags(),
            ],
            _ => return None,
        })
    }

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

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ScopeKey(Scope);

impl std::hash::Hash for ScopeKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        format!("{:?}", self.0).hash(state);
    }
}

impl RecordedScopes<'_> {
    pub fn reread(&self, query: &Query) -> u64 {
        crate::triggers::inputs::resolved_digest(|| match query {
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
        })
    }
}

impl<'a> RecordedScopes<'a> {
    pub fn read(frame: &'a dyn replay_frame::frame::DecodedSource, aliases: Arc<TagAliases>) -> Self {
        Self::read_in(HourFrame::Decoded(frame), aliases)
    }

    pub fn read_in(frame: HourFrame<'a>, aliases: Arc<TagAliases>) -> Self {
        Self::read_with(frame, OriginalTagLists::read_in(frame), aliases)
    }

    pub fn read_with(
        frame: impl Into<HourFrame<'a>>,
        original_tags: OriginalTagLists<'a>,
        aliases: Arc<TagAliases>,
    ) -> Self {
        let frame = frame.into();
        let complete = |id: replay_data::rows::TableId| whole(frame.status(id));
        let gap = |id: replay_data::rows::TableId| status_gap(frame.status(id));
        let mut gaps = BTreeMap::new();
        gaps.insert("CountryLinks", gap(tables::CountryLinks::ID));
        gaps.insert("Countries", gap(tables::Countries::ID));
        gaps.insert("States", gap(tables::States::ID));
        gaps.insert("Politics", gap(tables::Politics::ID));
        gaps.insert("CountryAppearance", gap(tables::CountryAppearance::ID));
        gaps.insert("FactionMembers", gap(tables::FactionMembers::ID));
        gaps.insert("CountryCounterInputs", gap(tables::CountryCounterInputs::ID));
        gaps.insert(
            "Characters",
            format!(
                "{} (lexical tokens: {})",
                gap(tables::Characters::ID),
                gap(tables::LexicalTokens::ID)
            ),
        );
        gaps.insert(
            "GlobalSavedEventTargets",
            gap(tables::GlobalSavedEventTargets::ID),
        );
        Self {
            frame: Some(frame),
            countries_complete: complete(tables::Countries::ID),
            faction_members_complete: complete(tables::FactionMembers::ID),
            characters_complete: complete(tables::Characters::ID)
                && complete(tables::LexicalTokens::ID),
            saved_targets_complete: complete(tables::GlobalSavedEventTargets::ID),
            original_tags,
            aliases,
            gaps,
            ..Self::default()
        }
    }

    fn complete(&self, table: replay_data::rows::TableId) -> bool {
        self.frame.is_some_and(|frame| whole(frame.status(table)))
    }

    fn input<T: TableSchema>(
        &self,
        field: &str,
        scope: Option<crate::triggers::Store>,
        message: impl Into<String>,
    ) -> Failure {
        let (field, scope) = if self.complete(T::ID) {
            (field, scope)
        } else {
            ("", None)
        };
        Failure::input(T::ID, field, scope, message)
    }

    fn rows<T: TableSchema>(&self, mask: u64) -> Rowset<'a, T::Row> {
        held_rows::<T>(self.frame, mask)
    }

    fn links(&self) -> &BTreeMap<u32, u32> {
        self.links.get_or_init(|| {
            let mut links = BTreeMap::new();
            for link in self.rows::<tables::CountryLinks>(LINK).iter() {
                match link {
                    Ok(link) => {
                        links.insert(link.id, link.canonical_slot);
                    }
                    Err(why) => walked::<tables::CountryLinks>(why).reached(),
                }
            }
            links
        })
    }

    fn tags(&self) -> &BTreeMap<u32, String> {
        self.tags.get_or_init(|| {
            let mut tags = BTreeMap::new();
            for country in self.rows::<tables::Countries>(SLOT).with_text(|row| &row.tag, TAG) {
                match country {
                    Ok((country, tag)) => {
                        tags.insert(
                            country.canonical_slot,
                            String::from_utf8_lossy(tag).into_owned(),
                        );
                    }
                    Err(why) => walked::<tables::Countries>(why).reached(),
                }
            }
            tags
        })
    }

    fn raw_ids(&self) -> &BTreeMap<u32, u32> {
        self.raw_ids.get_or_init(|| {
            let mut ids = BTreeMap::new();
            for row in self.rows::<tables::Countries>(SLOT).iter() {
                match row {
                    Ok(row) => {
                        ids.insert(row.canonical_slot, row.id);
                    }
                    Err(why) => walked::<tables::Countries>(why).reached(),
                }
            }
            ids
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
            let mut states = BTreeMap::new();
            for state in self.rows::<tables::States>(STATE).iter() {
                match state {
                    Ok(state) => {
                        states.insert(state.id, (state.owner, state.controller));
                    }
                    Err(why) => walked::<tables::States>(why).reached(),
                }
            }
            states
        })
    }

    fn country_values<T: TableSchema>(
        &self,
        mask: u64,
        value: impl Fn(&T::Row) -> (u32, i32),
    ) -> crate::triggers::refusal::Resolved<CountryValues> {
        crate::triggers::refusal::Resolved::new(|| {
            crate::triggers::refusal::deferred(|| {
                let mut out = CountryValues::default();
                for row in self.rows::<T>(mask).iter() {
                    let row = match row {
                        Ok(row) => row,
                        Err(why) => {
                            out.unresolved.get_or_insert_with(|| walked::<T>(why));
                            continue;
                        }
                    };
                    let (raw, value) = value(&row);
                    match self.slot(raw) {
                        Some(slot) => {
                            out.values.insert(slot, value);
                        }
                        None => {
                            out.unresolved.get_or_insert_with(|| {
                                self.input::<tables::CountryLinks>(
                                    "canonical_slot",
                                    None,
                                    format!("{} names a country that no recorded country link holds", T::ID.words()),
                                )
                            });
                        }
                    }
                }
                out
            })
        })
    }

    fn capitals(&self) -> &CountryValues {
        self.capitals
            .get_or_init(|| {
                self.country_values::<tables::Politics>(CAPITAL, |row| (row.country, row.capital))
            })
            .value()
    }

    fn overlords(&self) -> &CountryValues {
        self.overlords
            .get_or_init(|| {
                self.country_values::<tables::CountryAppearance>(OVERLORD, |row| {
                    (row.country, row.overlord)
                })
            })
            .value()
    }

    fn owned(&self) -> &CountryValues {
        self.owned
            .get_or_init(|| {
                self.country_values::<tables::CountryCounterInputs>(OWNED, |row| {
                    (row.country, row.owned_state_count)
                })
            })
            .value()
    }

    fn characters(&self) -> &CharactersByToken {
        self.characters
            .get_or_init(|| {
                crate::triggers::refusal::Resolved::new(|| {
                    let lexical = self.rows::<tables::LexicalTokens>(0);
                    let characters =
                        self.rows::<tables::Characters>(CHARACTER);
                    let mut by_token = CharactersByToken::new();
                    for row in characters.iter() {
                        let row = match row {
                            Ok(row) => row,
                            Err(why) => {
                                walked::<tables::Characters>(why).reached();
                                continue;
                            }
                        };
                        if let Some(token) = crate::identity::character_script_token_in(&row, lexical)
                        {
                            by_token
                                .entry(token.to_ascii_lowercase())
                                .or_default()
                                .push((row.id, row.country));
                        }
                    }
                    by_token
                })
            })
            .value()
    }

    fn faction_members(&self) -> impl Iterator<Item = Result<Cow<'a, FactionMemberRow>, Failure>> {
        self.rows::<tables::FactionMembers>(MEMBER)
            .iter()
            .map(|row| row.map_err(walked::<tables::FactionMembers>))
    }

    fn saved_targets(
        &self,
    ) -> impl Iterator<Item = Result<Cow<'a, GlobalSavedEventTargetRow>, Failure>> {
        self.rows::<tables::GlobalSavedEventTargets>(SAVED)
            .iter()
            .map(|row| row.map_err(walked::<tables::GlobalSavedEventTargets>))
    }

    pub fn saved_target(
        &self,
        name: &[u8],
    ) -> Result<Option<Cow<'a, GlobalSavedEventTargetRow>>, Failure> {
        crate::triggers::inputs::query_debug_resolved(
            || crate::triggers::inputs::Key::Scopes(Query::SavedTarget(name.to_vec())),
            || self.saved_target_unrecorded(name),
        )
    }

    fn saved_target_unrecorded(
        &self,
        name: &[u8],
    ) -> Result<Option<Cow<'a, GlobalSavedEventTargetRow>>, Failure> {
        if !self.saved_targets_complete {
            return Err(self.input::<tables::GlobalSavedEventTargets>(
                "",
                None,
                format!(
                    "saved event target {}: global saved event targets: {}",
                    String::from_utf8_lossy(name),
                    self.gaps
                        .get("GlobalSavedEventTargets")
                        .map(String::as_str)
                        .unwrap_or("is not recorded")
                ),
            ));
        }
        for row in self.saved_targets() {
            let row = row?;
            if row.name == name {
                return Ok(Some(row));
            }
        }
        Ok(None)
    }

    pub(crate) fn saved_selection(
        &self,
        name: &[u8],
        reader: crate::triggers::saved_event_targets::Reader,
    ) -> Result<crate::triggers::saved_event_targets::Selected, Failure> {
        crate::triggers::inputs::query_debug_resolved(
            || crate::triggers::inputs::Key::Scopes(Query::SavedSelection(name.to_vec(), reader)),
            || self.saved_selection_unrecorded(name, reader),
        )
    }

    fn saved_selection_unrecorded(
        &self,
        name: &[u8],
        reader: crate::triggers::saved_event_targets::Reader,
    ) -> Result<crate::triggers::saved_event_targets::Selected, Failure> {
        use crate::triggers::saved_event_targets::{Selected, select};
        Ok(self
            .saved_target(name)?
            .map_or(Selected::Missing, |r| select(&r, reader)))
    }

    pub fn saved_country(&self, raw: i32, name: &[u8]) -> Result<Option<u32>, Failure> {
        crate::triggers::inputs::query_resolved(
            || crate::triggers::inputs::Key::Scopes(Query::SavedCountry(raw, name.to_vec())),
            || self.saved_country_unrecorded(raw, name),
        )
    }

    fn saved_country_unrecorded(&self, raw: i32, name: &[u8]) -> Result<Option<u32>, Failure> {
        if raw <= 0 {
            return Ok(None);
        }
        self.slot(raw as u32).map(Some).ok_or_else(|| -> Failure {
            format!(
                "saved event target {}: country reference {raw} is not in country links",
                String::from_utf8_lossy(name)
            )
            .into()
        })
    }

    pub fn resolve_tag(&self, tag: &str) -> Result<u32, Failure> {
        crate::triggers::inputs::query_resolved(
            || crate::triggers::inputs::Key::Scopes(Query::ResolveTag(tag.to_owned())),
            || self.resolve_tag_unrecorded(tag),
        )
    }

    fn resolve_tag_unrecorded(&self, tag: &str) -> Result<u32, Failure> {
        if let Some(refusal) = self.alias(tag) {
            return Err(refusal.into());
        }
        let slot = self.country(tag).ok_or_else(|| -> Failure {
            format!("country {tag} has no recorded Countries row").into()
        })?;
        if self.owned_states(slot)? >= 1 {
            return Ok(slot);
        }
        let raw = *self.raw_ids().get(&slot).ok_or_else(|| -> Failure {
            self.input::<tables::Countries>(
                "id",
                Some(crate::triggers::Store::Country(slot)),
                format!("country slot {slot} has no own raw tag"),
            )
        })?;
        let canonical = self.raw_slot(raw)?;
        for &member in self.original_tags.bucket(canonical)? {
            let member = self.raw_slot(member)?;
            if self.owned_states(member)? >= 1 {
                let raw = if member == 0 {
                    0
                } else {
                    *self.raw_ids().get(&member).ok_or_else(|| -> Failure {
                        self.input::<tables::Countries>(
                            "id",
                            Some(crate::triggers::Store::Country(member)),
                            format!("country slot {member} has no own raw tag"),
                        )
                    })? as i32
                };
                return Ok(if raw > 0 { member } else { slot });
            }
        }
        Ok(slot)
    }

    pub fn raw_slot(&self, raw: u32) -> Result<u32, Failure> {
        crate::triggers::inputs::query_resolved(
            || crate::triggers::inputs::Key::Scopes(Query::RawSlot(raw)),
            || self.raw_slot_unrecorded(raw),
        )
    }

    fn raw_slot_unrecorded(&self, raw: u32) -> Result<u32, Failure> {
        if raw == 0 {
            return Ok(0);
        }
        self.slot(raw).ok_or_else(|| -> Failure {
            self.input::<tables::CountryLinks>(
                "canonical_slot",
                None,
                format!("country raw reference {raw} is not in country links"),
            )
        })
    }

    pub(crate) fn positive_country(&self, raw: u32) -> Result<Option<u32>, Failure> {
        crate::triggers::inputs::query_resolved(
            || crate::triggers::inputs::Key::Scopes(Query::PositiveCountry(raw)),
            || self.positive_country_unrecorded(raw),
        )
    }

    fn positive_country_unrecorded(&self, raw: u32) -> Result<Option<u32>, Failure> {
        let slot = self.raw_slot(raw)?;
        let own = if slot == 0 {
            0
        } else {
            *self.raw_ids().get(&slot).ok_or_else(|| -> Failure {
                self.input::<tables::Countries>(
                    "id",
                    Some(crate::triggers::Store::Country(slot)),
                    format!("country slot {slot} has no Countries row for its own raw tag"),
                )
            })?
        };
        Ok(((own as i32) > 0).then_some(slot))
    }

    pub fn original_bucket(&self, selected: i32) -> Result<&[u32], Failure> {
        crate::triggers::inputs::query_resolved(
            || crate::triggers::inputs::Key::Scopes(Query::OriginalBucket(selected)),
            || self.original_bucket_unrecorded(selected),
        )
    }

    fn original_bucket_unrecorded(&self, selected: i32) -> Result<&[u32], Failure> {
        if selected <= 0 {
            return Ok(&[]);
        }
        self.original_tags.bucket(self.raw_slot(selected as u32)?)
    }

    pub fn alias(&self, tag: &str) -> Option<String> {
        crate::triggers::inputs::query_resolved(
            || crate::triggers::inputs::Key::Scopes(Query::Alias(tag.to_owned())),
            || self.alias_unrecorded(tag),
        )
    }

    fn alias_unrecorded(&self, tag: &str) -> Option<String> {
        if self.country(tag).is_some() {
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

    pub fn alias_entry(&self, tag: &str) -> Option<&TagAlias> {
        crate::triggers::inputs::query_debug_resolved(
            || crate::triggers::inputs::Key::Scopes(Query::AliasEntry(tag.to_owned())),
            || self.alias_entry_unrecorded(tag),
        )
    }

    fn alias_entry_unrecorded(&self, tag: &str) -> Option<&TagAlias> {
        if self.country(tag).is_some() {
            return None;
        }
        self.aliases.get(tag)
    }

    pub fn roster(&self) -> Result<impl Iterator<Item = u32> + '_, Failure> {
        let roster = crate::triggers::inputs::query_resolved(
            || crate::triggers::inputs::Key::Scopes(Query::Roster),
            || self.roster_unrecorded().map(Iterator::collect::<Vec<u32>>),
        );
        roster.map(Vec::into_iter)
    }

    fn roster_unrecorded(&self) -> Result<impl Iterator<Item = u32> + '_, Failure> {
        if !self.countries_complete {
            return Err(self.input::<tables::Countries>(
                "",
                None,
                format!(
                    "the country roster is not complete: {}",
                    self.gaps.get("Countries").cloned().unwrap_or_default()
                ),
            ));
        }
        Ok(self.tags().keys().copied())
    }

    pub fn owned_states(&self, slot: u32) -> Result<i32, Failure> {
        crate::triggers::inputs::query_resolved(
            || crate::triggers::inputs::Key::Scopes(Query::OwnedStates(slot)),
            || self.owned_states_unrecorded(slot),
        )
    }

    fn owned_states_unrecorded(&self, slot: u32) -> Result<i32, Failure> {
        if slot == 0 {
            return self.original_tags.reserved_owned_states();
        }
        if let Some(count) = self.owned().get(slot)? {
            return Ok(count);
        }
        if let Some(raw) = self.raw_ids().get(&slot) {
            self.raw_slot(*raw)?;
        }
        Err(self.input::<tables::CountryCounterInputs>("owned_state_count", Some(crate::triggers::Store::Country(slot)), format!(
            "the owned state count of {} is not recorded: {}",
            self.describe(Target::Country(slot)), self.gaps.get("CountryCounterInputs").cloned().unwrap_or_default()
        )))
    }

    pub fn original_tags(&self) -> &OriginalTagLists<'a> {
        crate::triggers::inputs::query_debug_resolved(
            || crate::triggers::inputs::Key::Scopes(Query::OriginalTags),
            || self.original_tags_unrecorded(),
        )
    }

    fn original_tags_unrecorded(&self) -> &OriginalTagLists<'a> {
        &self.original_tags
    }

    pub fn tag(&self, slot: u32) -> Option<&str> {
        crate::triggers::inputs::query_resolved(
            || crate::triggers::inputs::Key::Scopes(Query::Tag(slot)),
            || self.tag_unrecorded(slot),
        )
    }

    fn tag_unrecorded(&self, slot: u32) -> Option<&str> {
        crate::triggers::refusal::reference(
            self.tags().get(&slot).map(String::as_str),
            slot != 0,
            || {
                self.input::<tables::Countries>(
                    "",
                    Some(crate::triggers::Store::Country(slot)),
                    "country tag is not recorded",
                )
            },
        )
    }

    pub fn country(&self, tag: &str) -> Option<u32> {
        crate::triggers::inputs::query_resolved(
            || crate::triggers::inputs::Key::Scopes(Query::Country(tag.to_owned())),
            || self.country_unrecorded(tag),
        )
    }

    fn country_unrecorded(&self, tag: &str) -> Option<u32> {
        let found = self.slots().get(tag).copied();
        crate::triggers::refusal::membership(found.is_some(), self.countries_complete, || {
            self.input::<tables::Countries>("", None, "country-tag membership is not complete")
        });
        found
    }

    pub fn slot(&self, link: u32) -> Option<u32> {
        crate::triggers::inputs::query_resolved(
            || crate::triggers::inputs::Key::Scopes(Query::Slot(link)),
            || self.slot_unrecorded(link),
        )
    }

    fn slot_unrecorded(&self, link: u32) -> Option<u32> {
        let slot = self.links().get(&link).copied();
        if slot.is_none() && link != 0 {
            self.input::<tables::CountryLinks>(
                "canonical_slot",
                None,
                format!("country raw reference {link} has no country links row"),
            )
            .resolution_failed();
        }
        slot
    }

    pub(crate) fn faction(&self, slot: u32) -> Result<Option<u64>, Failure> {
        crate::triggers::inputs::query_resolved(
            || crate::triggers::inputs::Key::Scopes(Query::Faction(slot)),
            || self.faction_unrecorded(slot),
        )
    }

    fn faction_unrecorded(&self, slot: u32) -> Result<Option<u64>, Failure> {
        if slot == 0 {
            return self.original_tags.reserved_faction();
        }
        for member in self.faction_members() {
            let member = member?;
            if member.country != 0 && self.slot(member.country) == Some(slot) {
                return Ok(Some(member.faction));
            }
        }
        if !self.faction_members_complete {
            return Err(self.input::<tables::FactionMembers>(
                "",
                None,
                format!(
                    "the faction of country slot {slot} is unknown: faction members: {}",
                    self.gaps["FactionMembers"]
                ),
            ));
        }
        for row in self.faction_members() {
            let row = row?;
            if row.country != 0 && self.slot(row.country).is_none() {
                return Err(self.input::<tables::CountryLinks>("canonical_slot", None, format!(
                    "the faction of country slot {slot} is unknown: a faction member's country link {} resolves to no country: country links: {}",
                    row.country, self.gaps["CountryLinks"]
                )));
            }
        }
        Ok(None)
    }

    pub(crate) fn faction_first_raw(&self, faction: u64) -> Result<u32, Failure> {
        crate::triggers::inputs::query_resolved(
            || crate::triggers::inputs::Key::Scopes(Query::FactionFirstRaw(faction)),
            || self.faction_first_raw_unrecorded(faction),
        )
    }

    fn faction_first_raw_unrecorded(&self, faction: u64) -> Result<u32, Failure> {
        let mut first = None;
        for row in self.faction_members() {
            let row = row?;
            if row.faction == faction && row.ordinal == 0 {
                first = Some(row.country);
                break;
            }
        }
        first.ok_or_else(|| -> Failure {
                self.input::<tables::FactionMembers>(
                    "",
                    None,
                    format!(
                        "faction {faction} has no first member: faction members: {}",
                        self.gaps["FactionMembers"]
                    ),
                )
            })
    }

    pub(crate) fn faction_leader_raw(&self, slot: u32) -> Result<Option<u32>, Failure> {
        crate::triggers::inputs::query_resolved(
            || crate::triggers::inputs::Key::Scopes(Query::FactionLeaderRaw(slot)),
            || self.faction_leader_raw_unrecorded(slot),
        )
    }

    fn faction_leader_raw_unrecorded(&self, slot: u32) -> Result<Option<u32>, Failure> {
        self.faction(slot)?
            .map(|faction| self.faction_first_raw(faction))
            .transpose()
    }

    pub fn faction_leader(&self, slot: u32) -> Result<Option<u32>, Failure> {
        crate::triggers::inputs::query_resolved(
            || crate::triggers::inputs::Key::Scopes(Query::FactionLeader(slot)),
            || self.faction_leader_unrecorded(slot),
        )
    }

    fn faction_leader_unrecorded(&self, slot: u32) -> Result<Option<u32>, Failure> {
        let Some(raw) = self.faction_leader_raw(slot)? else {
            return Ok(None);
        };
        self.slot(raw).map(Some).ok_or_else(|| -> Failure {
            format!(
                "the faction leader link {raw} resolves to no country: country links: {}",
                self.gaps["CountryLinks"]
            )
            .into()
        })
    }

    #[cfg(test)]
    pub fn resolve_text(
        &self,
        text: &str,
        quoted: bool,
        context: &Context,
    ) -> Result<Target, Unresolved> {
        crate::triggers::inputs::query_debug_resolved(
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
        self.resolve(&content_reader_script::scope::parse(text, quoted, self), context)
    }

    pub fn resolve(&self, scope: &Scope, context: &Context) -> Result<Target, Unresolved> {
        crate::triggers::inputs::query_debug_resolved(
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
                    Err(Missing(self.input::<tables::States>(
                        "",
                        Some(crate::triggers::Store::State(id)),
                        format!("state {id} has no recorded States row: {}", gap("States")),
                    )))
                }
            }
            Scope::Country(tag) => self.resolve_tag(tag).map(Target::Country).map_err(Missing),
            Scope::Character(token) => {
                let Some(Target::Country(country)) = context.this else {
                    return Err(Nothing(format!(
                        "character {token}: the scope has no country, which the game requires"
                    )));
                };
                if self.frame.is_some()
                    && !self.complete(tables::LexicalTokens::ID)
                {
                    return Err(Missing(self.input::<tables::LexicalTokens>(
                        "",
                        None,
                        "character lookup lacks its lexical tokens input",
                    )));
                }
                let found = self
                    .characters()
                    .get(&token.to_ascii_lowercase())
                    .and_then(|found| {
                        found
                            .iter()
                            .find(|(_, owner)| self.slot(*owner) == Some(country))
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
                        Missing(self.input::<tables::Characters>(
                            "",
                            Some(crate::triggers::Store::Country(country)),
                            format!("{why} in the recording: {}", gap("Characters")),
                        ))
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
            Scope::Variable(name) => Err(Missing(
                format!("var:{name}: runtime script variables are not recorded").into(),
            )),
            Scope::EventTarget { name, .. } => Err(Missing(
                format!("event_target:{name}: saved targets resolve in the trigger evaluator")
                    .into(),
            )),
            Scope::Prefixed { prefix, value }
                if prefix.eq_ignore_ascii_case("global_event_target") =>
            {
                Err(Nothing(format!("{prefix}:{value}: invalid context scope")))
            }
            Scope::Prefixed { prefix, value } => Err(Missing(
                format!("{prefix}:{value}: {prefix} scopes are not recorded").into(),
            )),
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
        let country = |link: i64, what: &str, of: &str| -> Result<Target, Unresolved> {
            let Some(link) = u32::try_from(link).ok().filter(|link| *link > 0) else {
                return Err(Nothing(format!("{name}: {of} has no {what}")));
            };
            self.slot(link).map(Target::Country).ok_or_else(|| {
                Missing(self.input::<tables::CountryLinks>(
                    "canonical_slot",
                    None,
                    format!(
                        "{name}: {of}'s {what} has no recorded country link: {}",
                        self.gaps.get("CountryLinks").cloned().unwrap_or_default()
                    ),
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
                        Missing(self.input::<tables::States>("", Some(crate::triggers::Store::State(id)), format!(
                            "{name}: state {id} has no recorded States row: {}",
                            self.gaps["States"]
                        )))
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
                ).into())),
            },
            Keyword::CapitalScope => match this()? {
                Target::Country(slot) => {
                    let capital = self.capitals().get(slot).map_err(Missing)?.ok_or_else(|| {
                        Missing(self.input::<tables::Politics>("capital", Some(crate::triggers::Store::Country(slot)), format!(
                            "{name}: {} has no recorded Politics row: {}",
                            self.describe(Target::Country(slot)),
                            self.gaps["Politics"]
                        )))
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
                    "{name} of {}: only a country's capital is recorded (capital of politics)",
                    self.describe(other)
                ).into())),
            },
            Keyword::Overlord => match this()? {
                Target::Country(slot) => {
                    let overlord = self.overlords().get(slot).map_err(Missing)?.ok_or_else(|| {
                        Missing(self.input::<tables::CountryAppearance>("overlord", Some(crate::triggers::Store::Country(slot)), format!(
                            "{name}: {} has no recorded country appearance row: {}",
                            self.describe(Target::Country(slot)),
                            self.gaps["CountryAppearance"]
                        )))
                    })?;
                    country(
                        overlord.into(),
                        "overlord",
                        &self.describe(Target::Country(slot)),
                    )
                }
                other => Err(Missing(format!(
                    "{name} of {}: only a country's overlord is recorded (overlord of country appearance)",
                    self.describe(other)
                ).into())),
            },
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
                    Ok(Target::State(crate::triggers::NULL_STATE))
                } else {
                    self.slot(raw).map(Target::Country).ok_or_else(|| {
                        Missing(self.input::<tables::CountryLinks>("canonical_slot", None, format!(
                            "{name}: the faction leader has no recorded country link: {}",
                            self.gaps["CountryLinks"]
                        )))
                    })
                }
            }
            Keyword::Occupied | Keyword::Character | Keyword::Operation | Keyword::Opponent => {
                Err(Missing(format!(
                    "{name}: this scope is set by the running script and is not recorded"
                ).into()))
            }
        }
    }

    pub fn describe(&self, target: Target) -> String {
        crate::triggers::inputs::query_resolved(
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

impl Names for RecordedScopes<'_> {
    fn is_country(&self, tag: &str) -> bool {
        self.country(tag).is_some() || self.alias(tag).is_some()
    }

    fn is_character(&self, token: &str) -> bool {
        let found = self.characters().contains_key(&token.to_ascii_lowercase());
        for (complete, table) in [
            (self.complete(tables::Characters::ID), tables::Characters::ID),
            (
                self.complete(tables::LexicalTokens::ID),
                tables::LexicalTokens::ID,
            ),
        ] {
            crate::triggers::refusal::membership(found, complete, || {
                crate::triggers::refusal::missing_reference(table, None)
            });
        }
        found
    }

    fn is_keyword(&self, text: &str) -> bool {
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
    use replay_frame::frame::DecodedFrame;
    use replay_data::{
        rows::CharacterRow, rows::CountryAppearanceRow, rows::CountryCounterInputRow,
        rows::CountryLinkRow, rows::CountryRow, rows::LexicalTokenRow, rows::PoliticsRow,
        rows::Recorded, rows::StateRow, rows::TableId, rows::TableRows,
    };
    use replay_frame::frame::Metrics;

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
                rows[1].name_tag = b"GER".to_vec();
                rows[1].original_tag_reference = Recorded::Value(Some(100));
                rows[1].original_tag = Recorded::Value(Some(b"GER".to_vec()));
            }
            frame
        };
        let resolve = |frame: DecodedFrame| {
            RecordedScopes::read(&frame, Arc::default()).resolve_tag("GER")
        };
        assert_eq!(resolve(counts(1, 3)), Ok(1));
        assert_eq!(resolve(counts(0, 3)), Ok(2));
        assert_eq!(resolve(counts(0, 0)), Ok(1));
        let mut frame = positive_frame();
        *frame.tables[TableId::CountryCounterInputs.index()] = TableView::Unavailable {
            reason: "not sampled".into(),
        };
        let why = RecordedScopes::read(&frame, Arc::default())
            .resolve_tag("GER")
            .unwrap_err();
        assert!(
            why.names("CountryCounterInputs")
                && why.starts_with("the owned state count of country GER is not recorded: "),
            "{why}"
        );
    }

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
        assert_eq!(
            RecordedScopes::read(&frame, Arc::default()).resolve_tag("GER"),
            Ok(1)
        );
        if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
            &mut *frame.tables[TableId::ReservedCountryNaming.index()]
        {
            rows[0].original_tag_buckets = Recorded::NotRecorded;
        }
        let why = RecordedScopes::read(&frame, Arc::default())
            .resolve_tag("GER")
            .unwrap_err();
        assert!(
            why.names("ReservedCountryNaming.original_tag_buckets")
                && why == "original tag buckets of reserved country naming not recorded",
            "{why}"
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
                    assert_eq!(result, Ok(if raw > 0 { 1 } else { 2 }));
                }
                _ => assert!(
                    result
                        .unwrap_err()
                        .contains("owned state count of reserved country naming")
                ),
            }
        }
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
    fn scopes_resolve_through_the_recorded_tables_and_refuse_the_rest() {
        let frame = positive_frame();
        let scopes = RecordedScopes::read(&frame, Arc::default());
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
        let mut frame = positive_frame();
        *frame.tables[TableId::Politics.index()] = TableView::Unavailable {
            reason: "not sampled".into(),
        };
        let scopes = RecordedScopes::read(&frame, Arc::default());
        assert_eq!(
            scopes.resolve_text("capital_scope", false, &Context::country(1)),
            Err(Unresolved::Missing(Failure::input(
                replay_data::rows::TableId::Politics,
                "",
                None,
                "capital_scope: country GER has no recorded Politics row: the table is unavailable: not sampled"
            )))
        );
    }

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

    #[test]
    fn a_tag_alias_scope_is_refused_by_name() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
        let aliases = content_reader_key_names::tag_aliases::parse(
            &[("a.txt", b"SPA = { }")],
            &content_reader_script::key_names::KeyNames::default(),
        );
        let frame = positive_frame();
        let scopes = RecordedScopes::read(&frame, Arc::new(aliases));
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
        let scopes = RecordedScopes::read(&frame, Arc::new(short));
        assert!(matches!(
            scopes.resolve_text("XYZ", false, &sov),
            Err(Unresolved::Missing(reason)) if reason.contains("whether it is a tag alias")
        ));
    }

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
                if reason.starts_with("controller: state 64's country in States has no recorded country link")
        ));
    }
}
