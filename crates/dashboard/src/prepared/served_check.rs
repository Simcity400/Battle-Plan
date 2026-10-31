//! The served-reply check of one prepared family: every product the family
//! serves asks its plan's requests of a reference store, whose replies are kept
//! as a transcript ([`record`]); a candidate store is then asked each of them
//! again and its replies compared with the kept ones, semantically ([`check`]).
//! Stored formats are free to differ; the data users are served is not.

use crate::{Source, SourceKey, products::ContentOptions, transport::ProductRequest};
use dashboard_base::prepared::served::{Ask, Branch, Plan};
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

#[cfg(test)]
mod tests;

/// The transcript's layout.
const VERSION: u32 = 1;
/// Requests one plan asks of each branch at most: past it the plan, not the
/// store, is wrong.
const PER_BRANCH: usize = 4096;
const SOURCE: &str = "served-check";

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

/// A store's reply to one request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Reply {
    /// The reply without the request it echoes: its data, and the gate a
    /// pending or partial one waits for.
    Answered(Value),
    Refused(String),
}

static NULL: Value = Value::Null;

impl Reply {
    fn data(&self) -> Option<&Value> {
        match self {
            Self::Answered(reply) => Some(reply.get("data").unwrap_or(&NULL)),
            Self::Refused(_) => None,
        }
    }
}

/// What asks a family's requests: a product's plan, or the generic plan of a
/// product with none.
#[derive(Clone, Copy)]
pub enum Owner {
    Plan(&'static str, &'static Plan),
    Generic(&'static str),
}

impl Owner {
    pub fn name(&self) -> &'static str {
        match *self {
            Self::Plan(name, _) | Self::Generic(name) => name,
        }
    }
}

/// The products `family` serves, as the registry lists them, each asked by
/// its plan or else the generic one; a plan several of them share, once.
pub fn owners(family: &str) -> io::Result<Vec<Owner>> {
    use super::driver::registry;
    let id = registry::FamilyId::parse(family)?;
    let descriptor = registry::FAMILIES
        .iter()
        .find(|descriptor| descriptor.id == id)
        .ok_or_else(|| invalid(format!("Unknown prepared family {family:?}")))?;
    let mut owners: Vec<Owner> = Vec::new();
    for &name in descriptor.queries {
        let owner = match crate::products::find(name).and_then(|product| product.served) {
            Some(plan) => Owner::Plan(name, plan),
            None => Owner::Generic(name),
        };
        let shared = owners.iter().any(|held| match (held, &owner) {
            (Owner::Plan(_, held), Owner::Plan(_, plan)) => std::ptr::eq(*held, *plan),
            _ => false,
        });
        if !shared {
            owners.push(owner);
        }
    }
    Ok(owners)
}

/// Asks `product`'s branch-wide request on each branch: all a product
/// without a plan of its own can be asked.
fn generic(ask: &mut dyn Ask, product: &str) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        ask.ask(product, json!({ "branchId": branch.name() }))?;
    }
    Ok(())
}

/// The name a request routes by: its series family kind, or else its product.
fn routed<'a>(product: &'a str, params: &'a Value) -> &'a str {
    match product {
        "series-state" | "series-summary" | "series-next-change" | "events-page"
        | "events-clusters" => params
            .get("family")
            .and_then(|family| family.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or(product),
        _ => product,
    }
}

/// What the report counts a request under: its product, and the series
/// family kind it routes by.
fn label(product: &str, params: &Value) -> String {
    match routed(product, params) {
        name if name == product => product.to_owned(),
        name => format!("{product}:{name}"),
    }
}

/// The transcript's first line.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Header {
    pub served_check: u32,
    pub family: String,
    /// The recording's revision the reference store was asked under.
    pub revision: String,
    pub branches: Vec<Branch>,
}

/// One request and the reference store's reply, as the transcript keeps it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Asked<'a> {
    owner: &'a str,
    product: &'a str,
    params: &'a Value,
    reply: &'a Reply,
    unordered: &'a [&'static [&'static str]],
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Exchange {
    product: String,
    params: Value,
    reply: Reply,
    #[serde(default)]
    unordered: Vec<Vec<String>>,
}

/// What one owner asked of the reference store.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recorded {
    pub owner: String,
    pub generic: bool,
    pub asked: usize,
    pub answered: usize,
    /// Answered null: its coverage not accepted.
    pub pending: usize,
    pub refused: usize,
    pub first_refusal: Option<String>,
}

