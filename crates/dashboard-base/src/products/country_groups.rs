//! Who belonged to each smart country group, hour by hour, over one branch:
//! human players, every recorded faction (including ones founded or dissolved
//! partway through) and the countries at war with each country.
//!
//! Membership is kept as runs of consecutive recorded hours; a gap or an hour
//! whose tables are not complete ends every run, so nothing is carried across
//! missing data. Those hours are listed as unobserved for each group history
//! (up to the branch's own first and last hour), so a break there is not
//! mistaken for a country joining or leaving. An hour whose list names a
//! country the links cannot resolve is unobserved for that history too, since
//! the list is incomplete, though the runs of the countries it names go on.
//! A war counts only while its
//! recorded relation is ongoing. Faction names use the literal bytes when
//! nonempty, else the recorded hour's template token through the content
//! stack and its bound localisation. Name runs end across gaps independently
//! of roster completeness; an unresolved name keeps its reason.
//!
//! A faction's leader is its member at order zero. The recorded member ordinal
//! preserves the roster's order. An hour whose order-zero member the links
//! cannot name has no known leader, and that faction's roster is incomplete then.

use super::{LoadContext, MAX_PRODUCT_REPLY_BYTES, Product, country_history::CountryIdentityRun};
use crate::{
    Source, catalogue, content, faction_names,
    metadata::{
        IdentityTrack, LinkTags, TaggedCountry, branch_bounds, check_cancel, link_slots,
        tagged_countries_indexed, validate_campaign,
    },
    transport::json_len,
};
use installed_content::{faction_templates::Catalog, localisation::Localisation};
use replay::{container::Replay, frame::FrameDecoder, rows::tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

pub mod changes;
pub mod prepared;

pub struct CountryGroups;

/// Branches kept per source, so tracks on different branches do not evict each other.
#[cfg(any(test, feature = "test-support"))]
pub const CACHED_BRANCHES: usize = 4;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CountryGroupParams {
    pub branch_id: String,
}

/// A country's unbroken membership of a group.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct MembershipRun {
    pub tag: String,
    pub first_hour: i32,
    pub last_hour: i32,
}

/// A faction's display name at consecutive recorded hours.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionNameRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub name: Option<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionGroup {
    /// The recorded faction id, as a decimal string.
    pub id: String,
    /// The faction's name as the game showed it; null when it cannot be resolved.
    pub name: Option<String>,
    /// Why the latest recorded name cannot be resolved.
    pub name_reason: Option<String>,
    /// Display names dated by the Factions row that supplied their operands.
    pub names: Vec<FactionNameRun>,
    /// The latest engine literal, copied unchanged; empty when there is none.
    /// Stable across languages and content, so settings can key a faction by it.
    pub recorded_name: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub members: Vec<MembershipRun>,
    /// Who led the faction (its member at order zero), as runs of consecutive
    /// recorded hours; a change of leader ends one run and starts another.
    pub leaders: Vec<MembershipRun>,
    /// Hours this faction's roster named a country the links cannot resolve: its
    /// roster is not known then (other factions' rosters are).
    pub incomplete: Vec<HourSpan>,
}

/// Resolved text with borrowed recorded histories until the complete reply fits.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FactionGroupView<'a> {
    id: String,
    name: Option<String>,
    name_reason: Option<String>,
    names: Vec<FactionNameRun>,
    recorded_name: &'a str,
    first_hour: i32,
    last_hour: i32,
    members: &'a RunHistory<MembershipRun>,
    leaders: &'a RunHistory<MembershipRun>,
    incomplete: &'a [HourSpan],
}

impl FactionGroupView<'_> {
    fn into_owned(self) -> FactionGroup {
        FactionGroup {
            id: self.id,
            name: self.name,
            name_reason: self.name_reason,
            names: self.names,
            recorded_name: self.recorded_name.to_owned(),
            first_hour: self.first_hour,
            last_hour: self.last_hour,
            members: self.members.iter().cloned().collect(),
            leaders: self.leaders.iter().cloned().collect(),
            incomplete: self.incomplete.to_vec(),
        }
    }
}

/// Two countries at war with each other over consecutive recorded hours.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct WarPairRun {
    pub first: String,
    pub second: String,
    pub first_hour: i32,
    pub last_hour: i32,
}

/// Consecutive hours of the branch whose group tables were not observed.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct HourSpan {
    pub first_hour: i32,
    pub last_hour: i32,
}

/// Hours of the branch whose memberships were not observed, per group history.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct UnobservedHours {
    pub humans: Vec<HourSpan>,
    pub factions: Vec<HourSpan>,
    pub wars: Vec<HourSpan>,
}

/// The start of a war between two countries as the engine dates it (the war
/// relation's start hour). `attacking` is the instigating side's country
/// (`first_was_instigator`); it may be an ally that joined rather than the
/// country that declared, which the relation does not record.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
pub struct WarStart {
    pub hour: i32,
    pub attacking: String,
    pub defending: String,
}

/// A capitulation as the engine dates it (Capitulation's capitulated hour).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
pub struct Capitulation {
    pub hour: i32,
    pub tag: String,
}

/// A recorded country id naming another tag from one recorded hour to the next
/// (a tag switch, or an id the game gives another country): runs kept by tag end
/// and start there without the country joining or leaving anything.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
pub struct IdentityChange {
    pub hour: i32,
    pub from: String,
    pub to: String,
}

/// Native holders of one literal tag, independent of group membership.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountryIdentityHistory {
    pub tag: String,
    pub identities: Vec<CountryIdentityRun>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountryGroupReply {
    pub branch_id: String,
    pub humans: Vec<MembershipRun>,
    pub factions: Vec<FactionGroup>,
    pub wars: Vec<WarPairRun>,
    /// Every war start the recorded war relations name, in hour order.
    pub war_starts: Vec<WarStart>,
    /// Every capitulation the recording saw, in hour order.
    pub capitulations: Vec<Capitulation>,
    /// Every change of an id's tag between two consecutive recorded hours, in hour order.
    pub identity_changes: Vec<IdentityChange>,
    /// Every recorded tag's native identities, including all reused holders.
    pub country_identities: Vec<CountryIdentityHistory>,
    /// Hours inside the branch where each history was not observed: a run that
    /// ends or starts next to one of these is a gap, not a join or a leave.
    pub unobserved: UnobservedHours,
    pub warnings: Vec<String>,
}

impl Product for CountryGroups {
    const KEY: &'static str = "country-groups";
    const CONTENT_DEPENDENT: bool = true;
    type Params = CountryGroupParams;
    type Reply = CountryGroupReply;

    fn types() -> Vec<String> {
        vec![
            CountryGroupParams::decl(),
            MembershipRun::decl(),
            FactionNameRun::decl(),
            FactionGroup::decl(),
            WarPairRun::decl(),
            WarStart::decl(),
            Capitulation::decl(),
            IdentityChange::decl(),
            CountryIdentityHistory::decl(),
            CountryIdentityRun::decl(),
            HourSpan::decl(),
            UnobservedHours::decl(),
            CountryGroupReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map(|_| ())
            .map_err(|_| "Invalid country group branch")
    }

    #[cfg(any(test, feature = "test-support"))]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().expect("validated branch");
        let scan = cached_scan(source, branch, cancelled)?;
        scan.reply(source, &params.branch_id, context)
    }
}

impl Scan {
    pub fn reply(
        &self,
        source: &Source,
        branch_id: &str,
        context: &LoadContext,
    ) -> io::Result<CountryGroupReply> {
        if self.continuation.deferred {
            let mut state = self.clone();
            state.seal();
            return state.reply(source, branch_id, context);
        }
        let mut warnings = Vec::new();
        let needs_templates = self
            .factions
            .values()
            .any(|faction| faction.names.iter().any(NameOperands::needs_templates));
        let factions = with_name_content(
            source,
            context,
            needs_templates,
            &mut warnings,
            |templates, localisation, recorded_language, refused| {
                self.faction_groups(templates, localisation, recorded_language, refused)
            },
        );
        warnings.extend(self.warnings.iter().cloned());
        // Count the complete borrowed history before copying its metadata and runs.
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Reply<'a> {
            branch_id: &'a str,
            humans: &'a RunHistory<MembershipRun>,
            factions: &'a [FactionGroupView<'a>],
            wars: &'a RunHistory<WarPairRun>,
            war_starts: &'a BTreeSet<WarStart>,
            capitulations: &'a BTreeSet<Capitulation>,
            identity_changes: &'a RunHistory<IdentityChange>,
            country_identities: IdentityCatalogue<'a>,
            unobserved: &'a UnobservedHours,
            warnings: &'a [String],
        }
        if json_len(
            &Reply {
                branch_id,
                humans: &self.humans,
                factions: &factions,
                wars: &self.wars,
                war_starts: &self.war_starts,
                capitulations: &self.capitulations,
                identity_changes: &self.identity_changes,
                country_identities: IdentityCatalogue(&self.identities),
                unobserved: &self.unobserved,
                warnings: &warnings,
            },
            MAX_PRODUCT_REPLY_BYTES,
        )
        .is_none()
        {
            return Err(reply_budget_error());
        }
        let reply = CountryGroupReply {
            branch_id: branch_id.to_owned(),
            humans: self.humans.iter().cloned().collect(),
            factions: factions
                .into_iter()
                .map(FactionGroupView::into_owned)
                .collect(),
            wars: self.wars.iter().cloned().collect(),
            war_starts: self.war_starts.iter().cloned().collect(),
            capitulations: self.capitulations.iter().cloned().collect(),
            identity_changes: self.identity_changes.iter().cloned().collect(),
            country_identities: self
                .identities
                .iter()
                .map(|(tag, identities)| CountryIdentityHistory {
                    tag: tag.clone(),
                    identities: identities.iter().cloned().collect(),
                })
                .collect(),
            unobserved: (*self.unobserved).clone(),
            warnings,
        };
        if json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_none() {
            return Err(reply_budget_error());
        }
        Ok(reply)
    }
}

fn reply_budget_error() -> io::Error {
    io::Error::other("The complete country group reply is too large; nothing was omitted")
}

struct IdentityCatalogue<'a>(&'a BTreeMap<String, RunHistory<CountryIdentityRun>>);

impl Serialize for IdentityCatalogue<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct History<'a> {
            tag: &'a str,
            identities: &'a RunHistory<CountryIdentityRun>,
        }
        serializer.collect_seq(
            self.0
                .iter()
                .map(|(tag, identities)| History { tag, identities }),
        )
    }
}

pub struct FactionNaming {
    templates: Result<Arc<Catalog>, String>,
    localisation: Arc<Localisation>,
    recorded_language: String,
}

#[cfg(any(test, feature = "test-support"))]
pub fn load_faction_naming(
    source: &Source,
    scan: &Scan,
    context: &LoadContext,
    warnings: &mut Vec<String>,
) -> FactionNaming {
    let needs_templates = scan
        .factions
        .values()
        .any(|faction| faction.names.iter().any(NameOperands::needs_templates));
    faction_naming(source, context, needs_templates, warnings)
}

