//! What an hour changed in the factions' inputs, read from its frame. The ordered emit
//! ([`super::super::emit`]) numbers its lanes, choices and goal books, resolves retained owners,
//! and reads every faction the publication holds but the hour lists nowhere.
use super::*;
use dashboard_base::prepared::frame_changes::field_mask;
use dashboard_base::products::faction_details::ViewerText;
use dashboard_world::{Mark, Viewed};
use replay_data::rows::{DeltaRow, FactionAccountRow, TABLE_COUNT};
use replay_frame::frame::{HourFrame, OperandStatus, Rowset};
#[cfg(test)]
use std::cell::RefCell;
use std::{borrow::Cow, cell::OnceCell, mem::size_of, sync::LazyLock};

/// What an hour changed since the hour before, as the projection reads it.
pub(in crate::products::faction_state) trait Edits {
    /// Which tables changed, and which the hour read.
    fn changes(&self) -> &FrameChanges;
    /// `table`'s changed rows, each with the fields it changed, and its deleted keys; None when
    /// any row may have changed, as [`FrameChanges::rows`].
    fn rows(&self, table: TableId) -> Option<(&[DeltaRow], &[u128])>;
    /// The changes as a trigger evaluation's inputs read them.
    fn tables(&self) -> &dyn reuse::TableChanges;
    /// Whether `table`'s status may differ from the hour before's.
    fn restated(&self, table: TableId) -> bool;
}

impl Edits for FrameChanges {
    fn changes(&self) -> &FrameChanges {
        self
    }
    fn rows(&self, table: TableId) -> Option<(&[DeltaRow], &[u128])> {
        FrameChanges::rows(self, table)
    }
    fn tables(&self) -> &dyn reuse::TableChanges {
        self
    }
    fn restated(&self, table: TableId) -> bool {
        self.rows_changed[table.index()]
    }
}

/// The hour's frames: the family's own tables, the tables a dynamic text reads, and the
/// programmes' supplying read.
pub(in crate::products::faction_state) trait Frames {
    /// The family's own tables, and how they tell a table unchanged since an hour that held it.
    fn own(&self) -> Viewed<'_>;
    /// The own tables and every table a dynamic text may read.
    fn whole(&self) -> io::Result<HourFrame<'_>>;
    /// The own tables and `tables`, at least: what a held text's reads are read again on. The
    /// hour's first ask decides it.
    fn reading(&self, tables: &[TableId]) -> io::Result<HourFrame<'_>>;
    /// The hour of the programmes' supplying read: the latest hour that read them within a
    /// stagger period, their table complete at every hour since.
    fn read_hour(&self, cancelled: &AtomicBool) -> io::Result<Option<i32>>;
    /// The supplying read at `hour`: the programme tables, with every scripted-text table
    /// when `wide`.
    fn supplying(
        &self,
        hour: i32,
        wide: bool,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<Supply>>;
    /// Tests: the recording each supplying read is sought in alone.
    #[cfg(test)]
    fn source(&self) -> Option<&Source> {
        None
    }
}

/// The walk's lexer folds, which its frames' own, whole and reading tables share.
fn lexicon<'f>(frames: &'f dyn Frames) -> Option<&'f dashboard_world::Lexicon> {
    frames.own().lexicon
}

/// A frame holding every table the hour reads, its supplying reads walked in the recording.
#[cfg(test)]
pub(in crate::products::faction_state) struct Whole<'f, 's, 'a> {
    pub(in crate::products::faction_state) frame: &'f DecodedFrame,
    pub(in crate::products::faction_state) walks: RefCell<&'s mut Supplying<'a>>,
}

#[cfg(test)]
impl<'f, 's, 'a> Whole<'f, 's, 'a> {
    pub(in crate::products::faction_state) fn new(frame: &'f DecodedFrame, walks: &'s mut Supplying<'a>) -> Self {
        Self {
            frame,
            walks: RefCell::new(walks),
        }
    }
}

/// A programmes' supplying read: the hour's own tables, which hold every table it reads, or a
/// frame read for it.
#[derive(Clone)]
pub(in crate::products::faction_state) enum Supply {
    Own,
    Read(Arc<DecodedFrame>),
}

#[cfg(test)]
impl Frames for Whole<'_, '_, '_> {
    fn own(&self) -> Viewed<'_> {
        Viewed::decoded(self.frame)
    }
    fn whole(&self) -> io::Result<HourFrame<'_>> {
        Ok(HourFrame::from(self.frame))
    }
    fn reading(&self, _: &[TableId]) -> io::Result<HourFrame<'_>> {
        Ok(HourFrame::from(self.frame))
    }
    fn read_hour(&self, cancelled: &AtomicBool) -> io::Result<Option<i32>> {
        self.walks
            .borrow_mut()
            .read_hour((self.frame.branch, self.frame.hour), cancelled)
    }
    fn supplying(
        &self,
        hour: i32,
        wide: bool,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<Supply>> {
        Ok(self
            .walks
            .borrow_mut()
            .frame(hour, wide, cancelled)?
            .map(Supply::Read))
    }
    #[cfg(test)]
    fn source(&self) -> Option<&Source> {
        Some(self.walks.borrow().hours.source())
    }
}