impl Recorded {
    /// Whether nothing it asked was answered: its plan checks nothing.
    pub fn unchecked(&self) -> bool {
        self.answered == 0
    }
}

struct Recorder<'a, 's, 'w> {
    branches: &'a [Branch],
    owners: &'a [Owner],
    serve: &'a mut (dyn FnMut(&str, &Value) -> Reply + 's),
    out: &'a mut (dyn Write + 'w),
    seen: HashSet<String>,
    countries: BTreeMap<u64, Vec<String>>,
    owner: &'static str,
    tally: Recorded,
    limit: usize,
}

impl Recorder<'_, '_, '_> {
    /// The arrays of `name`'s replies whose order means nothing, as its plan says.
    fn unordered(&self, name: &str) -> &'static [&'static [&'static str]] {
        self.owners
            .iter()
            .find_map(|owner| match owner {
                Owner::Plan(held, plan) if *held == name => Some(plan.unordered),
                _ => None,
            })
            .or_else(|| {
                crate::products::find(name)
                    .and_then(|product| product.served)
                    .map(|plan| plan.unordered)
            })
            .unwrap_or(&[])
    }
}

impl Ask for Recorder<'_, '_, '_> {
    fn branches(&self) -> &[Branch] {
        self.branches
    }

    fn ask(&mut self, product: &str, params: Value) -> io::Result<Option<Value>> {
        let reply = (self.serve)(product, &params);
        if self.seen.insert(format!("{product}\n{params}")) {
            self.tally.asked += 1;
            if self.tally.asked > self.limit {
                return Err(invalid(format!(
                    "{} asked more than {} requests",
                    self.owner, self.limit
                )));
            }
            match reply.data() {
                None => {
                    self.tally.refused += 1;
                    if self.tally.first_refusal.is_none()
                        && let Reply::Refused(why) = &reply
                    {
                        self.tally.first_refusal = Some(format!("{product}: {why}"));
                    }
                }
                Some(Value::Null) => self.tally.pending += 1,
                Some(_) => self.tally.answered += 1,
            }
            let asked = Asked {
                owner: self.owner,
                product,
                params: &params,
                reply: &reply,
                unordered: self.unordered(routed(product, &params)),
            };
            serde_json::to_writer(&mut *self.out, &asked)?;
            self.out.write_all(b"\n")?;
        }
        Ok(match reply {
            Reply::Answered(mut reply) => {
                Some(reply.get_mut("data").map_or(Value::Null, Value::take))
            }
            Reply::Refused(_) => None,
        })
    }

    fn countries(&mut self, branch: u64) -> io::Result<Vec<String>> {
        if let Some(held) = self.countries.get(&branch) {
            return Ok(held.clone());
        }
        // Discovery, not one of the owner's exchanges: neither recorded nor counted, so a store
        // holding the checked family alone is compared on that family's replies.
        let reply = (self.serve)(
            "country-identities",
            &json!({ "branchId": branch.to_string() }),
        );
        let data = match reply {
            Reply::Answered(mut reply) => reply.get_mut("data").map(Value::take),
            Reply::Refused(_) => None,
        };
        let mut tags: Vec<String> = data
            .as_ref()
            .and_then(|data| data.get("countries"))
            .and_then(Value::as_object)
            .map(|countries| countries.keys().cloned().collect())
            .unwrap_or_default();
        tags.sort_unstable();
        tags.dedup();
        self.countries.insert(branch, tags.clone());
        Ok(tags)
    }
}

