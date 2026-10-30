//! Shared, sparse faction observations and requested-key series reads.
use super::*;
use dashboard_base::{
    Source, catalogue,
    metadata::check_cancel,
    prepared::{
        facts::{Credit, MemoryBudget},
        frame_changes::FrameChanges,
        prefix::Prefix,
        query::{frames::Walk, progressive},
        series::{self, Accumulator, CatalogueWriter, Reader},
    },
    products::{
        check_reply_budget,
        faction_details::{self, FactionRuleDetail, GoalLabel, selectors},
    },
    triggers::inputs as reuse,
};
use replay_data::rows::{RowCodec, TableId, TableSchema, tables};
use replay_frame::frame::{DecodedFrame, SharedView, TableView};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, btree_map},
    hash::{DefaultHasher, Hash, Hasher},
    io,
    sync::{Arc, atomic::AtomicBool},
};
const NULL: i128 = i64::MIN as i128 - 1;
pub const CHANNELS: [(&str, TableId); 8] = [
    ("influence", TableId::FactionAccounts),
    ("rules", TableId::FactionRules),
    ("programmes", TableId::FactionPrograms),
    ("minimums", TableId::FactionMinimumUpgrades),
    ("members", TableId::FactionAccounts),
    ("doctrines", TableId::FactionDoctrine),
    ("manpower", TableId::FactionManpower),
    ("goals", TableId::FactionGoalEntries),
];
pub const FAMILY: &str = "faction-state";
pub fn state_section(branch: u64) -> String {
    format!("faction-state-continuation-{branch}")
}
pub fn section(branch: u64) -> String {
    format!("faction-state-{branch}")
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Kind {
    Share,
    Association,
    Upgrade,
    Known,
    Read,
    Rule,
    Programme,
    Minimum,
    Doctrine,
    Manpower,
    ManpowerDiagnostic,
    Total,
    Group,
    GoalList,
    GoalProgress,
    Roster,
}
impl Kind {
    fn selection_name(self) -> &'static str {
        match self {
            Self::Rule => "rule",
            Self::Programme => "programme",
            Self::Minimum => "minimum-upgrade",
            Self::Doctrine => "doctrine",
            Self::Manpower => "manpower",
            _ => unreachable!("Lane kind has no selected table"),
        }
    }
}
fn group_key(owner: u64, kind: Kind) -> String {
    serde_json::to_string(&(owner, kind)).expect("lane group identity")
}
/// One branch's programme supplying reads, each sought in rising hours.
pub struct Supplying<'a> {
    hours: Walk<'a>,
    frames: Walk<'a>,
}
impl<'a> Supplying<'a> {
    pub fn new(source: &'a Source, branch: u64) -> Self {
        Self {
            hours: Walk::new(source, branch),
            frames: Walk::new(source, branch),
        }
    }
    /// The hour of `frame`'s latest supplying read of programme operands.
    pub(super) fn read_hour(
        &mut self,
        frame: &DecodedFrame,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<i32>> {
        if self.hours.branch() != frame.branch {
            *self = Supplying::new(self.hours.source(), frame.branch);
        }
        progressive::walked_read_hour(
            &mut self.hours,
            frame.hour,
            TableId::ProjectPrograms,
            replay_data::cadence::STAGGER_PERIOD,
            cancelled,
        )
    }
    /// The supplying read at `hour`: the programme tables, and every scripted-text table
    /// when `wide`.
    pub(super) fn frame(
        &mut self,
        hour: i32,
        wide: bool,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<Arc<DecodedFrame>>> {
        let tables = if wide {
            selectors::programme_text_tables()
        } else {
            selectors::PROGRAMME_TABLES.to_vec()
        };
        Ok(progressive::walked_supplying_frame(
            &mut self.frames,
            hour,
            TableId::ProjectPrograms,
            &tables,
            cancelled,
        )?
        .map(Arc::from))
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct Choice {
    token: Option<String>,
    name: Option<String>,
    reason: Option<String>,
}
impl Choice {
    fn unknown(reason: String) -> Self {
        Self {
            reason: Some(projection::timeless(reason)),
            ..Default::default()
        }
    }
}
impl From<dashboard_base::products::faction_details::FactionRuleDetail> for Choice {
    fn from(detail: dashboard_base::products::faction_details::FactionRuleDetail) -> Self {
        Self {
            token: detail.token,
            name: detail.name,
            reason: detail.reason.map(projection::timeless),
        }
    }
}
impl From<dashboard_base::products::faction_details::FactionUpgradeDetail> for Choice {
    fn from(detail: dashboard_base::products::faction_details::FactionUpgradeDetail) -> Self {
        Self {
            token: detail.token,
            name: detail.name,
            reason: detail.reason.map(projection::timeless),
        }
    }
}
impl From<dashboard_base::products::faction_details::FactionProgramDetail> for Choice {
    fn from(detail: dashboard_base::products::faction_details::FactionProgramDetail) -> Self {
        Self {
            token: Some(detail.reference),
            name: detail.project,
            reason: detail.reason.map(projection::timeless),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Lane {
    faction: u64,
    kind: Kind,
    identity: String,
    tag: String,
    // Canonical slot and country id, independent of the selection slot.
    holder: Option<(u32, u32)>,
    slot: u32,
    group: u32,
    choices: Vec<Choice>,
}
mod goals;
mod projection;
use goals::{GoalList, GoalPages};
pub(super) use projection::{ACCOUNT_FIELDS, Edits, Frames, Projected, Runtime};
#[cfg(test)]
pub(super) use projection::timeless;
use projection::{
    AccountOp, AccountRow, Accounts, ChannelOp, FactionOps, Ghost, GoalsOp, ManpowerEntry,
    ManpowerOp, Owner,
};
#[cfg(test)]
mod columnar_parity;
#[derive(Clone, Default, Serialize, Deserialize)]
struct Catalogue {
    lanes: BTreeMap<u64, Lane>,
}
#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    series: Accumulator,
    keys: BTreeMap<String, u64>,
    catalogue: Catalogue,
    last: Option<i32>,
    // Retained association follows the native account and country.
    previous_owners: BTreeMap<u32, (String, u64)>,
    live: BTreeMap<String, BTreeSet<u64>>,
    read_keys: BTreeMap<u64, (usize, bool)>,
    unknown_keys: BTreeSet<u64>,
    unknown_choices: BTreeSet<u64>,
    #[serde(skip)]
    publisher: CatalogueWriter,
    /// Each faction's goal lists and labels, each published as its own page.
    goal_pages: GoalPages,
    #[serde(skip)]
    index: Index,
    /// The block fold's projection of the hours it observes.
    #[serde(skip)]
    pub(super) runtime: Runtime,
    /// The factions the last hour witnessed, and what one it lists nowhere reads: a
    /// projection sends them only when they change.
    witnessed: BTreeSet<u64>,
    ghost: Option<Ghost>,
    /// The factions the catalogue held that the last hour listed nowhere; None until read.
    #[serde(skip)]
    ghosts: Option<BTreeSet<u64>>,
    /// The catalogue may hold a new faction, or the witnessed changed, since `ghosts` were read.
    #[serde(skip)]
    grew: bool,
    /// Every account a share lane names; None until read.
    #[serde(skip)]
    share_slots: Option<BTreeSet<u32>>,
    /// Each account's share lane, and each faction's total and manpower lanes by entry and
    /// goal progress lanes by ordinal, as their last whole projection observed them: the
    /// lanes a projection's later values name.
    share_keys: BTreeMap<u32, u64>,
    manpower_keys: BTreeMap<u64, (u64, Vec<Option<u64>>)>,
    progress_keys: BTreeMap<u64, Vec<(u32, u64)>>,
    /// The channels' reads the last hour published; None until an hour does.
    #[serde(skip)]
    reads: Option<[bool; CHANNELS.len()]>,
}

fn digest(value: &impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

const LANE_ENTRY: usize = std::mem::size_of::<(u64, u64)>();
const CHOICE_ENTRY: usize = std::mem::size_of::<((u64, u64), usize)>();

/// An upper bound of the bytes a hash table able to hold `entries` allocates.
fn map_bytes(entries: usize, entry: usize) -> usize {
    let buckets = match entries {
        0 => return 0,
        1..4 => 4,
        4..8 => 8,
        _ => (entries.saturating_mul(8) / 7).next_power_of_two(),
    };
    buckets.saturating_mul(entry + 1).saturating_add(16)
}

/// An upper bound of a `BTreeSet<u64>`'s nodes: each non-root node holds at least 5 keys.
fn set_bytes(keys: usize) -> usize {
    const NODE: usize = 200;
    if keys == 0 {
        return 0;
    }
    NODE.saturating_mul(keys.div_ceil(5).saturating_add(2))
}

#[derive(Default)]
enum Held {
    #[default]
    Unbuilt,
    Held,
    /// Its memory was refused: every lookup reads the catalogue, as without the index.
    Dropped,
}

enum Position {
    At(usize),
    New,
    Unknown,
}

/// Lane keys, choice positions and owning factions, credited to the preparation's owner.
#[derive(Default)]
struct Index {
    held: Held,
    lanes: HashMap<u64, u64>,
    factions: BTreeSet<u64>,
    choices: HashMap<(u64, u64), usize>,
    credit: Option<Credit>,
}

impl Index {
    /// The tables' allocations now, by their capacity rather than their entries.
    fn held_bytes(&self) -> usize {
        map_bytes(self.lanes.capacity(), LANE_ENTRY)
            .saturating_add(map_bytes(self.choices.capacity(), CHOICE_ENTRY))
            .saturating_add(set_bytes(self.factions.len()))
    }

    /// Raises the credit to `bytes` before an allocation needs it; it never falls while held.
    fn admit(&mut self, bytes: usize) -> bool {
        if let Some(credit) = &mut self.credit {
            return credit.bytes() >= bytes || credit.resize(bytes).is_ok();
        }
        match MemoryBudget::preparation().admit(bytes) {
            Ok(credit) => {
                self.credit = Some(credit);
                true
            }
            Err(_) => false,
        }
    }

    /// Credits the old and new tables together when one more entry would grow `map`.
    fn admit_entry(&mut self, len: usize, capacity: usize, entry: usize) -> bool {
        let growth = if len < capacity {
            0
        } else {
            map_bytes(len + 1, entry)
        };
        self.admit(self.held_bytes().saturating_add(growth))
    }

    fn drop_all(&mut self) {
        *self = Self {
            held: Held::Dropped,
            ..Self::default()
        };
    }

    fn build(&mut self, catalogue: &Catalogue) {
        if !matches!(self.held, Held::Unbuilt) {
            return;
        }
        let lanes = catalogue.lanes.len();
        let choices = catalogue
            .lanes
            .values()
            .map(|lane| lane.choices.len())
            .sum();
        let bytes = map_bytes(lanes, LANE_ENTRY)
            .saturating_add(map_bytes(choices, CHOICE_ENTRY))
            .saturating_add(set_bytes(lanes));
        if !self.admit(bytes)
            || self.lanes.try_reserve(lanes).is_err()
            || self.choices.try_reserve(choices).is_err()
        {
            return self.drop_all();
        }
        for (&key, lane) in &catalogue.lanes {
            self.lanes
                .entry(digest(&(lane.faction, lane.kind, lane.identity.as_str())))
                .or_insert(key);
            if lane.faction != 0 {
                self.factions.insert(lane.faction);
            }
            for (index, choice) in lane.choices.iter().enumerate() {
                self.choices.entry((key, digest(choice))).or_insert(index);
            }
        }
        self.held = Held::Held;
    }

    fn lane(&self, digest: u64) -> Option<u64> {
        match self.held {
            Held::Held => self.lanes.get(&digest).copied(),
            _ => None,
        }
    }

    fn add_lane(&mut self, digest: u64, key: u64, faction: u64) {
        if !matches!(self.held, Held::Held) {
            return;
        }
        // A digest another lane holds adds no entry, so nothing grows.
        if !self.lanes.contains_key(&digest) {
            if !self.admit_entry(self.lanes.len(), self.lanes.capacity(), LANE_ENTRY)
                || self.lanes.try_reserve(1).is_err()
            {
                return self.drop_all();
            }
            self.lanes.insert(digest, key);
        }
        if faction != 0 && !self.factions.contains(&faction) {
            let bytes = self
                .held_bytes()
                .saturating_add(set_bytes(self.factions.len() + 1))
                .saturating_sub(set_bytes(self.factions.len()));
            if !self.admit(bytes) {
                return self.drop_all();
            }
            self.factions.insert(faction);
        }
    }

    fn factions(&self) -> Option<&BTreeSet<u64>> {
        matches!(self.held, Held::Held).then_some(&self.factions)
    }

    fn choice(&self, lane: u64, digest: u64) -> Position {
        match (&self.held, self.choices.get(&(lane, digest))) {
            (Held::Held, Some(&index)) => Position::At(index),
            (Held::Held, None) => Position::New,
            _ => Position::Unknown,
        }
    }

    fn add_choice(&mut self, lane: u64, digest: u64, index: usize) {
        if !matches!(self.held, Held::Held) || self.choices.contains_key(&(lane, digest)) {
            return;
        }
        if !self.admit_entry(self.choices.len(), self.choices.capacity(), CHOICE_ENTRY)
            || self.choices.try_reserve(1).is_err()
        {
            return self.drop_all();
        }
        self.choices.insert((lane, digest), index);
    }
}
impl Prepared {
    #[cfg(test)]
    pub fn country_rebuilds(&self) -> usize {
        self.runtime.country_rebuilds()
    }
    pub fn bytes(&self) -> usize {
        self.series.bytes() + self.goal_pages.pending()
    }
    /// Dynamic names need whole frames because kept frames may hold stale trigger-kit rows.
    pub fn needs_whole(
        &mut self,
        source: &Source,
        context: &LoadContext,
        frame: &DecodedFrame,
    ) -> bool {
        self.runtime.needs_whole(source, context, frame)
    }
    pub fn quiet(
        &mut self,
        first: i32,
        last: i32,
        reads: [bool; CHANNELS.len()],
    ) -> io::Result<()> {
        if self.runtime.holds_dynamic() {
            return Err(io::Error::other(
                "Faction dynamic names are observed on whole frames",
            ));
        }
        if self.last.and_then(|hour| hour.checked_add(1)) != Some(first) {
            return Err(io::Error::other(
                "Faction quiet span lacks its preceding observation",
            ));
        }
        self.series.advance(first)?;
        for (&key, &(index, known)) in &self.read_keys {
            self.series.observe_if_changed(
                key,
                first,
                Some(i128::from(known && reads[index])),
                false,
                false,
            );
        }
        self.series.advance_span(first, last)?;
        self.last = Some(last);
        self.reads = Some(reads);
        self.runtime.quiet(last);
        Ok(())
    }
    fn lane(
        &mut self,
        faction: u64,
        kind: Kind,
        identity: &str,
        tag: &str,
        holder: Option<(u32, u32)>,
        (slot, group): (u32, u32),
    ) -> u64 {
        self.index.build(&self.catalogue);
        let digest = digest(&(faction, kind, identity));
        // A digest two lanes share resolves the second through its JSON name.
        let indexed = self.index.lane(digest).filter(|key| {
            self.catalogue.lanes.get(key).is_some_and(|lane| {
                lane.faction == faction && lane.kind == kind && lane.identity == identity
            })
        });
        let key = match indexed {
            Some(key) => key,
            None => {
                let name = serde_json::to_string(&(faction.to_string(), kind, identity))
                    .expect("string identity");
                let next = self.keys.len() as u64;
                *self.keys.entry(name).or_insert(next)
            }
        };
        if let btree_map::Entry::Vacant(entry) = self.catalogue.lanes.entry(key) {
            // A faction the catalogue did not hold may be one an hour lists nowhere.
            self.grew |= faction != 0
                && self
                    .index
                    .factions()
                    .is_none_or(|factions| !factions.contains(&faction));
            self.index.add_lane(digest, key, faction);
            if kind == Kind::Share
                && let Some(slots) = &mut self.share_slots
            {
                slots.insert(slot);
            }
            entry.insert(Lane {
                faction,
                kind,
                identity: identity.into(),
                tag: tag.into(),
                holder,
                slot,
                group,
                choices: Vec::new(),
            });
        }
        key
    }
    fn value(&mut self, key: u64, hour: i32, value: i128, starts: bool) {
        let recovered = value != NULL && self.unknown_keys.remove(&key);
        if value == NULL {
            self.unknown_keys.insert(key);
        }
        self.series
            .observe_if_changed(key, hour, Some(value), starts || recovered, false);
    }
    fn choice(&mut self, key: u64, hour: i32, choice: Choice, starts: bool) {
        let unknown = choice.token.is_none() && choice.reason.is_some();
        let recovering = !unknown && self.unknown_choices.remove(&key);
        if unknown {
            self.unknown_choices.insert(key);
        }
        self.index.build(&self.catalogue);
        let digest = digest(&choice);
        let choices = &mut self
            .catalogue
            .lanes
            .get_mut(&key)
            .expect("registered lane")
            .choices;
        // A repeated choice keeps its first position; a shared digest falls back to the scan.
        let found = match self.index.choice(key, digest) {
            Position::At(index) if choices[index] == choice => Some(index),
            Position::New => None,
            _ => choices.iter().position(|old| *old == choice),
        };
        let value = match found {
            Some(index) => index,
            None => {
                choices.push(choice);
                let index = choices.len() - 1;
                self.index.add_choice(key, digest, index);
                index
            }
        };
        self.value(key, hour, value as i128, starts || recovering);
    }
    fn known(
        &mut self,
        faction: u64,
        channel: &str,
        hour: i32,
        known: bool,
        read: bool,
        starts: bool,
    ) {
        let key = self.lane(faction, Kind::Known, channel, "", None, (0, 0));
        self.value(key, hour, i128::from(known), starts);
        let key = self.lane(faction, Kind::Read, channel, "", None, (0, 0));
        self.value(key, hour, i128::from(known && read), starts);
        self.read_keys.insert(
            key,
            (
                CHANNELS
                    .iter()
                    .position(|(name, _)| *name == channel)
                    .expect("channel"),
                known,
            ),
        );
    }
    fn finish_group(
        &mut self,
        name: String,
        now: BTreeSet<u64>,
        hour: i32,
        unknown: Option<&str>,
        selections: bool,
        starts: bool,
    ) {
        let before = self.live.get(&name).cloned().unwrap_or_default();
        self.live.insert(
            name.clone(),
            if unknown.is_some() {
                before.union(&now).copied().collect()
            } else {
                now.clone()
            },
        );
        for key in before.difference(&now) {
            if selections {
                self.choice(
                    *key,
                    hour,
                    unknown.map_or_else(Choice::default, |why| Choice::unknown(why.to_owned())),
                    starts,
                );
            } else if unknown.is_some() {
                self.value(*key, hour, NULL, starts);
            } else {
                self.series.observe(*key, hour, None, starts);
            }
        }
        // Keep unavailable lanes live until a later read closes or recovers them.
    }
    pub fn observe(
        &mut self,
        source: &Source,
        context: &LoadContext,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let mut supplying = Supplying::new(source, frame.branch);
        self.observe_supplied(source, context, frame, changes, &mut supplying, cancelled)
    }
    /// [`Self::observe`] of one of the rising hours `supplying` reads programme operands for.
    pub fn observe_supplied(
        &mut self,
        source: &Source,
        context: &LoadContext,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        supplying: &mut Supplying<'_>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        check_cancel(cancelled)?;
        let cat = context
            .content
            .as_ref()
            .and_then(|options| catalogue::catalogue(source, options).ok());
        let projected = self.runtime.project(
            &projection::Whole::new(frame, supplying),
            changes,
            cat.as_deref(),
            cancelled,
        )?;
        self.apply(frame.hour, projected)
    }
    /// [`Self::observe`] of an hour the columnar walk projected ([`super::columnar`]).
    pub fn observe_projected(&mut self, hour: super::columnar::Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("faction emit", None);
        self.apply(hour.hour, hour.projected)
    }
    /// The ordered emit of one projected hour: lanes, choices and goal books numbered in the
    /// order the hour observes them, retained owners, and every faction the catalogue holds
    /// that the hour lists nowhere.
    pub(super) fn apply(&mut self, hour: i32, projected: Projected) -> io::Result<()> {
        let starts = self.last.and_then(|last| last.checked_add(1)) != Some(hour);
        self.series.advance(hour)?;
        let Projected {
            joined,
            roster,
            witnessed,
            ghost,
            accounts,
            factions,
            reads,
        } = projected;
        if starts || !joined {
            self.previous_owners.clear();
        }
        if let Some(witnessed) = witnessed {
            self.witnessed = Arc::unwrap_or_clone(witnessed);
            self.grew = true;
        }
        let renewed = ghost.is_some();
        if let Some(ghost) = ghost {
            self.ghost = Some(Arc::unwrap_or_clone(ghost));
        }
        self.index.build(&self.catalogue);
        // The hour's owners are the factions the catalogue held before its accounts.
        let before = if self.grew || self.ghosts.is_none() {
            self.grew = false;
            let mut now = self.catalogue_factions();
            now.retain(|faction| !self.witnessed.contains(faction));
            Some(self.ghosts.replace(now))
        } else {
            None
        };
        let ghosts = self.ghosts.as_ref().expect("factions listed nowhere");
        // One read as absent the hour before, for the same reasons, reads alike.
        let due: Vec<u64> = match before {
            Some(Some(before)) if !starts && !renewed => {
                ghosts.difference(&before).copied().collect()
            }
            None if !starts && !renewed => Vec::new(),
            _ => ghosts.iter().copied().collect(),
        };
        // Absence before a faction's first lane is proven only by a complete roster.
        let key = self.lane(0, Kind::Roster, "", "", None, (0, 0));
        self.value(key, hour, i128::from(roster), starts);
        self.accounts(hour, starts, accounts)?;
        // The factions whose every known lane the hour observes.
        let mut visited: BTreeSet<u64> = due.iter().copied().collect();
        let mut due = due.into_iter().peekable();
        for ops in factions {
            while let Some(faction) = due.next_if(|faction| *faction < ops.faction) {
                self.absent(hour, faction, starts);
            }
            if ops.known.is_some() {
                visited.insert(ops.faction);
            }
            self.faction(hour, ops, reads, starts)?;
        }
        for faction in due {
            self.absent(hour, faction, starts);
        }
        // Every other faction keeps what it knows; only the channels read anew move.
        let previous = self.reads.replace(reads);
        if !starts && previous != Some(reads) {
            for (&key, &(index, known)) in &self.read_keys {
                if previous.is_none_or(|previous| previous[index] != reads[index])
                    && !visited.contains(&self.catalogue.lanes[&key].faction)
                {
                    self.series.observe_if_changed(
                        key,
                        hour,
                        Some(i128::from(known && reads[index])),
                        false,
                        false,
                    );
                }
            }
        }
        self.last = Some(hour);
        Ok(())
    }
    fn catalogue_factions(&self) -> BTreeSet<u64> {
        match self.index.factions() {
            Some(factions) => factions.clone(),
            None => self
                .catalogue
                .lanes
                .values()
                .filter(|lane| lane.faction != 0)
                .map(|lane| lane.faction)
                .collect(),
        }
    }
    fn accounts(&mut self, hour: i32, starts: bool, accounts: Accounts) -> io::Result<()> {
        let failure = accounts
            .unavailable
            .then_some("Faction account inputs are unavailable");
        // A full hour also reads again every account a share lane names.
        let extra: Vec<u32> = if accounts.full {
            let present: BTreeSet<u32> = accounts.ops.iter().map(AccountOp::account).collect();
            let slots = self.share_slots.get_or_insert_with(|| {
                self.catalogue
                    .lanes
                    .values()
                    .filter(|lane| lane.kind == Kind::Share)
                    .map(|lane| lane.slot)
                    .collect()
            });
            slots
                .iter()
                .copied()
                .filter(|slot| !present.contains(slot))
                .collect()
        } else {
            Vec::new()
        };
        let mut extra = extra.into_iter().peekable();
        for op in accounts.ops {
            let next = op.account();
            while let Some(slot) = extra.next_if(|slot| *slot < next) {
                self.account(hour, starts, slot, None, failure);
            }
            match op {
                AccountOp::Whole { account, row } => {
                    self.account(hour, starts, account, row, failure)
                }
                AccountOp::Share { account, share } => {
                    let key = *self
                        .share_keys
                        .get(&account)
                        .ok_or_else(|| io::Error::other("A faction account share has no lane"))?;
                    self.value(key, hour, i128::from(share), starts);
                }
            }
        }
        for slot in extra {
            self.account(hour, starts, slot, None, failure);
        }
        Ok(())
    }
    fn account(
        &mut self,
        hour: i32,
        starts: bool,
        account: u32,
        row: Option<AccountRow>,
        mut failure: Option<&'static str>,
    ) {
        let (mut shares, mut associations, mut upgrades) =
            (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
        self.share_keys.remove(&account);
        match row {
            None => {}
            Some(AccountRow::Unidentified) => {
                failure = Some("Faction account country identity is unavailable");
            }
            Some(AccountRow::Identified {
                identity,
                tag,
                holder,
                share,
                owner,
            }) => {
                if self
                    .previous_owners
                    .get(&account)
                    .is_some_and(|(before, _)| before != &identity)
                {
                    self.previous_owners.remove(&account);
                }
                let key = self.lane(0, Kind::Share, &identity, &tag, Some(holder), (account, 0));
                shares.insert(key);
                self.value(key, hour, i128::from(share), starts);
                self.share_keys.insert(account, key);
                let (faction, member) = match owner {
                    Owner::Unjoined => {
                        failure = Some("Faction account membership inputs are unavailable");
                        (None, None)
                    }
                    Owner::Member {
                        faction,
                        upgrades: held,
                    } => {
                        self.previous_owners
                            .insert(account, (identity.clone(), faction));
                        (Some(faction), Some(held))
                    }
                    Owner::Ambiguous => {
                        failure = Some("Faction account country has ambiguous membership");
                        (None, None)
                    }
                    Owner::Influence(faction) => (Some(faction), None),
                    Owner::Retained => (
                        self.previous_owners
                            .get(&account)
                            .map(|(_, faction)| *faction),
                        None,
                    ),
                };
                if let Some(faction) = faction {
                    let key = self.lane(
                        faction,
                        Kind::Association,
                        &identity,
                        &tag,
                        Some(holder),
                        (account, 0),
                    );
                    associations.insert(key);
                    self.value(key, hour, if member.is_some() { 1 } else { 2 }, starts);
                    for (ordinal, upgrade) in member.into_iter().flatten().enumerate() {
                        self.group(hour, faction, upgrade.group, upgrade.name, starts);
                        let key = self.lane(
                            faction,
                            Kind::Upgrade,
                            &format!("{identity}:{ordinal}:{}", upgrade.group),
                            &tag,
                            Some(holder),
                            (account, upgrade.group),
                        );
                        upgrades.insert(key);
                        self.choice(key, hour, upgrade.choice, starts);
                    }
                }
            }
        }
        if shares.is_empty() || failure.is_some() {
            self.previous_owners.remove(&account);
        }
        self.finish_group(
            group_key(account.into(), Kind::Share),
            shares,
            hour,
            failure,
            false,
            starts,
        );
        self.finish_group(
            group_key(account.into(), Kind::Association),
            associations,
            hour,
            failure,
            false,
            starts,
        );
        self.finish_group(
            group_key(account.into(), Kind::Upgrade),
            upgrades,
            hour,
            failure,
            true,
            starts,
        );
    }
    fn faction(
        &mut self,
        hour: i32,
        ops: FactionOps,
        reads: [bool; CHANNELS.len()],
        starts: bool,
    ) -> io::Result<()> {
        let FactionOps {
            faction,
            known,
            channels,
        } = ops;
        if let Some(known) = known {
            for (index, (channel, _)) in CHANNELS.iter().enumerate() {
                self.known(faction, channel, hour, known[index], reads[index], starts);
            }
        }
        for channel in channels {
            match channel {
                ChannelOp::Selections {
                    kind,
                    values,
                    failure,
                } => self.selections(faction, kind, values, hour, failure.as_deref(), starts),
                ChannelOp::Group { group, choice } => {
                    self.group(hour, faction, group, choice, starts)
                }
                ChannelOp::Manpower(op) => self.manpower(hour, faction, op, starts),
                ChannelOp::ManpowerValues { total, values } => {
                    let (total_key, keys) = self
                        .manpower_keys
                        .get(&faction)
                        .ok_or_else(|| io::Error::other("Faction manpower values have no lanes"))?;
                    let total_key = *total_key;
                    let changed: Vec<(u64, i32)> = values
                        .into_iter()
                        .filter_map(|(at, value)| {
                            keys.get(at as usize)
                                .copied()
                                .flatten()
                                .map(|key| (key, value))
                        })
                        .collect();
                    self.value(total_key, hour, total, starts);
                    for (key, value) in changed {
                        self.value(key, hour, i128::from(value), starts);
                    }
                }
                ChannelOp::Goals(op) => self.goals(hour, faction, op, starts),
                ChannelOp::GoalProgress(values) => {
                    let keys = self
                        .progress_keys
                        .get(&faction)
                        .ok_or_else(|| io::Error::other("Faction goal progress has no lanes"))?;
                    let changed: Vec<(u64, i64)> = values
                        .into_iter()
                        .filter_map(|(ordinal, raw)| {
                            keys.iter()
                                .find(|(held, _)| *held == ordinal)
                                .map(|&(_, key)| (key, raw))
                        })
                        .collect();
                    for (key, raw) in changed {
                        self.value(key, hour, i128::from(raw), starts);
                    }
                }
            }
        }
        Ok(())
    }
    /// A faction the catalogue holds that the hour lists nowhere: it has no rows anywhere.
    fn absent(&mut self, hour: i32, faction: u64, starts: bool) {
        let Some(ghost) = self.ghost.clone() else {
            return;
        };
        for (channel, _) in CHANNELS {
            self.known(faction, channel, hour, false, false, starts);
        }
        for (kind, failure) in [Kind::Rule, Kind::Programme, Kind::Minimum, Kind::Doctrine]
            .into_iter()
            .zip(&ghost.selections)
        {
            self.selections(
                faction,
                kind,
                Vec::new(),
                hour,
                Some(failure.as_str()),
                starts,
            );
        }
        self.manpower(
            hour,
            faction,
            ManpowerOp {
                total: NULL,
                entries: Vec::new(),
                failure: Some(ghost.manpower.clone()),
            },
            starts,
        );
        self.goals(
            hour,
            faction,
            GoalsOp {
                absent: ghost.goals.absent,
                reason: ghost.goals.reason.clone(),
                entries: Vec::new(),
            },
            starts,
        );
    }
    fn group(&mut self, hour: i32, faction: u64, group: u32, choice: Choice, starts: bool) {
        let key = self.lane(
            faction,
            Kind::Group,
            &group.to_string(),
            "",
            None,
            (group, group),
        );
        self.choice(key, hour, choice, starts);
    }
    fn selections(
        &mut self,
        faction: u64,
        kind: Kind,
        values: Vec<(u32, u32, Choice)>,
        hour: i32,
        failure: Option<&str>,
        starts: bool,
    ) {
        let mut now = BTreeSet::new();
        for (slot, group, choice) in values {
            let key = self.lane(
                faction,
                kind,
                &format!("{slot}:{group}"),
                "",
                None,
                (slot, group),
            );
            now.insert(key);
            self.choice(key, hour, choice, starts);
        }
        self.finish_group(group_key(faction, kind), now, hour, failure, true, starts);
    }
    fn manpower(&mut self, hour: i32, faction: u64, op: ManpowerOp, starts: bool) {
        let total_key = self.lane(faction, Kind::Total, "", "", None, (0, 0));
        self.value(total_key, hour, op.total, starts);
        let mut now = BTreeSet::new();
        let mut diagnostics = BTreeSet::new();
        let mut keys = Vec::with_capacity(op.entries.len());
        for entry in op.entries {
            match entry {
                ManpowerEntry::Unidentified { ordinal, reason } => {
                    let key = self.lane(
                        faction,
                        Kind::ManpowerDiagnostic,
                        &format!("country:{ordinal}"),
                        "",
                        None,
                        (ordinal, 0),
                    );
                    diagnostics.insert(key);
                    self.choice(key, hour, Choice::unknown(reason), starts);
                    keys.push(None);
                }
                ManpowerEntry::Member {
                    ordinal,
                    identity,
                    tag,
                    holder,
                    value,
                } => {
                    let key = self.lane(
                        faction,
                        Kind::Manpower,
                        &identity,
                        &tag,
                        holder,
                        (ordinal, 0),
                    );
                    now.insert(key);
                    self.value(key, hour, i128::from(value), starts);
                    keys.push(Some(key));
                }
            }
        }
        let key = self.lane(faction, Kind::ManpowerDiagnostic, "pool", "", None, (0, 0));
        diagnostics.insert(key);
        self.choice(
            key,
            hour,
            op.failure
                .clone()
                .map_or_else(Choice::default, Choice::unknown),
            starts,
        );
        self.finish_group(
            group_key(faction, Kind::ManpowerDiagnostic),
            diagnostics,
            hour,
            None,
            true,
            starts,
        );
        self.finish_group(
            group_key(faction, Kind::Manpower),
            now,
            hour,
            op.failure.as_deref(),
            false,
            starts,
        );
        self.manpower_keys.insert(faction, (total_key, keys));
    }
    fn goals(&mut self, hour: i32, faction: u64, op: GoalsOp, starts: bool) {
        let key = self.lane(faction, Kind::GoalList, "", "", None, (0, 0));
        let mut list = GoalList {
            absent: op.absent,
            reason: op.reason,
            ..Default::default()
        };
        let mut progress = Vec::new();
        for entry in op.entries {
            let label = self.goal_pages.label(faction, entry.label);
            match entry.kind {
                0 => {
                    list.goals.push((entry.ordinal, label, entry.status));
                    progress.push((entry.ordinal, entry.progress));
                }
                1 => list.completed.push(label),
                _ => list.cancelled.push(label),
            }
        }
        let value = self.goal_pages.list(faction, list);
        self.value(key, hour, value as i128, starts);
        let mut now = BTreeSet::new();
        let mut keys = Vec::with_capacity(progress.len());
        for (ordinal, raw) in progress {
            let key = self.lane(
                faction,
                Kind::GoalProgress,
                &ordinal.to_string(),
                "",
                None,
                (ordinal, 0),
            );
            now.insert(key);
            self.value(key, hour, i128::from(raw), starts);
            keys.push((ordinal, key));
        }
        self.finish_group(
            group_key(faction, Kind::GoalProgress),
            now,
            hour,
            None,
            false,
            starts,
        );
        self.progress_keys.insert(faction, keys);
    }
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.series.flush(writer, &section(branch), 0, cancelled)?;
        self.goal_pages.flush(writer, branch, cancelled)?;
        self.publisher
            .write(writer, FAMILY, branch, &self.keys, &self.catalogue)
    }
    pub fn save(&self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 1)?;
        serde_json::to_writer(&mut encoder, self)?;
        writer.write_at(&state_section(branch), 0, encoder.finish()?.as_slice())?;
        Ok(())
    }
}

fn spans(runs: &[series::Run], value: i128) -> Vec<HourSpan> {
    let mut result: Vec<HourSpan> = Vec::new();
    for run in runs.iter().filter(|run| run.value == value) {
        if let Some(last) = result.last_mut()
            && last.last_hour.checked_add(1) == Some(run.first_hour)
            && !run.break_before
        {
            last.last_hour = run.last_hour;
        } else {
            result.push(HourSpan {
                first_hour: run.first_hour,
                last_hour: run.last_hour,
            });
        }
    }
    result
}
fn missing(first: i32, last: i32, known: &[HourSpan]) -> Vec<HourSpan> {
    let mut start = i64::from(first);
    let mut result = Vec::new();
    for span in known {
        if start < i64::from(span.first_hour) {
            result.push(HourSpan {
                first_hour: start as i32,
                last_hour: span.first_hour - 1,
            });
        }
        start = start.max(i64::from(span.last_hour) + 1);
    }
    if start <= i64::from(last) {
        result.push(HourSpan {
            first_hour: start as i32,
            last_hour: last,
        });
    }
    result
}
fn choices(lane: &Lane, runs: Vec<series::Run>) -> io::Result<Vec<FactionSelectionRun>> {
    runs.into_iter()
        .map(|run| {
            let choice = usize::try_from(run.value)
                .ok()
                .and_then(|value| lane.choices.get(value))
                .ok_or_else(|| {
                    dashboard_base::invalid("Faction selection variant is unavailable")
                })?;
            Ok(FactionSelectionRun {
                first_hour: run.first_hour,
                last_hour: run.last_hour,
                token: choice.token.clone(),
                name: choice.name.clone(),
                reason: choice.reason.clone(),
                starts_segment: run.break_before,
            })
        })
        .collect()
}
fn manpower_runs(runs: Vec<series::Run>) -> Vec<FactionManpowerRun> {
    runs.into_iter()
        .map(|run| FactionManpowerRun {
            first_hour: run.first_hour,
            last_hour: run.last_hour,
            value: i32::try_from(run.value).ok(),
            starts_segment: run.break_before,
        })
        .collect()
}
pub fn query(
    source: &Source,
    params: &FactionStateParams,
    context: &LoadContext,
    store: &store::Store,
    key: &store::SourceKey,
    prefix: Prefix,
    cancelled: &AtomicBool,
) -> io::Result<FactionStateReply> {
    check_cancel(cancelled)?;
    FactionState::validate(params).map_err(dashboard_base::invalid)?;
    let (branch, faction) = params.identity();
    let branch: u64 = branch
        .parse()
        .map_err(|_| dashboard_base::invalid("Invalid faction state branch"))?;
    let faction: u64 = faction
        .parse()
        .map_err(|_| dashboard_base::invalid("Invalid faction state faction"))?;
    let (keys, catalogue) =
        series::read_catalogue::<Catalogue>(store, key, FAMILY, branch)?.unwrap_or_default();
    let mut reader = Reader::new(store, key, section(branch), cancelled)?;
    if let FactionStateParams::Goals {
        first_hour,
        last_hour,
        ..
    } = params
    {
        let at = goals::Asked {
            branch,
            faction,
            first_hour: *first_hour,
            last_hour: *last_hour,
        };
        let mut reply = FactionStateReply::Goals(goals::query(
            source,
            context,
            (store, key),
            (&keys, &catalogue, &mut reader),
            at,
            prefix,
            cancelled,
        )?);
        if let Some(options) = &context.content {
            dashboard_base::content::check_replay(
                source.replay_ref(),
                options.allow_unverified,
                dashboard_base::products::ReplyWarnings::warnings_mut(&mut reply),
            )?;
        }
        check_reply_budget(&reply)?;
        return Ok(reply);
    }
    // A requested prefix can end inside a committed batch. These observations
    // never revise an earlier hour, so clip each requested run, not whole batches.
    let from = prefix.coverage.first_hour;
    let to = prefix.coverage.last_hour;
    let mode = match params {
        FactionStateParams::Influence { .. } => "influence",
        FactionStateParams::Rules { .. } => "rules",
        FactionStateParams::Manpower { .. } => "manpower",
        FactionStateParams::Goals { .. } => unreachable!("goal views are answered above"),
    };
    let wanted: &[&str] = match mode {
        "influence" => &["influence"],
        "manpower" => &["manpower"],
        _ => &["rules", "programmes", "minimums", "members", "doctrines"],
    };
    let mut channels = Vec::new();
    let mut warnings = BTreeMap::<String, u64>::new();
    for &channel in wanted {
        let read_spans = |reader: &mut Reader<'_>, kind: Kind| -> io::Result<Vec<HourSpan>> {
            let name = serde_json::to_string(&(faction.to_string(), kind, channel))?;
            Ok(match keys.get(&name) {
                Some(key) => spans(&reader.runs(*key, from, to)?, 1),
                None => Vec::new(),
            })
        };
        let known = read_spans(&mut reader, Kind::Known)?;
        let read = read_spans(&mut reader, Kind::Read)?;
        let unknown = missing(from, to, &known);
        if !unknown.is_empty() {
            warnings.insert(
                format!("Faction {channel} inputs are unobserved"),
                unknown.len() as u64,
            );
        }
        channels.push(FactionChannelCoverage {
            channel: channel.into(),
            known,
            read,
        });
    }
    let mut unobserved = Vec::<HourSpan>::new();
    let mut unknown: Vec<_> = channels
        .iter()
        .flat_map(|channel| missing(from, to, &channel.known))
        .collect();
    unknown.sort_by_key(|span| span.first_hour);
    for span in unknown {
        if let Some(last) = unobserved.last_mut()
            && i64::from(last.last_hour) + 1 >= i64::from(span.first_hour)
        {
            last.last_hour = last.last_hour.max(span.last_hour);
        } else {
            unobserved.push(span);
        }
    }
    let mut influence = FactionInfluenceReply {
        accounts: Vec::new(),
        unobserved: unobserved.clone(),
        pending: prefix.pending.clone(),
        coverage: prefix.coverage.clone(),
        channels: channels.clone(),
        warnings: Vec::new(),
    };
    let mut rules = FactionRulesReply {
        rules: Vec::new(),
        programmes: Vec::new(),
        minimums: Vec::new(),
        members: Vec::new(),
        doctrines: Vec::new(),
        groups: Vec::new(),
        unobserved: unobserved.clone(),
        pending: prefix.pending.clone(),
        coverage: prefix.coverage.clone(),
        channels: channels.clone(),
        warnings: Vec::new(),
    };
    let mut manpower = FactionManpowerReply {
        members: Vec::new(),
        total: Vec::new(),
        unobserved,
        pending: prefix.pending,
        coverage: prefix.coverage,
        channels,
        warnings: Vec::new(),
    };
    let mut groups = BTreeSet::new();
    for (&id, lane) in catalogue
        .lanes
        .iter()
        .filter(|(_, lane)| lane.faction == faction)
    {
        check_cancel(cancelled)?;
        match (mode, lane.kind) {
            ("influence", Kind::Association) => {
                let association = reader.runs(id, from, to)?;
                let share = serde_json::to_string(&("0", Kind::Share, &lane.identity))?;
                let mut runs = Vec::new();
                if let Some(key) = keys.get(&share) {
                    let shares = reader.runs(*key, from, to)?;
                    let mut at = 0;
                    for association in &association {
                        if !matches!(association.value, 1 | 2 | NULL) {
                            continue;
                        }
                        while at < shares.len() && shares[at].last_hour < association.first_hour {
                            at += 1;
                        }
                        for share in shares[at..]
                            .iter()
                            .take_while(|share| share.first_hour <= association.last_hour)
                        {
                            runs.push(FactionShareRun {
                                first_hour: share.first_hour.max(association.first_hour),
                                last_hour: share.last_hour.min(association.last_hour),
                                share_raw: (share.value != NULL && association.value != NULL)
                                    .then(|| share.value.to_string()),
                                starts_segment: (share.break_before
                                    && share.first_hour >= association.first_hour)
                                    || (association.break_before
                                        && association.first_hour >= share.first_hour),
                            });
                        }
                    }
                }
                let mut coalesced: Vec<FactionShareRun> = Vec::new();
                for run in runs {
                    if let Some(last) = coalesced.last_mut()
                        && last.last_hour.checked_add(1) == Some(run.first_hour)
                        && last.share_raw == run.share_raw
                        && !run.starts_segment
                    {
                        last.last_hour = run.last_hour;
                    } else {
                        coalesced.push(run);
                    }
                }
                if !association.is_empty() {
                    influence.accounts.push(FactionInfluenceAccount {
                        key: lane.identity.clone(),
                        tag: lane.tag.clone(),
                        slot: lane.holder.map(|(slot, _)| slot),
                        country_id: lane.holder.map(|(_, id)| id),
                        runs: coalesced,
                        member: spans(&association, 1),
                        retained: spans(&association, 2),
                    });
                }
            }
            (
                "rules",
                Kind::Rule | Kind::Programme | Kind::Minimum | Kind::Doctrine | Kind::Upgrade,
            ) => {
                let runs = choices(lane, reader.runs(id, from, to)?)?;
                if runs.is_empty() {
                    continue;
                }
                for run in &runs {
                    if let Some(reason) = &run.reason {
                        *warnings.entry(reason.clone()).or_default() += 1;
                    }
                }
                match lane.kind {
                    Kind::Rule => rules.rules.push(FactionSelectionLane {
                        slot: lane.slot,
                        runs,
                    }),
                    Kind::Programme => rules.programmes.push(FactionSelectionLane {
                        slot: lane.slot,
                        runs,
                    }),
                    Kind::Doctrine => rules.doctrines.push(FactionSelectionLane {
                        slot: lane.slot,
                        runs,
                    }),
                    Kind::Minimum => {
                        groups.insert(lane.group);
                        rules.minimums.push(FactionMinimumLane {
                            group: lane.group,
                            runs,
                        });
                    }
                    Kind::Upgrade => {
                        groups.insert(lane.group);
                        rules.members.push(FactionMemberUpgradeLane {
                            key: lane.identity.clone(),
                            tag: lane.tag.clone(),
                            slot: lane.holder.map(|(slot, _)| slot),
                            country_id: lane.holder.map(|(_, id)| id),
                            group: lane.group,
                            runs,
                        });
                    }
                    _ => unreachable!(),
                }
            }
            ("manpower", Kind::Manpower) => {
                let runs = manpower_runs(reader.runs(id, from, to)?);
                if !runs.is_empty() {
                    manpower.members.push(FactionManpowerMember {
                        key: lane.identity.clone(),
                        tag: lane.tag.clone(),
                        slot: lane.holder.map(|(slot, _)| slot),
                        country_id: lane.holder.map(|(_, id)| id),
                        runs,
                    });
                }
            }
            ("manpower", Kind::Total) => manpower.total = manpower_runs(reader.runs(id, from, to)?),
            ("manpower", Kind::ManpowerDiagnostic) => {
                for run in choices(lane, reader.runs(id, from, to)?)? {
                    if let Some(reason) = run.reason {
                        *warnings.entry(reason).or_default() += 1;
                    }
                }
            }
            _ => {}
        }
    }
    for group in groups {
        let identity =
            serde_json::to_string(&(faction.to_string(), Kind::Group, group.to_string()))?;
        let mut name = None;
        if let Some(key) = keys.get(&identity)
            && let Some(lane) = catalogue.lanes.get(key)
        {
            for run in choices(lane, reader.runs(*key, from, to)?)? {
                name = run.name;
                if let Some(reason) = run.reason {
                    *warnings.entry(reason).or_default() += 1;
                }
            }
        }
        rules.groups.push(FactionUpgradeGroup { group, name });
    }
    rules.rules.sort_by_key(|lane| lane.slot);
    rules.programmes.sort_by_key(|lane| lane.slot);
    rules.doctrines.sort_by_key(|lane| lane.slot);
    let mut reply = match params {
        FactionStateParams::Influence { .. } => FactionStateReply::Influence(influence),
        FactionStateParams::Rules { .. } => FactionStateReply::Rules(rules),
        FactionStateParams::Manpower { .. } => FactionStateReply::Manpower(manpower),
        FactionStateParams::Goals { .. } => unreachable!("goal views are answered above"),
    };
    dashboard_base::products::ReplyWarnings::warnings_mut(&mut reply).extend(
        warnings
            .into_iter()
            .map(|(reason, count)| format!("{count} faction facts: {reason}")),
    );
    if let Some(options) = &context.content {
        dashboard_base::content::check_replay(
            source.replay_ref(),
            options.allow_unverified,
            dashboard_base::products::ReplyWarnings::warnings_mut(&mut reply),
        )?;
    }
    check_reply_budget(&reply)?;
    Ok(reply)
}