/// `text` without the hour a frame's table refusal names ("hour 7 FactionRules is ..."): a
/// reason read again equals the one held while its table stays refused, wherever a block or
/// segment starts.
pub(in crate::products::faction_state) fn timeless(text: String) -> String {
    if !text.contains("hour ") {
        return text;
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(at) = rest.find("hour ") {
        let after = &rest[at + 5..];
        let number = after.strip_prefix('-').unwrap_or(after);
        let digits = number.bytes().take_while(u8::is_ascii_digit).count();
        let named = number[digits..]
            .strip_prefix(' ')
            .filter(|name| name.starts_with(|c: char| c.is_ascii_uppercase()));
        let word = at == 0 || !rest.as_bytes()[at - 1].is_ascii_alphanumeric();
        match named {
            Some(name) if word && digits > 0 => {
                out.push_str(&rest[..at]);
                rest = name;
            }
            _ => {
                out.push_str(&rest[..at + 5]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `T`'s rows, each of the fields `mask` names, or why the hour lacks them, without its hour.
fn complete<T: TableSchema>(
    frame: HourFrame<'_>,
    mask: u64,
) -> Result<Rowset<'_, T::Row>, String> {
    frame
        .complete::<T>(mask)
        .map_err(|why| timeless(why.to_string()))
}

/// Whether the hour holds every row of `table`.
fn is_complete(frame: HourFrame<'_>, table: TableId) -> bool {
    matches!(frame.status(table), OperandStatus::Complete)
}

fn timeless_label(mut label: GoalLabel) -> GoalLabel {
    label.failures = std::mem::take(&mut label.failures)
        .into_iter()
        .map(timeless)
        .collect();
    label
}

pub(in crate::products::faction_state) trait SelectionTable: TableSchema {
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
struct Selected<'a, T: SelectionTable>
where
    T::Row: Clone,
{
    rows: Vec<Cow<'a, T::Row>>,
    invalid: BTreeSet<usize>,
    failure: Option<String>,
}
impl<T: SelectionTable> Selected<'_, T>
where
    T::Row: Clone,
{
    fn available(&self) -> bool {
        self.failure.is_none() && self.invalid.is_empty()
    }
}
/// A witnessed faction's rows of `T`, every field built: the selection tables are small. A
/// row that cannot be built refuses them all.
fn selected<T: SelectionTable>(frame: HourFrame<'_>, faction: u64) -> Selected<'_, T>
where
    T::Row: Clone,
{
    let (low, high) = selectors::owned(faction);
    let rows = complete::<T>(frame, u64::MAX).and_then(|rows| {
        rows.span(low, high)
            .iter()
            .collect::<io::Result<Vec<_>>>()
            .map_err(|why| timeless(why.to_string()))
    });
    let failure = rows.as_ref().err().cloned();
    let rows = rows.unwrap_or_default();
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
/// Why a faction with no rows of `T` has no selections.
fn unowned<T: SelectionTable>(frame: HourFrame<'_>) -> String {
    complete::<T>(frame, 0)
        .err()
        .unwrap_or_else(|| format!("Faction {} owner is unavailable", T::KIND.selection_name()))
}
const DIFF_TABLES: [TableId; 8] = [
    TableId::FactionAccounts,
    TableId::FactionRules,
    TableId::FactionPrograms,
    TableId::FactionMinimumUpgrades,
    TableId::FactionDoctrine,
    TableId::FactionManpower,
    TableId::FactionGoalStatus,
    TableId::FactionGoalEntries,
];
struct Previous {
    tables: [SharedView; DIFF_TABLES.len()],
}
impl Previous {
    /// A decoded frame's diffed tables; a walk, whose changes are exact, keeps none.
    fn read(frame: HourFrame<'_>) -> Option<Self> {
        let frame = frame.decoded()?.decoded_frame();
        Some(Self {
            tables: DIFF_TABLES.map(|table| frame.tables[table.index()].clone()),
        })
    }
    fn complete<T: TableSchema>(&self) -> Option<&[T::Row]> {
        let index = DIFF_TABLES.iter().position(|table| *table == T::ID)?;
        match &*self.tables[index] {
            TableView::Complete(rows) => T::borrow(rows),
            _ => None,
        }
    }
}
/// The tables whose rows name a witnessed faction ([`selectors::owners`]).
const OWNER_TABLES: [TableId; 10] = [
    TableId::Factions,
    TableId::FactionMembers,
    TableId::FactionProjection,
    TableId::FactionRules,
    TableId::FactionPrograms,
    TableId::FactionMinimumUpgrades,
    TableId::FactionDoctrine,
    TableId::FactionManpower,
    TableId::FactionUnlocks,
    TableId::FactionGoalStatus,
];
/// The faction a row of an owner table other than FactionMembers names, by its key.
fn owner_of(table: TableId, key: u128) -> u64 {
    match table {
        TableId::FactionRules
        | TableId::FactionPrograms
        | TableId::FactionMinimumUpgrades
        | TableId::FactionDoctrine
        | TableId::FactionManpower => (key >> 32) as u64,
        _ => key as u64,
    }
}
/// The tables whose status decides every faction's known channels.
const KNOWN_TABLES: [TableId; 8] = [
    TableId::Factions,
    TableId::FactionRules,
    TableId::FactionPrograms,
    TableId::FactionMinimumUpgrades,
    TableId::FactionDoctrine,
    TableId::FactionManpower,
    TableId::FactionGoalStatus,
    TableId::FactionGoalEntries,
];
/// The account fields the projection reads, as the block pre-pass keeps them.
pub(in crate::products::faction_state) static ACCOUNT_FIELDS: LazyLock<u64> = LazyLock::new(|| {
    field_mask(
        TableId::FactionAccounts,
        &[
            "country",
            "upgrade_groups",
            "upgrade_tokens",
            "influence_share_raw",
            "influence_numerator_raw",
            "influence_denominator_raw",
            "influence_refresh",
            "influence_faction",
        ],
    )
});
static ACCOUNT_COUNTRY: LazyLock<u64> =
    LazyLock::new(|| field_mask(TableId::FactionAccounts, &["country"]));
static MANPOWER_COUNTRY: LazyLock<u64> =
    LazyLock::new(|| field_mask(TableId::FactionManpower, &["country"]));
struct Changed {
    accounts: Option<BTreeSet<u128>>,
    owners: BTreeMap<Kind, Option<BTreeSet<u64>>>,
    /// Owners witnessed now but not the hour before: an unchanged table may hold their rows.
    rewitnessed: BTreeSet<u64>,
    refresh: bool,
    programme_operands: bool,
}
impl Changed {
    fn affects(&self, kind: Kind, faction: u64) -> bool {
        self.refresh
            || self.rewitnessed.contains(&faction)
            || self.owners[&kind]
                .as_ref()
                .is_none_or(|keys| keys.contains(&faction))
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
    frame: HourFrame<'_>,
    edits: &dyn Edits,
) {
    let Some(keys) = keys else {
        return;
    };
    // Exact changes name the fields each row changed; a deleted row is always read again.
    if let Some((changed, _)) = edits.rows(TableId::FactionAccounts) {
        let fields = *ACCOUNT_FIELDS;
        keys.retain(|key| {
            (*key >> 32) as u32 == 0
                && changed
                    .binary_search_by_key(key, |row| row.key)
                    .map_or(true, |at| changed[at].mask & fields != 0)
        });
        return;
    }
    // Only a decoded frame keeps the hour before's rows.
    if let Some(before) = previous.and_then(Previous::complete::<tables::FactionAccounts>)
        && let Some(decoded) = frame.decoded()
        && let Ok(now) = decoded.decoded_frame().complete::<tables::FactionAccounts>()
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
fn group_choice(frame: HourFrame<'_>, group: u32, cat: Option<&catalogue::Catalogue>) -> Choice {
    match selectors::group_name(frame, group, cat) {
        Ok(name) => Choice {
            name,
            ..Default::default()
        },
        Err(reason) => Choice::unknown(reason),
    }
}
/// The hour's complete faction roster does not list `faction`.
fn goal_absent(frame: HourFrame<'_>, faction: u64) -> bool {
    // A faction's row is keyed by its id.
    frame
        .complete::<tables::Factions>(0)
        .is_ok_and(|rows| !rows.contains(u128::from(faction)))
}

/// One hour's projection, for the ordered emit.
#[derive(Default)]
#[derive(replay_spool::Spooled)]
pub(in crate::products::faction_state) struct Projected {
    /// Every membership reference joined: a retained owner survives only then.
    pub(in crate::products::faction_state) joined: bool,
    /// The hour's Factions roster is complete.
    pub(in crate::products::faction_state) roster: bool,
    /// The factions the hour witnesses, when they differ from those the emit holds.
    pub(in crate::products::faction_state) witnessed: Option<Arc<BTreeSet<u64>>>,
    /// What a faction the hour lists nowhere reads, when it differs from the emit's.
    pub(in crate::products::faction_state) ghost: Option<Arc<Ghost>>,
    pub(in crate::products::faction_state) accounts: Accounts,
    /// The witnessed factions whose inputs may have changed, ascending.
    pub(in crate::products::faction_state) factions: Vec<FactionOps>,
    /// Whether the hour read each channel's table.
    pub(in crate::products::faction_state) reads: [bool; CHANNELS.len()],
}
impl Projected {
    /// The bytes it owns on the heap; the witnessed set and the template are the projection's.
    pub(in crate::products::faction_state) fn heap(&self) -> usize {
        let accounts = self.accounts.ops.capacity() * size_of::<AccountOp>()
            + self.accounts.ops.iter().map(AccountOp::heap).sum::<usize>();
        let factions = self.factions.capacity() * size_of::<FactionOps>()
            + self
                .factions
                .iter()
                .map(|ops| {
                    ops.channels.capacity() * size_of::<ChannelOp>()
                        + ops.channels.iter().map(ChannelOp::heap).sum::<usize>()
                })
                .sum::<usize>();
        accounts + factions
    }
}
fn text_heap(text: &Option<String>) -> usize {
    text.as_ref().map_or(0, String::capacity)
}
fn choice_heap(choice: &Choice) -> usize {
    text_heap(&choice.token) + text_heap(&choice.name) + text_heap(&choice.reason)
}
fn label_heap(label: &GoalLabel) -> usize {
    [
        &label.token,
        &label.name,
        &label.description,
        &label.category,
        &label.category_name,
    ]
    .into_iter()
    .map(text_heap)
    .sum::<usize>()
        + label.group.as_ref().map_or(0, Vec::capacity)
        + label.failures.capacity() * size_of::<String>()
        + label.failures.iter().map(String::capacity).sum::<usize>()
}
impl AccountOp {
    fn heap(&self) -> usize {
        match self {
            Self::Whole {
                row:
                    Some(AccountRow::Identified {
                        identity,
                        tag,
                        owner,
                        ..
                    }),
                ..
            } => {
                identity.capacity()
                    + tag.capacity()
                    + match owner {
                        Owner::Member { upgrades, .. } => {
                            upgrades.capacity() * size_of::<UpgradeOp>()
                                + upgrades
                                    .iter()
                                    .map(|upgrade| {
                                        choice_heap(&upgrade.name) + choice_heap(&upgrade.choice)
                                    })
                                    .sum::<usize>()
                        }
                        _ => 0,
                    }
            }
            Self::Whole { .. } | Self::Share { .. } => 0,
        }
    }
}
impl ChannelOp {
    fn heap(&self) -> usize {
        match self {
            Self::Selections {
                values, failure, ..
            } => {
                values.capacity() * size_of::<(u32, u32, Choice)>()
                    + values
                        .iter()
                        .map(|(_, _, choice)| choice_heap(choice))
                        .sum::<usize>()
                    + text_heap(failure)
            }
            Self::Group { choice, .. } => choice_heap(choice),
            Self::Manpower(op) => {
                op.entries.capacity() * size_of::<ManpowerEntry>()
                    + op.entries
                        .iter()
                        .map(|entry| match entry {
                            ManpowerEntry::Unidentified { reason, .. } => reason.capacity(),
                            ManpowerEntry::Member { identity, tag, .. } => {
                                identity.capacity() + tag.capacity()
                            }
                        })
                        .sum::<usize>()
                    + text_heap(&op.failure)
            }
            Self::ManpowerValues { values, .. } => values.capacity() * size_of::<(u32, i32)>(),
            Self::Goals(op) => {
                text_heap(&op.reason)
                    + op.entries.capacity() * size_of::<GoalEntry>()
                    + op.entries
                        .iter()
                        .map(|entry| label_heap(&entry.label))
                        .sum::<usize>()
            }
            Self::GoalProgress(values) => values.capacity() * size_of::<(u32, i64)>(),
        }
    }
}
/// What a faction no table of the hour names reads: it has no rows anywhere.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub(in crate::products::faction_state) struct Ghost {
    /// Why each of rules, programmes, minimums and doctrines is unavailable.
    pub(in crate::products::faction_state) selections: [String; 4],
    pub(in crate::products::faction_state) manpower: String,
    pub(in crate::products::faction_state) goals: GoalList,
}
fn ghost(frame: HourFrame<'_>) -> Ghost {
    let goals = if is_complete(frame, TableId::Factions) {
        GoalList::absent()
    } else {
        // Its goal header is absent, as faction details find it.
        let reason = match (
            frame.complete::<tables::FactionGoalStatus>(0),
            frame.complete::<tables::FactionGoalEntries>(0),
        ) {
            (Err(why), _) | (_, Err(why)) => timeless(why.to_string()),
            _ => "Faction goal status is absent".to_owned(),
        };
        GoalList {
            reason: Some(reason),
            ..Default::default()
        }
    };
    Ghost {
        selections: [
            unowned::<tables::FactionRules>(frame),
            unowned::<tables::FactionPrograms>(frame),
            unowned::<tables::FactionMinimumUpgrades>(frame),
            unowned::<tables::FactionDoctrine>(frame),
        ],
        manpower: complete::<tables::FactionManpower>(frame, 0)
            .err()
            .unwrap_or_else(|| {
                "Faction manpower owner is unavailable; empty pool is not certified".to_owned()
            }),
        goals,
    }
}
#[derive(Default)]
#[derive(replay_spool::Spooled)]
pub(in crate::products::faction_state) struct Accounts {
    /// Every account of the hour's rows is projected; the emit adds each its catalogue holds.
    pub(in crate::products::faction_state) full: bool,
    pub(in crate::products::faction_state) unavailable: bool,
    /// Ascending by account.
    pub(in crate::products::faction_state) ops: Vec<AccountOp>,
}
#[derive(replay_spool::Spooled)]
pub(in crate::products::faction_state) enum AccountOp {
    Whole {
        account: u32,
        /// None when the hour has no row of the account.
        row: Option<AccountRow>,
    },
    /// Only its share changed since its last whole projection, which still holds.
    Share { account: u32, share: i64 },
}
impl AccountOp {
    pub(in crate::products::faction_state) fn account(&self) -> u32 {
        match self {
            Self::Whole { account, .. } | Self::Share { account, .. } => *account,
        }
    }
}
#[derive(replay_spool::Spooled)]
pub(in crate::products::faction_state) enum AccountRow {
    /// Its country's identity is unavailable.
    Unidentified,
    Identified {
        identity: String,
        tag: String,
        holder: (u32, u32),
        share: i64,
        owner: Owner,
    },
}
#[derive(replay_spool::Spooled)]
pub(in crate::products::faction_state) enum Owner {
    /// Membership inputs are unavailable and its country is no explicit member.
    Unjoined,
    Member {
        faction: u64,
        upgrades: Vec<UpgradeOp>,
    },
    Ambiguous,
    /// No membership: the account's own supplying faction.
    Influence(u64),
    /// No membership and no supplying faction: the account's retained owner, if any.
    Retained,
}
#[derive(replay_spool::Spooled)]
pub(in crate::products::faction_state) struct UpgradeOp {
    pub(in crate::products::faction_state) group: u32,
    pub(in crate::products::faction_state) name: Choice,
    pub(in crate::products::faction_state) choice: Choice,
}
#[derive(replay_spool::Spooled)]
pub(in crate::products::faction_state) struct FactionOps {
    pub(in crate::products::faction_state) faction: u64,
    /// Whether each channel is known, when that changed; the emit reads it with the hour's reads.
    pub(in crate::products::faction_state) known: Option<[bool; CHANNELS.len()]>,
    /// In the order the faction's lanes are observed.
    pub(in crate::products::faction_state) channels: Vec<ChannelOp>,
}
#[derive(replay_spool::Spooled)]
pub(in crate::products::faction_state) enum ChannelOp {
    Selections {
        kind: Kind,
        values: Vec<(u32, u32, Choice)>,
        failure: Option<String>,
    },
    Group {
        group: u32,
        choice: Choice,
    },
    Manpower(ManpowerOp),
    /// Only values changed since the faction's last whole manpower, which still holds: each
    /// changed entry's position and value.
    ManpowerValues {
        total: i128,
        values: Vec<(u32, i32)>,
    },
    Goals(GoalsOp),
    /// Only progress changed since the faction's last whole goals: by ordinal.
    GoalProgress(Vec<(u32, i64)>),
}
#[derive(replay_spool::Spooled)]
pub(in crate::products::faction_state) struct ManpowerOp {
    pub(in crate::products::faction_state) total: i128,
    pub(in crate::products::faction_state) entries: Vec<ManpowerEntry>,
    pub(in crate::products::faction_state) failure: Option<String>,
}
#[derive(replay_spool::Spooled)]
pub(in crate::products::faction_state) enum ManpowerEntry {
    Unidentified {
        ordinal: u32,
        reason: String,
    },
    Member {
        ordinal: u32,
        /// The lane's identity.
        identity: String,
        tag: String,
        holder: Option<(u32, u32)>,
        value: i32,
    },
}
#[derive(replay_spool::Spooled)]
pub(in crate::products::faction_state) struct GoalsOp {
    pub(in crate::products::faction_state) absent: bool,
    pub(in crate::products::faction_state) reason: Option<String>,
    /// Each held goal of the first three lists, in list order.
    pub(in crate::products::faction_state) entries: Vec<GoalEntry>,
}
#[derive(replay_spool::Spooled)]
pub(in crate::products::faction_state) struct GoalEntry {
    pub(in crate::products::faction_state) kind: u8,
    pub(in crate::products::faction_state) ordinal: u32,
    pub(in crate::products::faction_state) label: GoalLabel,
    pub(in crate::products::faction_state) status: i32,
    pub(in crate::products::faction_state) progress: i64,
}
/// An account's projected inputs, whose share alone may change without a whole projection.
struct AccountMemo {
    country: selectors::Country,
    owner: Option<Option<u64>>,
    joined: bool,
    influence_faction: u64,
    groups: Vec<u32>,
    tokens: Vec<u32>,
    share: i64,
}
impl AccountMemo {
    fn holds(
        &self,
        row: &FactionAccountRow,
        country: &selectors::Country,
        owner: Option<Option<u64>>,
        joined: bool,
    ) -> bool {
        self.country == *country
            && self.owner == owner
            && self.joined == joined
            && self.influence_faction == row.influence_faction
            && self.groups == row.upgrade_groups
            && self.tokens == row.upgrade_tokens
    }
}
/// A dynamic text's last evaluation and what it read; stale once a read may read otherwise.
struct Held<T> {
    value: T,
    inputs: reuse::Inputs,
    stale: bool,
}

impl<T> Held<T> {
    fn new(value: T, inputs: reuse::Inputs) -> Self {
        Self {
            value,
            inputs,
            stale: false,
        }
    }
}

/// The tables whose status decides what a faction no table names reads.
const GHOST_TABLES: [TableId; 8] = [
    TableId::Factions,
    TableId::FactionRules,
    TableId::FactionPrograms,
    TableId::FactionMinimumUpgrades,
    TableId::FactionDoctrine,
    TableId::FactionManpower,
    TableId::FactionGoalStatus,
    TableId::FactionGoalEntries,
];

/// What decides the known channels of every witnessed faction at once.
#[derive(Clone, Copy, PartialEq, Eq)]
struct KnownInputs {
    accounts: bool,
    complete: [bool; KNOWN_TABLES.len()],
}

/// The hour-to-hour state of one walk's projection: country joins, memberships, label
/// caches and reuse inputs, and what it last projected of each account, manpower pool and
/// goal list. A walk that starts cold projects every input whole.
#[derive(Default)]
pub(in crate::products::faction_state) struct Runtime {
    /// The hours come with each table's changed rows exactly, so none is kept to compare.
    exact: bool,
    last: Option<i32>,
    countries: selectors::Countries,
    memberships: BTreeMap<u32, Option<u64>>,
    members_joined: Option<bool>,
    accounts_joined: Option<bool>,
    witnessed: Arc<BTreeSet<u64>>,
    ready: bool,
    previous: Option<Previous>,
    /// Each dynamic rule or doctrine name, by faction, kind and ordinal, with its token.
    labels: BTreeMap<(u64, Kind, u32), Held<(u32, FactionRuleDetail)>>,
    dynamic: BTreeMap<u32, bool>,
    /// Labels of goals whose texts read only the goal tables, for `goal_dictionary`.
    goal_labels: BTreeMap<Option<u32>, GoalLabel>,
    goal_expands: BTreeMap<u32, bool>,
    goal_dictionary: Option<Mark>,
    /// Each scripted goal label, by faction and token.
    goal_texts: BTreeMap<(u64, Option<u32>), Held<GoalLabel>>,
    /// The hour the held texts were last checked at.
    checked: Option<i32>,
    /// The held texts changed since [`Self::gather_reads`] gathered their reads.
    reads_moved: bool,
    /// The last supplying read of programme operands: its branch, hour, whether it holds the
    /// scripted-text tables, and frame.
    supplying: Option<(u64, i32, bool, Arc<DecodedFrame>)>,
    /// While the hour's supplying read is the one before a walk's start, every faction but
    /// these, whose programme rows moved since, keeps the programmes the emit holds: they
    /// read alike.
    pub(in crate::products::faction_state) kept_programmes: Option<BTreeSet<u64>>,
    ghost: Option<Arc<Ghost>>,
    /// The last hour's membership join and roster, which an hour that changed nothing keeps.
    joined: bool,
    roster: bool,
    known_inputs: Option<KnownInputs>,
    known: HashMap<u64, [bool; CHANNELS.len()]>,
    accounts: HashMap<u32, AccountMemo>,
    /// Each faction's manpower entries: ordinal, country and value.
    manpower: HashMap<u64, Vec<(u32, u32, i32)>>,
    /// Each faction's goal entries: kind, ordinal, token, status and progress.
    goals: HashMap<u64, Vec<(u8, u32, Option<u32>, i32, i64)>>,
    #[cfg(test)]
    pub(in crate::products::faction_state) account_projections: usize,
    #[cfg(test)]
    pub(in crate::products::faction_state) supplying_reads: usize,
    /// Tests: each hour reads its supplying read on its own, as before reads were walked.
    #[cfg(test)]
    pub(in crate::products::faction_state) point_seeks: bool,
}

impl Runtime {
    /// A projection of hours that come with their rows' exact changes.
    pub(in crate::products::faction_state) fn exact() -> Self {
        Self {
            exact: true,
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(in crate::products::faction_state) fn country_rebuilds(&self) -> usize {
        self.countries.rebuilds
    }

    /// A dynamic name or scripted goal text is held: each hour checks its reads.
    pub(in crate::products::faction_state) fn holds_dynamic(&self) -> bool {
        !self.labels.is_empty() || !self.goal_texts.is_empty()
    }

    /// Gathers into `fields` the fields of each table the held texts read, when they changed
    /// since the last gathering: whether it did.
    pub(in crate::products::faction_state) fn gather_reads(
        &mut self,
        fields: &mut [u64; TABLE_COUNT],
    ) -> bool {
        if !std::mem::take(&mut self.reads_moved) {
            return false;
        }
        fields.fill(0);
        let held = self
            .labels
            .values()
            .map(|held| &held.inputs)
            .chain(self.goal_texts.values().map(|held| &held.inputs));
        for reads in held.filter_map(reuse::Inputs::declared) {
            for read in reads {
                fields[read.table.index()] |= read.fields;
            }
        }
        true
    }

    /// Checks every held text against what changed since the hour before: one whose every
    /// read reads alike goes on to `hour`, the others are stale. Whether none is.
    pub(in crate::products::faction_state) fn check_held(
        &mut self,
        since: Option<&reuse::Since<'_>>,
        hour: i32,
    ) -> bool {
        self.checked = Some(hour);
        let mut fresh = true;
        let mut check = |inputs: &mut reuse::Inputs| {
            let stale = !since.is_some_and(|since| inputs.advance(since));
            fresh &= !stale;
            stale
        };
        for held in self.labels.values_mut() {
            held.stale = check(&mut held.inputs);
        }
        for held in self.goal_texts.values_mut() {
            held.stale = check(&mut held.inputs);
        }
        fresh
    }

    /// Reads again the inputs of each stale text whose reads moved, on a frame of the tables
    /// they read: one reading alike is fresh, without its evaluation's world.
    fn confirm_stale(
        &mut self,
        frames: &dyn Frames,
        since: Option<&reuse::Since<'_>>,
        cat: Option<&catalogue::Catalogue>,
    ) -> io::Result<()> {
        let (Some(since), Some(cat)) = (since, cat) else {
            return Ok(());
        };
        let mut tables = Vec::new();
        let stale = self
            .labels
            .values()
            .filter(|held| held.stale)
            .map(|held| &held.inputs)
            .chain(
                self.goal_texts
                    .values()
                    .filter(|held| held.stale)
                    .map(|held| &held.inputs),
            );
        for reads in stale.filter_map(reuse::Inputs::declared) {
            tables.extend(reads.iter().map(|read| read.table));
        }
        if tables.is_empty() {
            return Ok(());
        }
        tables.sort_unstable();
        tables.dedup();
        let frame = frames.reading(&tables)?;
        let text = ViewerText::new_in(frame, cat).with_lexicon(lexicon(frames));
        let mut confirm = |inputs: &mut reuse::Inputs| {
            inputs.declared().is_some()
                && inputs.confirm_since_in(frame, Some(since), |key| text.reread(key))
        };
        for held in self.labels.values_mut().filter(|held| held.stale) {
            held.stale = !confirm(&mut held.inputs);
        }
        for held in self.goal_texts.values_mut().filter(|held| held.stale) {
            held.stale = !confirm(&mut held.inputs);
        }
        Ok(())
    }

    /// The projection of an hour that changed nothing it reads since the hour before, which
    /// it follows: every value holds, and only the hour's reads are new.
    pub(in crate::products::faction_state) fn idle(
        &mut self,
        hour: i32,
        reads: [bool; CHANNELS.len()],
    ) -> Projected {
        debug_assert!(self.ready && self.last.and_then(|last| last.checked_add(1)) == Some(hour));
        self.last = Some(hour);
        Projected {
            joined: self.joined,
            roster: self.roster,
            reads,
            ..Default::default()
        }
    }

    /// Whether the hour after the last projected one may be projected idle, once nothing it
    /// reads changed and every held text read alike ([`Self::check_held`]).
    pub(in crate::products::faction_state) fn may_idle(&self, hour: i32) -> bool {
        self.ready && self.last.and_then(|last| last.checked_add(1)) == Some(hour)
    }

    /// Lets go of the last supplying read, which a later hour asks for again.
    pub(in crate::products::faction_state) fn forget_supplying(&mut self) {
        self.supplying = None;
    }

    /// The hour's projection: what may have changed since the hour before, evaluated.
    pub(in crate::products::faction_state) fn project(
        &mut self,
        frames: &dyn Frames,
        edits: &dyn Edits,
        cat: Option<&catalogue::Catalogue>,
        cancelled: &AtomicBool,
    ) -> io::Result<Projected> {
        let viewed = frames.own();
        let frame = viewed.frame;
        let hour = frame.hour();
        let changes = edits.changes();
        let continues = self.last.and_then(|last| last.checked_add(1)) == Some(hour);
        let joins = self.refresh_joins(viewed, edits, continues);
        let recomputed = self.witness(frame, edits, continues);
        let witnessed_changed = recomputed
            .as_ref()
            .is_some_and(|now| *now != *self.witnessed);
        let rewitnessed: BTreeSet<u64> = recomputed
            .as_ref()
            .map(|now| now.difference(&self.witnessed).copied().collect())
            .unwrap_or_default();
        let dirty = self.changed(frame, edits, continues, rewitnessed);
        if let Some(now) = recomputed.filter(|_| witnessed_changed) {
            self.witnessed = Arc::new(now);
            let now = Arc::clone(&self.witnessed);
            // A faction seen again is projected whole: the emit read it as absent meanwhile.
            self.known.retain(|faction, _| now.contains(faction));
            self.manpower.retain(|faction, _| now.contains(faction));
            self.goals.retain(|faction, _| now.contains(faction));
        }
        let owners = Arc::clone(&self.witnessed);
        self.goal_dictionary(viewed);
        // A refresh observes every lane again, as a block start or a break must.
        if dirty.refresh {
            self.goal_labels.clear();
            self.known.clear();
            self.accounts.clear();
            self.manpower.clear();
            self.goals.clear();
            self.labels.clear();
            self.goal_texts.clear();
            self.reads_moved = true;
        }
        let mut projected = Projected {
            joined: joins.joined,
            roster: is_complete(frame, TableId::Factions),
            witnessed: (witnessed_changed || !self.ready).then(|| Arc::clone(&owners)),
            reads: CHANNELS.map(|(_, table)| changes.slot_read[table.index()]),
            ..Default::default()
        };
        (self.joined, self.roster) = (projected.joined, projected.roster);
        // What a faction no table names reads follows its tables' statuses alone.
        if !self.ready || !continues || GHOST_TABLES.iter().any(|&table| edits.restated(table)) {
            let template = ghost(frame);
            if !self.ready || self.ghost.as_deref() != Some(&template) {
                let template = Arc::new(template);
                projected.ghost = Some(Arc::clone(&template));
                self.ghost = Some(template);
            }
        }
        // Read once a faction's programmes are evaluated.
        let mut supplying = None;
        projected.accounts = self.accounts(frame, &dirty, &joins, cat);
        // Dynamic names use the viewing country; changing its identity or link invalidates them.
        let viewer_changed = [TableId::LocalPlayer, TableId::CountryLinks]
            .iter()
            .any(|table| changes.rows_changed[table.index()]);
        // A held text holds while every input it read reads alike; otherwise it is evaluated.
        let count = self.labels.len() + self.goal_texts.len();
        self.labels
            .retain(|&(faction, _, _), _| owners.contains(&faction));
        self.goal_texts
            .retain(|&(faction, _), _| owners.contains(&faction));
        self.reads_moved |= self.labels.len() + self.goal_texts.len() != count;
        let since = reuse::Since::hour_before_in(frame, edits.tables());
        if self.checked != Some(hour) {
            self.check_held(since.as_ref(), hour);
        }
        if viewer_changed {
            for held in self.labels.values_mut() {
                held.stale = true;
            }
            for held in self.goal_texts.values_mut() {
                held.stale = true;
            }
        } else {
            self.confirm_stale(frames, since.as_ref(), cat)?;
        }
        let stale: BTreeSet<u64> = self
            .labels
            .iter()
            .filter(|(_, held)| held.stale)
            .map(|(&(faction, _, _), _)| faction)
            .collect();
        let stale_goals: BTreeSet<u64> = self
            .goal_texts
            .iter()
            .filter(|(_, held)| held.stale)
            .map(|(&(faction, _), _)| faction)
            .collect();
        for faction in &stale_goals {
            self.goals.remove(faction);
        }
        // The hour's shared scripted text, once a text is evaluated.
        let shared = OnceCell::new();
        let reserved = changes.rows_changed[TableId::ReservedCountryNaming.index()];
        let known = KnownInputs {
            accounts: joins.joined && joins.accounts_joined,
            complete: KNOWN_TABLES.map(|table| is_complete(frame, table)),
        };
        // A faction no change names keeps every lane and known flag it held.
        let every = dirty.refresh
            || self.known_inputs != Some(known)
            || dirty.owners.values().any(Option::is_none)
            || viewer_changed
            || joins.identities_changed
            || !joins.joined;
        self.known_inputs = Some(known);
        let visit: Vec<u64> = if every {
            owners.iter().copied().collect()
        } else {
            let mut visit = dirty.rewitnessed.clone();
            for changed in dirty.owners.values().flatten() {
                visit.extend(changed);
            }
            visit.extend(&stale);
            visit.extend(&stale_goals);
            // A programme's or manpower entry's faction is its key's above the ordinal.
            if dirty.programme_operands
                && let Ok(rows) = frame.complete::<tables::FactionPrograms>(0)
            {
                visit.extend(rows.keys().map(|key| (key >> 32) as u64));
            }
            if reserved && let Ok(rows) = frame.complete::<tables::FactionManpower>(*MANPOWER_COUNTRY)
            {
                visit.extend(rows.iter().filter_map(|row| {
                    row.ok()
                        .filter(|row| row.country == 0)
                        .map(|row| row.faction)
                }));
            }
            visit.retain(|faction| owners.contains(faction));
            visit.into_iter().collect()
        };
        for faction in visit {
            check_cancel(cancelled)?;
            let rules = selected::<tables::FactionRules>(frame, faction);
            let programmes = selected::<tables::FactionPrograms>(frame, faction);
            let minimums = selected::<tables::FactionMinimumUpgrades>(frame, faction);
            let doctrines = selected::<tables::FactionDoctrine>(frame, faction);
            let manpower = selected::<tables::FactionManpower>(frame, faction);
            let accounts_known = joins.joined && joins.accounts_joined;
            let goals_absent = goal_absent(frame, faction);
            let flags = [
                accounts_known,
                rules.available(),
                programmes.available(),
                minimums.available(),
                accounts_known,
                doctrines.available(),
                manpower.available(),
                !goals_absent && faction_details::held_goals_in(frame, faction).is_ok(),
            ];
            let mut ops = FactionOps {
                faction,
                known: None,
                channels: Vec::new(),
            };
            if self.known.get(&faction) != Some(&flags) {
                self.known.insert(faction, flags);
                ops.known = Some(flags);
            }
            let renamed = viewer_changed || stale.contains(&faction);
            if dirty.affects(Kind::Rule, faction) || renamed {
                self.channel(&rules, &mut ops.channels, |state, row, _| {
                    let detail = state.label(
                        (frames, &shared),
                        Kind::Rule,
                        (faction, row.ordinal),
                        row.token,
                        cat,
                        |frame, held| selectors::rule_in(frame, faction, row, cat, held),
                    )?;
                    Ok((row.ordinal, 0, detail.into()))
                })?;
                self.forget_labels(faction, Kind::Rule, rules.rows.iter().map(|row| row.ordinal));
            }
            let kept = self
                .kept_programmes
                .as_ref()
                .is_some_and(|moved| !moved.contains(&faction));
            if !kept && (dirty.programme_operands || dirty.affects(Kind::Programme, faction)) {
                if supplying.is_none() {
                    supplying = Some(self.supplying(&dirty, frames, cat, cancelled)?);
                }
                let read = match supplying.as_ref().and_then(Option::as_ref) {
                    Some(Supply::Own) => Some(frames.whole()?),
                    Some(Supply::Read(read)) => Some(HourFrame::from(&**read)),
                    None => None,
                };
                self.channel(&programmes, &mut ops.channels, |_, row, _| {
                    Ok((
                        row.ordinal,
                        0,
                        selectors::programme_detail(read, row.ordinal, row.reference, cat).into(),
                    ))
                })?;
            }
            if dirty.affects(Kind::Minimum, faction) {
                self.channel(&minimums, &mut ops.channels, |_, row, ops| {
                    let detail = selectors::upgrade(frame, row.group, row.token, cat);
                    ops.push(ChannelOp::Group {
                        group: row.group,
                        choice: group_choice(frame, row.group, cat),
                    });
                    Ok((row.ordinal, row.group, detail.into()))
                })?;
            }
            if dirty.affects(Kind::Doctrine, faction) || renamed {
                self.channel(&doctrines, &mut ops.channels, |state, row, _| {
                    let detail = state.label(
                        (frames, &shared),
                        Kind::Doctrine,
                        (faction, row.ordinal),
                        row.token,
                        cat,
                        |frame, held| selectors::doctrine_in(frame, faction, row, cat, held),
                    )?;
                    Ok((row.ordinal, 0, detail.into()))
                })?;
                let ordinals = doctrines.rows.iter().map(|row| row.ordinal);
                self.forget_labels(faction, Kind::Doctrine, ordinals);
            }
            // Country zero's member is named by its own reserved naming read.
            let reserved_renamed = reserved && manpower.rows.iter().any(|row| row.country == 0);
            let reread = joins.identities_changed || reserved_renamed || !joins.joined;
            if dirty.affects(Kind::Manpower, faction) || reread {
                if reread {
                    self.manpower.remove(&faction);
                }
                if let Some(op) = self.manpower(frame, faction, &manpower) {
                    ops.channels.push(op);
                }
            }
            if dirty.affects(Kind::GoalList, faction) || stale_goals.contains(&faction) {
                if let Some(op) = self.goals((frames, &shared), faction, goals_absent, cat)? {
                    ops.channels.push(op);
                }
            }
            if ops.known.is_some() || !ops.channels.is_empty() {
                projected.factions.push(ops);
            }
        }
        self.previous = if self.exact { None } else { Previous::read(frame) };
        self.ready = true;
        self.last = Some(hour);
        Ok(projected)
    }

    /// The hour's witnessed owners, read again unless its changed rows prove them unchanged.
    fn witness(
        &self,
        frame: HourFrame<'_>,
        edits: &dyn Edits,
        continues: bool,
    ) -> Option<BTreeSet<u64>> {
        let changes = edits.changes();
        let held = self.ready
            && continues
            && OWNER_TABLES.iter().all(|&table| {
                !changes.rows_changed[table.index()]
                    || (table != TableId::FactionMembers
                        && edits.rows(table).is_some_and(|(changed, deleted)| {
                            deleted.is_empty()
                                && changed.iter().all(|row| {
                                    let owner = owner_of(table, row.key);
                                    owner == 0 || self.witnessed.contains(&owner)
                                })
                        }))
            });
        (!held).then(|| selectors::owners(frame))
    }

    fn dynamic_name(
        &mut self,
        frame: HourFrame<'_>,
        token: u32,
        cat: Option<&catalogue::Catalogue>,
    ) -> bool {
        let Some(cat) = cat else {
            return false;
        };
        if let Some(&dynamic) = self.dynamic.get(&token) {
            return dynamic;
        }
        let Ok(Some(spelling)) =
            dashboard_base::faction_names::template_spelling_in(&frame, Some(token))
        else {
            return false;
        };
        let dynamic = cat
            .localisation
            .localise(spelling.as_bytes())
            .is_ok_and(|text| text.contains('['));
        self.dynamic.insert(token, dynamic);
        dynamic
    }

    /// A rule or doctrine's name; a dynamic one is held while its inputs read alike, else
    /// evaluated on the whole frame in the hour's shared text.
    fn label<'f>(
        &mut self,
        (frames, shared): (&'f dyn Frames, &OnceCell<ViewerText<'f>>),
        kind: Kind,
        (faction, ordinal): (u64, u32),
        token: u32,
        cat: Option<&'f catalogue::Catalogue>,
        name: impl FnOnce(HourFrame<'f>, Option<&ViewerText<'_>>) -> FactionRuleDetail,
    ) -> io::Result<FactionRuleDetail> {
        let frame = frames.own().frame;
        let at = (faction, kind, ordinal);
        if !self.dynamic_name(frame, token, cat) {
            self.reads_moved |= self.labels.remove(&at).is_some();
            return Ok(name(frame, None));
        }
        if let Some(held) = self.labels.get(&at)
            && !held.stale
            && held.value.0 == token
        {
            return Ok(held.value.1.clone());
        }
        // A name is dynamic only by a catalogue's localisation.
        let Some(cat) = cat else {
            return Ok(name(frame, None));
        };
        let whole = frames.whole()?;
        let text = shared
            .get_or_init(|| ViewerText::new_in(whole, cat).with_lexicon(lexicon(frames)));
        let (detail, inputs) = reuse::capture_in(whole, || name(whole, Some(text)));
        self.labels
            .insert(at, Held::new((token, detail.clone()), inputs));
        self.reads_moved = true;
        Ok(detail)
    }

    /// Lets go of `faction`'s held `kind` names whose ordinal its rows no longer hold.
    fn forget_labels(&mut self, faction: u64, kind: Kind, ordinals: impl Iterator<Item = u32>) {
        let ordinals: BTreeSet<u32> = ordinals.collect();
        let count = self.labels.len();
        self.labels.retain(|&(owner, held, ordinal), _| {
            owner != faction || held != kind || ordinals.contains(&ordinal)
        });
        self.reads_moved |= self.labels.len() != count;
    }

    /// Goal label caches hold for one dictionary read; another read clears them.
    fn goal_dictionary(&mut self, viewed: Viewed<'_>) {
        let table = TableId::LexicalTokens;
        if !self
            .goal_dictionary
            .as_ref()
            .is_some_and(|held| viewed.is(table, held))
        {
            self.goal_expands.clear();
            self.goal_labels.clear();
            self.goals.clear();
            self.reads_moved |= !self.goal_texts.is_empty();
            self.goal_texts.clear();
            self.goal_dictionary = Some(viewed.mark(table));
        }
    }

    fn goal_expands(
        &mut self,
        frame: HourFrame<'_>,
        token: u32,
        cat: &catalogue::Catalogue,
    ) -> bool {
        if let Some(&expands) = self.goal_expands.get(&token) {
            return expands;
        }
        // An unresolved spelling or definition is asked again, never held as static.
        match faction_details::goal_label_expands(frame, cat, token) {
            Some(expands) => *self.goal_expands.entry(token).or_insert(expands),
            None => false,
        }
    }

    /// The programmes' supplying read, kept while later hours read the same one: a read of
    /// the hour's own tables is the hour's alone.
    fn supplying(
        &mut self,
        dirty: &Changed,
        frames: &dyn Frames,
        cat: Option<&catalogue::Catalogue>,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<Supply>> {
        let frame = frames.own().frame;
        let branch = frame.branch();
        let changed = dirty.programme_operands
            || !dirty.rewitnessed.is_empty()
            || dirty.owners[&Kind::Programme]
                .as_ref()
                .is_none_or(|keys| !keys.is_empty());
        if !changed
            || !frame
                .complete::<tables::FactionPrograms>(0)
                .is_ok_and(|rows| !rows.is_empty())
        {
            return Ok(None);
        }
        // A scripted project name reads its supplying hour's world, as the faction details do.
        let scripted = |read: &DecodedFrame| {
            cat.is_some_and(|cat| {
                selectors::programmes_need_environment(frame, HourFrame::from(read), cat)
            })
        };
        #[cfg(test)]
        if self.point_seeks
            && let Some(source) = frames.source()
        {
            self.supplying_reads += 1;
            let Some(read) = progressive::latest_read(
                source,
                branch,
                frame.hour(),
                TableId::ProjectPrograms,
                replay_data::cadence::STAGGER_PERIOD,
                selectors::PROGRAMME_TABLES,
                cancelled,
            )?
            else {
                return Ok(None);
            };
            if !scripted(&*read) {
                return Ok(Some(Supply::Read(Arc::from(read))));
            }
            let tables = selectors::programme_text_tables();
            return Ok(
                progressive::frame_at(source, branch, read.hour, &tables, cancelled)?
                    .map(|read| Supply::Read(Arc::from(read))),
            );
        }
        let Some(hour) = frames.read_hour(cancelled)? else {
            return Ok(None);
        };
        let mut wide = false;
        // A read of another hour is let go first: a walk patches the views it shared.
        if let Some((held_branch, at, held_wide, held)) = self.supplying.take()
            && (held_branch, at) == (branch, hour)
        {
            if held_wide || !scripted(&held) {
                self.supplying = Some((held_branch, at, held_wide, Arc::clone(&held)));
                return Ok(Some(Supply::Read(held)));
            }
            wide = true;
        }
        #[cfg(test)]
        {
            self.supplying_reads += 1;
        }
        let mut read = None;
        if !wide {
            read = frames.supplying(hour, false, cancelled)?;
            // The hour's own tables hold every table a scripted name reads.
            wide = matches!(&read, Some(Supply::Read(read)) if scripted(read));
        }
        if wide {
            read = frames.supplying(hour, true, cancelled)?;
        }
        self.supplying = match &read {
            Some(Supply::Read(held)) => Some((branch, hour, wide, Arc::clone(held))),
            _ => None,
        };
        Ok(read)
    }

    fn refresh_joins(&mut self, viewed: Viewed<'_>, edits: &dyn Edits, continues: bool) -> Joins {
        let frame = viewed.frame;
        let changes = edits.changes();
        let identity = self.countries.refresh(viewed, changes, continues);
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
        // Countries change only with their identities, so both joins hold until their rows do.
        let members = match self.members_joined {
            Some(joined) if !membership_changed => joined,
            _ => {
                // A member's row is keyed by its country.
                let joined = frame
                    .complete::<tables::FactionMembers>(0)
                    .is_ok_and(|rows| {
                        rows.keys()
                            .all(|key| self.countries.get(key as u32).is_some())
                    });
                self.members_joined = Some(joined);
                joined
            }
        };
        let joined = identity.is_ok() && members;
        let accounts_changed = changes.rows_changed[TableId::FactionAccounts.index()]
            && edits
                .rows(TableId::FactionAccounts)
                .is_none_or(|(changed, deleted)| {
                    !deleted.is_empty()
                        || changed.iter().any(|row| row.mask & *ACCOUNT_COUNTRY != 0)
                });
        let accounts_joined = match self.accounts_joined {
            Some(joined)
                if !accounts_changed
                    && !identities_changed
                    && identity.is_ok()
                    && self.ready
                    && continues =>
            {
                joined
            }
            _ => {
                let joined = frame
                    .complete::<tables::FactionAccounts>(*ACCOUNT_COUNTRY)
                    .is_ok_and(|rows| {
                        rows.iter().all(|row| {
                            row.is_ok_and(|row| {
                                row.kind != 0 || self.countries.get(row.country).is_some()
                            })
                        })
                    });
                self.accounts_joined = Some(joined);
                joined
            }
        };
        Joins {
            identities_changed,
            identity_known: identity.is_ok(),
            membership_changed,
            previous_members,
            joined,
            accounts_joined,
        }
    }

    fn changed(
        &self,
        frame: HourFrame<'_>,
        edits: &dyn Edits,
        continues: bool,
        rewitnessed: BTreeSet<u64>,
    ) -> Changed {
        macro_rules! dirty {
            ($table:ident) => {
                self.dirty::<tables::$table>(frame, edits, TableId::$table, continues)
            };
        }
        let mut accounts = dirty!(FactionAccounts);
        narrow_accounts(&mut accounts, self.previous.as_ref(), frame, edits);
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
            (Kind::GoalList, self.goal_owners(frame, edits, continues)),
        ]);
        let changes = edits.changes();
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
            rewitnessed,
            refresh,
            programme_operands,
        }
    }

    fn dirty<T: TableSchema>(
        &self,
        frame: HourFrame<'_>,
        edits: &dyn Edits,
        table: TableId,
        continues: bool,
    ) -> Option<BTreeSet<u128>> {
        if !self.ready || !continues {
            return None;
        }
        if let Some((changed, deleted)) = edits.rows(table) {
            return Some(
                changed
                    .iter()
                    .map(|row| row.key)
                    .chain(deleted.iter().copied())
                    .collect(),
            );
        }
        // Only a decoded frame keeps the hour before's rows to compare.
        let before = self.previous.as_ref()?.complete::<T>()?;
        let now = frame.decoded()?.decoded_frame().complete::<T>().ok()?;
        Some(dashboard_base::prepared::frame_changes::differing_keys(
            before, now,
        ))
    }

    /// Factions whose goal lists may differ from the hour before; None for all.
    fn goal_owners(
        &self,
        frame: HourFrame<'_>,
        edits: &dyn Edits,
        continues: bool,
    ) -> Option<BTreeSet<u64>> {
        if edits.changes().rows_changed[TableId::Factions.index()] {
            return None;
        }
        let entries = self.dirty::<tables::FactionGoalEntries>(
            frame,
            edits,
            TableId::FactionGoalEntries,
            continues,
        )?;
        let status = self.dirty::<tables::FactionGoalStatus>(
            frame,
            edits,
            TableId::FactionGoalStatus,
            continues,
        )?;
        Some(
            entries
                .into_iter()
                .map(|key| (key >> 40) as u64)
                .chain(status.into_iter().map(|key| key as u64))
                .collect(),
        )
    }

    fn channel<T: SelectionTable>(
        &mut self,
        selected: &Selected<'_, T>,
        ops: &mut Vec<ChannelOp>,
        mut map: impl FnMut(&mut Self, &T::Row, &mut Vec<ChannelOp>) -> io::Result<(u32, u32, Choice)>,
    ) -> io::Result<()>
    where
        T::Row: Clone,
    {
        let mut values = Vec::with_capacity(selected.rows.len());
        for (ordinal, row) in selected.rows.iter().enumerate() {
            let (slot, group, mut choice) = map(self, &**row, ops)?;
            if selected.invalid.contains(&ordinal) {
                choice.reason = Some(format!(
                    "Faction {} ordinals are not contiguous",
                    T::KIND.selection_name()
                ));
            }
            values.push((slot, group, choice));
        }
        ops.push(ChannelOp::Selections {
            kind: T::KIND,
            values,
            failure: selected.failure.clone(),
        });
        Ok(())
    }

    /// The accounts whose projection may have changed; when any may have, every account the
    /// hour holds, which the emit joins with every account its catalogue holds.
    fn accounts(
        &mut self,
        frame: HourFrame<'_>,
        dirty: &Changed,
        joins: &Joins,
        cat: Option<&catalogue::Catalogue>,
    ) -> Accounts {
        let full =
            dirty.refresh || joins.identities_changed || !joins.identity_known || !joins.joined;
        // Any row may have changed: every account is read, those the catalogue holds too.
        let whole = full || dirty.accounts.is_none();
        let rows = frame.complete::<tables::FactionAccounts>(*ACCOUNT_FIELDS);
        // An account's rows are keyed by account, kind and ordinal; its own are of kind zero.
        let own = |key: u128| (key >> 32) as u32 == 0;
        let mut accounts: BTreeSet<u32> = if !whole && let Some(changed) = &dirty.accounts {
            changed
                .iter()
                .copied()
                .filter(|key| own(*key))
                .map(|key| (key >> 64) as u32)
                .collect()
        } else {
            rows.iter()
                .flat_map(|rows| rows.keys())
                .filter(|key| own(*key))
                .map(|key| (key >> 64) as u32)
                .collect()
        };
        if joins.membership_changed && !full {
            let changed_slots: BTreeSet<_> = joins
                .previous_members
                .keys()
                .chain(self.memberships.keys())
                .filter(|slot| joins.previous_members.get(slot) != self.memberships.get(slot))
                .copied()
                .collect();
            if let Ok(rows) = &rows {
                accounts.extend(rows.iter().filter_map(|row| {
                    let row = row.ok()?;
                    (row.kind == 0
                        && self
                            .countries
                            .get(row.country)
                            .is_some_and(|country| changed_slots.contains(&country.slot)))
                    .then_some(row.account)
                }));
            }
        }
        if whole {
            self.accounts.clear();
        }
        let mut ops = Vec::with_capacity(accounts.len());
        for account in accounts {
            #[cfg(test)]
            {
                self.account_projections += 1;
            }
            // A row that cannot be built is one the hour does not hold.
            let row = rows
                .as_ref()
                .ok()
                .and_then(|rows| rows.get((account as u128) << 64).ok().flatten());
            if let Some(op) = self.account(frame, account, row.as_deref(), joins.joined, whole, cat)
            {
                ops.push(op);
            }
        }
        Accounts {
            full: whole,
            unavailable: rows.is_err(),
            ops,
        }
    }

    fn account(
        &mut self,
        frame: HourFrame<'_>,
        account: u32,
        row: Option<&FactionAccountRow>,
        joined: bool,
        full: bool,
        cat: Option<&catalogue::Catalogue>,
    ) -> Option<AccountOp> {
        let Some(row) = row else {
            self.accounts.remove(&account);
            return Some(AccountOp::Whole { account, row: None });
        };
        let Some(country) = self.countries.get(row.country).cloned() else {
            self.accounts.remove(&account);
            return Some(AccountOp::Whole {
                account,
                row: Some(AccountRow::Unidentified),
            });
        };
        let owner = self.memberships.get(&country.slot).copied();
        let share = selectors::influence_share(row);
        if !full
            && let Some(held) = self.accounts.get_mut(&account)
            && held.holds(row, &country, owner, joined)
        {
            if held.share == share {
                return None;
            }
            held.share = share;
            return Some(AccountOp::Share { account, share });
        }
        let identity = format!(
            "{}:{}:{}:{}",
            row.account, country.slot, country.id, country.tag
        );
        // An explicit membership witness remains usable when an unrelated country's member
        // reference cannot be joined.
        let resolved = if joined || matches!(owner, Some(Some(_))) {
            match owner {
                Some(Some(faction)) => Owner::Member {
                    faction,
                    upgrades: row
                        .upgrade_groups
                        .iter()
                        .zip(&row.upgrade_tokens)
                        .map(|(&group, &token)| UpgradeOp {
                            group,
                            name: group_choice(frame, group, cat),
                            choice: selectors::upgrade(frame, group, token, cat).into(),
                        })
                        .collect(),
                },
                Some(None) => Owner::Ambiguous,
                None if row.influence_faction != 0 => Owner::Influence(row.influence_faction),
                None => Owner::Retained,
            }
        } else {
            Owner::Unjoined
        };
        let op = AccountOp::Whole {
            account,
            row: Some(AccountRow::Identified {
                identity,
                tag: country.tag.clone(),
                holder: (country.slot, country.id),
                share,
                owner: resolved,
            }),
        };
        self.accounts.insert(
            account,
            AccountMemo {
                country,
                owner,
                joined,
                influence_faction: row.influence_faction,
                groups: row.upgrade_groups.clone(),
                tokens: row.upgrade_tokens.clone(),
                share,
            },
        );
        Some(op)
    }

    /// A witnessed faction's manpower, of its rows `selected`: whole, only its changed values,
    /// or None when nothing changed.
    fn manpower(
        &mut self,
        frame: HourFrame<'_>,
        faction: u64,
        selected: &Selected<'_, tables::FactionManpower>,
    ) -> Option<ChannelOp> {
        if let Some(why) = &selected.failure {
            self.manpower.remove(&faction);
            return Some(ChannelOp::Manpower(ManpowerOp {
                total: NULL,
                entries: Vec::new(),
                failure: Some(why.clone()),
            }));
        }
        let rows = &selected.rows;
        let mut failure = None;
        let total = match selectors::manpower_total_of(rows.iter().map(|row| &**row)) {
            Ok(total) => i128::from(total),
            Err(why) => {
                failure = Some(why);
                NULL
            }
        };
        if let Some(held) = self.manpower.get_mut(&faction)
            && held.len() == rows.len()
            && held
                .iter()
                .zip(rows)
                .all(|(held, row)| (held.0, held.1) == (row.ordinal, row.country))
        {
            let mut values = Vec::new();
            for (at, (held, row)) in held.iter_mut().zip(rows).enumerate() {
                if held.2 != row.value {
                    held.2 = row.value;
                    values.push((at as u32, row.value));
                }
            }
            return (!values.is_empty()).then_some(ChannelOp::ManpowerValues { total, values });
        }
        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            let identity = if row.country == 0 {
                Some((
                    "reserved".to_owned(),
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
                entries.push(ManpowerEntry::Unidentified {
                    ordinal: row.ordinal,
                    reason: format!(
                        "Faction manpower entry {} country {} identity is unavailable",
                        row.ordinal, row.country
                    ),
                });
                failure = Some("Faction manpower country identity is unavailable".into());
                continue;
            };
            entries.push(ManpowerEntry::Member {
                ordinal: row.ordinal,
                identity: format!("{}:{identity}", row.ordinal),
                tag,
                holder,
                value: row.value,
            });
        }
        self.manpower.insert(
            faction,
            rows.iter()
                .map(|row| (row.ordinal, row.country, row.value))
                .collect(),
        );
        Some(ChannelOp::Manpower(ManpowerOp {
            total,
            entries,
            failure,
        }))
    }

    /// `faction`'s labels of `tokens`: a scripted one held while its inputs read alike, else
    /// evaluated on the whole frame; the others as the dictionary spells them.
    fn goal_labels<'f>(
        &mut self,
        (frames, shared): (&'f dyn Frames, &OnceCell<ViewerText<'f>>),
        faction: u64,
        cat: Option<&'f catalogue::Catalogue>,
        tokens: &[Option<u32>],
    ) -> io::Result<Vec<GoalLabel>> {
        let frame = frames.own().frame;
        let mut expands = Vec::with_capacity(tokens.len());
        for token in tokens {
            expands.push(match (cat, token) {
                (Some(cat), Some(token)) => self.goal_expands(frame, *token, cat),
                _ => false,
            });
        }
        let count = self.goal_texts.len();
        self.goal_texts.retain(|&(owner, token), _| {
            owner != faction
                || tokens
                    .iter()
                    .zip(&expands)
                    .any(|(held, expands)| *expands && *held == token)
        });
        self.reads_moved |= self.goal_texts.len() != count;
        let missing: BTreeSet<Option<u32>> = tokens
            .iter()
            .zip(&expands)
            .filter(|&(token, expands)| !*expands && !self.goal_labels.contains_key(token))
            .map(|(token, _)| *token)
            .collect();
        if !missing.is_empty() {
            let labels =
                faction_details::goal_labels_in(frame, faction, cat, missing.iter().copied());
            self.goal_labels.extend(
                missing
                    .into_iter()
                    .zip(labels.into_iter().map(timeless_label)),
            );
        }
        let mut labels = Vec::with_capacity(tokens.len());
        for (&token, &expands) in tokens.iter().zip(&expands) {
            let (true, Some(cat)) = (expands, cat) else {
                labels.push(self.goal_labels[&token].clone());
                continue;
            };
            let at = (faction, token);
            if let Some(held) = self.goal_texts.get(&at)
                && !held.stale
            {
                labels.push(held.value.clone());
                continue;
            }
            let whole = frames.whole()?;
            let text = shared
                .get_or_init(|| ViewerText::new_in(whole, cat).with_lexicon(lexicon(frames)));
            let (label, inputs) = reuse::capture_in(whole, || {
                faction_details::goal_label_in(whole, faction, Some(cat), token, text)
            });
            let label = timeless_label(label);
            self.goal_texts
                .insert(at, Held::new(label.clone(), inputs));
            self.reads_moved = true;
            labels.push(label);
        }
        Ok(labels)
    }

    /// A witnessed faction's goal lists: whole, only its changed progress, or None when
    /// nothing changed.
    fn goals<'f>(
        &mut self,
        (frames, shared): (&'f dyn Frames, &OnceCell<ViewerText<'f>>),
        faction: u64,
        absent: bool,
        cat: Option<&'f catalogue::Catalogue>,
    ) -> io::Result<Option<ChannelOp>> {
        let frame = frames.own().frame;
        let held = if absent {
            Err(None)
        } else {
            faction_details::held_goals_in(frame, faction)
                .map_err(|why| Some(timeless(why.to_string())))
        };
        let held = match held {
            Ok(held) => held,
            Err(reason) => {
                let count = self.goal_texts.len();
                self.goal_texts.retain(|&(owner, _), _| owner != faction);
                self.reads_moved |= self.goal_texts.len() != count;
                self.goals.remove(&faction);
                return Ok(Some(ChannelOp::Goals(GoalsOp {
                    absent,
                    reason,
                    entries: Vec::new(),
                })));
            }
        };
        let rows: Vec<_> = held
            .entries
            .iter()
            .map(|row| &**row)
            .filter(|row| row.kind <= 2)
            .collect();
        if let Some(memo) = self.goals.get_mut(&faction)
            && memo.len() == rows.len()
            && memo.iter().zip(&rows).all(|(held, row)| {
                (held.0, held.1, held.2, held.3) == (row.kind, row.ordinal, row.token, row.status)
            })
        {
            let mut values = Vec::new();
            for (held, row) in memo.iter_mut().zip(&rows) {
                if row.kind == 0 && held.4 != row.progress_raw {
                    held.4 = row.progress_raw;
                    values.push((row.ordinal, row.progress_raw));
                }
            }
            return Ok((!values.is_empty()).then_some(ChannelOp::GoalProgress(values)));
        }
        let tokens: Vec<_> = rows.iter().map(|row| row.token).collect();
        let labels = self.goal_labels((frames, shared), faction, cat, &tokens)?;
        let entries = rows
            .iter()
            .zip(labels)
            .map(|(row, label)| GoalEntry {
                kind: row.kind,
                ordinal: row.ordinal,
                label,
                status: row.status,
                progress: row.progress_raw,
            })
            .collect();
        self.goals.insert(
            faction,
            rows.iter()
                .map(|row| {
                    (
                        row.kind,
                        row.ordinal,
                        row.token,
                        row.status,
                        row.progress_raw,
                    )
                })
                .collect(),
        );
        Ok(Some(ChannelOp::Goals(GoalsOp {
            absent: false,
            reason: None,
            entries,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::timeless;
    #[test]
    fn a_refusal_reads_alike_at_every_hour_and_other_text_is_kept() {
        assert_eq!(
            timeless("hour 105 FactionRules is unavailable: named".into()),
            "FactionRules is unavailable: named"
        );
        assert_eq!(
            timeless(
                "LexicalTokens is not recorded completely: hour -3 LexicalTokens is truncated (2 missing rows)"
                    .into()
            ),
            "LexicalTokens is not recorded completely: LexicalTokens is truncated (2 missing rows)"
        );
        for kept in [
            "Faction goal status is absent",
            "within hour 5 days",
            "lunchhour 7 Rules",
            "hour X Rules",
        ] {
            assert_eq!(timeless(kept.into()), kept);
        }
    }
}