/// Asks every owner's requests of the reference store `serve` answers,
/// writing the transcript to `out`: its header, then each request once, with
/// its reply. What each owner asked, in order.
pub fn record(
    family: &str,
    revision: &str,
    owners: &[Owner],
    branches: &[Branch],
    serve: &mut dyn FnMut(&str, &Value) -> Reply,
    out: &mut dyn Write,
) -> io::Result<Vec<Recorded>> {
    let header = Header {
        served_check: VERSION,
        family: family.to_owned(),
        revision: revision.to_owned(),
        branches: branches.to_vec(),
    };
    serde_json::to_writer(&mut *out, &header)?;
    out.write_all(b"\n")?;
    let mut recorder = Recorder {
        branches,
        owners,
        serve,
        out,
        seen: HashSet::new(),
        countries: BTreeMap::new(),
        owner: "",
        tally: Recorded::default(),
        limit: PER_BRANCH * branches.len().max(1),
    };
    let mut recorded = Vec::with_capacity(owners.len());
    for owner in owners {
        recorder.owner = owner.name();
        recorder.tally = Recorded {
            owner: owner.name().to_owned(),
            generic: matches!(owner, Owner::Generic(_)),
            ..Recorded::default()
        };
        match owner {
            Owner::Plan(_, plan) => (plan.queries)(&mut recorder)?,
            Owner::Generic(name) => generic(&mut recorder, name)?,
        }
        recorded.push(std::mem::take(&mut recorder.tally));
    }
    Ok(recorded)
}

/// The transcript's header, read from its first line.
pub fn read_header(transcript: &mut dyn BufRead) -> io::Result<Header> {
    let mut line = String::new();
    transcript.read_line(&mut line)?;
    let header: Header = serde_json::from_str(&line)?;
    if header.served_check != VERSION {
        return Err(invalid(format!(
            "The transcript's layout {} is not this check's {VERSION}",
            header.served_check
        )));
    }
    Ok(header)
}

/// Asks the candidate store `serve` answers every request of the transcript
/// after its header, comparing each reply with the reference's.
pub fn check(
    transcript: &mut dyn BufRead,
    serve: &mut dyn FnMut(&str, &Value) -> Reply,
) -> io::Result<Report> {
    let mut report = Report::default();
    let mut line = String::new();
    loop {
        line.clear();
        if transcript.read_line(&mut line)? == 0 {
            return Ok(report);
        }
        if line.trim().is_empty() {
            continue;
        }
        let mut exchange: Exchange = serde_json::from_str(&line)?;
        let mut candidate = serve(&exchange.product, &exchange.params);
        if let Some(form) = compared_as(&exchange.product) {
            exchange.reply = in_form(form, &exchange.reply);
            candidate = in_form(form, &candidate);
        }
        report.add(exchange, &candidate);
    }
}

/// The form a product's replies are compared in, for one that names its parts by ids its
/// preparation numbers as it likes: each id read as what it names.
fn compared_as(product: &str) -> Option<fn(&Value) -> io::Result<Value>> {
    use crate::products::division_equipment::history;
    (product == history::QUERY)
        .then_some(history::served::canonical as fn(&Value) -> io::Result<Value>)
}

/// `reply` with its data in `form`; a reply whose data `form` cannot read is refused by why.
fn in_form(form: fn(&Value) -> io::Result<Value>, reply: &Reply) -> Reply {
    let Reply::Answered(envelope) = reply else {
        return reply.clone();
    };
    let mut envelope = envelope.clone();
    if let Some(data) = envelope.get_mut("data") {
        match form(data) {
            Ok(formed) => *data = formed,
            Err(error) => return Reply::Refused(error.to_string()),
        }
    }
    Reply::Answered(envelope)
}

/// How the candidate's replies compare with the reference's.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub queries: usize,
    pub differing: usize,
    /// By product, or by product and the series family kind it routes by.
    pub products: BTreeMap<String, Tally>,
}

/// The requests of one product, by the reference's replies.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tally {
    pub queries: usize,
    pub answered: usize,
    /// Answered null: its coverage not accepted.
    pub pending: usize,
    pub refused: usize,
    pub differing: usize,
    /// Its first request whose replies differ, and where.
    pub first: Option<Mismatch>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mismatch {
    pub product: String,
    pub params: Value,
    pub difference: Difference,
}

/// Where two replies first differ, and what each holds there.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Difference {
    /// From the reply's root, as `data.series[3].runs[1]`; empty for the
    /// replies as a whole.
    pub path: String,
    pub reference: Side,
    pub candidate: Side,
}

/// What one reply holds where two differ.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Side {
    /// Nothing, not even null.
    Absent,
    Value(Value),
    Refused(String),
}