fn faction_naming(
    source: &Source,
    context: &LoadContext,
    needs_templates: bool,
    warnings: &mut Vec<String>,
) -> FactionNaming {
    let language = &source.replay_ref().identity.language;
    let recorded_language = language.strip_prefix("l_").unwrap_or(language);
    // Stored literals need only the selected language, not game assets.
    let viewer_language = context
        .content
        .as_ref()
        .and_then(|options| content::language(&options.options).ok())
        .unwrap_or_else(|| recorded_language.to_owned());
    let naming = context
        .content
        .as_ref()
        .filter(|_| needs_templates)
        .map(|options| {
            content::check_replay(source.replay_ref(), options.allow_unverified, warnings)?;
            let layers = source.content(&options.options)?;
            let localisation = content::localisation_with_key(
                source,
                options,
                (
                    content::content_key(Some(options)).expect("content options given"),
                    options.options.cache_dir.clone(),
                    viewer_language.clone(),
                ),
            )?;
            Ok::<_, io::Error>((layers.layers.faction_templates(), localisation))
        });
    let mut stored_language = Localisation::default();
    stored_language.language = viewer_language;
    let (templates, localisation) = match naming {
        Some(Ok((templates, localisation))) => {
            if let Ok(templates) = &templates {
                warnings.extend(templates.warnings.iter().cloned());
            }
            (templates, localisation)
        }
        Some(Err(error)) => (
            Err(format!("Faction naming content is unavailable: {error}")),
            Arc::new(stored_language),
        ),
        None => (
            Err("Faction naming content is unavailable until the game folders are chosen".into()),
            Arc::new(stored_language),
        ),
    };
    FactionNaming {
        templates,
        localisation,
        recorded_language: recorded_language.to_owned(),
    }
}

/// Literal and template names use one content/language rule across faction products.
pub fn with_name_content<T>(
    source: &Source,
    context: &LoadContext,
    needs_templates: bool,
    warnings: &mut Vec<String>,
    resolve: impl FnOnce(Result<&Catalog, String>, &Localisation, &str, &mut catalogue::Refusals) -> T,
) -> T {
    let naming = faction_naming(source, context, needs_templates, warnings);
    let mut refused = catalogue::Refusals::default();
    let result = resolve(
        naming.templates.as_deref().map_err(Clone::clone),
        &naming.localisation,
        &naming.recorded_language,
        &mut refused,
    );
    warnings.extend(refused.lines("faction names are"));
    result
}

