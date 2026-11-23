
use content_reader_politics::faction_templates::Catalog;
use content_reader_script::localization_binding::{BoundLocalization, BoundNode, BoundValue, FormattedLocalization};
use content_reader_localisation::{localisation::Argument, localisation::Localisation};
use replay_data::{
    codec::{Decoder, invalid_static, put_fixed, put_varint},
    rows::LexicalTokenRow,
    rows::TableRows,
    rows::TableSchema,
    rows::{column, mask, tables},
};
use replay_frame::frame::{HourFrame, OperandStatus, SharedView, TableView, Tables};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError},
};

pub fn template_spelling(
    frame: &dyn replay_frame::frame::RecordedSource,
    token: Option<u32>,
) -> Result<Option<String>, String> {
    let Some(token) = token else {
        return Ok(None);
    };
    let lexical = frame
        .lexical()
        .map_err(|reason| format!("lexical tokens is not recorded completely: {reason}"))?;
    let index = lexical
        .binary_search_by_key(&token, |row| row.token)
        .map_err(|_| format!("Faction template token {token} has no lexical tokens spelling"))?;
    String::from_utf8(lexical[index].literal.clone())
        .map(Some)
        .map_err(|_| format!("Faction template token {token} is not UTF-8"))
}

const LITERAL: u64 = mask::<LexicalTokenRow>(&["literal"]);
const LITERAL_COLUMN: usize = column::<LexicalTokenRow>("literal");

pub fn template_spelling_in<F: Tables + ?Sized>(
    frame: &F,
    token: Option<u32>,
) -> Result<Option<String>, String> {
    let Some(token) = token else {
        return Ok(None);
    };
    let lexical = frame
        .complete_rows::<tables::LexicalTokens>(LITERAL)
        .map_err(|reason| format!("lexical tokens is not recorded completely: {reason}"))?;
    let literal = lexical
        .text(u128::from(token), |row| &row.literal, LITERAL_COLUMN)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Faction template token {token} has no lexical tokens spelling"))?;
    String::from_utf8(literal.to_vec())
        .map(Some)
        .map_err(|_| format!("Faction template token {token} is not UTF-8"))
}

fn reverse<'r>(
    rows: impl IntoIterator<Item = io::Result<(u32, &'r [u8])>>,
) -> Result<BTreeMap<Vec<u8>, Result<u32, String>>, String> {
    let mut names = BTreeMap::new();
    for row in rows {
        let (token, literal) = row.map_err(|error| error.to_string())?;
        match names.entry(literal.to_ascii_lowercase()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(Ok(token));
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                *entry.get_mut() = Err(format!(
                    "{} names several lexer tokens",
                    String::from_utf8_lossy(literal)
                ));
            }
        }
    }
    Ok(names)
}

fn unrecorded(status: OperandStatus<'_>) -> Option<String> {
    match status {
        OperandStatus::Complete => None,
        OperandStatus::Unavailable(reason) => Some(format!(
            "lexical tokens is not recorded completely: {reason}"
        )),
        OperandStatus::Truncated(missing) | OperandStatus::TruncatedWrongLayout(missing) => Some(
            format!("lexical tokens is not recorded completely: {missing} rows missing"),
        ),
        _ => Some("lexical tokens has the wrong table layout".into()),
    }
}

#[derive(Debug, Default)]
pub struct LexicalIndex {
    names: OnceLock<ReverseIndex>,
}

impl LexicalIndex {
    pub fn token_of<F: Tables + ?Sized>(
        &self,
        frame: &F,
        name: &[u8],
    ) -> Result<Option<u32>, String> {
        let names = self
            .names
            .get_or_init(|| {
                if let Some(reason) = unrecorded(frame.table_status(tables::LexicalTokens::ID)) {
                    return Err(reason);
                }
                let rows = frame
                    .complete_rows::<tables::LexicalTokens>(LITERAL)
                    .map_err(|error| error.to_string())?;
                reverse(
                    rows.keys()
                        .zip(rows.texts(|row| &row.literal, LITERAL_COLUMN))
                        .map(|(key, literal)| literal.map(|literal| (key as u32, literal))),
                )
            })
            .as_ref()
            .map_err(Clone::clone)?;
        names
            .get(&name.to_ascii_lowercase())
            .map(|value| value.clone().map(Some))
            .unwrap_or(Ok(None))
    }
}

pub fn name(
    literal: &[u8],
    template: Result<Option<&str>, String>,
    templates: Result<&Catalog, String>,
    localisation: &Localisation,
    recorded_language: &str,
    token_of: impl FnMut(&[u8]) -> Result<Option<u32>, String>,
) -> Result<String, String> {
    if !literal.is_empty() {
        return crate::scripted_text::stored_text(
            localisation,
            recorded_language,
            literal,
            "faction name",
        );
    }
    let Some(token) = template? else {
        return Ok(String::new());
    };
    let bound = templates?
        .get(token)?
        .ok_or_else(|| format!("Faction template {token:?} is not in the recorded content"))?;
    if bound.is_empty() {
        return Ok(String::new());
    }
    bound_name(bound, localisation, token_of)
        .map_err(|why| format!("Faction template {token:?}: {why}"))
}

type ReverseIndex = Result<BTreeMap<Vec<u8>, Result<u32, String>>, String>;

pub struct LexicalNames<'a> {
    table: Lexicon<'a>,
    identity: Option<replay_frame::frame::ViewIdentity>,
    names: OnceLock<ReverseIndex>,
}

enum Lexicon<'a> {
    Shared(replay_frame::frame::SharedLexical, std::marker::PhantomData<&'a ()>),
}