impl Report {
    fn add(&mut self, exchange: Exchange, candidate: &Reply) {
        let tally = self
            .products
            .entry(label(&exchange.product, &exchange.params))
            .or_default();
        self.queries += 1;
        tally.queries += 1;
        match exchange.reply.data() {
            None => tally.refused += 1,
            Some(Value::Null) => tally.pending += 1,
            Some(_) => tally.answered += 1,
        }
        if let Some(difference) = difference(&exchange.reply, candidate, &exchange.unordered) {
            self.differing += 1;
            tally.differing += 1;
            if tally.first.is_none() {
                tally.first = Some(Mismatch {
                    product: exchange.product,
                    params: exchange.params,
                    difference,
                });
            }
        }
    }
}

/// Where `candidate` first differs from `reference`: objects member by
/// member, arrays element by element, those at an `unordered` path from the
/// data as multisets, numbers by their exact value, absent apart from null.
pub fn difference(
    reference: &Reply,
    candidate: &Reply,
    unordered: &[Vec<String>],
) -> Option<Difference> {
    let side = |reply: &Reply| match reply {
        Reply::Answered(reply) => Side::Value(reply.clone()),
        Reply::Refused(why) => Side::Refused(why.clone()),
    };
    match (reference, candidate) {
        (Reply::Answered(reference), Reply::Answered(candidate)) => {
            first(reference, candidate, &mut Vec::new(), unordered)
        }
        (Reply::Refused(reference), Reply::Refused(candidate)) if reference == candidate => None,
        _ => Some(Difference {
            path: String::new(),
            reference: side(reference),
            candidate: side(candidate),
        }),
    }
}

enum Step {
    Key(String),
    Index(usize),
}

fn render(path: &[Step]) -> String {
    let mut text = String::new();
    for step in path {
        match step {
            Step::Key(key) => {
                if !text.is_empty() {
                    text.push('.');
                }
                text.push_str(key);
            }
            Step::Index(at) => text.push_str(&format!("[{at}]")),
        }
    }
    text
}

fn present(value: Option<&Value>) -> Side {
    value.map_or(Side::Absent, |value| Side::Value(value.clone()))
}

fn differs(path: &[Step], reference: Option<&Value>, candidate: Option<&Value>) -> Difference {
    Difference {
        path: render(path),
        reference: present(reference),
        candidate: present(candidate),
    }
}

/// Whether the array at `path` is one whose order means nothing.
fn unordered_at(path: &[Step], unordered: &[Vec<String>]) -> bool {
    let Some((Step::Key(root), rest)) = path.split_first() else {
        return false;
    };
    root == "data"
        && unordered.iter().any(|pattern| {
            pattern.len() == rest.len()
                && pattern.iter().zip(rest).all(|(want, step)| {
                    want == "*" || matches!(step, Step::Key(key) if key == want)
                })
        })
}

fn first(
    reference: &Value,
    candidate: &Value,
    path: &mut Vec<Step>,
    unordered: &[Vec<String>],
) -> Option<Difference> {
    match (reference, candidate) {
        (Value::Object(held), Value::Object(other)) => {
            let mut keys: Vec<&String> = held.keys().chain(other.keys()).collect();
            keys.sort_unstable();
            keys.dedup();
            for key in keys {
                path.push(Step::Key(key.clone()));
                let found = match (held.get(key), other.get(key)) {
                    (Some(a), Some(b)) => first(a, b, path, unordered),
                    (a, b) => Some(differs(path, a, b)),
                };
                path.pop();
                if found.is_some() {
                    return found;
                }
            }
            None
        }
        (Value::Array(held), Value::Array(other)) if unordered_at(path, unordered) => {
            multiset(held, other, path)
        }
        (Value::Array(held), Value::Array(other)) => {
            for at in 0..held.len().max(other.len()) {
                path.push(Step::Index(at));
                let found = match (held.get(at), other.get(at)) {
                    (Some(a), Some(b)) => first(a, b, path, unordered),
                    (a, b) => Some(differs(path, a, b)),
                };
                path.pop();
                if found.is_some() {
                    return found;
                }
            }
            None
        }
        (Value::Number(a), Value::Number(b)) if same_number(a, b) => None,
        (a, b) if !matches!(a, Value::Number(_)) && a == b => None,
        (a, b) => Some(differs(path, Some(a), Some(b))),
    }
}

