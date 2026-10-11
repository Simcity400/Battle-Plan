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
        tagged_countries, tagged_countries_indexed, validate_campaign,
    },
    transport::json_len,
};
use installed_content::{faction_templates::Catalog, localisation::Localisation};
use replay::{container::Replay, frame::FrameDecoder, rows::tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct CountryGroups;

/// Branches kept per source, so tracks on different branches do not evict each other.
const CACHED_BRANCHES: usize = 4;

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
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
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

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().expect("validated branch");
        let scan = cached_scan(source, branch, cancelled)?;
        let mut warnings = Vec::new();
        let needs_templates = scan
            .factions
            .values()
            .any(|faction| faction.names.iter().any(NameOperands::needs_templates));
        let factions = with_name_content(
            source,
            context,
            needs_templates,
            &mut warnings,
            |templates, localisation, recorded_language, refused| {
                scan.faction_groups(templates, localisation, recorded_language, refused)
            },
        );
        warnings.extend(scan.warnings.iter().cloned());
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
                branch_id: &params.branch_id,
                humans: &scan.humans,
                factions: &factions,
                wars: &scan.wars,
                war_starts: &scan.war_starts,
                capitulations: &scan.capitulations,
                identity_changes: &scan.identity_changes,
                country_identities: IdentityCatalogue(&scan.identities),
                unobserved: &scan.unobserved,
                warnings: &warnings,
            },
            MAX_PRODUCT_REPLY_BYTES,
        )
        .is_none()
        {
            return Err(reply_budget_error());
        }
        let reply = CountryGroupReply {
            branch_id: params.branch_id.clone(),
            humans: scan.humans.iter().cloned().collect(),
            factions: factions
                .into_iter()
                .map(FactionGroupView::into_owned)
                .collect(),
            wars: scan.wars.iter().cloned().collect(),
            war_starts: scan.war_starts.iter().cloned().collect(),
            capitulations: scan.capitulations.iter().cloned().collect(),
            identity_changes: scan.identity_changes.iter().cloned().collect(),
            country_identities: scan
                .identities
                .iter()
                .map(|(tag, identities)| CountryIdentityHistory {
                    tag: tag.clone(),
                    identities: identities.iter().cloned().collect(),
                })
                .collect(),
            unobserved: (*scan.unobserved).clone(),
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

pub(crate) struct FactionNaming {
    templates: Result<Arc<Catalog>, String>,
    localisation: Arc<Localisation>,
    recorded_language: String,
}

pub(crate) fn load_faction_naming(
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
pub(crate) fn with_name_content<T>(
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
pub(crate) fn faction_name_at(
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

#[derive(Clone, Debug)]
pub(crate) struct NameOperands {
    pub(super) first_hour: i32,
    pub(super) last_hour: i32,
    pub(crate) literal: Vec<u8>,
    template_token: Option<u32>,
    lexical_read: i32,
    tokens: Arc<faction_names::LexicalNames>,
    template: Result<Option<String>, String>,
}

impl NameOperands {
    pub(crate) fn needs_templates(&self) -> bool {
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
pub(super) struct RunHistory<T> {
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
impl<T> RunHistory<T> {
    fn begin_block(&mut self) {
        self.segments
            .entry(self.len)
            .or_insert_with(|| Arc::new(Vec::new()));
    }
    pub(super) fn iter(&self) -> impl DoubleEndedIterator<Item = &T> {
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
#[cfg(test)]
impl<T, const N: usize> From<[T; N]> for RunHistory<T> {
    fn from(rows: [T; N]) -> Self {
        Self {
            segments: [(0, Arc::new(Vec::from(rows)))].into(),
            len: N,
        }
    }
}

#[derive(Clone)]
pub(super) struct ScannedFaction {
    pub(super) name: String,
    /// Names are dated too: a later rename cannot relabel an earlier air cell.
    pub(super) names: RunHistory<NameOperands>,
    pub(super) first_hour: i32,
    pub(super) last_hour: i32,
    pub(super) members: RunHistory<MembershipRun>,
    pub(super) leaders: RunHistory<MembershipRun>,
    pub(super) incomplete: Arc<Vec<HourSpan>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RosterRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub name: Option<String>,
    pub members: Vec<String>,
    pub complete: bool,
}

#[derive(Default)]
pub(crate) struct FactionHistory {
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
#[derive(Clone, Default)]
struct RosterHistory {
    factions: BTreeMap<String, RunHistory<RosterRun>>,
    country_reads: RunHistory<HourSpan>,
    country_presence: HashMap<String, RunHistory<HourSpan>>,
    faction_reads: RunHistory<HourSpan>,
    incomplete: usize,
    views: Option<[replay::frame::ViewIdentity; 2]>,
    current: Vec<(String, bool)>,
    #[cfg(test)]
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

    fn observe(
        &mut self,
        decoded: &replay::frame::DecodedFrame,
        hour: i32,
        countries_known: bool,
        tags: &HashMap<u32, String>,
        identities_changed: bool,
    ) {
        if countries_known {
            self.country_reads.record_hour(hour);
            for country in decoded
                .complete::<tables::Countries>()
                .expect("known countries")
            {
                if let Ok(tag) = std::str::from_utf8(&country.tag)
                    && !tag.is_empty()
                {
                    self.country_presence
                        .entry(tag.to_owned())
                        .or_default()
                        .record_hour(hour);
                }
            }
        }
        let Ok(rows) = decoded.complete::<tables::Factions>() else {
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
        #[cfg(test)]
        {
            self.rebuilds += 1;
        }
        let members = decoded.complete::<tables::FactionMembers>().ok();
        let mut rosters: BTreeMap<u64, (Option<String>, BTreeSet<String>, bool)> = BTreeMap::new();
        for row in rows {
            let name = std::str::from_utf8(&row.name).ok().map(str::to_owned);
            if rosters
                .insert(row.id, (name, BTreeSet::new(), members.is_some()))
                .is_some()
            {
                rosters.get_mut(&row.id).expect("inserted roster").2 = false;
            }
        }
        let mut membership = HashMap::new();
        for member in members.unwrap_or_default() {
            if !rosters.contains_key(&member.faction) {
                continue;
            }
            if let Some(tag) = countries_known.then(|| tags.get(&member.country)).flatten() {
                rosters
                    .get_mut(&member.faction)
                    .expect("recorded roster")
                    .1
                    .insert(tag.clone());
                if let Some(previous) = membership.insert(tag, member.faction)
                    && previous != member.faction
                {
                    rosters.get_mut(&previous).expect("previous roster").2 = false;
                    rosters.get_mut(&member.faction).expect("recorded roster").2 = false;
                }
            } else {
                rosters.get_mut(&member.faction).expect("recorded roster").2 = false;
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
#[derive(Clone)]
pub struct Scan {
    humans: RunHistory<MembershipRun>,
    pub(super) factions: BTreeMap<u64, ScannedFaction>,
    pub(super) wars: RunHistory<WarPairRun>,
    pub(super) war_starts: Arc<BTreeSet<WarStart>>,
    pub(super) capitulations: Arc<BTreeSet<Capitulation>>,
    pub(super) identity_changes: RunHistory<IdentityChange>,
    pub(super) war_start_reads: Arc<BTreeMap<WarStart, i32>>,
    pub(super) capitulation_reads: Arc<BTreeMap<Capitulation, i32>>,
    pub(super) capitulations_observed: bool,
    pub(super) unattributed_war_starts: usize,
    pub(super) unattributed_capitulations: usize,
    pub(super) capitulation_unread_hours: usize,
    pub(super) identities: BTreeMap<String, RunHistory<super::country_history::CountryIdentityRun>>,
    pub(super) unobserved: Arc<UnobservedHours>,
    pub(super) warnings: Vec<String>,
    rosters: RosterHistory,
    history: Arc<std::sync::Mutex<Option<Arc<FactionHistory>>>>,
    continuation: Continuation,
}

#[derive(Clone, Default)]
struct Continuation {
    blocks: usize,
    bounds: Option<(i32, i32)>,
    /// Coverage before finalising the source's unread trailing hours. A later
    /// append recomputes that tail without inheriting stale missing coverage.
    unobserved: Arc<UnobservedHours>,
    human_open: BTreeMap<String, usize>,
    member_open: BTreeMap<(u64, String), usize>,
    leader_open: BTreeMap<(u64, String), usize>,
    named_starts: BTreeSet<(u32, u32, i32)>,
    named_capitulations: BTreeSet<(u32, i32)>,
    named_since: HashMap<u32, (String, i32)>,
    continuous_since: Option<i32>,
    unattributed: usize,
    capitulations_unread: usize,
    war_open: BTreeMap<(String, String), usize>,
    unmapped: BTreeSet<u32>,
    incomplete: u32,
    unreadable_blocks: u32,
    stopped: u32,
    previous_hour: Option<i32>,
    tags: LinkTags,
    faction_tags: HashMap<u32, String>,
    faction_ids: Vec<u32>,
    invalid_names: BTreeSet<u64>,
    names: BTreeMap<u64, RunHistory<NameOperands>>,
    /// The latest dictionary read, named without holding its rows, and the
    /// first read of equal rows, which name operands share.
    lexical: Option<(
        replay::frame::ViewIdentity,
        Arc<faction_names::LexicalNames>,
    )>,
    lexical_read: i32,
}

#[derive(Default)]
pub(crate) struct GroupCache {
    pub(crate) entries: Vec<(u64, Arc<GroupBranch>)>,
}

#[derive(Default)]
pub(crate) struct GroupBranch {
    gate: std::sync::Mutex<()>,
    pub(crate) scan: std::sync::Mutex<Option<Arc<Scan>>>,
}

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
pub(crate) fn cached_scan(
    source: &Source,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Arc<Scan>> {
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
pub(crate) fn record_hour(spans: &mut Vec<HourSpan>, hour: i32) -> bool {
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

pub(crate) fn name_runs<'a>(
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
    pub(crate) fn faction_history(
        &self,
        cancelled: &AtomicBool,
    ) -> io::Result<Arc<FactionHistory>> {
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
            country_reads: copy(&self.rosters.country_reads, cancelled)?,
            faction_reads: copy(&self.rosters.faction_reads, cancelled)?,
            last_hour,
            incomplete: self.rosters.incomplete
                + self.continuation.unreadable_blocks as usize
                + self.continuation.stopped as usize,
            ..FactionHistory::default()
        };
        for (id, runs) in &self.rosters.factions {
            history.factions.insert(id.clone(), copy(runs, cancelled)?);
        }
        for (tag, runs) in &self.rosters.country_presence {
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
pub(super) trait HourRun {
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
pub(super) fn extend<K: Ord, R: HourRun>(
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
pub(super) fn war_ongoing(start_hour: i32, end_hour: i32, hour: i32) -> bool {
    end_hour <= start_hour || hour < end_hour
}

/// The branch's first hour, if it is known.
fn bounds_first(replay: &Replay, branch: u64) -> Option<i32> {
    replay
        .branches
        .iter()
        .find(|start| start.id == branch)
        .map(|start| branch_bounds(replay, start).0)
}

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
pub(super) fn mark_unobserved(spans: &mut Vec<HourSpan>, first_hour: i32, last_hour: i32) {
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
    let cancel = || {
        io::Error::new(
            io::ErrorKind::Interrupted,
            "Country group request cancelled",
        )
    };
    let mut humans = previous.map_or_else(RunHistory::default, |scan| scan.humans.clone());
    let mut factions = previous.map_or_else(BTreeMap::new, |scan| scan.factions.clone());
    let mut wars = previous.map_or_else(RunHistory::default, |scan| scan.wars.clone());
    let mut rosters = previous.map_or_else(RosterHistory::default, |scan| scan.rosters.clone());
    let mut war_starts = previous.map_or_else(
        || Arc::new(BTreeSet::new()),
        |scan| Arc::clone(&scan.war_starts),
    );
    let mut capitulations = previous.map_or_else(
        || Arc::new(BTreeSet::new()),
        |scan| Arc::clone(&scan.capitulations),
    );
    let mut war_start_reads =
        previous.map_or_else(Arc::default, |scan| Arc::clone(&scan.war_start_reads));
    let mut capitulation_reads =
        previous.map_or_else(Arc::default, |scan| Arc::clone(&scan.capitulation_reads));
    let mut capitulations_observed = previous.is_some_and(|scan| scan.capitulations_observed);
    let mut unattributed_war_starts = previous.map_or(0, |scan| scan.unattributed_war_starts);
    let mut unattributed_capitulations = previous.map_or(0, |scan| scan.unattributed_capitulations);
    let Continuation {
        blocks,
        bounds: _,
        unobserved: previous_unobserved,
        mut human_open,
        mut member_open,
        mut leader_open,
        mut named_starts,
        mut named_capitulations,
        mut named_since,
        mut continuous_since,
        mut unattributed,
        mut capitulations_unread,
        mut war_open,
        mut unmapped,
        mut incomplete,
        mut unreadable_blocks,
        mut stopped,
        previous_hour: previous_read,
        mut tags,
        mut faction_tags,
        mut faction_ids,
        mut invalid_names,
        mut names,
        mut lexical,
        mut lexical_read,
    } = previous.map_or_else(Continuation::default, |scan| scan.continuation.clone());
    // Each event is considered once, when first seen, keyed by raw ids and its hour:
    // a later hour may map the same id to another identity.
    // An event is named by the tags its ids had at the event's own hour. That is
    // known only when every hour since was recorded with the same tag for the id:
    // `named_since` holds each id's tag and the hour it has had it without a break,
    // `continuous_since` the hour since which every hour was recorded.
    let mut identity_changes =
        previous.map_or_else(RunHistory::default, |scan| scan.identity_changes.clone());
    let mut identities = previous.map_or_else(BTreeMap::new, |scan| scan.identities.clone());
    // Events in the branch whose countries at the event hour cannot be established.
    let branch_first = bounds_first(replay, branch);
    // Hours whose capitulations could not be read, so an empty list is not passed off as none.
    let mut unobserved = previous_unobserved;
    // Hours before the first readable frame and after the last are unobserved too,
    // so a branch that starts or ends in a gap or an unreadable block shows no
    // false joins or leaves there.
    let bounds = replay
        .branches
        .iter()
        .find(|start| start.id == branch)
        .map(|start| branch_bounds(replay, start));
    let mut previous_hour =
        previous_read.or_else(|| bounds.and_then(|(first, _)| first.checked_sub(1)));
    // Link id to tag, rebuilt only when the country identities change.
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
                if cancelled.load(Ordering::Acquire) {
                    return Err(cancel());
                }
                humans.begin_block();
                wars.begin_block();
                identity_changes.begin_block();
                rosters.begin_block();
                for history in identities.values_mut() {
                    history.begin_block();
                }
                for faction in factions.values_mut() {
                    faction.members.begin_block();
                    faction.leaders.begin_block();
                }
                for history in names.values_mut() {
                    history.begin_block();
                }
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    unreadable_blocks += 1;
                    continue;
                };
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(cancel());
                    }
                    let Ok(decoded) = decoder.advance(&frame) else {
                        stopped += 1;
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        stopped += 1;
                        break;
                    }
                    let hour = frame.hour;
                    if let Some(previous) = previous_hour
                        && hour > previous + 1
                    {
                        mark_all(Arc::make_mut(&mut unobserved), previous + 1, hour - 1);
                        continuous_since = None;
                    }
                    previous_hour = Some(hour);
                    let current = &decoded.tables[TableId::LexicalTokens.index()];
                    if lexical.as_ref().is_none_or(|(seen, _)| !seen.is(current)) {
                        // Every block reads the static dictionary again; equal
                        // rows keep the one copy (and token index) already held.
                        let names = lexical
                            .take()
                            .map(|(_, names)| names)
                            .filter(|names| names.holds(current))
                            .unwrap_or_else(|| {
                                Arc::new(faction_names::LexicalNames::new(current.clone()))
                            });
                        lexical = Some((current.identity(), names));
                        lexical_read = hour;
                    }
                    if let Ok(rows) = decoded.complete::<tables::Factions>() {
                        for row in rows {
                            if std::str::from_utf8(&row.name).is_err() {
                                invalid_names.insert(row.id);
                            }
                            let history = names.entry(row.id).or_default();
                            if let Some(previous) = history.last_mut()
                                && previous.last_hour.checked_add(1) == Some(hour)
                                && previous.literal == row.name
                                && previous.template_token == row.template_token
                                && (row.template_token.is_none()
                                    || previous.lexical_read == lexical_read)
                            {
                                previous.last_hour = hour;
                            } else {
                                history.push(NameOperands {
                                    first_hour: hour,
                                    last_hour: hour,
                                    literal: row.name.clone(),
                                    template_token: row.template_token,
                                    lexical_read,
                                    tokens: Arc::clone(
                                        &lexical
                                            .as_ref()
                                            .expect("the hour has a lexical table view")
                                            .1,
                                    ),
                                    template: faction_names::template_spelling(
                                        decoded,
                                        row.template_token,
                                    ),
                                });
                            }
                        }
                    }
                    let updated = tags.update(decoded);
                    let countries_known = updated.is_ok();
                    let ids_changed = countries_known && {
                        let rows = decoded.complete::<tables::Countries>()?;
                        let changed = rows.len() != faction_ids.len()
                            || rows.iter().zip(&faction_ids).any(|(row, id)| row.id != *id);
                        if changed {
                            faction_ids = rows.iter().map(|row| row.id).collect();
                        }
                        changed
                    };
                    if updated.as_ref().is_ok_and(|rebuilt| *rebuilt) || ids_changed {
                        // Sum rosters use the same canonical holder as their country measures.
                        let slots = link_slots(decoded.complete::<tables::CountryLinks>()?);
                        let mut slot_tags: HashMap<u32, Option<&[u8]>> = HashMap::new();
                        for (tag, country) in tagged_countries_indexed(
                            decoded.complete::<tables::Countries>()?,
                            &slots,
                        ) {
                            if let TaggedCountry::Found(_, slot) = country {
                                let entry = slot_tags.entry(slot).or_insert(Some(tag));
                                if *entry != Some(tag) {
                                    *entry = None;
                                }
                            }
                        }
                        faction_tags = slots
                            .into_iter()
                            .filter_map(|(raw, slot)| {
                                let tag =
                                    std::str::from_utf8(slot_tags.get(&slot).copied().flatten()?)
                                        .ok()?;
                                (!tag.is_empty()).then(|| (raw, tag.to_owned()))
                            })
                            .collect();
                    }
                    rosters.observe(
                        decoded,
                        hour,
                        countries_known,
                        &faction_tags,
                        !updated.as_ref().is_ok_and(|rebuilt| !*rebuilt) || ids_changed,
                    );
                    let capitulation_rows = decoded.complete::<tables::Capitulation>();
                    if capitulation_rows.is_err() {
                        capitulations_unread += 1;
                    } else {
                        capitulations_observed = true;
                    }
                    let Ok(rebuilt) = updated else {
                        incomplete += 1;
                        mark_all(Arc::make_mut(&mut unobserved), hour, hour);
                        continuous_since = None;
                        continue;
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
                    let holders = tagged_countries(
                        decoded.complete::<tables::Countries>()?,
                        decoded.complete::<tables::CountryLinks>()?,
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
                    match decoded.complete::<tables::HumanControllers>() {
                        Ok(rows) => {
                            // A country the links cannot name leaves the hour's list incomplete.
                            let mut incomplete = false;
                            for row in rows.iter().filter(|row| row.count > 0) {
                                let Some(tag) = tag_of(row.country, &mut unmapped) else {
                                    incomplete = true;
                                    continue;
                                };
                                extend_history(
                                    &mut human_open,
                                    &mut humans,
                                    tag.clone(),
                                    hour,
                                    || MembershipRun {
                                        tag: tag.clone(),
                                        first_hour: hour,
                                        last_hour: hour,
                                    },
                                );
                            }
                            if incomplete {
                                mark_unobserved(
                                    &mut Arc::make_mut(&mut unobserved).humans,
                                    hour,
                                    hour,
                                );
                            }
                        }
                        Err(_) => {
                            missed = true;
                            mark_unobserved(&mut Arc::make_mut(&mut unobserved).humans, hour, hour);
                        }
                    }
                    match (
                        decoded.complete::<tables::Factions>(),
                        decoded.complete::<tables::FactionMembers>(),
                    ) {
                        (Ok(rows), Ok(members)) => {
                            let present: BTreeSet<_> = rows.iter().map(|row| row.id).collect();
                            for row in rows {
                                let name = std::str::from_utf8(&row.name).unwrap_or_default();
                                let faction =
                                    factions.entry(row.id).or_insert_with(|| ScannedFaction {
                                        name: name.to_owned(),
                                        names: RunHistory::default(),
                                        first_hour: hour,
                                        last_hour: hour,
                                        members: RunHistory::default(),
                                        leaders: RunHistory::default(),
                                        incomplete: Arc::new(Vec::new()),
                                    });
                                faction.last_hour = hour;
                                if rosters.factions[&row.id.to_string()]
                                    .iter()
                                    .next_back()
                                    .is_some_and(|roster| !roster.complete)
                                {
                                    mark_unobserved(
                                        Arc::make_mut(&mut faction.incomplete),
                                        hour,
                                        hour,
                                    );
                                }
                                if faction.name != name {
                                    faction.name = name.to_owned();
                                }
                            }
                            let mut ordered: Vec<_> = members.iter().collect();
                            ordered.sort_by_key(|member| (member.faction, member.ordinal));
                            // A member the links cannot name leaves that faction's roster
                            // incomplete this hour: no join, leave or founding is read there. A
                            // member of a faction the Factions table does not list belongs to
                            // no faction this reply names.
                            for member in ordered {
                                if !present.contains(&member.faction) {
                                    continue;
                                }
                                let Some(faction) = factions.get_mut(&member.faction) else {
                                    continue;
                                };
                                let tag = tag_of(member.country, &mut unmapped);
                                if tag.is_none() {
                                    mark_unobserved(
                                        Arc::make_mut(&mut faction.incomplete),
                                        hour,
                                        hour,
                                    );
                                }
                                if let Some(tag) = tag {
                                    extend_history(
                                        &mut member_open,
                                        &mut faction.members,
                                        (member.faction, tag.clone()),
                                        hour,
                                        || MembershipRun {
                                            tag: tag.clone(),
                                            first_hour: hour,
                                            last_hour: hour,
                                        },
                                    );
                                    // The member at order zero leads the faction (module notes).
                                    if member.ordinal == 0 {
                                        extend_history(
                                            &mut leader_open,
                                            &mut faction.leaders,
                                            (member.faction, tag.clone()),
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
                            mark_unobserved(
                                &mut Arc::make_mut(&mut unobserved).factions,
                                hour,
                                hour,
                            );
                        }
                    }
                    match decoded.complete::<tables::WarRelations>() {
                        Ok(rows) => {
                            // When each war started, and which side instigated it.
                            for row in rows.iter() {
                                let key = (row.first_country, row.second_country, row.start_hour);
                                if named_starts.contains(&key) {
                                    continue;
                                }
                                named_starts.insert(key);
                                let (Some(first), Some(second)) = (
                                    tag_at(row.first_country, row.start_hour),
                                    tag_at(row.second_country, row.start_hour),
                                ) else {
                                    // Before the branch it is not this branch's event at all.
                                    let refused = usize::from(
                                        branch_first.is_some_and(|first| row.start_hour >= first),
                                    );
                                    unattributed += refused;
                                    unattributed_war_starts += refused;
                                    continue;
                                };
                                let (attacking, defending) = if row.first_was_instigator != 0 {
                                    (first, second)
                                } else {
                                    (second, first)
                                };
                                let start = WarStart {
                                    hour: row.start_hour,
                                    attacking: attacking.clone(),
                                    defending: defending.clone(),
                                };
                                Arc::make_mut(&mut war_start_reads)
                                    .entry(start.clone())
                                    .or_insert(hour);
                                Arc::make_mut(&mut war_starts).insert(start);
                            }
                            // A war the links cannot name leaves the hour's wars incomplete.
                            let mut incomplete = false;
                            for row in rows
                                .iter()
                                .filter(|row| war_ongoing(row.start_hour, row.end_hour, hour))
                            {
                                let (Some(first), Some(second)) = (
                                    tag_of(row.first_country, &mut unmapped),
                                    tag_of(row.second_country, &mut unmapped),
                                ) else {
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
                                    &mut war_open,
                                    &mut wars,
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
                                mark_unobserved(
                                    &mut Arc::make_mut(&mut unobserved).wars,
                                    hour,
                                    hour,
                                );
                            }
                        }
                        Err(_) => {
                            missed = true;
                            mark_unobserved(&mut Arc::make_mut(&mut unobserved).wars, hour, hour);
                        }
                    }
                    // Capitulations as the engine dates them, read while the flag is set; the
                    // date stays with the event after recovery. The hour word is certified
                    // only while the flag is set (the fresh-save oracle lists the retained word
                    // of an inactive capitulation as a limitation), so a capitulation that began
                    // and ended between two readable samples is not recorded, never guessed
                    // from that word.
                    if let Ok(rows) = capitulation_rows {
                        for row in rows.iter().filter(|row| row.capitulated != 0) {
                            let key = (row.country, row.capitulated_hour);
                            if !named_capitulations.insert(key) {
                                continue;
                            }
                            let Some(tag) = tag_at(row.country, row.capitulated_hour) else {
                                let refused = usize::from(
                                    branch_first.is_some_and(|first| row.capitulated_hour >= first),
                                );
                                unattributed += refused;
                                unattributed_capitulations += refused;
                                continue;
                            };
                            let event = Capitulation {
                                hour: row.capitulated_hour,
                                tag: tag.clone(),
                            };
                            Arc::make_mut(&mut capitulation_reads)
                                .entry(event.clone())
                                .or_insert(hour);
                            Arc::make_mut(&mut capitulations).insert(event);
                        }
                    }
                    if missed {
                        incomplete += 1;
                    }
                }
            }
            Ok(())
        },
    )?;
    let continuation_unobserved = unobserved.clone();
    if let (Some((first, last)), previous) = (bounds, previous_hour) {
        let from = previous.map_or(first, |previous| previous.saturating_add(1));
        if from <= last {
            mark_all(Arc::make_mut(&mut unobserved), from, last);
        }
    }
    let mut warnings = Vec::new();
    if !invalid_names.is_empty() {
        warnings.push(format!(
            "{} faction names were not valid text and are not recorded.",
            invalid_names.len()
        ));
    }
    if !unmapped.is_empty() {
        warnings.push(format!(
            "{} recorded country references had no country identity and are left out of groups.",
            unmapped.len()
        ));
    }
    if incomplete > 0 {
        warnings.push(format!(
            "Group tables were incomplete at {incomplete} hours; memberships break there rather than carry over."
        ));
    }
    if unreadable_blocks > 0 || stopped > 0 {
        warnings.push(format!(
            "Part of this branch could not be read ({unreadable_blocks} unreadable blocks, {stopped} stopped reads); group history is missing there."
        ));
    }
    if capitulations_unread > 0 {
        warnings.push(format!(
            "Capitulations could not be read at {capitulations_unread} recorded hours; some may be missing."
        ));
    }
    if unattributed > 0 {
        warnings.push(format!(
            "{unattributed} war starts or capitulations were first seen after an unrecorded hour or a change of country; which countries they were is not known, so they are left out."
        ));
    }
    for (id, faction) in &mut factions {
        faction.names = names
            .get(id)
            .expect("a roster-known faction has a name read")
            .clone();
    }
    Ok(Scan {
        humans,
        factions,
        wars,
        war_starts,
        capitulations,
        identity_changes,
        war_start_reads,
        capitulation_reads,
        capitulations_observed,
        unattributed_war_starts,
        unattributed_capitulations,
        capitulation_unread_hours: capitulations_unread,
        identities,
        unobserved,
        warnings,
        rosters,
        history: Arc::default(),
        continuation: Continuation {
            blocks: replay
                .index
                .iter()
                .filter(|block| block.branch == branch)
                .count(),
            bounds,
            unobserved: continuation_unobserved,
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
            unreadable_blocks,
            stopped,
            previous_hour,
            tags,
            faction_tags,
            faction_ids,
            invalid_names,
            names,
            lexical,
            lexical_read,
        },
    })
}

#[cfg(test)]
type ScannedFactionView<'a> = (&'a str, i32, i32, Vec<MembershipRun>);

impl Scan {
    #[cfg(test)]
    pub fn humans(&self) -> Vec<MembershipRun> {
        self.humans.iter().cloned().collect()
    }
    #[cfg(test)]
    pub fn wars(&self) -> Vec<WarPairRun> {
        self.wars.iter().cloned().collect()
    }
    pub(super) fn unobserved(&self) -> &UnobservedHours {
        &self.unobserved
    }
    #[cfg(test)]
    pub fn faction_leaders(&self, id: u64) -> Option<Vec<MembershipRun>> {
        self.factions
            .get(&id)
            .map(|faction| faction.leaders.iter().cloned().collect())
    }
    /// Assignment identity before resolving main's literal/template name rule.
    pub(super) fn faction_name_at(&self, id: u64, hour: i32) -> Option<String> {
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
        self.factions
            .get(&id)?
            .names
            .iter()
            .rfind(|name| name.first_hour <= hour && hour <= name.last_hour)
    }
    pub(super) fn resolved_faction_name_at(
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
    #[cfg(test)]
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
    #[cfg(test)]
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
    #[cfg(test)]
    pub fn faction_incomplete(&self, id: u64) -> Option<&[HourSpan]> {
        self.factions
            .get(&id)
            .map(|faction| faction.incomplete.as_slice())
    }
    #[cfg(test)]
    pub fn war_starts(&self) -> Vec<WarStart> {
        self.war_starts.iter().cloned().collect()
    }
    #[cfg(test)]
    pub fn capitulations(&self) -> Vec<Capitulation> {
        self.capitulations.iter().cloned().collect()
    }
    #[cfg(test)]
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