impl std::fmt::Debug for LexicalNames<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Lexicon::Shared(table, _) = &self.table;
        f.debug_struct("LexicalNames")
            .field("table", table)
            .field("identity", &self.identity)
            .field("names", &self.names)
            .finish()
    }
}

impl Default for LexicalNames<'_> {
    fn default() -> Self {
        Self::new(TableView::Complete(TableRows::LexicalTokens(Vec::new())).into())
    }
}

impl<'a> LexicalNames<'a> {
    pub fn new(table: SharedView) -> Self {
        Self {
            identity: Some(table.identity()),
            table: Lexicon::Shared(table.into(), std::marker::PhantomData),
            names: OnceLock::new(),
        }
    }

    pub fn from_source(frame: &dyn replay_frame::frame::RecordedSource) -> Self {
        Self {
            table: Lexicon::Shared(frame.shared_lexical(), std::marker::PhantomData),
            identity: frame.identity(tables::LexicalTokens::ID.index()),
            names: OnceLock::new(),
        }
    }

    pub fn from_tables(frame: HourFrame<'a>) -> Self {
        match frame {
            HourFrame::Decoded(decoded) => Self::from_source(decoded),
        }
    }

    fn status(&self) -> OperandStatus<'_> {
        let Lexicon::Shared(table, _) = &self.table;
        table.status()
    }

    pub fn matches(&self, table: &SharedView) -> bool {
        self.identity
            .as_ref()
            .is_some_and(|identity| identity.is(table))
    }

    pub fn token_of(&self, name: &[u8]) -> Result<Option<u32>, String> {
        let names = self
            .names
            .get_or_init(|| {
                if let Some(reason) = unrecorded(self.status()) {
                    return Err(reason);
                }
                let Lexicon::Shared(table, _) = &self.table;
                reverse(table.rows().into_iter().map(|row| Ok((row.token, row.literal.as_slice()))))
            })
            .as_ref()
            .map_err(Clone::clone)?;
        names
            .get(&name.to_ascii_lowercase())
            .map(|value| value.clone().map(Some))
            .unwrap_or(Ok(None))
    }
}

#[derive(Clone, Default)]
pub struct DictionaryHistory {
    shared: Arc<Mutex<Versions>>,
    count: u32,
}

#[derive(Clone)]
pub struct HistoricalDictionary {
    shared: Arc<Mutex<Versions>>,
    version: u32,
}

type Spellings = Vec<(u32, Option<Box<[u8]>>)>;

#[derive(Default)]
struct Versions {
    count: u32,
    spellings: BTreeMap<u32, Spellings>,
    folded: Option<HashMap<Box<[u8]>, Folded>>,
}

#[derive(Default)]
struct Folded {
    tokens: Vec<u32>,
    outcomes: Vec<(u32, Outcome)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    None,
    One(u32),
    Several(u32),
}

type Refold = (u32, Option<Box<[u8]>>, Option<Box<[u8]>>);