/// The first element one array holds more often than the other.
fn multiset(reference: &[Value], candidate: &[Value], path: &mut Vec<Step>) -> Option<Difference> {
    let mut surplus: HashMap<String, isize> = HashMap::new();
    for item in candidate {
        *surplus.entry(canonical(item)).or_default() += 1;
    }
    for (at, item) in reference.iter().enumerate() {
        let count = surplus.entry(canonical(item)).or_default();
        if *count == 0 {
            path.push(Step::Index(at));
            let found = differs(path, Some(item), None);
            path.pop();
            return Some(found);
        }
        *count -= 1;
    }
    for (at, item) in candidate.iter().enumerate() {
        if let Some(count) = surplus.get_mut(&canonical(item))
            && *count > 0
        {
            path.push(Step::Index(at));
            let found = differs(path, None, Some(item));
            path.pop();
            return Some(found);
        }
    }
    None
}

/// `value`'s text with every object's members in key order.
fn canonical(value: &Value) -> String {
    match value {
        Value::Object(members) => {
            let mut keys: Vec<&String> = members.keys().collect();
            keys.sort_unstable();
            let fields: Vec<String> = keys
                .into_iter()
                .map(|key| format!("{}:{}", Value::from(key.as_str()), canonical(&members[key])))
                .collect();
            format!("{{{}}}", fields.join(","))
        }
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(canonical).collect();
            format!("[{}]", items.join(","))
        }
        other => other.to_string(),
    }
}

fn integer(number: &Number) -> Option<i128> {
    number
        .as_i64()
        .map(i128::from)
        .or_else(|| number.as_u64().map(i128::from))
}

/// Whether two numbers hold the same value exactly: floats bit for bit, an
/// integer and a float only when the float is that integer.
fn same_number(a: &Number, b: &Number) -> bool {
    let whole = |integer: i128, float: f64| {
        float.fract() == 0.0
            && float.abs() < 2f64.powi(64)
            && float as i128 == integer
            && (float != 0.0 || float.is_sign_positive())
    };
    match (integer(a), integer(b)) {
        (Some(a), Some(b)) => a == b,
        (None, None) => a
            .as_f64()
            .zip(b.as_f64())
            .is_some_and(|(a, b)| a.to_bits() == b.to_bits()),
        (Some(integer), None) => b.as_f64().is_some_and(|float| whole(integer, float)),
        (None, Some(integer)) => a.as_f64().is_some_and(|float| whole(integer, float)),
    }
}

/// The local root a prepared store folder (`…\HOI4Replay\prepared\<store>`)
/// lies under: what `LOCALAPPDATA` names while it is read.
pub fn store_root(folder: &Path) -> io::Result<PathBuf> {
    use std::ffi::OsStr;
    let folder = std::fs::canonicalize(folder)?;
    folder
        .parent()
        .filter(|prepared| prepared.file_name() == Some(OsStr::new("prepared")))
        .and_then(Path::parent)
        .filter(|app| app.file_name() == Some(OsStr::new("HOI4Replay")))
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            invalid(format!(
                "{} is not a prepared store folder (…\\HOI4Replay\\prepared\\<store>)",
                folder.display()
            ))
        })
}

/// Each branch's columnar segments' first hours, from the columnar conversion
/// at `path`.
pub fn segments(path: &Path) -> io::Result<BTreeMap<u64, Vec<i32>>> {
    use replay::columnar_frames::{Limits, Recording};
    let file = Recording::open_path(path, Limits::default())?;
    if let Some(damage) = file.damage().first() {
        return Err(invalid(format!("The columnar file is damaged: {damage}")));
    }
    let mut segments: BTreeMap<u64, Vec<i32>> = BTreeMap::new();
    for segment in file.segments() {
        let run = file.run(segment.run)?;
        let first = i32::try_from(i64::from(run.first_hour) + i64::from(segment.first))
            .map_err(|_| invalid("A columnar segment starts outside the hours"))?;
        segments.entry(run.branch).or_default().push(first);
    }
    for hours in segments.values_mut() {
        hours.sort_unstable();
        hours.dedup();
    }
    Ok(segments)
}

/// A prepared store's replies to a recording's requests, through the prepared
/// query path the app serves them by.
pub struct Served {
    source: Source,
    content: Option<ContentOptions>,
}