/// Shared native operands at the event's naming hour, including a dissolved faction.
#[cfg(any(test, feature = "test-support"))]
pub fn faction_name_at(
    scan: &Scan,
    id: u64,
    hour: i32,
    naming: &FactionNaming,
    refused: &mut catalogue::Refusals,
) -> Result<String, String> {
    scan.resolved_faction_name_at(
        id,
        hour,
        naming.templates.as_deref().map_err(Clone::clone),
        &naming.localisation,
        &naming.recorded_language,
    )
    .inspect_err(|reason| refused.note(reason, format!("faction {id}")))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NameOperands {
    pub first_hour: i32,
    pub last_hour: i32,
    pub(crate) literal: Vec<u8>,
    template_token: Option<u32>,
    lexical_read: i32,
    #[serde(skip)]
    tokens: Arc<faction_names::LexicalNames>,
    dictionary: usize,
    template: Result<Option<String>, String>,
}

impl NameOperands {
    pub fn needs_templates(&self) -> bool {
        self.literal.is_empty() && self.template_token.is_some()
    }

    fn resolve(
        &self,
        templates: Result<&Catalog, String>,
        localisation: &Localisation,
        recorded_language: &str,
    ) -> Result<String, String> {
        faction_names::name(
            &self.literal,
            self.template
                .as_ref()
                .map(|value| value.as_deref())
                .map_err(Clone::clone),
            templates,
            localisation,
            recorded_language,
            |name| self.tokens.token_of(name),
        )
        .and_then(|text| {
            let plain = catalogue::plain_text(&text)?;
            // Faction naming can return known empty text; nonempty rich text
            // still needs a plain representation.
            if !text.is_empty() && plain.is_empty() {
                Err("Faction name has no plain display text".into())
            } else {
                Ok(plain)
            }
        })
    }
}

/// Recording-sized chunks share closed history; only a touched run's block is copied.
#[derive(Clone)]
pub struct RunHistory<T> {
    segments: BTreeMap<usize, Arc<Vec<T>>>,
    len: usize,
}
impl<T> Default for RunHistory<T> {
    fn default() -> Self {
        Self {
            segments: BTreeMap::new(),
            len: 0,
        }
    }
}
impl<T: Serialize> Serialize for RunHistory<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter())
    }
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for RunHistory<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let rows = Vec::<T>::deserialize(deserializer)?;
        Ok(Self {
            len: rows.len(),
            segments: [(0, Arc::new(rows))].into(),
        })
    }
}
mod arc_pairs {
    use super::*;
    pub fn serialize<S: serde::Serializer, K: Serialize, V: Serialize>(
        map: &Arc<BTreeMap<K, V>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        crate::prepared_production::map_pairs::serialize(&**map, serializer)
    }
    pub fn deserialize<
        'de,
        D: serde::Deserializer<'de>,
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
    >(
        deserializer: D,
    ) -> Result<Arc<BTreeMap<K, V>>, D::Error> {
        crate::prepared_production::map_pairs::deserialize(deserializer).map(Arc::new)
    }
}
impl<T> RunHistory<T> {
    fn begin_block(&mut self) {
        self.segments
            .entry(self.len)
            .or_insert_with(|| Arc::new(Vec::new()));
    }
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> {
        self.segments.values().flat_map(|rows| rows.iter())
    }
    fn len(&self) -> usize {
        self.len
    }
}
impl<T: Clone> RunHistory<T> {
    fn push(&mut self, value: T) {
        if self.segments.is_empty() {
            self.begin_block();
        }
        Arc::make_mut(
            self.segments
                .last_entry()
                .expect("history segment")
                .get_mut(),
        )
        .push(value);
        self.len += 1;
    }
    fn last_mut(&mut self) -> Option<&mut T> {
        if self.len == 0 {
            return None;
        }
        let index = self.len - 1;
        Some(&mut self[index])
    }
}
impl<T> std::ops::Index<usize> for RunHistory<T> {
    type Output = T;
    fn index(&self, index: usize) -> &T {
        let (first, rows) = self
            .segments
            .range(..=index)
            .next_back()
            .expect("history index");
        &rows[index - *first]
    }
}
impl<T: Clone> std::ops::IndexMut<usize> for RunHistory<T> {
    fn index_mut(&mut self, index: usize) -> &mut T {
        let first = *self
            .segments
            .range(..=index)
            .next_back()
            .expect("history index")
            .0;
        &mut Arc::make_mut(self.segments.get_mut(&first).expect("history segment"))[index - first]
    }
}
#[cfg(any(test, feature = "test-support"))]
impl<T, const N: usize> From<[T; N]> for RunHistory<T> {
    fn from(rows: [T; N]) -> Self {
        Self {
            segments: [(0, Arc::new(Vec::from(rows)))].into(),
            len: N,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ScannedFaction {
    pub name: String,
    /// Names are dated too: a later rename cannot relabel an earlier air cell.
    #[serde(skip)]
    pub names: RunHistory<NameOperands>,
    pub first_hour: i32,
    pub(super) last_hour: i32,
    pub members: RunHistory<MembershipRun>,
    pub leaders: RunHistory<MembershipRun>,
    pub incomplete: Arc<Vec<HourSpan>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub name: Option<String>,
    pub members: Vec<String>,
    pub complete: bool,
}

#[derive(Default)]
pub struct FactionHistory {
    pub factions: BTreeMap<String, Vec<RosterRun>>,
    pub names: BTreeMap<u64, Vec<NameOperands>>,
    pub country_reads: Vec<HourSpan>,
    pub country_presence: HashMap<String, Vec<HourSpan>>,
    pub faction_reads: Vec<HourSpan>,
    pub unobserved: Vec<HourSpan>,
    pub last_hour: i32,
    pub end_recorded: bool,
    pub incomplete: usize,
}

/// Raw sum evidence shares the scan's closed blocks, without caching any measure.
#[derive(Clone, Default, Serialize, Deserialize)]
struct RosterHistory {
    factions: BTreeMap<String, RunHistory<RosterRun>>,
    country_reads: RunHistory<HourSpan>,
    #[serde(serialize_with = "crate::prepared_production::map_pairs::ordered_map")]
    country_presence: HashMap<String, RunHistory<HourSpan>>,
    faction_reads: RunHistory<HourSpan>,
    incomplete: usize,
    #[serde(skip)]
    views: Option<[replay::frame::ViewIdentity; 2]>,
    #[serde(skip)]
    current: Vec<(String, bool)>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    rebuilds: usize,
}

impl RosterHistory {
    fn begin_block(&mut self) {
        self.country_reads.begin_block();
        self.faction_reads.begin_block();
        for history in self.country_presence.values_mut() {
            history.begin_block();
        }
        for history in self.factions.values_mut() {
            history.begin_block();
        }
    }

    #[cfg(test)]
    pub(crate) fn observe(
        &mut self,
        decoded: &replay::frame::DecodedFrame,
        hour: i32,
        countries_known: bool,
        tags: &HashMap<u32, String>,
        identities_changed: bool,
    ) {
        let mut operands = GroupOperands::default();
        operands.refresh(decoded, true, true, true, true, true);
        self.observe_projected(
            decoded,
            &operands,
            true,
            hour,
            countries_known,
            decoded
                .complete::<tables::Countries>()
                .ok()
                .map(crate::metadata::CountryKeys::read)
                .as_ref(),
            tags,
            identities_changed,
        );
    }
    #[allow(clippy::too_many_arguments)]
    fn observe_projected(
        &mut self,
        decoded: &replay::frame::DecodedFrame,
        operands: &GroupOperands,
        refresh_countries: bool,
        hour: i32,
        countries_known: bool,
        countries: Option<&crate::metadata::CountryKeys>,
        tags: &HashMap<u32, String>,
        identities_changed: bool,
    ) {
        if refresh_countries && countries_known {
            self.country_reads.record_hour(hour);
            for (_, _, tag) in countries.expect("known country operands").iter() {
                if let Ok(tag) = std::str::from_utf8(tag)
                    && !tag.is_empty()
                {
                    self.country_presence
                        .entry(tag.to_owned())
                        .or_default()
                        .record_hour(hour);
                }
            }
        }
        let Some(rows) = &operands.factions else {
            self.incomplete += 1;
            return;
        };
        self.faction_reads.record_hour(hour);
        let views = [
            replay::rows::TableId::Factions,
            replay::rows::TableId::FactionMembers,
        ]
        .map(|id| &decoded.tables[id.index()]);
        if !identities_changed
            && self
                .views
                .as_ref()
                .is_some_and(|old| old.iter().zip(views).all(|(old, view)| old.is(view)))
        {
            for (id, complete) in &self.current {
                self.incomplete += usize::from(!complete);
                let history = self.factions.get_mut(id).expect("current roster");
                if history
                    .iter()
                    .next_back()
                    .is_some_and(|run| run.last_hour.checked_add(1) == Some(hour))
                {
                    history.last_mut().expect("roster run").last_hour = hour;
                } else {
                    let mut run = history
                        .iter()
                        .next_back()
                        .expect("current roster run")
                        .clone();
                    run.first_hour = hour;
                    run.last_hour = hour;
                    history.push(run);
                }
            }
            return;
        }
        self.views = Some(views.map(|view| view.identity()));
        self.current.clear();
        #[cfg(any(test, feature = "test-support"))]
        {
            self.rebuilds += 1;
        }
        let members = operands.members.as_deref();
        let mut rosters: BTreeMap<u64, (Option<String>, BTreeSet<String>, bool)> = BTreeMap::new();
        for (id, literal, _) in rows {
            let name = std::str::from_utf8(literal).ok().map(str::to_owned);
            if rosters
                .insert(*id, (name, BTreeSet::new(), members.is_some()))
                .is_some()
            {
                rosters.get_mut(id).expect("inserted roster").2 = false;
            }
        }
        let mut membership = HashMap::new();
        for member in members.unwrap_or_default() {
            if !rosters.contains_key(&member.0) {
                continue;
            }
            if let Some(tag) = countries_known.then(|| tags.get(&member.2)).flatten() {
                rosters
                    .get_mut(&member.0)
                    .expect("recorded roster")
                    .1
                    .insert(tag.clone());
                if let Some(previous) = membership.insert(tag, member.0)
                    && previous != member.0
                {
                    rosters.get_mut(&previous).expect("previous roster").2 = false;
                    rosters.get_mut(&member.0).expect("recorded roster").2 = false;
                }
            } else {
                rosters.get_mut(&member.0).expect("recorded roster").2 = false;
            }
        }
        for (id, (name, members, complete)) in rosters {
            self.incomplete += usize::from(!complete);
            let members: Vec<_> = members.into_iter().collect();
            let id = id.to_string();
            self.current.push((id.clone(), complete));
            let history = self.factions.entry(id).or_default();
            if history.iter().next_back().is_some_and(|previous| {
                previous.last_hour.checked_add(1) == Some(hour)
                    && previous.name == name
                    && previous.members == members
                    && previous.complete == complete
            }) {
                history.last_mut().expect("roster run").last_hour = hour;
            } else {
                history.push(RosterRun {
                    first_hour: hour,
                    last_hour: hour,
                    name,
                    members,
                    complete,
                });
            }
        }
    }
}

impl RunHistory<HourSpan> {
    fn record_hour(&mut self, hour: i32) {
        if self
            .iter()
            .next_back()
            .is_some_and(|span| i64::from(span.last_hour) + 1 >= i64::from(hour))
        {
            let span = self.last_mut().expect("read span");
            span.last_hour = span.last_hour.max(hour);
        } else {
            self.push(HourSpan {
                first_hour: hour,
                last_hour: hour,
            });
        }
    }
}

/// One branch's group history, before names are localised.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Scan {
    #[serde(skip)]
    recent_starts: Vec<WarStart>,
    #[serde(skip)]
    members_changed: bool,
    #[serde(skip)]
    wars_changed: bool,
    #[serde(skip)]
    recent_capitulations: Vec<Capitulation>,
    humans: RunHistory<MembershipRun>,
    pub factions: BTreeMap<u64, ScannedFaction>,
    pub wars: RunHistory<WarPairRun>,
    pub war_starts: Arc<BTreeSet<WarStart>>,
    pub capitulations: Arc<BTreeSet<Capitulation>>,
    pub(super) identity_changes: RunHistory<IdentityChange>,
    #[serde(with = "arc_pairs")]
    pub(super) war_start_reads: Arc<BTreeMap<WarStart, i32>>,
    #[serde(with = "arc_pairs")]
    pub(super) capitulation_reads: Arc<BTreeMap<Capitulation, i32>>,
    pub(super) capitulations_observed: bool,
    pub unattributed_war_starts: usize,
    pub unattributed_capitulations: usize,
    pub capitulation_unread_hours: usize,
    pub identities: BTreeMap<String, RunHistory<super::country_history::CountryIdentityRun>>,
    pub unobserved: Arc<UnobservedHours>,
    pub warnings: Vec<String>,
    #[serde(skip)]
    history: Arc<std::sync::Mutex<Option<Arc<FactionHistory>>>>,
    continuation: Continuation,
    #[serde(with = "faction_names::dictionary_history")]
    dictionary_changes: Vec<Arc<faction_names::LexicalNames>>,
}

/// The scan state an hour's unchanged test reads besides the hour's own
/// change facts: the held country keys and war operands (as digests), the
/// last observed hour and the next war expiry. Two scans holding equal inputs
/// find the same hours of one block unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct QuietInputs {
    countries: Option<u64>,
    wars: Option<u64>,
    previous_hour: Option<i32>,
    next_expiry: Option<i32>,
}

/// `value` with `rate` added once per hour, as that many additions give it.
fn repeated_u32(value: u32, rate: u32, hours: i64) -> u32 {
    u32::try_from(hours)
        .ok()
        .and_then(|hours| rate.checked_mul(hours))
        .and_then(|added| value.checked_add(added))
        .unwrap_or_else(|| (0..hours).fold(value, |value, _| value + rate))
}

/// `value` with `rate` added once per hour, as that many additions give it.
fn repeated(value: usize, rate: usize, hours: i64) -> usize {
    usize::try_from(hours)
        .ok()
        .and_then(|hours| rate.checked_mul(hours))
        .and_then(|added| value.checked_add(added))
        .unwrap_or_else(|| (0..hours).fold(value, |value, _| value + rate))
}

#[derive(Clone, Copy)]
struct Updates {
    holders: bool,
    names: bool,
    dictionary: bool,
    dictionary_templates: bool,
    humans: bool,
    factions: bool,
    wars: bool,
    capitulations: bool,
}
impl Updates {
    const ALL: Self = Self {
        holders: true,
        names: true,
        dictionary: false,
        dictionary_templates: false,
        humans: true,
        factions: true,
        wars: true,
        capitulations: true,
    };
}
fn projected_countries(keys: &crate::metadata::CountryKeys) -> Vec<replay::rows::CountryRow> {
    keys.iter()
        .map(|(id, canonical_slot, tag)| replay::rows::CountryRow {
            id,
            canonical_slot,
            tag: tag.to_vec(),
            ..Default::default()
        })
        .collect()
}
/// Runtime current naming and membership operands, without recorded rows/history.
#[allow(clippy::type_complexity)]
#[derive(Clone, Default)]
struct GroupOperands {
    factions: Option<Vec<(u64, Vec<u8>, Option<u32>)>>,
    templated: Vec<(u64, Vec<u8>, Option<u32>)>,
    members: Option<Vec<(u64, u32, u32)>>,
    humans: Option<Vec<(u32, bool)>>,
    wars: Option<Vec<(u32, u32, i32, i32, u8)>>,
    capitulations: Option<Vec<(u32, bool, i32)>>,
}
impl GroupOperands {
    fn refresh(
        &mut self,
        frame: &replay::frame::DecodedFrame,
        factions: bool,
        members: bool,
        humans: bool,
        wars: bool,
        capitulations: bool,
    ) -> usize {
        let mut visits = 0;
        if factions {
            self.factions = frame.complete::<tables::Factions>().ok().map(|rows| {
                visits += rows.len();
                rows.iter()
                    .map(|row| (row.id, row.name.clone(), row.template_token))
                    .collect()
            });
            self.templated = self
                .factions
                .as_ref()
                .into_iter()
                .flatten()
                .filter(|(_, literal, token)| literal.is_empty() && token.is_some())
                .cloned()
                .collect();
        }
        if members {
            self.members = frame.complete::<tables::FactionMembers>().ok().map(|rows| {
                visits += rows.len();
                rows.iter()
                    .map(|row| (row.faction, row.ordinal, row.country))
                    .collect()
            });
        }
        if humans {
            self.humans = frame
                .complete::<tables::HumanControllers>()
                .ok()
                .map(|rows| {
                    visits += rows.len();
                    rows.iter()
                        .map(|row| (row.country, row.count > 0))
                        .collect()
                });
        }
        if wars {
            self.wars = Self::war_operands(frame);
            visits += self.wars.as_ref().map_or(0, Vec::len);
        }
        if capitulations {
            self.capitulations = frame.complete::<tables::Capitulation>().ok().map(|rows| {
                visits += rows.len();
                rows.iter()
                    .map(|row| (row.country, row.capitulated != 0, row.capitulated_hour))
                    .collect()
            });
        }
        visits
    }
    #[allow(clippy::type_complexity)]
    fn war_operands(frame: &replay::frame::DecodedFrame) -> Option<Vec<(u32, u32, i32, i32, u8)>> {
        Some(
            frame
                .complete::<tables::WarRelations>()
                .ok()?
                .iter()
                .map(|row| {
                    (
                        row.first_country,
                        row.second_country,
                        row.start_hour,
                        row.end_hour,
                        row.first_was_instigator,
                    )
                })
                .collect(),
        )
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Continuation {
    /// Merged membership changes and proof spans, shared with faction sums.
    rosters: RosterHistory,
    blocks: usize,
    bounds: Option<(i32, i32)>,
    /// Coverage before finalising the source's unread trailing hours. A later
    /// append recomputes that tail without inheriting stale missing coverage.
    unobserved: Arc<UnobservedHours>,
    #[serde(with = "crate::prepared_production::map_pairs")]
    human_open: BTreeMap<String, usize>,
    #[serde(with = "crate::prepared_production::map_pairs")]
    member_open: BTreeMap<(u64, String), usize>,
    #[serde(with = "crate::prepared_production::map_pairs")]
    leader_open: BTreeMap<(u64, String), usize>,
    named_starts: BTreeSet<(u32, u32, i32)>,
    named_capitulations: BTreeSet<(u32, i32)>,
    #[serde(serialize_with = "crate::prepared_production::map_pairs::ordered_map")]
    named_since: HashMap<u32, (String, i32)>,
    continuous_since: Option<i32>,
    unattributed: usize,
    capitulations_unread: usize,
    #[serde(with = "crate::prepared_production::map_pairs")]
    war_open: BTreeMap<(String, String), usize>,
    unmapped: BTreeSet<u32>,
    incomplete: u32,
    unreadable_blocks: u32,
    stopped: u32,
    previous_hour: Option<i32>,
    tags: LinkTags,
    #[serde(serialize_with = "crate::prepared_production::map_pairs::ordered_map")]
    faction_tags: HashMap<u32, String>,
    faction_ids: Vec<u32>,
    invalid_names: BTreeSet<u64>,
    names: BTreeMap<u64, RunHistory<NameOperands>>,
    /// The latest dictionary read, named without holding its rows, and the
    /// first read of equal rows, which name operands share.
    #[serde(skip)]
    lexical: Option<Arc<faction_names::LexicalNames>>,
    lexical_dictionary: Option<usize>,
    #[serde(skip)]
    lexical_seen: Option<replay::frame::ViewIdentity>,
    lexical_read: i32,
    deferred: bool,
    rates: (u32, usize, usize),
    unknown: [bool; 3],
    next_expiry: Option<i32>,
    #[serde(skip)]
    country_keys: Option<crate::metadata::CountryKeys>,
    #[serde(skip)]
    link_inputs: Option<HashMap<u32, u32>>,
    #[serde(skip)]
    live_factions: BTreeSet<u64>,
    #[serde(skip)]
    live_countries: BTreeSet<String>,
    #[serde(skip)]
    countries_known: bool,
    #[serde(skip)]
    factions_known: bool,
    #[allow(clippy::type_complexity)]
    #[serde(skip)]
    war_inputs: Option<Vec<(u32, u32, i32, i32, u8)>>,
    #[serde(skip)]
    operands: Option<GroupOperands>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    pub work: crate::prepared::observer_tests::Work,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
pub struct GroupCache {
    pub entries: Vec<(u64, Arc<GroupBranch>)>,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
pub struct GroupBranch {
    gate: std::sync::Mutex<()>,
    pub scan: std::sync::Mutex<Option<Arc<Scan>>>,
}

#[cfg(any(test, feature = "test-support"))]
impl Clone for GroupCache {
    fn clone(&self) -> Self {
        Self {
            entries: self
                .entries
                .iter()
                .map(|(branch, entry)| {
                    let scan = entry
                        .scan
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .clone();
                    (
                        *branch,
                        Arc::new(GroupBranch {
                            gate: std::sync::Mutex::new(()),
                            scan: std::sync::Mutex::new(scan),
                        }),
                    )
                })
                .collect(),
        }
    }
}

/// Requests for one branch share its scan; other branches and source opens
/// never wait on the registry lock while a scan runs.
#[cfg(any(test, feature = "test-support"))]
pub fn cached_scan(source: &Source, branch: u64, cancelled: &AtomicBool) -> io::Result<Arc<Scan>> {
    check_cancel(cancelled)?;
    let state = source.append_state::<GroupCache>();
    let entry = {
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        let entry = if let Some(index) = state
            .entries
            .iter()
            .position(|(cached, _)| *cached == branch)
        {
            state.entries.remove(index)
        } else {
            (branch, Arc::new(GroupBranch::default()))
        };
        let result = Arc::clone(&entry.1);
        state.entries.insert(0, entry);
        state.entries.truncate(CACHED_BRANCHES); // Retention only; evicted branches can be rebuilt.
        result
    };
    let _scan = entry.gate.lock().unwrap_or_else(|error| error.into_inner());
    let cached = entry
        .scan
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    if let Some(scan) = cached.as_ref() {
        let blocks = source
            .replay_ref()
            .index
            .iter()
            .filter(|block| block.branch == branch)
            .count();
        let bounds = source
            .replay_ref()
            .branches
            .iter()
            .find(|start| start.id == branch)
            .map(|start| branch_bounds(source.replay_ref(), start));
        if scan.continuation.blocks == blocks && scan.continuation.bounds == bounds {
            return Ok(Arc::clone(scan));
        }
        let scan = Arc::new(scan_from(
            source.replay_ref(),
            branch,
            cancelled,
            Some(scan),
        )?);
        *entry.scan.lock().unwrap_or_else(|error| error.into_inner()) = Some(Arc::clone(&scan));
        return Ok(scan);
    }
    let scan = Arc::new(scan_branch(source.replay_ref(), branch, cancelled)?);
    *entry.scan.lock().unwrap_or_else(|error| error.into_inner()) = Some(Arc::clone(&scan));
    Ok(scan)
}

/// Records one observed hour without splitting duplicate or consecutive reads.
pub fn record_hour(spans: &mut Vec<HourSpan>, hour: i32) -> bool {
    if let Some(span) = spans.last_mut()
        && i64::from(span.last_hour) + 1 >= i64::from(hour)
    {
        span.last_hour = span.last_hour.max(hour);
        false
    } else {
        spans.push(HourSpan {
            first_hour: hour,
            last_hour: hour,
        });
        true
    }
}

pub fn name_runs<'a>(
    id: u64,
    operands: impl IntoIterator<Item = &'a NameOperands>,
    templates: Result<&Catalog, String>,
    localisation: &Localisation,
    recorded_language: &str,
    refused: &mut catalogue::Refusals,
) -> Vec<FactionNameRun> {
    let mut names: Vec<FactionNameRun> = Vec::new();
    for operands in operands {
        let resolved = operands.resolve(
            templates.as_ref().copied().map_err(Clone::clone),
            localisation,
            recorded_language,
        );
        let (name, reason) = match resolved {
            Ok(name) => (Some(name), None),
            Err(reason) => (None, Some(reason)),
        };
        if let Some(reason) = &reason {
            refused.note(reason, format!("faction {id}"));
        }
        if let Some(previous) = names.last_mut()
            && previous.last_hour.checked_add(1) == Some(operands.first_hour)
            && previous.name == name
            && previous.reason == reason
        {
            previous.last_hour = operands.last_hour;
        } else {
            names.push(FactionNameRun {
                first_hour: operands.first_hour,
                last_hour: operands.last_hour,
                name,
                reason,
            });
        }
    }
    names
}

impl Scan {
    /// Sum requests share one raw projection per immutable scan. Appends build
    /// another scan with an empty projection, including bounds-only changes.
    pub fn faction_history(&self, cancelled: &AtomicBool) -> io::Result<Arc<FactionHistory>> {
        if self.continuation.deferred {
            let mut state = self.clone();
            state.seal();
            return state.faction_history(cancelled);
        }
        check_cancel(cancelled)?;
        let mut cached = self
            .history
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        check_cancel(cancelled)?;
        if let Some(history) = cached.as_ref() {
            return Ok(Arc::clone(history));
        }
        let history = Arc::new(self.project_faction_history(cancelled)?);
        check_cancel(cancelled)?;
        *cached = Some(Arc::clone(&history));
        Ok(history)
    }

    fn project_faction_history(&self, cancelled: &AtomicBool) -> io::Result<FactionHistory> {
        fn copy<T: Clone>(history: &RunHistory<T>, cancelled: &AtomicBool) -> io::Result<Vec<T>> {
            history
                .iter()
                .map(|row| {
                    check_cancel(cancelled)?;
                    Ok(row.clone())
                })
                .collect()
        }
        check_cancel(cancelled)?;
        let (first_hour, last_hour) = self
            .continuation
            .bounds
            .ok_or_else(|| crate::invalid("Country group branch is not in this recording"))?;
        let mut history = FactionHistory {
            country_reads: copy(&self.continuation.rosters.country_reads, cancelled)?,
            faction_reads: copy(&self.continuation.rosters.faction_reads, cancelled)?,
            last_hour,
            incomplete: self.continuation.rosters.incomplete
                + self.continuation.unreadable_blocks as usize
                + self.continuation.stopped as usize,
            ..FactionHistory::default()
        };
        for (id, runs) in &self.continuation.rosters.factions {
            history.factions.insert(id.clone(), copy(runs, cancelled)?);
        }
        for (tag, runs) in &self.continuation.rosters.country_presence {
            history
                .country_presence
                .insert(tag.clone(), copy(runs, cancelled)?);
        }
        for (id, runs) in &self.continuation.names {
            history.names.insert(*id, copy(runs, cancelled)?);
        }
        let mut cursor = i64::from(first_hour);
        for read in &history.faction_reads {
            check_cancel(cancelled)?;
            if cursor < i64::from(read.first_hour) {
                mark_unobserved(&mut history.unobserved, cursor as i32, read.first_hour - 1);
            }
            cursor = i64::from(read.last_hour) + 1;
        }
        if cursor <= i64::from(last_hour) {
            mark_unobserved(&mut history.unobserved, cursor as i32, last_hour);
        }
        history.end_recorded = history
            .faction_reads
            .last()
            .is_some_and(|read| read.last_hour == last_hour);
        check_cancel(cancelled)?;
        Ok(history)
    }

    fn faction_groups(
        &self,
        templates: Result<&Catalog, String>,
        localisation: &Localisation,
        recorded_language: &str,
        refused: &mut catalogue::Refusals,
    ) -> Vec<FactionGroupView<'_>> {
        self.factions
            .iter()
            .map(|(id, faction)| {
                let names = name_runs(
                    *id,
                    faction.names.iter(),
                    templates.as_ref().copied().map_err(Clone::clone),
                    localisation,
                    recorded_language,
                    refused,
                );
                let latest = names.last().expect("a scanned faction has a name read");
                FactionGroupView {
                    id: id.to_string(),
                    name: latest.name.clone(),
                    name_reason: latest.reason.clone(),
                    names,
                    recorded_name: &faction.name,
                    first_hour: faction.first_hour,
                    last_hour: faction.last_hour,
                    members: &faction.members,
                    leaders: &faction.leaders,
                    incomplete: &faction.incomplete,
                }
            })
            .collect()
    }
}

/// A run of consecutive hours.
pub trait HourRun {
    fn last_hour(&self) -> i32;
    fn set_last_hour(&mut self, hour: i32);
}
impl HourRun for MembershipRun {
    fn last_hour(&self) -> i32 {
        self.last_hour
    }
    fn set_last_hour(&mut self, hour: i32) {
        self.last_hour = hour;
    }
}
impl HourRun for WarPairRun {
    fn last_hour(&self) -> i32 {
        self.last_hour
    }
    fn set_last_hour(&mut self, hour: i32) {
        self.last_hour = hour;
    }
}

/// Extends the open run for `key` when `hour` follows its last hour, else starts
/// one with `start`. A second row for the same key in the same hour changes
/// nothing. Returns whether a run was started.
pub fn extend<K: Ord, R: HourRun>(
    open: &mut BTreeMap<K, usize>,
    runs: &mut Vec<R>,
    key: K,
    hour: i32,
    start: impl FnOnce() -> R,
) -> bool {
    if let Some(&index) = open.get(&key) {
        let last = runs[index].last_hour();
        if last == hour {
            return false;
        }
        if last.checked_add(1) == Some(hour) {
            runs[index].set_last_hour(hour);
            return false;
        }
    }
    open.insert(key, runs.len());
    runs.push(start());
    true
}

fn extend_history<K: Ord, R: HourRun + Clone>(
    open: &mut BTreeMap<K, usize>,
    runs: &mut RunHistory<R>,
    key: K,
    hour: i32,
    start: impl FnOnce() -> R,
) {
    if let Some(&index) = open.get(&key) {
        let last = runs[index].last_hour();
        if last == hour {
            return;
        }
        if last.checked_add(1) == Some(hour) {
            runs[index].set_last_hour(hour);
            return;
        }
    }
    open.insert(key, runs.len());
    runs.push(start());
}

/// Whether a recorded war relation is ongoing at `hour`. The engine writes an end
/// only when the raw start is below the raw end, so a row whose end is not after
/// its start has no end; the viewer reads it the same way (`allegiance::ongoing`
/// in the viewer crate, verified by the recorder's war-relation oracle).
pub fn war_ongoing(start_hour: i32, end_hour: i32, hour: i32) -> bool {
    end_hour <= start_hour || hour < end_hour
}

/// The branch's first hour, if it is known.
/// Marks hours unobserved for every group history.
fn mark_all(unobserved: &mut UnobservedHours, first_hour: i32, last_hour: i32) {
    for spans in [
        &mut unobserved.humans,
        &mut unobserved.factions,
        &mut unobserved.wars,
    ] {
        mark_unobserved(spans, first_hour, last_hour);
    }
}

/// Adds unobserved hours, merging overlapping or adjacent spans.
pub fn mark_unobserved(spans: &mut Vec<HourSpan>, first_hour: i32, last_hour: i32) {
    match spans.last_mut() {
        Some(span) if span.last_hour.checked_add(1) >= Some(first_hour) => {
            span.last_hour = span.last_hour.max(last_hour);
        }
        _ => spans.push(HourSpan {
            first_hour,
            last_hour,
        }),
    }
}

pub fn scan_branch(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<Scan> {
    scan_from(replay, branch, cancelled, None)
}

fn scan_from(
    replay: &Replay,
    branch: u64,
    cancelled: &AtomicBool,
    previous: Option<&Scan>,
) -> io::Result<Scan> {
    use replay::rows::TableId;
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::HumanControllers,
        TableId::Factions,
        TableId::LexicalTokens,
        TableId::FactionMembers,
        TableId::WarRelations,
        TableId::Capitulation,
    ];
    let mut scan = previous
        .cloned()
        .unwrap_or_else(|| Scan::new(replay, branch));
    let blocks = scan.continuation.blocks;
    crate::blocks::scan_blocks(
        replay,
        replay
            .index
            .iter()
            .filter(|block| block.branch == branch)
            .skip(blocks),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                check_cancel(cancelled)?;
                scan.begin_block();
                let Ok(frames) = frames else {
                    scan.continuation.unreadable_blocks += 1;
                    continue;
                };
                let mut decoder = FrameDecoder::for_tables(&selected);
                for frame in frames {
                    check_cancel(cancelled)?;
                    let Ok(decoded) = decoder.advance(&frame) else {
                        scan.continuation.stopped += 1;
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        scan.continuation.stopped += 1;
                        break;
                    }
                    scan.observe(decoded)?;
                }
            }
            Ok(())
        },
    )?;
    scan.finish(replay, branch);
    Ok(scan)
}

impl Scan {
    /// Preserve shared dictionaries and their row changes in the scan snapshot.
    pub fn saved_bytes(&self) -> io::Result<Vec<u8>> {
        let mut state = self.clone();
        state.seal();
        state.dictionary_changes = vec![faction_names::LexicalNames::empty()];
        let mut ids = HashMap::new();
        let mut dictionary = |tokens: &Arc<faction_names::LexicalNames>| {
            *ids.entry(Arc::as_ptr(tokens) as usize).or_insert_with(|| {
                state.dictionary_changes.push(Arc::clone(tokens));
                state.dictionary_changes.len() - 1
            })
        };
        for history in state.continuation.names.values_mut() {
            // Group replies depend on the operands, not repeated supplying reads.
            // Keep the latest read date for the next observation after resume.
            let mut compact: RunHistory<NameOperands> = RunHistory::default();
            for row in history.iter() {
                if let Some(previous) = compact.last_mut()
                    && previous.last_hour.checked_add(1) == Some(row.first_hour)
                    && previous.literal == row.literal
                    && previous.needs_templates() == row.needs_templates()
                    && (!row.needs_templates()
                        || (previous.template_token == row.template_token
                            && previous.template == row.template
                            // Only a resolved template reads its dictionary,
                            // and only its dictionary is saved.
                            && (!matches!(&row.template, Ok(Some(_)))
                                || Arc::ptr_eq(&previous.tokens, &row.tokens))))
                {
                    previous.last_hour = row.last_hour;
                    previous.lexical_read = row.lexical_read;
                    previous.template_token = row.template_token;
                    previous.template = row.template.clone();
                } else {
                    compact.push(row.clone());
                }
            }
            *history = compact;
            for index in 0..history.len() {
                let row = &mut history[index];
                row.dictionary = if row.needs_templates() && matches!(&row.template, Ok(Some(_))) {
                    dictionary(&row.tokens)
                } else {
                    0
                };
            }
        }
        state.continuation.lexical_dictionary = state
            .continuation
            .lexical
            .as_ref()
            .and_then(|tokens| ids.get(&(Arc::as_ptr(tokens) as usize)).copied());
        crate::prepared::series::compress(serde_json::to_vec(&state)?.as_slice(), 3)
    }
    pub fn read(reader: impl io::Read) -> io::Result<Self> {
        let mut state: Self = serde_json::from_reader(zstd::stream::Decoder::new(reader)?)?;
        let dictionary = |index: usize| {
            state
                .dictionary_changes
                .get(index)
                .cloned()
                .ok_or_else(|| crate::invalid("Missing country group dictionary"))
        };
        state.continuation.lexical = state
            .continuation
            .lexical_dictionary
            .map(&dictionary)
            .transpose()?;
        for history in state.continuation.names.values_mut() {
            for index in 0..history.len() {
                history[index].tokens = dictionary(history[index].dictionary)?;
            }
        }
        for (id, faction) in &mut state.factions {
            faction.names = state.continuation.names[id].clone();
        }
        let rosters = &mut state.continuation.rosters;
        let last = rosters
            .faction_reads
            .iter()
            .next_back()
            .map(|span| span.last_hour);
        rosters.current = rosters
            .factions
            .iter()
            .filter_map(|(id, history)| {
                history
                    .iter()
                    .next_back()
                    .filter(|run| Some(run.last_hour) == last)
                    .map(|run| (id.clone(), run.complete))
            })
            .collect();
        Ok(state)
    }
    pub fn new(replay: &Replay, branch: u64) -> Self {
        let mut scan = Self::default();
        scan.continuation.bounds = replay
            .branches
            .iter()
            .find(|start| start.id == branch)
            .map(|start| branch_bounds(replay, start));
        scan.continuation.previous_hour = scan
            .continuation
            .bounds
            .and_then(|(first, _)| first.checked_sub(1));
        scan
    }
    pub fn begin_block(&mut self) {
        // A new mutable scan must not reuse its predecessor's sum projection.
        self.history = Arc::default();
        self.continuation.lexical_seen = None;
        self.humans.begin_block();
        self.wars.begin_block();
        self.identity_changes.begin_block();
        self.continuation.rosters.begin_block();
        for history in self.identities.values_mut() {
            history.begin_block();
        }
        for faction in self.factions.values_mut() {
            faction.members.begin_block();
            faction.leaders.begin_block();
        }
        for history in self.continuation.names.values_mut() {
            history.begin_block();
        }
    }
    /// [`Self::advance_changed`] at every hour `first..=last` in order when
    /// none of them changes the scan, in constant time: the hourly counts
    /// grow at their current rates and the frontier moves to `last`.
    pub(crate) fn advance_quiet(&mut self, first: i32, last: i32) {
        let hours = i64::from(last) - i64::from(first) + 1;
        self.recent_starts.clear();
        self.recent_capitulations.clear();
        let state = &mut self.continuation;
        state.incomplete = repeated_u32(state.incomplete, state.rates.0, hours);
        state.capitulations_unread = repeated(state.capitulations_unread, state.rates.1, hours);
        state.rosters.incomplete = repeated(state.rosters.incomplete, state.rates.2, hours);
        state.previous_hour = Some(last);
        state.deferred = true;
        self.unobserved = state.unobserved.clone();
        *self
            .history
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
    }
    /// What, besides an hour's own change facts, decides whether
    /// [`Self::advance_changed`] leaves the scan unchanged at that hour.
    pub(crate) fn quiet_inputs(&self) -> QuietInputs {
        use std::hash::{Hash, Hasher};
        let state = &self.continuation;
        QuietInputs {
            countries: state.country_keys.as_ref().map(|keys| {
                let mut hasher = std::hash::DefaultHasher::new();
                for key in keys.iter() {
                    key.hash(&mut hasher);
                }
                hasher.finish()
            }),
            wars: state.war_inputs.as_ref().map(|rows| {
                let mut hasher = std::hash::DefaultHasher::new();
                rows.hash(&mut hasher);
                hasher.finish()
            }),
            previous_hour: state.previous_hour,
            next_expiry: state.next_expiry,
        }
    }
    #[allow(clippy::type_complexity)]
    pub fn advance_changed(
        &mut self,
        frame: &replay::frame::DecodedFrame,
        changes: &crate::prepared::frame_changes::FrameChanges,
    ) -> io::Result<Option<(Vec<(u64, usize)>, Vec<usize>)>> {
        use replay::rows::TableId;
        let state = &mut self.continuation;
        let countries = changes.rows_changed[TableId::Countries.index()]
            && state.country_keys.as_ref().is_none_or(|keys| {
                frame
                    .complete::<tables::Countries>()
                    .map_or(true, |rows| !keys.same(rows))
            });
        let mut wars = false;
        if changes.rows_changed[TableId::WarRelations.index()] {
            let rows = GroupOperands::war_operands(frame);
            #[cfg(any(test, feature = "test-support"))]
            {
                state.work.row_visits += rows.as_ref().map_or(0, Vec::len);
            }
            wars = state.war_inputs != rows;
            state.war_inputs = rows;
        }
        let dictionary = changes.rows_changed[TableId::LexicalTokens.index()]
            || changes.table_read[TableId::LexicalTokens.index()];
        let changed = countries
            || wars
            || dictionary
            || [
                TableId::CountryLinks,
                TableId::Factions,
                TableId::FactionMembers,
                TableId::HumanControllers,
                TableId::Capitulation,
                TableId::LexicalTokens,
            ]
            .iter()
            .any(|table| changes.rows_changed[table.index()])
            || state.previous_hour.and_then(|hour| hour.checked_add(1)) != Some(frame.hour)
            || state.next_expiry.is_some_and(|hour| hour <= frame.hour);
        if !changed {
            self.advance_quiet(frame.hour, frame.hour);
            return Ok(None);
        }
        let boundary = state.previous_hour.and_then(|hour| hour.checked_add(1)) != Some(frame.hour)
            || state.operands.is_none();
        let holders = countries || changes.rows_changed[TableId::CountryLinks.index()] || boundary;
        let mut updates = Updates {
            holders,
            names: boundary || changes.rows_changed[TableId::Factions.index()] || dictionary,
            dictionary,
            dictionary_templates: !boundary
                && !changes.rows_changed[TableId::Factions.index()]
                && dictionary,
            humans: boundary || holders || changes.rows_changed[TableId::HumanControllers.index()],
            factions: boundary
                || holders
                || changes.rows_changed[TableId::Factions.index()]
                || changes.rows_changed[TableId::FactionMembers.index()],
            wars: boundary
                || holders
                || wars
                || state.next_expiry.is_some_and(|hour| hour <= frame.hour),
            capitulations: boundary
                || holders
                || changes.rows_changed[TableId::Capitulation.index()],
        };
        let mut operands = state.operands.take().unwrap_or_default();
        let old_factions = if changes.rows_changed[TableId::Factions.index()] {
            operands.factions.take()
        } else {
            None
        };
        let visits = operands.refresh(
            frame,
            boundary || changes.rows_changed[TableId::Factions.index()],
            boundary || changes.rows_changed[TableId::FactionMembers.index()],
            boundary || changes.rows_changed[TableId::HumanControllers.index()],
            false,
            boundary || changes.rows_changed[TableId::Capitulation.index()],
        );
        if wars || boundary {
            operands.wars = state.war_inputs.clone();
        }
        let mut renamed = Vec::new();
        if !boundary
            && !holders
            && !changes.rows_changed[TableId::FactionMembers.index()]
            && changes.rows_changed[TableId::Factions.index()]
            && let Some((old, new)) = old_factions.as_ref().zip(operands.factions.as_ref())
            && old.len() == new.len()
            && old.iter().zip(new).all(|(old, new)| old.0 == new.0)
        {
            updates.factions = false;
            renamed.extend(
                old.iter()
                    .zip(new)
                    .filter(|(old, new)| old.1 != new.1)
                    .map(|(_, new)| new.0),
            );
        }
        #[cfg(any(test, feature = "test-support"))]
        {
            state.work.row_visits += visits;
        }
        #[cfg(not(test))]
        let _ = visits;
        self.refresh_holders(
            frame,
            holders && changes.rows_changed[TableId::Countries.index()],
            changes.rows_changed[TableId::CountryLinks.index()],
        );
        if updates.dictionary_templates {
            let mut other = updates;
            other.names = false;
            self.seal_selected(other);
            if let Some(frontier) = self.continuation.previous_hour {
                for (id, _, _) in &operands.templated {
                    self.seal_name(*id, frontier);
                }
            }
        } else {
            self.seal_selected(updates);
        }
        let prior = (
            if updates.factions {
                self.continuation
                    .member_open
                    .iter()
                    .map(|((id, _), &index)| (*id, index))
                    .collect()
            } else {
                Vec::new()
            },
            if updates.wars {
                self.continuation.war_open.values().copied().collect()
            } else {
                Vec::new()
            },
        );
        let before = (
            self.continuation.incomplete,
            self.continuation.capitulations_unread,
            self.continuation.rosters.incomplete,
        );
        self.observe_selected(frame, &operands, updates, &renamed)?;
        let state = &mut self.continuation;
        state.operands = Some(operands);
        state.deferred = true;
        state.rates = (
            state.incomplete - before.0,
            state.capitulations_unread - before.1,
            state.rosters.incomplete - before.2,
        );
        state.unknown = [
            &state.unobserved.humans,
            &state.unobserved.factions,
            &state.unobserved.wars,
        ]
        .map(|spans| {
            spans
                .last()
                .is_some_and(|span| span.last_hour == frame.hour)
        });
        if updates.holders {
            state.countries_known = state.country_keys.is_some() && state.link_inputs.is_some();
            state.live_countries = state
                .country_keys
                .as_ref()
                .into_iter()
                .flat_map(|keys| keys.iter())
                .filter_map(|(_, _, tag)| std::str::from_utf8(tag).ok().map(str::to_owned))
                .collect();
        }
        if updates.factions {
            let operands = state.operands.as_ref().expect("group operands");
            state.factions_known = operands.factions.is_some();
            state.live_factions = operands
                .factions
                .as_ref()
                .map_or_else(BTreeSet::new, |rows| rows.iter().map(|row| row.0).collect());
        }
        if updates.humans {
            state
                .human_open
                .retain(|_, index| self.humans[*index].last_hour == frame.hour);
        }
        if updates.factions {
            state
                .leader_open
                .retain(|(id, _), index| self.factions[id].leaders[*index].last_hour == frame.hour);
            state
                .member_open
                .retain(|(id, _), index| self.factions[id].members[*index].last_hour == frame.hour);
        }
        if updates.wars {
            state
                .war_open
                .retain(|_, index| self.wars[*index].last_hour == frame.hour);
            state.next_expiry = state.war_inputs.as_ref().and_then(|rows| {
                rows.iter()
                    .filter(|row| row.3 > row.2 && row.3 > frame.hour)
                    .map(|row| row.3)
                    .min()
            });
        }
        Ok(Some(prior))
    }
    pub(crate) fn seal(&mut self) {
        self.seal_selected(Updates::ALL);
        self.continuation.deferred = false;
        self.unobserved = self.continuation.unobserved.clone();
        for id in &self.continuation.live_factions {
            if let Some(faction) = self.factions.get_mut(id)
                && let Some(names) = self.continuation.names.get(id)
            {
                faction.names = names.clone();
            }
        }
    }
    fn seal_name(&mut self, id: u64, hour: i32) {
        if self.continuation.factions_known
            && self.continuation.live_factions.contains(&id)
            && let Some(name) = self
                .continuation
                .names
                .get_mut(&id)
                .and_then(RunHistory::last_mut)
        {
            name.last_hour = name.last_hour.max(hour);
        }
    }
    fn seal_selected(&mut self, updates: Updates) {
        self.unobserved = Arc::default();
        let state = &mut self.continuation;
        if !state.deferred {
            return;
        }
        let Some(hour) = state.previous_hour else {
            return;
        };
        if state.unknown.iter().any(|missing| *missing) {
            let missing = Arc::make_mut(&mut state.unobserved);
            for (unknown, spans) in state.unknown.into_iter().zip([
                &mut missing.humans,
                &mut missing.factions,
                &mut missing.wars,
            ]) {
                if unknown {
                    let from = spans
                        .last()
                        .and_then(|span| span.last_hour.checked_add(1))
                        .unwrap_or(hour);
                    if from <= hour {
                        mark_unobserved(spans, from, hour);
                    }
                }
            }
        }
        if updates.humans {
            for index in state.human_open.values() {
                self.humans[*index].last_hour = hour;
            }
        }
        if updates.factions {
            for ((id, _), index) in &state.member_open {
                self.factions.get_mut(id).expect("live faction").members[*index].last_hour = hour;
            }
            for ((id, _), index) in &state.leader_open {
                self.factions.get_mut(id).expect("live faction").leaders[*index].last_hour = hour;
            }
        }
        if updates.wars {
            for index in state.war_open.values() {
                self.wars[*index].last_hour = hour;
            }
        }
        if updates.factions || updates.names {
            for id in &state.live_factions {
                if updates.factions
                    && let Some(faction) = self.factions.get_mut(id)
                {
                    faction.last_hour = hour;
                }
                if updates.names
                    && let Some(last) = state.names.get_mut(id).and_then(RunHistory::last_mut)
                {
                    last.last_hour = hour;
                }
            }
        }
        if updates.holders && state.countries_known {
            for tag in &state.live_countries {
                if let Some(last) = self.identities.get_mut(tag).and_then(RunHistory::last_mut)
                    && last.last_hour < hour
                {
                    last.last_hour = hour;
                }
                if let Some(history) = state.rosters.country_presence.get_mut(tag) {
                    history.last_mut().expect("live country read").last_hour = hour;
                }
            }
            // These reads stayed authenticated through the deferred frontier.
            // Extending an open endpoint must retain the intervening quiet hours.
            state
                .rosters
                .country_reads
                .last_mut()
                .expect("country read")
                .last_hour = hour;
        }
        if updates.factions && state.factions_known {
            state
                .rosters
                .faction_reads
                .last_mut()
                .expect("faction read")
                .last_hour = hour;
        }
        if updates.factions && state.factions_known {
            for (id, complete) in &state.rosters.current {
                if let Some(last) = state
                    .rosters
                    .factions
                    .get_mut(id)
                    .and_then(RunHistory::last_mut)
                {
                    last.last_hour = hour;
                }
                if !complete
                    && let Ok(id) = id.parse::<u64>()
                    && let Some(faction) = self.factions.get_mut(&id)
                {
                    let from = faction
                        .incomplete
                        .last()
                        .and_then(|span| span.last_hour.checked_add(1))
                        .unwrap_or(hour);
                    mark_unobserved(Arc::make_mut(&mut faction.incomplete), from, hour);
                }
            }
        }
    }
    pub fn observe(&mut self, decoded: &replay::frame::DecodedFrame) -> io::Result<()> {
        let _cpu = replay::cpu::scope("groups-read", None);
        self.refresh_holders(decoded, true, true);
        let mut operands = GroupOperands::default();
        let visits = operands.refresh(decoded, true, true, true, true, true);
        #[cfg(any(test, feature = "test-support"))]
        {
            self.continuation.work.row_visits += visits;
        }
        #[cfg(not(test))]
        let _ = visits;
        self.observe_selected(decoded, &operands, Updates::ALL, &[])
    }
    fn refresh_holders(
        &mut self,
        frame: &replay::frame::DecodedFrame,
        countries: bool,
        links: bool,
    ) {
        if countries {
            self.continuation.country_keys = frame
                .complete::<tables::Countries>()
                .ok()
                .map(crate::metadata::CountryKeys::read);
            #[cfg(any(test, feature = "test-support"))]
            {
                self.continuation.work.row_visits += frame
                    .complete::<tables::Countries>()
                    .map_or(0, |rows| rows.len());
            }
        }
        if links {
            self.continuation.link_inputs = frame
                .complete::<tables::CountryLinks>()
                .ok()
                .map(link_slots);
            #[cfg(any(test, feature = "test-support"))]
            {
                self.continuation.work.row_visits += self
                    .continuation
                    .link_inputs
                    .as_ref()
                    .map_or(0, HashMap::len);
            }
        }
    }
    fn observe_selected(
        &mut self,
        decoded: &replay::frame::DecodedFrame,
        operands: &GroupOperands,
        updates: Updates,
        renamed: &[u64],
    ) -> io::Result<()> {
        use replay::rows::TableId;
        let hour = decoded.hour;
        self.unobserved = Arc::default();
        #[cfg(any(test, feature = "test-support"))]
        {
            use replay::frame::TableView;
            if updates.dictionary {
                self.continuation.work.row_visits +=
                    match &*decoded.tables[TableId::LexicalTokens.index()] {
                        TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.len(),
                        _ => 0,
                    };
            }
        }
        #[cfg(any(test, feature = "test-support"))]
        if updates.dictionary {
            self.continuation.work.dictionary_interpretations += operands.templated.len();
        }
        self.members_changed = updates.factions;
        self.wars_changed = updates.wars;
        let held_unknown = self.continuation.unknown;
        let roster_rate = self.continuation.rates.2;
        let old_frontier = self.continuation.previous_hour;
        self.recent_starts.clear();
        self.recent_capitulations.clear();
        let Self {
            recent_starts,
            recent_capitulations,
            humans,
            factions,
            wars,
            war_starts,
            capitulations,
            war_start_reads,
            capitulation_reads,
            capitulations_observed,
            unattributed_war_starts,
            unattributed_capitulations,
            identities,
            identity_changes,
            continuation,
            ..
        } = self;
        let Continuation {
            rosters,
            unobserved,
            human_open,
            member_open,
            leader_open,
            named_starts,
            named_capitulations,
            named_since,
            continuous_since,
            unattributed,
            capitulations_unread,
            war_open,
            unmapped,
            incomplete,
            previous_hour,
            tags,
            faction_tags,
            faction_ids,
            invalid_names,
            names,
            lexical,
            lexical_seen,
            lexical_read,
            bounds,
            country_keys,
            link_inputs,
            ..
        } = continuation;
        let branch_first = bounds.map(|(first, _)| first);
        if let Some(previous) = *previous_hour
            && hour > previous + 1
        {
            mark_all(Arc::make_mut(unobserved), previous + 1, hour - 1);
            *continuous_since = None;
        }
        *previous_hour = Some(hour);
        let current = &decoded.tables[TableId::LexicalTokens.index()];
        if updates.dictionary
            || decoded.metrics.table_ns[TableId::LexicalTokens.index()] != 0
            || lexical_seen.as_ref().is_none_or(|seen| !seen.is(current))
        {
            // Every block reads the static dictionary again; equal
            // rows keep the one copy (and token index) already held.
            let names = lexical
                .take()
                .filter(|names| names.holds(current))
                .unwrap_or_else(|| Arc::new(faction_names::LexicalNames::new(current.clone())));
            *lexical = Some(names);
            *lexical_seen = Some(current.identity());
            *lexical_read = hour;
        }
        if updates.names
            && let Some(rows) = &operands.factions
        {
            let rows = if updates.dictionary_templates {
                &operands.templated
            } else {
                rows
            };
            for (id, literal, token) in rows {
                if std::str::from_utf8(literal).is_err() {
                    invalid_names.insert(*id);
                }
                let history = names.entry(*id).or_default();
                if let Some(previous) = history.last_mut()
                    && previous.last_hour.checked_add(1) == Some(hour)
                    && previous.literal == *literal
                    && (!literal.is_empty()
                        || (previous.template_token == *token
                            && (token.is_none() || previous.lexical_read == *lexical_read)))
                {
                    previous.last_hour = hour;
                } else {
                    history.push(NameOperands {
                        first_hour: hour,
                        last_hour: hour,
                        literal: literal.clone(),
                        template_token: *token,
                        lexical_read: *lexical_read,
                        dictionary: 0,
                        tokens: if literal.is_empty() && token.is_some() {
                            Arc::clone(lexical.as_ref().expect("the hour has a lexical table view"))
                        } else {
                            Arc::new(faction_names::LexicalNames::new(
                                replay::frame::TableView::Complete(
                                    replay::rows::TableRows::LexicalTokens(Vec::new()),
                                )
                                .into(),
                            ))
                        },
                        template: faction_names::template_spelling(decoded, *token),
                    });
                }
            }
        }
        let updated = if updates.holders {
            tags.update_projected(country_keys.as_ref(), link_inputs.as_ref())
        } else {
            decoded
                .complete::<tables::Countries>()
                .and_then(|_| decoded.complete::<tables::CountryLinks>())
                .map(|_| false)
        };
        let countries_known = updated.is_ok();
        let ids_changed = updates.holders && countries_known && {
            let rows = country_keys.as_ref().expect("known country operands");
            let changed = rows
                .iter()
                .map(|(id, _, _)| id)
                .ne(faction_ids.iter().copied());
            if changed {
                *faction_ids = rows.iter().map(|(id, _, _)| id).collect();
            }
            changed
        };
        if updated.as_ref().is_ok_and(|rebuilt| *rebuilt) || ids_changed {
            // Sum rosters use the same canonical holder as their country measures.
            let slots = link_inputs.as_ref().expect("known link operands");
            let countries =
                projected_countries(country_keys.as_ref().expect("known country operands"));
            let mut slot_tags: HashMap<u32, Option<&[u8]>> = HashMap::new();
            for (tag, country) in tagged_countries_indexed(&countries, slots) {
                if let TaggedCountry::Found(_, slot) = country {
                    let entry = slot_tags.entry(slot).or_insert(Some(tag));
                    if *entry != Some(tag) {
                        *entry = None;
                    }
                }
            }
            *faction_tags = slots
                .iter()
                .filter_map(|(&raw, &slot)| {
                    let tag = std::str::from_utf8(slot_tags.get(&slot).copied().flatten()?).ok()?;
                    (!tag.is_empty()).then(|| (raw, tag.to_owned()))
                })
                .collect();
        }
        if updates.factions || updates.holders {
            rosters.observe_projected(
                decoded,
                operands,
                updates.holders,
                hour,
                countries_known,
                country_keys.as_ref(),
                faction_tags,
                !updated.as_ref().is_ok_and(|rebuilt| !*rebuilt) || ids_changed,
            );
        } else {
            rosters.incomplete += roster_rate;
            for id in renamed {
                if let Some(faction) = factions.get_mut(id) {
                    let literal = operands
                        .factions
                        .as_ref()
                        .expect("known factions")
                        .binary_search_by_key(id, |row| row.0)
                        .ok()
                        .map(|index| &operands.factions.as_ref().unwrap()[index].1)
                        .expect("renamed faction");
                    faction.name = std::str::from_utf8(literal).unwrap_or_default().to_owned();
                    if let Some(history) = rosters.factions.get_mut(&id.to_string())
                        && let Some(previous) = history.last_mut()
                    {
                        if let Some(frontier) = old_frontier {
                            previous.last_hour = frontier;
                        }
                        let mut run = previous.clone();
                        run.first_hour = hour;
                        run.last_hour = hour;
                        run.name = std::str::from_utf8(literal).ok().map(str::to_owned);
                        history.push(run);
                    }
                }
            }
        }
        let capitulation_rows = operands.capitulations.as_ref();
        if capitulation_rows.is_none() {
            *capitulations_unread += 1;
        } else {
            *capitulations_observed = true;
        }
        let Ok(rebuilt) = updated else {
            *incomplete += 1;
            mark_all(Arc::make_mut(unobserved), hour, hour);
            *continuous_since = None;
            return Ok(());
        };
        if rebuilt {
            // An id naming another tag than the hour before, with no gap between.
            if continuous_since.is_some() {
                let mut changed: Vec<IdentityChange> = named_since
                    .iter()
                    .filter_map(|(id, (from, _))| {
                        let to = tags.get(*id)?;
                        (to != from).then(|| IdentityChange {
                            hour,
                            from: from.clone(),
                            to: to.clone(),
                        })
                    })
                    .collect();
                // Several ids may name one country: each change is listed once.
                changed.sort();
                changed.dedup();
                for change in changed {
                    identity_changes.push(change);
                }
            }
            named_since.retain(|id, (tag, _)| tags.get(*id) == Some(tag));
            for (id, tag) in tags.iter() {
                named_since
                    .entry(*id)
                    .or_insert_with(|| (tag.clone(), hour));
            }
        }
        let since = *continuous_since.get_or_insert(hour);
        // Native holder bounds are independent of memberships and air activity.
        if updates.holders {
            let countries =
                projected_countries(country_keys.as_ref().expect("known country operands"));
            let holders = tagged_countries_indexed(
                &countries,
                link_inputs.as_ref().expect("known link operands"),
            );
            for (tag, holder) in holders {
                let Some((country, slot)) = holder.found() else {
                    continue;
                };
                let Ok(tag) = std::str::from_utf8(tag) else {
                    continue;
                };
                let history = identities.entry(tag.to_owned()).or_default();
                let mut track = IdentityTrack::default();
                if let Some(last) = history.iter().next_back() {
                    track.step(Some((last.slot, last.country_id)), last.last_hour);
                }
                let contiguous = track.step(Some((slot, country.id)), hour);
                if contiguous {
                    history.last_mut().expect("identity run").last_hour = hour;
                } else {
                    history.push(super::country_history::CountryIdentityRun {
                        first_hour: hour,
                        last_hour: hour,
                        slot,
                        country_id: country.id,
                    });
                }
            }
        }
        // The tag an id had at `at`, when that is known (see `named_since`).
        let tag_at = |raw: u32, at: i32| -> Option<&String> {
            let (tag, from) = named_since.get(&raw)?;
            (since.max(*from) <= at).then_some(tag)
        };
        // Every group table names countries by their raw link id.
        let tag_of = |raw: u32, unmapped: &mut BTreeSet<u32>| -> Option<&String> {
            let tag = tags.get(raw);
            if tag.is_none() {
                unmapped.insert(raw);
            }
            tag
        };
        // Each history is unobserved only where its own tables are incomplete.
        let mut missed = false;
        if updates.humans {
            match operands.humans.as_ref() {
                Some(rows) => {
                    // A country the links cannot name leaves the hour's list incomplete.
                    let mut incomplete = false;
                    for row in rows.iter().filter(|row| row.1) {
                        let Some(tag) = tag_of(row.0, unmapped) else {
                            incomplete = true;
                            continue;
                        };
                        extend_history(human_open, humans, tag.clone(), hour, || MembershipRun {
                            tag: tag.clone(),
                            first_hour: hour,
                            last_hour: hour,
                        });
                    }
                    if incomplete {
                        mark_unobserved(&mut Arc::make_mut(unobserved).humans, hour, hour);
                    }
                }
                None => {
                    missed = true;
                    mark_unobserved(&mut Arc::make_mut(unobserved).humans, hour, hour);
                }
            }
        } else {
            missed |= held_unknown[0] && operands.humans.is_none();
            if held_unknown[0] {
                mark_unobserved(&mut Arc::make_mut(unobserved).humans, hour, hour);
            }
        }
        if updates.factions {
            match (operands.factions.as_ref(), operands.members.as_ref()) {
                (Some(rows), Some(members)) => {
                    let present: BTreeSet<_> = rows.iter().map(|row| row.0).collect();
                    for (id, literal, _) in rows {
                        let name = std::str::from_utf8(literal).unwrap_or_default();
                        let faction = factions.entry(*id).or_insert_with(|| ScannedFaction {
                            name: name.to_owned(),
                            names: RunHistory::default(),
                            first_hour: hour,
                            last_hour: hour,
                            members: RunHistory::default(),
                            leaders: RunHistory::default(),
                            incomplete: Arc::new(Vec::new()),
                        });
                        faction.last_hour = hour;
                        if rosters.factions[&id.to_string()]
                            .iter()
                            .next_back()
                            .is_some_and(|roster| !roster.complete)
                        {
                            mark_unobserved(Arc::make_mut(&mut faction.incomplete), hour, hour);
                        }
                        if faction.name != name {
                            faction.name = name.to_owned();
                        }
                    }
                    let mut ordered: Vec<_> = members.iter().collect();
                    ordered.sort_by_key(|member| (member.0, member.1));
                    // A member the links cannot name leaves that faction's roster
                    // incomplete this hour: no join, leave or founding is read there. A
                    // member of a faction the Factions table does not list belongs to
                    // no faction this reply names.
                    for member in ordered {
                        if !present.contains(&member.0) {
                            continue;
                        }
                        let Some(faction) = factions.get_mut(&member.0) else {
                            continue;
                        };
                        let tag = tag_of(member.2, unmapped);
                        if tag.is_none() {
                            mark_unobserved(Arc::make_mut(&mut faction.incomplete), hour, hour);
                        }
                        if let Some(tag) = tag {
                            extend_history(
                                member_open,
                                &mut faction.members,
                                (member.0, tag.clone()),
                                hour,
                                || MembershipRun {
                                    tag: tag.clone(),
                                    first_hour: hour,
                                    last_hour: hour,
                                },
                            );
                            // The member at order zero leads the faction (module notes).
                            if member.1 == 0 {
                                extend_history(
                                    leader_open,
                                    &mut faction.leaders,
                                    (member.0, tag.clone()),
                                    hour,
                                    || MembershipRun {
                                        tag: tag.clone(),
                                        first_hour: hour,
                                        last_hour: hour,
                                    },
                                );
                            }
                        }
                    }
                }
                _ => {
                    missed = true;
                    mark_unobserved(&mut Arc::make_mut(unobserved).factions, hour, hour);
                }
            }
        } else {
            missed |=
                held_unknown[1] && (operands.factions.is_none() || operands.members.is_none());
            if held_unknown[1] {
                mark_unobserved(&mut Arc::make_mut(unobserved).factions, hour, hour);
            }
        }
        if updates.wars {
            match operands.wars.as_ref() {
                Some(rows) => {
                    // When each war started, and which side instigated it.
                    for row in rows.iter() {
                        let key = (row.0, row.1, row.2);
                        if named_starts.contains(&key) {
                            continue;
                        }
                        named_starts.insert(key);
                        let (Some(first), Some(second)) =
                            (tag_at(row.0, row.2), tag_at(row.1, row.2))
                        else {
                            // Before the branch it is not this branch's event at all.
                            let refused =
                                usize::from(branch_first.is_some_and(|first| row.2 >= first));
                            *unattributed += refused;
                            *unattributed_war_starts += refused;
                            continue;
                        };
                        let (attacking, defending) = if row.4 != 0 {
                            (first, second)
                        } else {
                            (second, first)
                        };
                        let start = WarStart {
                            hour: row.2,
                            attacking: attacking.clone(),
                            defending: defending.clone(),
                        };
                        Arc::make_mut(war_start_reads)
                            .entry(start.clone())
                            .or_insert(hour);
                        if Arc::make_mut(war_starts).insert(start.clone()) {
                            recent_starts.push(start);
                        }
                    }
                    // A war the links cannot name leaves the hour's wars incomplete.
                    let mut incomplete = false;
                    for row in rows.iter().filter(|row| war_ongoing(row.2, row.3, hour)) {
                        let (Some(first), Some(second)) =
                            (tag_of(row.0, unmapped), tag_of(row.1, unmapped))
                        else {
                            incomplete = true;
                            continue;
                        };
                        // A war is one pair whichever side the row lists first.
                        let (first, second) = if first <= second {
                            (first, second)
                        } else {
                            (second, first)
                        };
                        extend_history(
                            war_open,
                            wars,
                            (first.clone(), second.clone()),
                            hour,
                            || WarPairRun {
                                first: first.clone(),
                                second: second.clone(),
                                first_hour: hour,
                                last_hour: hour,
                            },
                        );
                    }
                    if incomplete {
                        mark_unobserved(&mut Arc::make_mut(unobserved).wars, hour, hour);
                    }
                }
                None => {
                    missed = true;
                    mark_unobserved(&mut Arc::make_mut(unobserved).wars, hour, hour);
                }
            }
        } else {
            missed |= held_unknown[2] && operands.wars.is_none();
            if held_unknown[2] {
                mark_unobserved(&mut Arc::make_mut(unobserved).wars, hour, hour);
            }
        }
        // Capitulations as the engine dates them, read while the flag is set; the
        // date stays with the event after recovery. The hour word is certified
        // only while the flag is set (the fresh-save oracle lists the retained word
        // of an inactive capitulation as a limitation), so a capitulation that began
        // and ended between two readable samples is not recorded, never guessed
        // from that word.
        if updates.capitulations
            && let Some(rows) = capitulation_rows
        {
            for row in rows.iter().filter(|row| row.1) {
                let key = (row.0, row.2);
                if !named_capitulations.insert(key) {
                    continue;
                }
                let Some(tag) = tag_at(row.0, row.2) else {
                    let refused = usize::from(branch_first.is_some_and(|first| row.2 >= first));
                    *unattributed += refused;
                    *unattributed_capitulations += refused;
                    continue;
                };
                let event = Capitulation {
                    hour: row.2,
                    tag: tag.clone(),
                };
                Arc::make_mut(capitulation_reads)
                    .entry(event.clone())
                    .or_insert(hour);
                if Arc::make_mut(capitulations).insert(event.clone()) {
                    recent_capitulations.push(event);
                }
            }
        }
        if missed {
            *incomplete += 1;
        }
        Ok(())
    }
    pub fn finish(&mut self, replay: &Replay, branch: u64) {
        self.seal();
        self.continuation.bounds = replay
            .branches
            .iter()
            .find(|start| start.id == branch)
            .map(|start| branch_bounds(replay, start));
        self.continuation.blocks = replay
            .index
            .iter()
            .filter(|block| block.branch == branch)
            .count();
        self.unobserved = self.continuation.unobserved.clone();
        let unobserved = &mut self.unobserved;
        let bounds = self.continuation.bounds;
        let previous_hour = self.continuation.previous_hour;
        if let (Some((first, last)), previous) = (bounds, previous_hour) {
            let from = previous.map_or(first, |previous| previous.saturating_add(1));
            if from <= last {
                mark_all(Arc::make_mut(unobserved), from, last);
            }
        }
        let mut warnings = Vec::new();
        let (incomplete, unreadable_blocks, stopped, capitulations_unread, unattributed) = (
            self.continuation.incomplete,
            self.continuation.unreadable_blocks,
            self.continuation.stopped,
            self.continuation.capitulations_unread,
            self.continuation.unattributed,
        );
        if !self.continuation.invalid_names.is_empty() {
            warnings.push(format!(
                "{} faction names were not valid text and are not recorded.",
                self.continuation.invalid_names.len()
            ));
        }
        if !self.continuation.unmapped.is_empty() {
            warnings.push(format!(
            "{} recorded country references had no country identity and are left out of groups.",
            self.continuation.unmapped.len()
        ));
        }
        if self.continuation.incomplete > 0 {
            warnings.push(format!(
            "Group tables were incomplete at {incomplete} hours; memberships break there rather than carry over."
        ));
        }
        if self.continuation.unreadable_blocks > 0 || self.continuation.stopped > 0 {
            warnings.push(format!(
            "Part of this branch could not be read ({unreadable_blocks} unreadable blocks, {stopped} stopped reads); group history is missing there."
        ));
        }
        if self.continuation.capitulations_unread > 0 {
            warnings.push(format!(
            "Capitulations could not be read at {capitulations_unread} recorded hours; some may be missing."
        ));
        }
        if self.continuation.unattributed > 0 {
            warnings.push(format!(
            "{unattributed} war starts or capitulations were first seen after an unrecorded hour or a change of country; which countries they were is not known, so they are left out."
        ));
        }
        for (id, faction) in &mut self.factions {
            faction.names = self
                .continuation
                .names
                .get(id)
                .expect("a roster-known faction has a name read")
                .clone();
        }
        self.warnings = warnings;
        self.capitulation_unread_hours = self.continuation.capitulations_unread;
        self.history = Arc::default();
    }
}

#[cfg(any(test, feature = "test-support"))]
pub type ScannedFactionView<'a> = (&'a str, i32, i32, Vec<MembershipRun>);