impl std::fmt::Debug for DictionaryHistory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DictionaryHistory")
            .field("count", &self.count)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for HistoricalDictionary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HistoricalDictionary")
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
thread_local! {
    static SEARCHED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn search<T>(items: &[T], before: impl Fn(&T) -> bool) -> usize {
    items.partition_point(|item| {
        #[cfg(test)]
        SEARCHED.with(|searched| searched.set(searched.get() + 1));
        before(item)
    })
}

fn lock(shared: &Mutex<Versions>) -> MutexGuard<'_, Versions> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

fn tree_nodes<K, V>() -> (usize, usize) {
    use std::mem::size_of;
    let leaf = size_of::<usize>() + 2 * size_of::<u16>() + 11 * (size_of::<K>() + size_of::<V>());
    (leaf, leaf + 12 * size_of::<usize>())
}

fn fold(literal: Option<&[u8]>) -> Option<Box<[u8]>> {
    literal.map(|literal| literal.to_ascii_lowercase().into_boxed_slice())
}

fn spelling_at(spellings: &Spellings, version: u32) -> Option<&[u8]> {
    let set = search(spellings, |(from, _)| *from <= version);
    spellings.get(set.checked_sub(1)?)?.1.as_deref()
}

fn held(spellings: &Spellings, count: u32) -> &[(u32, Option<Box<[u8]>>)] {
    &spellings[..spellings.partition_point(|(version, _)| *version < count)]
}

fn refold(folded: &mut HashMap<Box<[u8]>, Folded>, version: u32, changes: Vec<Refold>) {
    let mut touched = Vec::new();
    for (token, old, new) in changes {
        if old == new {
            continue;
        }
        if let Some(old) = old {
            let name = folded.get_mut(&old).expect("an indexed spelling");
            if let Ok(at) = name.tokens.binary_search(&token) {
                name.tokens.remove(at);
            }
            touched.push(old);
        }
        if let Some(new) = new {
            let name = folded.entry(new.clone()).or_default();
            if let Err(at) = name.tokens.binary_search(&token) {
                name.tokens.insert(at, token);
            }
            touched.push(new);
        }
    }
    touched.sort();
    touched.dedup();
    for name in touched {
        let name = folded.get_mut(&name).expect("a touched name");
        let outcome = match name.tokens.as_slice() {
            [] => Outcome::None,
            [token] => Outcome::One(*token),
            [.., last] => Outcome::Several(*last),
        };
        if name.outcomes.last().map(|(_, last)| *last) != Some(outcome) {
            name.outcomes.push((version, outcome));
        }
    }
}

impl Versions {
    fn prefix(&self, count: u32) -> Self {
        Self {
            count,
            spellings: self
                .spellings
                .iter()
                .map(|(token, spellings)| (*token, held(spellings, count).to_vec()))
                .filter(|(_, spellings)| !spellings.is_empty())
                .collect(),
            folded: None,
        }
    }

    fn index(&mut self) -> &HashMap<Box<[u8]>, Folded> {
        let Self {
            spellings, folded, ..
        } = self;
        folded.get_or_insert_with(|| {
            let mut changes: Vec<(u32, Refold)> = Vec::new();
            for (token, spellings) in spellings.iter() {
                let mut old = None;
                for (version, literal) in spellings {
                    let new = fold(literal.as_deref());
                    changes.push((*version, (*token, old, new.clone())));
                    old = new;
                }
            }
            changes.sort_by_key(|(version, (token, ..))| (*version, *token));
            let mut folded = HashMap::new();
            let mut changes = changes.into_iter().peekable();
            while let Some((version, first)) = changes.next() {
                let mut at = vec![first];
                while let Some((_, change)) = changes.next_if(|(next, _)| *next == version) {
                    at.push(change);
                }
                refold(&mut folded, version, at);
            }
            folded
        })
    }
}

impl DictionaryHistory {
    pub fn add(&mut self, table: &SharedView) -> io::Result<Option<HistoricalDictionary>> {
        let TableView::Complete(TableRows::LexicalTokens(rows)) = &**table else {
            return Ok(None);
        };
        self.add_spellings(
            rows.iter()
                .map(|row| (row.token, row.literal.as_slice()))
                .collect(),
        )
    }

    pub fn add_in<F: Tables + ?Sized>(
        &mut self,
        frame: &F,
    ) -> io::Result<Option<HistoricalDictionary>> {
        let Ok(rows) = frame.complete_rows::<tables::LexicalTokens>(LITERAL) else {
            return Ok(None);
        };
        let mut next = Vec::with_capacity(rows.len());
        for (key, literal) in rows
            .keys()
            .zip(rows.texts(|row| &row.literal, LITERAL_COLUMN))
        {
            next.push((key as u32, literal?));
        }
        self.add_spellings(next)
    }

    fn add_spellings(
        &mut self,
        mut next: Vec<(u32, &[u8])>,
    ) -> io::Result<Option<HistoricalDictionary>> {
        if !next.is_sorted_by(|left, right| left.0 < right.0) {
            next = next.into_iter().collect::<BTreeMap<_, _>>().into_iter().collect();
        }
        let mut versions = lock(&self.shared);
        if versions.count != self.count {
            let own = versions.prefix(self.count);
            drop(versions);
            self.shared = Arc::new(Mutex::new(own));
            versions = lock(&self.shared);
        }
        let version = versions.count;
        let count = version
            .checked_add(1)
            .ok_or_else(|| invalid_static("dictionary versions exceed u32"))?;
        let Versions {
            spellings, folded, ..
        } = &mut *versions;
        let mut changes = Vec::new();
        let mut rows = next.iter().copied().peekable();
        let mut added = Vec::new();
        for (token, spellings) in spellings.iter_mut() {
            while let Some(row) = rows.next_if(|(new, _)| new < token) {
                added.push(row);
            }
            let now = rows.next_if(|(new, _)| new == token).map(|(_, literal)| literal);
            let before = spellings.last().and_then(|(_, literal)| literal.as_deref());
            if before != now {
                if folded.is_some() {
                    changes.push((*token, fold(before), fold(now)));
                }
                spellings.push((version, now.map(Box::from)));
            }
        }
        added.extend(rows);
        for (token, literal) in added {
            spellings.insert(token, vec![(version, Some(Box::from(literal)))]);
            if folded.is_some() {
                changes.push((token, None, fold(Some(literal))));
            }
        }
        if let Some(folded) = folded {
            refold(folded, version, changes);
        }
        versions.count = count;
        self.count = count;
        Ok(Some(HistoricalDictionary {
            shared: Arc::clone(&self.shared),
            version,
        }))
    }

    pub fn count(&self) -> u32 {
        self.count
    }

    pub fn held_bytes(&self) -> usize {
        use std::mem::size_of;
        let allocated = |len: usize| if len == 0 { 0 } else { len + len / 4 + 16 };
        let versions = lock(&self.shared);
        let (mut spellings, mut named) = (0, 0);
        let mut bytes = allocated(2 * size_of::<usize>() + size_of::<Mutex<Versions>>());
        let (leaf, internal) = tree_nodes::<u32, Spellings>();
        let leaves = versions.spellings.len().div_ceil(5);
        bytes += leaves * allocated(leaf) + leaves.saturating_sub(1) * allocated(internal);
        for held in versions.spellings.values() {
            bytes += allocated(held.capacity() * size_of::<(u32, Option<Box<[u8]>>)>());
            for (_, literal) in held {
                spellings += 1;
                if let Some(literal) = literal {
                    named += 1;
                    bytes += 2 * allocated(literal.len());
                }
            }
        }
        let tokens = (4 * named + 2 * named) * size_of::<u32>();
        let outcomes = (4 * named + 4 * spellings) * size_of::<(u32, Outcome)>();
        bytes
            + allocated(4 * named * (size_of::<(Box<[u8]>, Folded)>() + 1) + 16)
            + allocated(tokens)
            + allocated(outcomes)
            + 2 * named * 16
    }

    #[cfg(test)]
    fn measured_bytes(&self) -> usize {
        use std::mem::size_of;
        let versions = lock(&self.shared);
        let mut bytes = 2 * size_of::<usize>() + size_of::<Mutex<Versions>>();
        let (leaf, internal) = tree_nodes::<u32, Spellings>();
        let leaves = versions.spellings.len().div_ceil(11);
        bytes += leaves * leaf + leaves.saturating_sub(1).div_ceil(11) * internal;
        for held in versions.spellings.values() {
            bytes += held.capacity() * size_of::<(u32, Option<Box<[u8]>>)>()
                + held
                    .iter()
                    .map(|(_, literal)| literal.as_ref().map_or(0, |literal| literal.len()))
                    .sum::<usize>();
        }
        if let Some(folded) = &versions.folded {
            let capacity = folded.capacity();
            let buckets = if capacity < 8 {
                capacity + 1
            } else {
                capacity * 8 / 7
            }
            .next_power_of_two();
            bytes += buckets * (size_of::<(Box<[u8]>, Folded)>() + 1) + 16;
            for (key, name) in folded {
                bytes += key.len()
                    + name.tokens.capacity() * size_of::<u32>()
                    + name.outcomes.capacity() * size_of::<(u32, Outcome)>();
            }
        }
        bytes
    }

    pub fn version(&self, version: u32) -> Option<HistoricalDictionary> {
        (version < self.count).then(|| HistoricalDictionary {
            shared: Arc::clone(&self.shared),
            version,
        })
    }

    pub fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let versions = lock(&self.shared);
        let tokens = || {
            versions
                .spellings
                .iter()
                .map(|(token, spellings)| (*token, held(spellings, self.count)))
                .filter(|(_, spellings)| !spellings.is_empty())
        };
        put_varint(out, self.count.into());
        put_varint(out, tokens().count() as u128);
        let mut previous_token = None;
        for (token, spellings) in tokens() {
            put_varint(out, gap(previous_token, token));
            previous_token = Some(token);
            put_varint(out, spellings.len() as u128);
            let mut previous_version = None;
            for (version, literal) in spellings {
                put_varint(out, gap(previous_version, *version));
                previous_version = Some(*version);
                match literal {
                    None => put_varint(out, 0),
                    Some(literal) => {
                        put_varint(out, literal.len() as u128 + 1);
                        put_fixed(out, literal)?;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let count = u32::try_from(input.varint()?)
            .map_err(|_| invalid_static("dictionary versions exceed u32"))?;
        let tokens = bounded(input.varint()?, input.remaining() / 3)?;
        let mut spellings = BTreeMap::new();
        let mut previous_token = None;
        for _ in 0..tokens {
            let token = after(previous_token, input.varint()?)?;
            previous_token = Some(token);
            let entries = bounded(input.varint()?, input.remaining() / 2)?;
            let mut history: Spellings = Vec::with_capacity(entries);
            let mut previous_version = None;
            for _ in 0..entries {
                let version = after(previous_version, input.varint()?)?;
                previous_version = Some(version);
                let literal = match input.varint()? {
                    0 => None,
                    length => Some(Box::from(input.take(bounded(length - 1, usize::MAX)?)?)),
                };
                if version >= count
                    || history
                        .last()
                        .map_or(literal.is_none(), |(_, last)| *last == literal)
                {
                    return Err(invalid_static("dictionary spellings are not canonical"));
                }
                history.push((version, literal));
            }
            if history.is_empty() {
                return Err(invalid_static("dictionary token has no spelling"));
            }
            spellings.insert(token, history);
        }
        Ok(Self {
            shared: Arc::new(Mutex::new(Versions {
                count,
                spellings,
                folded: None,
            })),
            count,
        })
    }
}

fn gap(previous: Option<u32>, next: u32) -> u128 {
    u128::from(previous.map_or(next, |previous| next - previous - 1))
}

fn after(previous: Option<u32>, gap: u128) -> io::Result<u32> {
    previous
        .map_or(Some(gap), |previous| {
            u128::from(previous).checked_add(1)?.checked_add(gap)
        })
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| invalid_static("dictionary key exceeds u32"))
}

fn bounded(value: u128, limit: usize) -> io::Result<usize> {
    usize::try_from(value)
        .ok()
        .filter(|value| *value <= limit)
        .ok_or_else(|| invalid_static("dictionary length exceeds remaining payload"))
}

impl HistoricalDictionary {
    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn spells_in<F: Tables + ?Sized>(&self, frame: &F) -> bool {
        let Ok(rows) = frame.complete_rows::<tables::LexicalTokens>(LITERAL) else {
            return false;
        };
        let versions = lock(&self.shared);
        let mut rows = rows
            .keys()
            .zip(rows.texts(|row| &row.literal, LITERAL_COLUMN));
        versions.spellings.iter().all(|(token, spellings)| {
            spelling_at(spellings, self.version).is_none_or(|literal| {
                rows.next().is_some_and(|(key, text)| {
                    key as u32 == *token && text.is_ok_and(|text| text == literal)
                })
            })
        }) && rows.next().is_none()
    }

    pub fn token_of(&self, name: &[u8]) -> Result<Option<u32>, String> {
        let mut versions = lock(&self.shared);
        let outcome = versions
            .index()
            .get(&*name.to_ascii_lowercase())
            .and_then(|name| {
                let set = search(&name.outcomes, |(from, _)| *from <= self.version);
                Some(name.outcomes[set.checked_sub(1)?].1)
            });
        match outcome {
            None | Some(Outcome::None) => Ok(None),
            Some(Outcome::One(token)) => Ok(Some(token)),
            Some(Outcome::Several(token)) => Err(format!(
                "{} names several lexer tokens",
                String::from_utf8_lossy(
                    spelling_at(&versions.spellings[&token], self.version)
                        .expect("a spelled token")
                )
            )),
        }
    }
}

enum BoundArgument {
    Text(String),
    Token(i32),
}

fn formatted(
    value: &FormattedLocalization,
    arguments: &[(&str, Argument<'_>)],
    localisation: &Localisation,
    token_of: &mut impl FnMut(&[u8]) -> Result<Option<u32>, String>,
) -> Result<BoundArgument, String> {
    match value {
        FormattedLocalization::Key(key) => localisation
            .localise_with(key, arguments)
            .map(BoundArgument::Text)
            .map_err(|why| why.to_string()),
        FormattedLocalization::Token(token) => token_of(token)?
            .map(|token| BoundArgument::Token(token as i32))
            .ok_or_else(|| {
                format!(
                    "faction bound localisation token {} is absent from lexical tokens",
                    String::from_utf8_lossy(token)
                )
            }),
        FormattedLocalization::Formatter { formatter, .. } => Err(format!(
            "faction bound localisation formatter {} requires core registry callback",
            String::from_utf8_lossy(formatter)
        )),
    }
}

pub fn bound_name(
    bound: &BoundLocalization,
    localisation: &Localisation,
    mut token_of: impl FnMut(&[u8]) -> Result<Option<u32>, String>,
) -> Result<String, String> {
    enum Work<'a> {
        Node(usize),
        Value(&'a FormattedLocalization),
        CheckName(&'a [u8]),
        Finish(&'a BoundNode),
    }
    let mut work = vec![Work::Node(0)];
    let mut values = Vec::new();
    while let Some(next) = work.pop() {
        match next {
            Work::Node(index) => {
                let node = bound
                    .nodes
                    .get(index)
                    .ok_or("faction bound localisation node is absent")?;
                work.push(Work::Finish(node));
                for argument in node.arguments.iter().rev() {
                    work.push(Work::CheckName(&argument.name));
                    work.push(match &argument.value {
                        BoundValue::Localization(value) => Work::Value(value),
                        BoundValue::Node(child) => {
                            if *child <= index {
                                return Err(
                                    "faction bound localisation child is not a later preorder node"
                                        .into(),
                                );
                            }
                            Work::Node(*child)
                        }
                    });
                }
            }
            Work::Value(value) => values.push(formatted(value, &[], localisation, &mut token_of)?),
            Work::CheckName(name) => {
                std::str::from_utf8(name)
                    .map_err(|_| "faction bound argument name is not UTF-8")?;
            }
            Work::Finish(node) => {
                let first = values
                    .len()
                    .checked_sub(node.arguments.len())
                    .ok_or("faction bound localisation lacks argument values")?;
                let inputs: Vec<_> = values.drain(first..).collect();
                let arguments = node
                    .arguments
                    .iter()
                    .zip(&inputs)
                    .map(|(argument, value)| {
                        let name = std::str::from_utf8(&argument.name)
                            .map_err(|_| "faction bound argument name is not UTF-8")?;
                        Ok((
                            name,
                            match value {
                                BoundArgument::Text(text) => Argument::Text(text),
                                BoundArgument::Token(token) => Argument::Int32(*token),
                            },
                        ))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                values.push(formatted(
                    &node.key,
                    &arguments,
                    localisation,
                    &mut token_of,
                )?);
            }
        }
    }
    match values
        .pop()
        .ok_or("faction bound localisation node is absent")?
    {
        BoundArgument::Text(text) => Ok(text),
        BoundArgument::Token(_) => {
            Err("faction bound root returns a token integer, not the string variant".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spells(dictionary: &HistoricalDictionary, table: &SharedView) -> bool {
        let mut frame = replay_frame::frame::DecodedFrame {
            branch: 0,
            hour: 0,
            tables: std::array::from_fn(|_| replay_frame::frame::not_selected()),
            metrics: Default::default(),
        };
        frame.tables[tables::LexicalTokens::ID.index()] = table.clone();
        dictionary.spells_in(&frame)
    }
    use crate::catalogue::tests::{loaded, loaded_templates};

    #[test]
    fn faction_names_evaluate_nested_arguments_in_authored_order() {
        let db = loaded_templates(
            "pact={name={localization_key=ROOT \
             FIRST={localization_key=CHILD ID=token:first} SECOND=token:second}}",
        );
        let loc = loaded("l_english:\n ROOT:0 \"$FIRST$ / $SECOND$\"\n CHILD:0 \"Pact $ID$\"\n");
        let mut reads = Vec::new();
        let name = name(b"", Ok(Some("pact")), Ok(&db), &loc, "english", |token| {
            reads.push(token.to_vec());
            Ok(Some(if token == b"first" { 31 } else { 47 }))
        });
        assert_eq!(name.unwrap(), "Pact 31 / 47");
        assert_eq!(reads, [b"first".to_vec(), b"second".to_vec()]);
    }

    #[test]
    fn faction_names_keep_native_token_formatter_refusals_and_known_empty_names() {
        let db = loaded_templates(
            "token={name=token:17} formatted={name=country|target} \
             empty={name={ignored=value}} omitted={} localised_empty={name=EMPTY_NAME} \
             missing={name={localization_key=ROOT ID=token:absent}}",
        );
        let loc = loaded("l_english:\n ROOT:0 \"Pact $ID$\"\n EMPTY_NAME:0 \"\"\n");
        let resolved = |token| {
            name(b"", Ok(Some(token)), Ok(&db), &loc, "english", |_| {
                Ok(Some(99))
            })
        };
        assert!(
            resolved("token")
                .unwrap_err()
                .contains("root returns a token integer")
        );
        assert!(
            resolved("formatted")
                .unwrap_err()
                .contains("formatter country")
        );
        for template in ["empty", "omitted", "localised_empty"] {
            assert_eq!(
                name(b"", Ok(Some(template)), Ok(&db), &loc, "english", |_| {
                    panic!("this name must not read bound token arguments")
                }),
                Ok(String::new()),
                "{template}",
            );
        }
        assert!(
            name(b"", Ok(Some("missing")), Ok(&db), &loc, "english", |_| Ok(
                None
            ))
            .unwrap_err()
            .contains("token absent is absent from lexical tokens")
        );
    }

    #[test]
    fn a_recorded_null_template_is_empty_but_unread_template_inputs_remain_errors() {
        let loc = loaded("l_english:\n");
        assert_eq!(
            name(
                b"",
                Ok(None),
                Err("content gap".into()),
                &loc,
                "english",
                |_| { panic!("a null template must not read lexer tokens") }
            ),
            Ok(String::new()),
        );
        assert_eq!(
            name(
                b"",
                Err("dictionary gap".into()),
                Err("content gap".into()),
                &loc,
                "english",
                |_| panic!("unread template input must stop before arguments"),
            ),
            Err("dictionary gap".into()),
        );
    }

    fn lexical_table(rows: impl IntoIterator<Item = (u32, Vec<u8>)>) -> SharedView {
        TableView::Complete(TableRows::LexicalTokens(
            rows.into_iter()
                .map(|(token, literal)| replay_data::rows::LexicalTokenRow { token, literal })
                .collect(),
        ))
        .into()
    }

    fn table_rows(table: &SharedView) -> &[replay_data::rows::LexicalTokenRow] {
        match &**table {
            TableView::Complete(TableRows::LexicalTokens(rows)) => rows,
            _ => &[],
        }
    }

    fn encoded(history: &DictionaryHistory) -> Vec<u8> {
        let mut bytes = Vec::new();
        history.encode(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn dictionary_charges_hold_what_the_versions_and_their_lookup_take() {
        for count in [1_u32, 3, 4, 7, 8, 14, 15, 28, 29, 56, 57, 112, 113, 224, 225, 448, 449] {
            let mut history = DictionaryHistory::default();
            let mut dictionary = history
                .add(&lexical_table(
                    (0..count).map(|token| (token, format!("n{token}").into_bytes())),
                ))
                .unwrap()
                .unwrap();
            for version in 1..4 {
                let charged = history.held_bytes();
                assert!(charged >= history.measured_bytes(), "{count} before a lookup");
                dictionary.token_of(b"n0").unwrap();
                assert!(
                    charged >= history.measured_bytes(),
                    "{count} after a lookup at version {version}"
                );
                dictionary = history
                    .add(&lexical_table((0..count).map(|token| {
                        let name = if token % 2 == 0 {
                            format!("N{}", token / 2)
                        } else {
                            format!("v{version}x{token}")
                        };
                        (token, name.into_bytes())
                    })))
                    .unwrap()
                    .unwrap();
            }
            assert!(history.held_bytes() >= history.measured_bytes(), "{count}");
        }
    }

    #[test]
    fn historical_dictionaries_resolve_every_name_as_their_table_reads_do() {
        fn random(seed: &mut u64, bound: usize) -> usize {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 7;
            *seed ^= *seed << 17;
            (*seed % bound as u64) as usize
        }
        fn spelling(seed: &mut u64, token: u32, version: usize) -> Vec<u8> {
            if random(seed, 2) == 0 {
                return format!("unique_{token}_{version}").into_bytes();
            }
            let mut word = format!("word{}", random(seed, 24)).into_bytes();
            for byte in &mut word {
                if random(seed, 2) == 0 {
                    byte.make_ascii_uppercase();
                }
            }
            word
        }
        let check = |read: &[(SharedView, HistoricalDictionary)]| {
            let names: std::collections::BTreeSet<Vec<u8>> = (0..24)
                .flat_map(|word| [format!("word{word}"), format!("WORD{word}")])
                .map(String::into_bytes)
                .chain([b"absent".to_vec(), Vec::new()])
                .chain(read.iter().flat_map(|(table, _)| {
                    table_rows(table).iter().map(|row| row.literal.clone())
                }))
                .collect();
            let mut outcomes = [0; 3];
            for (table, dictionary) in read {
                let expected = LexicalNames::new(table.clone());
                for name in &names {
                    let resolved = dictionary.token_of(name);
                    assert_eq!(
                        resolved,
                        expected.token_of(name),
                        "{}",
                        String::from_utf8_lossy(name)
                    );
                    outcomes[match resolved {
                        Ok(None) => 0,
                        Ok(Some(_)) => 1,
                        Err(_) => 2,
                    }] += 1;
                }
            }
            assert!(outcomes.iter().all(|count| *count > 0), "{outcomes:?}");
        };
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut rows: BTreeMap<u32, Vec<u8>> = (0..300)
            .map(|token| (token * 3, spelling(&mut seed, token * 3, 0)))
            .collect();
        let mut history = DictionaryHistory::default();
        let mut read = Vec::new();
        for version in 0..48 {
            for _ in 0..random(&mut seed, 6) * usize::from(version > 0) {
                let present: Vec<u32> = rows.keys().copied().collect();
                let token = present[random(&mut seed, present.len())];
                match random(&mut seed, 4) {
                    0 => {
                        rows.remove(&token);
                    }
                    1 => {
                        let literal = rows[&token].to_ascii_uppercase();
                        rows.insert(token, literal);
                    }
                    2 => {
                        rows.insert(token, spelling(&mut seed, token, version));
                    }
                    _ => {
                        let token = random(&mut seed, 1_200) as u32;
                        rows.insert(token, spelling(&mut seed, token, version));
                    }
                }
            }
            let table = lexical_table(rows.clone());
            let dictionary = history.add(&table).unwrap().unwrap();
            assert_eq!(dictionary.version(), version as u32);
            read.push((table, dictionary));
            if version == 20 {
                check(&read);
            }
        }
        check(&read);
        for pair in read.windows(2) {
            assert!(spells(&pair[0].1, &pair[0].0));
            assert_eq!(spells(&pair[0].1, &pair[1].0), pair[0].0 == pair[1].0);
            assert_eq!(spells(&pair[1].1, &pair[0].0), pair[0].0 == pair[1].0);
        }
        assert!(
            !spells(&read[0].1,
                &TableView::Unavailable {
                    reason: "gap".into()
                }
                .into()
            )
        );
        let saved = encoded(&history);
        let one_table: usize = table_rows(&read[0].0)
            .iter()
            .map(|row| row.literal.len() + 2)
            .sum();
        assert!(
            saved.len() < 4 * one_table,
            "a version costs its changes, not its rows"
        );
        let mut input = Decoder::new(&saved);
        let restored = DictionaryHistory::decode(&mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(restored.count(), 48);
        assert!(restored.version(48).is_none());
        assert_eq!(encoded(&restored), saved);
        check(
            &read
                .iter()
                .map(|(table, dictionary)| {
                    (
                        table.clone(),
                        restored.version(dictionary.version()).unwrap(),
                    )
                })
                .collect::<Vec<_>>(),
        );
    }

    #[test]
    fn holders_adding_after_another_keep_their_own_versions() {
        let table = |rows: &[(u32, &str)]| {
            lexical_table(
                rows.iter()
                    .map(|(token, literal)| (*token, literal.as_bytes().to_vec())),
            )
        };
        let base = [
            table(&[(1, "a"), (2, "b")]),
            table(&[(1, "a"), (2, "B"), (3, "c")]),
        ];
        let mut first = DictionaryHistory::default();
        let kept: Vec<_> = base
            .iter()
            .map(|table| first.add(table).unwrap().unwrap())
            .collect();
        let untouched = first.clone();
        let mut second = first.clone();
        let left = table(&[(2, "x"), (3, "c")]);
        let right = table(&[(1, "x"), (4, "X")]);
        let from_first = first.add(&left).unwrap().unwrap();
        let from_second = second.add(&right).unwrap().unwrap();
        assert_eq!((from_first.version(), from_second.version()), (2, 2));
        assert_eq!(from_first.token_of(b"X"), Ok(Some(2)));
        assert_eq!(
            from_second.token_of(b"x"),
            Err("X names several lexer tokens".into())
        );
        assert_eq!(kept[1].token_of(b"b"), Ok(Some(2)));
        assert!(spells(&kept[1], &base[1]) && spells(&from_first, &left) && spells(&from_second, &right));
        for (holder, last) in [
            (&untouched, None),
            (&first, Some(&left)),
            (&second, Some(&right)),
        ] {
            let mut fresh = DictionaryHistory::default();
            for table in base.iter().chain(last) {
                fresh.add(table).unwrap();
            }
            assert_eq!(encoded(holder), encoded(&fresh));
        }
        assert!(
            DictionaryHistory::default()
                .add(
                    &TableView::Unavailable {
                        reason: "gap".into()
                    }
                    .into()
                )
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn lookups_search_each_name_s_outcomes_however_often_its_tokens_respell() {
        let mut history = DictionaryHistory::default();
        let read: Vec<_> = (0..4_096_u32)
            .map(|version| {
                let seven: &[u8] = match version % 3 {
                    0 => b"arg",
                    1 => b"ARG",
                    _ => b"other",
                };
                let mut rows = vec![(7, seven.to_vec())];
                if version % 5 == 0 {
                    rows.push((9, b"Arg".to_vec()));
                }
                let table = lexical_table(rows);
                let dictionary = history.add(&table).unwrap().unwrap();
                (table, dictionary)
            })
            .collect();
        let _ = read[0].1.token_of(b"arg");
        SEARCHED.with(|searched| searched.set(0));
        let mut lookups = 0;
        for (table, dictionary) in &read {
            let expected = LexicalNames::new(table.clone());
            for name in [&b"arg"[..], b"OTHER", b"absent"] {
                assert_eq!(dictionary.token_of(name), expected.token_of(name));
                lookups += 1;
            }
        }
        let searched = SEARCHED.with(std::cell::Cell::get);
        assert!(searched <= lookups * 2 * 13, "{searched} for {lookups}");
    }

    #[test]
    fn decoded_histories_refuse_truncation_long_literals_overflowing_gaps_and_a_full_count() {
        let mut history = DictionaryHistory::default();
        for rows in [
            vec![(3, b"a".to_vec()), (u32::MAX, b"top".to_vec())],
            vec![(3, b"b".to_vec())],
        ] {
            history.add(&lexical_table(rows)).unwrap().unwrap();
        }
        let saved = encoded(&history);
        assert!(DictionaryHistory::decode(&mut Decoder::new(&saved)).is_ok());
        for cut in 0..saved.len() {
            assert!(
                DictionaryHistory::decode(&mut Decoder::new(&saved[..cut])).is_err(),
                "{cut}"
            );
        }
        let decode = |values: &[u128]| {
            let mut bytes = Vec::new();
            for value in values {
                put_varint(&mut bytes, *value);
            }
            DictionaryHistory::decode(&mut Decoder::new(&bytes))
        };
        let top = u128::from(u32::MAX);
        assert!(decode(&[1, 1, 5, 1, 0, 2, 97]).is_ok());
        assert!(decode(&[1, 1, 5, 1, 0, 9, 97, 97]).is_err());
        assert!(decode(&[1, 1, 5, 1_000, 0, 2, 97]).is_err());
        assert!(decode(&[1, 1, top + 1, 1, 0, 2, 97]).is_err());
        assert!(decode(&[1, 2, 5, 1, 0, 2, 97, top, 1, 0, 2, 97]).is_err());
        assert!(decode(&[2, 1, 5, 2, 0, 2, 97, top, 0]).is_err());
        assert!(decode(&[top + 1, 0]).is_err());
        let mut full = decode(&[top, 0]).unwrap();
        let before = encoded(&full);
        assert!(full.add(&lexical_table([(1, b"a".to_vec())])).is_err());
        assert_eq!(full.count(), u32::MAX);
        assert_eq!(encoded(&full), before);
    }

    #[test]
    fn decoded_dictionary_histories_refuse_spellings_that_change_nothing() {
        let decode = |count: u128, entries: &[Option<&[u8]>]| {
            let mut bytes = Vec::new();
            for value in [count, 1, 7, entries.len() as u128] {
                put_varint(&mut bytes, value);
            }
            for literal in entries {
                put_varint(&mut bytes, 0);
                match literal {
                    None => put_varint(&mut bytes, 0),
                    Some(literal) => {
                        put_varint(&mut bytes, literal.len() as u128 + 1);
                        bytes.extend_from_slice(literal);
                    }
                }
            }
            DictionaryHistory::decode(&mut Decoder::new(&bytes)).map(|history| history.count())
        };
        let a = Some(&b"a"[..]);
        assert_eq!(decode(2, &[a, None]).unwrap(), 2);
        assert!(decode(1, &[None]).is_err());
        assert!(decode(2, &[a, a]).is_err());
        assert!(decode(1, &[a, None]).is_err());
    }

    #[test]
    fn faction_names_dictionary_folds_ascii_and_refuses_ambiguity_and_gaps() {
        let dictionary =
            |rows| LexicalNames::new(TableView::Complete(TableRows::LexicalTokens(rows)).into());
        let tokens = dictionary(vec![replay_data::rows::LexicalTokenRow {
            token: 31,
            literal: b"MiXeD".to_vec(),
        }]);
        assert_eq!(tokens.token_of(b"mixed").unwrap(), Some(31));
        assert_eq!(tokens.token_of(b"absent").unwrap(), None);
        let ambiguous = dictionary(vec![
            replay_data::rows::LexicalTokenRow {
                token: 31,
                literal: b"Mixed".to_vec(),
            },
            replay_data::rows::LexicalTokenRow {
                token: 47,
                literal: b"MIXED".to_vec(),
            },
        ]);
        assert!(
            ambiguous
                .token_of(b"mixed")
                .unwrap_err()
                .contains("several lexer tokens")
        );
        let missing = LexicalNames::new(
            TableView::Unavailable {
                reason: "not captured".into(),
            }
            .into(),
        );
        assert!(
            missing
                .token_of(b"mixed")
                .unwrap_err()
                .contains("not captured")
        );
    }

    #[test]
    fn faction_names_keep_every_bound_node() {
        let count = 300;
        let loc = loaded("l_english:\n PASS:0 \"$ARG$\"\n");
        let bound = BoundLocalization {
            nodes: (0..count)
                .map(|index| BoundNode {
                    key: FormattedLocalization::Key(b"PASS".to_vec()),
                    arguments: vec![content_reader_script::localization_binding::BoundArgument {
                        name: b"ARG".to_vec(),
                        value: if index + 1 == count {
                            BoundValue::Localization(FormattedLocalization::Key(
                                b"complete".to_vec(),
                            ))
                        } else {
                            BoundValue::Node(index + 1)
                        },
                    }],
                })
                .collect(),
        };
        assert_eq!(bound_name(&bound, &loc, |_| Ok(None)).unwrap(), "complete");
    }

    #[test]
    fn faction_names_refuse_an_invalid_argument_name_before_later_arguments() {
        let loc = loaded("l_english:\n ROOT:0 \"$ARG$\"\n");
        let bound = BoundLocalization {
            nodes: vec![BoundNode {
                key: FormattedLocalization::Key(b"ROOT".to_vec()),
                arguments: vec![
                    content_reader_script::localization_binding::BoundArgument {
                        name: vec![0xff],
                        value: BoundValue::Localization(FormattedLocalization::Key(
                            b"first".to_vec(),
                        )),
                    },
                    content_reader_script::localization_binding::BoundArgument {
                        name: b"ARG".to_vec(),
                        value: BoundValue::Localization(FormattedLocalization::Token(
                            b"later".to_vec(),
                        )),
                    },
                ],
            }],
        };
        let mut reads = 0;
        let result = bound_name(&bound, &loc, |_| {
            reads += 1;
            Ok(Some(31))
        });
        assert_eq!(
            result.unwrap_err(),
            "faction bound argument name is not UTF-8"
        );
        assert_eq!(reads, 0);
    }
}