impl Served {
    /// The store in `folder` holding a finished preparation of `recording`
    /// under `content`, or `family`'s native publication complete, as its
    /// measured preparation alone leaves it, while `LOCALAPPDATA` names
    /// [`store_root`] of it.
    pub fn open(
        recording: &Path,
        folder: &Path,
        content: Option<ContentOptions>,
        family: &str,
    ) -> io::Result<Self> {
        let expected = store::local_root()?
            .join(dashboard_base::prepared::recording_identity(recording)?.to_string());
        if std::fs::canonicalize(&expected).ok() != Some(std::fs::canonicalize(folder)?) {
            return Err(invalid(format!(
                "{} is not the prepared store of {}",
                folder.display(),
                recording.display()
            )));
        }
        let cancelled = AtomicBool::new(false);
        let source = Source::open(
            SourceKey {
                source_id: SOURCE.into(),
                revision: crate::protocol::replay_revision(recording)?,
            },
            recording.to_owned(),
            &cancelled,
        )?;
        let (store, key) = super::files(&source, content.as_ref())?;
        if !store.is_reusable(&key)? {
            return Err(invalid(format!(
                "{} holds no preparation of this recording under this content",
                folder.display()
            )));
        }
        if super::pipeline::restore(&source, content.as_ref(), &cancelled)?.is_none()
            && !super::driver::native::complete(&source, content.as_ref(), family)?
        {
            return Err(invalid(format!(
                "{} holds an unfinished preparation of {family} of this recording",
                folder.display()
            )));
        }
        Ok(Self { source, content })
    }

    /// The recording's revision, which requests name.
    pub fn revision(&self) -> &str {
        &self.source.key().revision
    }

    /// The recording's branches, with each one's columnar segments' first
    /// hours from `segments`.
    pub fn branches(&self, segments: &BTreeMap<u64, Vec<i32>>) -> Vec<Branch> {
        let replay = self.source.replay_ref();
        replay
            .branches
            .iter()
            .map(|start| {
                let (first_hour, last_hour) = crate::metadata::branch_bounds(replay, start);
                Branch {
                    id: start.id,
                    first_hour,
                    last_hour,
                    windows: self
                        .source
                        .block_windows(start.id)
                        .iter()
                        .map(|span| (span.first_hour, span.last_hour))
                        .collect(),
                    gaps: replay
                        .gaps
                        .iter()
                        .filter(|gap| gap.branch == start.id)
                        .map(|gap| (gap.first_hour, gap.last_hour))
                        .collect(),
                    segments: segments.get(&start.id).cloned().unwrap_or_default(),
                }
            })
            .collect()
    }

    /// `product`'s reply to `params`, as the app serves it.
    pub fn serve(&self, product: &str, params: &Value) -> Reply {
        let request = ProductRequest {
            source_id: SOURCE.into(),
            revision: self.source.key().revision.clone(),
            product: product.into(),
            params: params.clone(),
        };
        let cancelled = AtomicBool::new(false);
        let routed = crate::products::find(product).filter(|_| !super::query::answers(product));
        let answered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match routed {
            // The product's own route, as the service asks it.
            Some(registered) => {
                let context = crate::config::LoadContext {
                    content: self.content.clone(),
                };
                (registered.load)(&self.source, SOURCE, params.clone(), &context, &cancelled)
                    .map_err(io::Error::other)
            }
            None => super::query::query(&self.source, &request, self.content.as_ref(), &cancelled),
        }));
        match answered {
            Ok(Ok(text)) => match serde_json::from_str::<Value>(&text) {
                Ok(Value::Object(mut reply)) => {
                    for echo in ["sourceId", "revision", "product"] {
                        reply.remove(echo);
                    }
                    Reply::Answered(Value::Object(reply))
                }
                Ok(_) => Reply::Refused("The reply is not an object".into()),
                Err(error) => Reply::Refused(format!("The reply is not JSON: {error}")),
            },
            Ok(Err(error)) => Reply::Refused(error.to_string()),
            Err(panic) => Reply::Refused(format!(
                "The query panicked: {}",
                panic
                    .downcast_ref::<&str>()
                    .map(|text| (*text).to_owned())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_default()
            )),
        }
    }
}
