//! Shared, sparse faction observations and requested-key series reads.
use super::*;
use dashboard_base::{
    Source, catalogue,
    metadata::check_cancel,
    prepared::{
        frame_changes::FrameChanges,
        prefix::Prefix,
        series::{self, Accumulator, CatalogueWriter, Reader},
    },
    products::{check_reply_budget, faction_details::selectors},
};
use replay_data::rows::{RowCodec, TableId, TableSchema, tables};
use replay_frame::frame::{DecodedFrame, SharedView, TableView};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};
const NULL: i128 = i64::MIN as i128 - 1;
pub const CHANNELS: [(&str, TableId); 7] = [
    ("influence", TableId::FactionAccounts),
    ("rules", TableId::FactionRules),
    ("programmes", TableId::FactionPrograms),
    ("minimums", TableId::FactionMinimumUpgrades),
    ("members", TableId::FactionAccounts),
    ("doctrines", TableId::FactionDoctrine),
    ("manpower", TableId::FactionManpower),
];
pub const FAMILY: &str = "faction-state";
pub fn state_section(branch: u64) -> String {
    format!("faction-state-continuation-{branch}")
}
pub fn section(branch: u64) -> String {
    format!("faction-state-{branch}")
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
trait SelectionTable: TableSchema {
    const KIND: Kind;
    fn owner(row: &Self::Row) -> u64;
    fn ordinal(row: &Self::Row) -> u32;
}
macro_rules! selection_tables {
    ($($table:ident => $kind:ident),+ $(,)?) => { $(
        impl SelectionTable for tables::$table {
            const KIND: Kind = Kind::$kind;
            fn owner(row: &Self::Row) -> u64 { row.faction }
            fn ordinal(row: &Self::Row) -> u32 { row.ordinal }
        }
    )+ };
}
selection_tables!(
    FactionRules => Rule,
    FactionPrograms => Programme,
    FactionMinimumUpgrades => Minimum,
    FactionDoctrine => Doctrine,
    FactionManpower => Manpower,
);
struct Selected<'a, T: SelectionTable> {
    rows: &'a [T::Row],
    invalid: BTreeSet<usize>,
    failure: Option<String>,
}
impl<T: SelectionTable> Selected<'_, T> {
    fn available(&self) -> bool {
        self.failure.is_none() && self.invalid.is_empty()
    }
}
fn selected<T: SelectionTable>(
    frame: &DecodedFrame,
    faction: u64,
    owner_known: bool,
) -> Selected<'_, T> {
    let rows = frame.complete::<T>().map_err(|why| why.to_string());
    let failure = rows.as_ref().err().cloned().or_else(|| {
        (!owner_known).then(|| format!("Faction {} owner is unavailable", T::KIND.selection_name()))
    });
    let rows = rows
        .map(|rows| selectors::selected(rows, faction, T::owner))
        .unwrap_or_default();
    let invalid = rows
        .iter()
        .enumerate()
        .filter_map(|(ordinal, row)| (T::ordinal(row) as usize != ordinal).then_some(ordinal))
        .collect();
    Selected {
        rows,
        invalid,
        failure,
    }
}
const DIFF_TABLES: [TableId; 6] = [
    TableId::FactionAccounts,
    TableId::FactionRules,
    TableId::FactionPrograms,
    TableId::FactionMinimumUpgrades,
    TableId::FactionDoctrine,
    TableId::FactionManpower,
];
struct Previous {
    tables: [SharedView; DIFF_TABLES.len()],
}
impl Previous {
    fn read(frame: &DecodedFrame) -> Self {
        Self {
            tables: DIFF_TABLES.map(|table| frame.tables[table.index()].clone()),
        }
    }
    fn complete<T: TableSchema>(&self) -> Option<&[T::Row]> {
        let index = DIFF_TABLES.iter().position(|table| *table == T::ID)?;
        match &*self.tables[index] {
            TableView::Complete(rows) => T::borrow(rows),
            _ => None,
        }
    }
}
struct Changed {
    accounts: Option<BTreeSet<u128>>,
    owners: BTreeMap<Kind, Option<BTreeSet<u64>>>,
    refresh: bool,
    programme_operands: bool,
}
impl Changed {
    fn affects(&self, kind: Kind, faction: u64) -> bool {
        self.refresh
            || self.owners[&kind]
                .as_ref()
                .is_none_or(|keys| keys.contains(&faction))
    }
    fn supplying(
        &self,
        source: &Source,
        frame: &DecodedFrame,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<Box<DecodedFrame>>> {
        let changed = self.programme_operands
            || self.owners[&Kind::Programme]
                .as_ref()
                .is_none_or(|keys| !keys.is_empty());
        if changed
            && frame
                .complete::<tables::FactionPrograms>()
                .is_ok_and(|rows| !rows.is_empty())
        {
            dashboard_base::prepared::query::progressive::latest_read(
                source,
                frame.branch,
                frame.hour,
                TableId::ProjectPrograms,
                replay_data::cadence::STAGGER_PERIOD,
                selectors::PROGRAMME_TABLES,
                cancelled,
            )
        } else {
            Ok(None)
        }
    }
}
struct Joins {
    identities_changed: bool,
    identity_known: bool,
    membership_changed: bool,
    previous_members: BTreeMap<u32, Option<u64>>,
    joined: bool,
    accounts_joined: bool,
}
fn narrow_accounts(
    keys: &mut Option<BTreeSet<u128>>,
    previous: Option<&Previous>,
    frame: &DecodedFrame,
) {
    if let Some(keys) = keys
        && let Some(before) = previous.and_then(Previous::complete::<tables::FactionAccounts>)
        && let Ok(now) = frame.complete::<tables::FactionAccounts>()
    {
        keys.retain(|key| {
            if (*key >> 32) as u32 != 0 {
                return false;
            }
            let old = before
                .binary_search_by_key(key, RowCodec::key)
                .ok()
                .map(|index| &before[index]);
            let new = now
                .binary_search_by_key(key, RowCodec::key)
                .ok()
                .map(|index| &now[index]);
            match (old, new) {
                (Some(a), Some(b)) => {
                    a.country != b.country
                        || a.influence_faction != b.influence_faction
                        || selectors::influence_share(a) != selectors::influence_share(b)
                        || a.upgrade_groups != b.upgrade_groups
                        || a.upgrade_tokens != b.upgrade_tokens
                }
                _ => true,
            }
        });
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Choice {
    token: Option<String>,
    name: Option<String>,
    reason: Option<String>,
}
impl Choice {
    fn unknown(reason: String) -> Self {
        Self {
            reason: Some(reason),
            ..Default::default()
        }
    }
}
impl From<dashboard_base::products::faction_details::FactionRuleDetail> for Choice {
    fn from(detail: dashboard_base::products::faction_details::FactionRuleDetail) -> Self {
        Self {
            token: detail.token,
            name: detail.name,
            reason: detail.reason,
        }
    }
}
impl From<dashboard_base::products::faction_details::FactionUpgradeDetail> for Choice {
    fn from(detail: dashboard_base::products::faction_details::FactionUpgradeDetail) -> Self {
        Self {
            token: detail.token,
            name: detail.name,
            reason: detail.reason,
        }
    }
}
impl From<dashboard_base::products::faction_details::FactionProgramDetail> for Choice {
    fn from(detail: dashboard_base::products::faction_details::FactionProgramDetail) -> Self {
        Self {
            token: Some(detail.reference),
            name: detail.project,
            reason: detail.reason,
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
    #[serde(skip)]
    countries: selectors::Countries,
    #[serde(skip)]
    memberships: BTreeMap<u32, Option<u64>>,
    #[serde(skip)]
    ready: bool,
    unknown_keys: BTreeSet<u64>,
    unknown_choices: BTreeSet<u64>,
    #[serde(skip)]
    previous: Option<Previous>,
    #[cfg(test)]
    #[serde(skip)]
    pub(super) account_projections: usize,
    #[serde(skip)]
    publisher: CatalogueWriter,
}
impl Prepared {
    #[cfg(test)]
    pub fn country_rebuilds(&self) -> usize {
        self.countries.rebuilds
    }
    pub fn bytes(&self) -> usize {
        self.series.bytes()
    }
    pub fn quiet(&mut self, first: i32, last: i32, reads: [bool; 7]) -> io::Result<()> {
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
        let name =
            serde_json::to_string(&(faction.to_string(), kind, identity)).expect("string identity");
        let next = self.keys.len() as u64;
        let key = *self.keys.entry(name).or_insert(next);
        self.catalogue.lanes.entry(key).or_insert_with(|| Lane {
            faction,
            kind,
            identity: identity.into(),
            tag: tag.into(),
            holder,
            slot,
            group,
            choices: Vec::new(),
        });
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
        let choices = &mut self
            .catalogue
            .lanes
            .get_mut(&key)
            .expect("registered lane")
            .choices;
        let value = choices
            .iter()
            .position(|old| *old == choice)
            .unwrap_or_else(|| {
                choices.push(choice);
                choices.len() - 1
            });
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
        check_cancel(cancelled)?;
        let continues = self.last.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
        self.series.advance(frame.hour)?;
        let joins = self.refresh_joins(frame, changes, continues);
        let cat = context
            .content
            .as_ref()
            .and_then(|options| catalogue::catalogue(source, options).ok());
        let witnessed = selectors::owners(frame);
        let owners: BTreeSet<u64> = witnessed
            .iter()
            .copied()
            .chain(
                self.catalogue
                    .lanes
                    .values()
                    .filter(|lane| lane.faction != 0)
                    .map(|lane| lane.faction),
            )
            .collect();
        let dirty = self.changed(frame, changes, continues);
        let supplying = dirty.supplying(source, frame, cancelled)?;
        let accounts = self.changed_accounts(frame, &dirty, &joins);
        self.accounts(frame, accounts, joins.joined, !continues, cat.as_deref());
        for faction in owners {
            check_cancel(cancelled)?;
            let owner_known = witnessed.contains(&faction);
            let rules = selected::<tables::FactionRules>(frame, faction, owner_known);
            let programmes = selected::<tables::FactionPrograms>(frame, faction, owner_known);
            let minimums = selected::<tables::FactionMinimumUpgrades>(frame, faction, owner_known);
            let doctrines = selected::<tables::FactionDoctrine>(frame, faction, owner_known);
            let manpower = selected::<tables::FactionManpower>(frame, faction, owner_known);
            let accounts_known = owner_known && joins.joined && joins.accounts_joined;
            let available = [
                accounts_known,
                rules.available(),
                programmes.available(),
                minimums.available(),
                accounts_known,
                doctrines.available(),
                manpower.available(),
            ];
            for (index, (channel, table)) in CHANNELS.iter().enumerate() {
                self.known(
                    faction,
                    channel,
                    frame.hour,
                    available[index],
                    changes.slot_read[table.index()],
                    !continues,
                );
            }
            if dirty.affects(Kind::Rule, faction) || !owner_known {
                self.channel(frame, faction, &rules, !continues, |_, row| {
                    (
                        row.ordinal,
                        0,
                        selectors::rule(frame, faction, row, cat.as_deref()).into(),
                    )
                });
            }
            if dirty.programme_operands || dirty.affects(Kind::Programme, faction) || !owner_known {
                self.channel(frame, faction, &programmes, !continues, |_, row| {
                    (
                        row.ordinal,
                        0,
                        selectors::programme_detail(
                            supplying.as_deref(),
                            row.ordinal,
                            row.reference,
                            cat.as_deref(),
                        )
                        .into(),
                    )
                });
            }
            if dirty.affects(Kind::Minimum, faction) || !owner_known {
                self.channel(frame, faction, &minimums, !continues, |state, row| {
                    let detail = selectors::upgrade(frame, row.group, row.token, cat.as_deref());
                    state.group(frame, faction, row.group, cat.as_deref(), !continues);
                    (row.ordinal, row.group, detail.into())
                });
            }
            if dirty.affects(Kind::Doctrine, faction) || !owner_known {
                self.channel(frame, faction, &doctrines, !continues, |_, row| {
                    (
                        row.ordinal,
                        0,
                        selectors::doctrine(frame, faction, row, cat.as_deref()).into(),
                    )
                });
            }
            if dirty.affects(Kind::Manpower, faction)
                || joins.identities_changed
                || !joins.joined
                || !owner_known
            {
                self.manpower(frame, faction, !continues, owner_known);
            }
        }
        self.previous = Some(Previous::read(frame));
        self.ready = true;
        self.last = Some(frame.hour);
        Ok(())
    }
    fn refresh_joins(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        continues: bool,
    ) -> Joins {
        if !continues {
            self.previous_owners.clear();
        }
        let identity = self.countries.refresh(frame, changes, continues);
        let identities_changed = identity.as_ref().is_ok_and(|changed| *changed);
        let membership_changed = identities_changed
            || !self.ready
            || !continues
            || changes.rows_changed[TableId::FactionMembers.index()];
        let previous_members = if membership_changed {
            self.memberships.clone()
        } else {
            BTreeMap::new()
        };
        if membership_changed {
            self.memberships = selectors::membership(frame, &self.countries).unwrap_or_default();
        }
        let joined = identity.is_ok()
            && frame
                .complete::<tables::FactionMembers>()
                .is_ok_and(|rows| {
                    rows.iter()
                        .all(|row| self.countries.get(row.country).is_some())
                });
        if !joined {
            self.previous_owners.clear();
        }
        let accounts_joined = frame
            .complete::<tables::FactionAccounts>()
            .is_ok_and(|rows| {
                rows.iter()
                    .filter(|row| row.kind == 0)
                    .all(|row| self.countries.get(row.country).is_some())
            });
        Joins {
            identities_changed,
            identity_known: identity.is_ok(),
            membership_changed,
            previous_members,
            joined,
            accounts_joined,
        }
    }
    fn changed(&self, frame: &DecodedFrame, changes: &FrameChanges, continues: bool) -> Changed {
        macro_rules! dirty {
            ($table:ident) => {
                self.dirty::<tables::$table>(frame, changes, TableId::$table, continues)
            };
        }
        let mut accounts = dirty!(FactionAccounts);
        narrow_accounts(&mut accounts, self.previous.as_ref(), frame);
        let owner_keys = |keys: BTreeSet<u128>| {
            keys.into_iter()
                .map(|key| (key >> 32) as u64)
                .collect::<BTreeSet<_>>()
        };
        let owners = BTreeMap::from([
            (Kind::Rule, dirty!(FactionRules).map(owner_keys)),
            (Kind::Programme, dirty!(FactionPrograms).map(owner_keys)),
            (
                Kind::Minimum,
                dirty!(FactionMinimumUpgrades).map(owner_keys),
            ),
            (Kind::Doctrine, dirty!(FactionDoctrine).map(owner_keys)),
            (Kind::Manpower, dirty!(FactionManpower).map(owner_keys)),
        ]);
        let refresh =
            !self.ready || !continues || changes.rows_changed[TableId::LexicalTokens.index()];
        let programme_operands = refresh
            || changes.slot_read[TableId::ProjectPrograms.index()]
            || [TableId::ProjectPrograms, TableId::Projects]
                .iter()
                .any(|table| changes.rows_changed[table.index()]);
        Changed {
            accounts,
            owners,
            refresh,
            programme_operands,
        }
    }
    fn changed_accounts(
        &self,
        frame: &DecodedFrame,
        dirty: &Changed,
        joins: &Joins,
    ) -> BTreeSet<u32> {
        let full_accounts =
            dirty.refresh || joins.identities_changed || !joins.identity_known || !joins.joined;
        let mut accounts: BTreeSet<u32> = if !full_accounts && let Some(changed) = &dirty.accounts {
            changed
                .iter()
                .copied()
                .filter(|key| (*key >> 32) as u32 == 0)
                .map(|key| (key >> 64) as u32)
                .collect()
        } else {
            frame
                .complete::<tables::FactionAccounts>()
                .ok()
                .into_iter()
                .flatten()
                .filter(|row| row.kind == 0)
                .map(|row| row.account)
                .chain(
                    self.catalogue
                        .lanes
                        .values()
                        .filter(|lane| lane.kind == Kind::Share)
                        .map(|lane| lane.slot),
                )
                .collect()
        };
        if joins.membership_changed && !full_accounts {
            let changed_slots: BTreeSet<_> = joins
                .previous_members
                .keys()
                .chain(self.memberships.keys())
                .filter(|slot| joins.previous_members.get(slot) != self.memberships.get(slot))
                .copied()
                .collect();
            if let Ok(rows) = frame.complete::<tables::FactionAccounts>() {
                accounts.extend(
                    rows.iter()
                        .filter(|row| {
                            row.kind == 0
                                && self
                                    .countries
                                    .get(row.country)
                                    .is_some_and(|country| changed_slots.contains(&country.slot))
                        })
                        .map(|row| row.account),
                );
            }
        }
        accounts
    }
    fn channel<T: SelectionTable>(
        &mut self,
        frame: &DecodedFrame,
        faction: u64,
        selected: &Selected<'_, T>,
        starts: bool,
        mut map: impl FnMut(&mut Self, &T::Row) -> (u32, u32, Choice),
    ) {
        let values = selected
            .rows
            .iter()
            .enumerate()
            .map(|(ordinal, row)| {
                let (slot, group, mut choice) = map(self, row);
                if selected.invalid.contains(&ordinal) {
                    choice.reason = Some(format!(
                        "Faction {} ordinals are not contiguous",
                        T::KIND.selection_name()
                    ));
                }
                (slot, group, choice)
            })
            .collect();
        self.selections(
            faction,
            T::KIND,
            values,
            frame.hour,
            selected.failure.as_deref(),
            starts,
        );
    }
    fn dirty<T: replay_data::rows::TableSchema>(
        &self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        table: TableId,
        continues: bool,
    ) -> Option<BTreeSet<u128>> {
        if !self.ready || !continues {
            return None;
        }
        if let Some((changed, deleted)) = changes.rows(table) {
            return Some(
                changed
                    .iter()
                    .map(|row| row.key)
                    .chain(deleted.iter().copied())
                    .collect(),
            );
        }
        let before = self.previous.as_ref()?.complete::<T>()?;
        let now = frame.complete::<T>().ok()?;
        Some(dashboard_base::prepared::frame_changes::differing_keys(
            before, now,
        ))
    }
    fn group(
        &mut self,
        frame: &DecodedFrame,
        faction: u64,
        group: u32,
        cat: Option<&catalogue::Catalogue>,
        starts: bool,
    ) {
        let key = self.lane(
            faction,
            Kind::Group,
            &group.to_string(),
            "",
            None,
            (group, group),
        );
        let choice = match selectors::group_name(frame, group, cat) {
            Ok(name) => Choice {
                name,
                ..Default::default()
            },
            Err(reason) => Choice::unknown(reason),
        };
        self.choice(key, frame.hour, choice, starts);
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
    fn accounts(
        &mut self,
        frame: &DecodedFrame,
        dirty: BTreeSet<u32>,
        joined: bool,
        starts: bool,
        cat: Option<&catalogue::Catalogue>,
    ) {
        let rows = frame.complete::<tables::FactionAccounts>();
        for account in dirty {
            #[cfg(test)]
            {
                self.account_projections += 1;
            }
            let (mut shares, mut associations, mut upgrades) =
                (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
            let mut failure = rows
                .is_err()
                .then_some("Faction account inputs are unavailable");
            if let Ok(rows) = &rows
                && let Ok(index) =
                    rows.binary_search_by_key(&((account as u128) << 64), RowCodec::key)
            {
                let row = &rows[index];
                if let Some(country) = self.countries.get(row.country).cloned() {
                    let identity = format!(
                        "{}:{}:{}:{}",
                        row.account, country.slot, country.id, country.tag
                    );
                    if self
                        .previous_owners
                        .get(&account)
                        .is_some_and(|(before, _)| before != &identity)
                    {
                        self.previous_owners.remove(&account);
                    }
                    let key = self.lane(
                        0,
                        Kind::Share,
                        &identity,
                        &country.tag,
                        Some((country.slot, country.id)),
                        (row.account, 0),
                    );
                    shares.insert(key);
                    self.value(
                        key,
                        frame.hour,
                        selectors::influence_share(row) as i128,
                        starts,
                    );
                    let owner = self.memberships.get(&country.slot).copied();
                    // An explicit membership witness remains usable when an
                    // unrelated country's member reference cannot be joined.
                    if joined || matches!(owner, Some(Some(_))) {
                        let faction = match owner {
                            Some(Some(faction)) => {
                                self.previous_owners
                                    .insert(account, (identity.clone(), faction));
                                Some(faction)
                            }
                            Some(None) => {
                                failure = Some("Faction account country has ambiguous membership");
                                None
                            }
                            None if row.influence_faction != 0 => Some(row.influence_faction),
                            None => self
                                .previous_owners
                                .get(&account)
                                .map(|(_, faction)| *faction),
                        };
                        if let Some(faction) = faction {
                            let member = owner == Some(Some(faction));
                            let key = self.lane(
                                faction,
                                Kind::Association,
                                &identity,
                                &country.tag,
                                Some((country.slot, country.id)),
                                (row.account, 0),
                            );
                            associations.insert(key);
                            self.value(key, frame.hour, if member { 1 } else { 2 }, starts);
                            if member {
                                for (ordinal, (&group, &token)) in row
                                    .upgrade_groups
                                    .iter()
                                    .zip(&row.upgrade_tokens)
                                    .enumerate()
                                {
                                    let detail = selectors::upgrade(frame, group, token, cat);
                                    self.group(frame, faction, group, cat, starts);
                                    let key = self.lane(
                                        faction,
                                        Kind::Upgrade,
                                        &format!("{identity}:{ordinal}:{group}"),
                                        &country.tag,
                                        Some((country.slot, country.id)),
                                        (row.account, group),
                                    );
                                    upgrades.insert(key);
                                    self.choice(
                                        key,
                                        frame.hour,
                                        Choice {
                                            token: detail.token,
                                            name: detail.name,
                                            reason: detail.reason,
                                        },
                                        starts,
                                    );
                                }
                            }
                        }
                    } else {
                        failure = Some("Faction account membership inputs are unavailable");
                    }
                } else {
                    failure = Some("Faction account country identity is unavailable");
                }
            }
            if shares.is_empty() || failure.is_some() {
                self.previous_owners.remove(&account);
            }
            self.finish_group(
                group_key(account.into(), Kind::Share),
                shares,
                frame.hour,
                failure,
                false,
                starts,
            );
            self.finish_group(
                group_key(account.into(), Kind::Association),
                associations,
                frame.hour,
                failure,
                false,
                starts,
            );
            self.finish_group(
                group_key(account.into(), Kind::Upgrade),
                upgrades,
                frame.hour,
                failure,
                true,
                starts,
            );
        }
    }
    fn manpower(&mut self, frame: &DecodedFrame, faction: u64, starts: bool, owner_known: bool) {
        let rows = frame
            .complete::<tables::FactionManpower>()
            .map_err(|why| why.to_string());
        let mut now = BTreeSet::new();
        let mut diagnostics = BTreeSet::new();
        let mut failure = rows.as_ref().err().cloned().or_else(|| {
            (!owner_known).then(|| {
                "Faction manpower owner is unavailable; empty pool is not certified".into()
            })
        });
        let total_key = self.lane(faction, Kind::Total, "", "", None, (0, 0));
        if let Ok(rows) = rows
            && owner_known
        {
            let rows = selectors::selected(rows, faction, |row| row.faction);
            match selectors::manpower_total(rows) {
                Ok(total) => self.value(total_key, frame.hour, total as i128, starts),
                Err(why) => {
                    failure = Some(why);
                    self.value(total_key, frame.hour, NULL, starts);
                }
            }
            for row in rows {
                let identity = if row.country == 0 {
                    Some((
                        "reserved".into(),
                        selectors::country_tag(frame, 0).ok(),
                        None,
                    ))
                } else {
                    self.countries.get(row.country).map(|country| {
                        (
                            format!("{}:{}:{}", country.slot, country.id, country.tag),
                            Some(country.tag.clone()),
                            Some((country.slot, country.id)),
                        )
                    })
                };
                let Some((identity, Some(tag), holder)) = identity else {
                    let key = self.lane(
                        faction,
                        Kind::ManpowerDiagnostic,
                        &format!("country:{}", row.ordinal),
                        "",
                        None,
                        (row.ordinal, 0),
                    );
                    diagnostics.insert(key);
                    self.choice(
                        key,
                        frame.hour,
                        Choice::unknown(format!(
                            "Faction manpower entry {} country {} identity is unavailable",
                            row.ordinal, row.country
                        )),
                        starts,
                    );
                    failure = Some("Faction manpower country identity is unavailable".into());
                    continue;
                };
                let key = self.lane(
                    faction,
                    Kind::Manpower,
                    &format!("{}:{identity}", row.ordinal),
                    &tag,
                    holder,
                    (row.ordinal, 0),
                );
                now.insert(key);
                self.value(key, frame.hour, row.value as i128, starts);
            }
        } else {
            self.value(total_key, frame.hour, NULL, starts);
        }
        let key = self.lane(faction, Kind::ManpowerDiagnostic, "pool", "", None, (0, 0));
        diagnostics.insert(key);
        self.choice(
            key,
            frame.hour,
            failure
                .clone()
                .map_or_else(Choice::default, Choice::unknown),
            starts,
        );
        self.finish_group(
            group_key(faction, Kind::ManpowerDiagnostic),
            diagnostics,
            frame.hour,
            None,
            true,
            starts,
        );
        self.finish_group(
            group_key(faction, Kind::Manpower),
            now,
            frame.hour,
            failure.as_deref(),
            false,
            starts,
        );
    }
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.series.flush(writer, &section(branch), 0, cancelled)?;
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
    // A requested prefix can end inside a committed batch. These observations
    // never revise an earlier hour, so clip each requested run, not whole batches.
    let from = prefix.coverage.first_hour;
    let to = prefix.coverage.last_hour;
    let mode = match params {
        FactionStateParams::Influence { .. } => "influence",
        FactionStateParams::Rules { .. } => "rules",
        FactionStateParams::Manpower { .. } => "manpower",
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