impl Scan {
    #[cfg(any(test, feature = "test-support"))]
    pub fn humans(&self) -> Vec<MembershipRun> {
        self.humans.iter().cloned().collect()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn wars(&self) -> Vec<WarPairRun> {
        self.wars.iter().cloned().collect()
    }
    pub fn unobserved(&self) -> &UnobservedHours {
        &self.unobserved
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn faction_leaders(&self, id: u64) -> Option<Vec<MembershipRun>> {
        self.factions
            .get(&id)
            .map(|faction| faction.leaders.iter().cloned().collect())
    }
    /// Assignment identity before resolving main's literal/template name rule.
    pub fn faction_name_at(&self, id: u64, hour: i32) -> Option<String> {
        let operands = self.faction_name_operands(id, hour)?;
        Some(if operands.literal.is_empty() {
            // Dictionary changes may change bound token arguments without changing
            // the template spelling. Keep that read in the assignment identity.
            format!("{:?}@{}", operands.template, operands.lexical_read)
        } else {
            String::from_utf8_lossy(&operands.literal).into_owned()
        })
    }
    fn faction_name_operands(&self, id: u64, hour: i32) -> Option<&NameOperands> {
        self.factions.get(&id)?;
        let history = self.continuation.names.get(&id)?;
        let current = history.iter().next_back()?;
        if self.continuation.factions_known
            && self.continuation.live_factions.contains(&id)
            && current.first_hour <= hour
            && self
                .continuation
                .previous_hour
                .is_some_and(|frontier| hour <= frontier)
        {
            return Some(current);
        }
        history
            .iter()
            .rfind(|name| name.first_hour <= hour && hour <= name.last_hour)
    }
    pub fn resolved_faction_name_at(
        &self,
        id: u64,
        hour: i32,
        templates: Result<&Catalog, String>,
        localisation: &Localisation,
        recorded_language: &str,
    ) -> Result<String, String> {
        self.faction_name_operands(id, hour)
            .ok_or_else(|| format!("Faction {id} has no name read at hour {hour}"))?
            .resolve(templates, localisation, recorded_language)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn faction(&self, id: u64) -> Option<ScannedFactionView<'_>> {
        self.factions.get(&id).map(|faction| {
            (
                faction.name.as_str(),
                faction.first_hour,
                faction.last_hour,
                faction.members.iter().cloned().collect(),
            )
        })
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn faction_incomplete(&self, id: u64) -> Option<&[HourSpan]> {
        self.factions
            .get(&id)
            .map(|faction| faction.incomplete.as_slice())
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn war_starts(&self) -> Vec<WarStart> {
        self.war_starts.iter().cloned().collect()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn capitulations(&self) -> Vec<Capitulation> {
        self.capitulations.iter().cloned().collect()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn identity_changes(&self) -> Vec<IdentityChange> {
        self.identity_changes.iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod roster_reuse_tests {
    use super::*;
    use replay::{
        frame::TableView,
        rows::{FactionMemberRow, FactionRow, TableId, TableRows},
    };

    #[test]
    fn unchanged_rosters_reuse_members_and_match_uncached_history_at_5000_countries() {
        let mut frame = crate::test_fixtures::scale_frame(1, 0, 5000);
        *frame.tables[TableId::Factions.index()] =
            TableView::Complete(TableRows::Factions(vec![FactionRow {
                id: 77,
                name: b"Pact".to_vec(),
                ..Default::default()
            }]));
        *frame.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(
                (0..5000)
                    .map(|index| FactionMemberRow {
                        country: 100_000 + index,
                        faction: 77,
                        ordinal: index,
                    })
                    .collect(),
            ));
        let mut tags: HashMap<_, _> = (0..5000)
            .map(|index| (100_000 + index, format!("Country {index}")))
            .collect();
        let mut cached = RosterHistory::default();
        let mut uncached = RosterHistory::default();
        for (hour, changed, known) in [
            (0, true, true),
            (1, false, true),
            (3, false, true),
            (4, true, true),
            (5, true, false),
            (6, true, true),
            (7, false, true),
        ] {
            frame.hour = hour;
            if hour == 4 {
                tags.insert(100_000, "Renamed country".into());
                tags.insert(104_999, "Country 0".into());
                if let TableView::Complete(TableRows::Countries(countries)) =
                    &mut *frame.tables[TableId::Countries.index()]
                {
                    countries[0].tag = b"Renamed country".to_vec();
                    countries.last_mut().unwrap().tag = b"Country 0".to_vec();
                }
            }
            cached.observe(&frame, hour, known, &tags, changed);
            uncached.observe(&frame, hour, known, &tags, true);
            let bytes = |history: &RosterHistory| {
                let runs: Vec<_> = history.factions["77"]
                    .iter()
                    .map(|run| {
                        (
                            run.first_hour,
                            run.last_hour,
                            &run.name,
                            &run.members,
                            run.complete,
                        )
                    })
                    .collect();
                serde_json::to_vec(&(runs, history.incomplete)).unwrap()
            };
            assert_eq!(bytes(&cached), bytes(&uncached));
            assert_eq!(
                cached.country_reads.iter().collect::<Vec<_>>(),
                uncached.country_reads.iter().collect::<Vec<_>>()
            );
            assert_eq!(
                cached.faction_reads.iter().collect::<Vec<_>>(),
                uncached.faction_reads.iter().collect::<Vec<_>>()
            );
        }
        assert_eq!(cached.rebuilds, 4);
        assert_eq!(uncached.rebuilds, 7);
        let runs: Vec<_> = cached.factions["77"].iter().collect();
        assert_eq!((runs[0].first_hour, runs[0].last_hour), (0, 1));
        assert_eq!((runs[1].first_hour, runs[1].last_hour), (3, 3));
        assert!(!runs[3].complete);
        assert_eq!(runs.last().unwrap().members.len(), 5000);
    }
}
