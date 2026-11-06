//! The served-reply check of one prepared family: every product the family
//! serves asks its plan's requests of a reference store, whose replies are kept
//! as a transcript ([`record`]); a candidate store is then asked each of them
//! again and its replies compared with the kept ones, semantically ([`check`]).
//! Stored formats are free to differ; the data users are served is not.

use crate::{Source, SourceKey, products::ContentOptions, transport::ProductRequest};
use dashboard_base::prepared::served::{Ask, Branch, Holder, Plan};
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::{self, BufRead, Write},
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

#[cfg(test)]
mod tests;

/// The transcript's layout.
const VERSION: u32 = 1;
/// Default requests per plan and branch; discovery-driven plans declare their own bound.
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
/// its plan or else the generic one; a plan several of them share, once. A
/// recording-wide job serves the products it gates.
pub fn owners(family: &str) -> io::Result<Vec<Owner>> {
    use super::driver::registry;
    let job = registry::job(family);
    let queries = match job {
        Some(job) => job.gates,
        None => {
            let id = registry::FamilyId::parse(family)?;
            registry::FAMILIES
                .iter()
                .find(|descriptor| descriptor.id == id)
                .ok_or_else(|| invalid(format!("Unknown prepared family {family:?}")))?
                .queries
        }
    };
    let mut owners: Vec<Owner> = Vec::new();
    for &name in queries {
        let plan = crate::products::find(name)
            .and_then(|product| product.served)
            .or_else(|| unnamed_plan(name));
        let owner = match plan {
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

/// The plan of a prepared query no product is named after.
fn unnamed_plan(name: &str) -> Option<&'static Plan> {
    match name {
        "game-score" => Some(&crate::products::game_score::served::PLAN),
        "focus-hour" | "focus-unavailable" | "focus-order-page" | "focus-completions" => {
            Some(&crate::prepared_focus::served::PLAN)
        }
        _ => None,
    }
}

pub fn family_name(name: &str) -> io::Result<&'static str> {
    use super::driver::registry;
    match registry::job(name) {
        Some(job) => Ok(job.name),
        None => Ok(registry::FamilyId::parse(name)?.name()),
    }
}

pub fn families(selection: &str) -> io::Result<Vec<String>> {
    use super::driver::registry;
    if selection == "all" {
        return Ok(registry::FAMILIES
            .iter()
            .map(|family| family.id.name())
            .chain(
                registry::JOBS
                    .iter()
                    .filter(|job| !job.gates.is_empty())
                    .map(|job| job.name),
            )
            .map(str::to_owned)
            .collect());
    }
    let mut seen = HashSet::new();
    let mut families = Vec::new();
    for name in selection.split(',') {
        let id = family_name(name)?;
        if !seen.insert(id) {
            return Err(invalid(format!(
                "Prepared family {name:?} was selected twice"
            )));
        }
        families.push(name.to_owned());
    }
    Ok(families)
}

pub fn parallel<T: Sync, R: Send>(
    jobs: &[T],
    threads: NonZeroUsize,
    run: impl Fn(&T) -> R + Sync,
) -> io::Result<Vec<R>> {
    let next = AtomicUsize::new(0);
    let results = Mutex::new(Vec::with_capacity(jobs.len()));
    std::thread::scope(|scope| -> io::Result<()> {
        let mut workers = Vec::new();
        for worker in 0..threads.get().min(jobs.len()) {
            let (run, next, results) = (&run, &next, &results);
            workers.push(
                std::thread::Builder::new()
                    .name(format!("served-check-{worker}"))
                    .spawn_scoped(scope, move || {
                        loop {
                            let index = next.fetch_add(1, Ordering::Relaxed);
                            let Some(job) = jobs.get(index) else { break };
                            let result = run(job);
                            results.lock().unwrap().push((index, result));
                        }
                    })?,
            );
        }
        for worker in workers {
            worker
                .join()
                .map_err(|_| io::Error::other("A served-check worker panicked"))?;
        }
        Ok(())
    })?;
    let mut results = results.into_inner().unwrap();
    results.sort_unstable_by_key(|(index, _)| *index);
    Ok(results.into_iter().map(|(_, result)| result).collect())
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
    recording: Option<&'a replay::container::Replay>,
    owners: &'a [Owner],
    serve: &'a mut (dyn FnMut(&str, &Value) -> Reply + 's),
    out: &'a mut (dyn Write + 'w),
    seen: HashSet<String>,
    countries: BTreeMap<u64, Discovered>,
    owner: &'static str,
    tally: Recorded,
    limit: usize,
    allowances: HashMap<u64, usize>,
}

/// A branch's countries as its identity history names them.
#[derive(Default)]
struct Discovered {
    /// In tag order.
    tags: Vec<String>,
    holders: Vec<Holder>,
}

impl Recorder<'_, '_, '_> {
    /// `branch`'s countries. Discovery, not one of the owner's exchanges: neither recorded nor
    /// counted, so a store holding the checked family alone is compared on that family's
    /// replies.
    fn discover(&mut self, branch: u64) -> &Discovered {
        if !self.countries.contains_key(&branch) {
            let reply = (self.serve)(
                "country-identities",
                &json!({ "branchId": branch.to_string() }),
            );
            let data = match reply {
                Reply::Answered(mut reply) => reply.get_mut("data").map(Value::take),
                Reply::Refused(_) => None,
            };
            let countries = data
                .as_ref()
                .and_then(|data| data.get("countries"))
                .and_then(Value::as_object);
            let mut tags: Vec<String> = countries
                .map(|countries| countries.keys().cloned().collect())
                .unwrap_or_default();
            tags.sort_unstable();
            tags.dedup();
            let number = |value: &Value, field: &str| {
                value
                    .get(field)
                    .and_then(Value::as_u64)
                    .and_then(|number| u32::try_from(number).ok())
            };
            let mut origins = BTreeMap::new();
            for (tag, country) in countries.into_iter().flatten() {
                let identities = country.get("identities").and_then(Value::as_array);
                for identity in identities.into_iter().flatten() {
                    if let (Some(slot), Some(id), Some(first), Some(last)) = (
                        number(identity, "slot"),
                        number(identity, "countryId"),
                        identity["firstHour"]
                            .as_i64()
                            .and_then(|hour| i32::try_from(hour).ok()),
                        identity["lastHour"]
                            .as_i64()
                            .and_then(|hour| i32::try_from(hour).ok()),
                    ) && first <= last
                    {
                        origins
                            .entry((tag.clone(), slot, id))
                            .and_modify(|origin: &mut i32| *origin = (*origin).min(first))
                            .or_insert(first);
                    }
                }
            }
            let holders = origins
                .into_iter()
                .map(|((tag, slot, country_id), origin_hour)| Holder {
                    tag,
                    slot,
                    country_id,
                    origin_hour,
                })
                .collect();
            self.countries.insert(branch, Discovered { tags, holders });
        }
        &self.countries[&branch]
    }

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

    fn recording(&self) -> io::Result<&replay::container::Replay> {
        self.recording
            .ok_or_else(|| invalid("This served plan needs the recording's rows"))
    }

    fn request_allowance(&mut self, branch: u64, requests: usize) -> io::Result<()> {
        if !self.branches.iter().any(|held| held.id == branch) {
            return Err(invalid(format!(
                "Unknown request allowance branch {branch}"
            )));
        }
        let previous = self.allowances.get(&branch).copied().unwrap_or(PER_BRANCH);
        self.limit = self
            .limit
            .checked_sub(previous)
            .and_then(|limit| limit.checked_add(requests))
            .ok_or_else(|| invalid("Request allowance overflow"))?;
        self.allowances.insert(branch, requests);
        Ok(())
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
        Ok(self.discover(branch).tags.clone())
    }

    fn holders(&mut self, branch: u64) -> io::Result<Vec<Holder>> {
        Ok(self.discover(branch).holders.clone())
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
    recording: Option<&replay::container::Replay>,
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
        recording,
        owners,
        serve,
        out,
        seen: HashSet::new(),
        countries: BTreeMap::new(),
        owner: "",
        tally: Recorded::default(),
        limit: 0,
        allowances: HashMap::new(),
    };
    let mut recorded = Vec::with_capacity(owners.len());
    for owner in owners {
        recorder.owner = owner.name();
        recorder.limit = PER_BRANCH * branches.len().max(1);
        recorder.allowances.clear();
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
    check_ignoring(transcript, serve, &[])
}

/// [`check`], not comparing what lies at an `ignored` path (dotted from the root, `*` for any).
pub fn check_ignoring(
    transcript: &mut dyn BufRead,
    serve: &mut dyn FnMut(&str, &Value) -> Reply,
    ignored: &[Vec<String>],
) -> io::Result<Report> {
    let mut report = Report::ignoring(ignored);
    let mut line = String::new();
    for index in 0.. {
        line.clear();
        if transcript.read_line(&mut line)? == 0 {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let exchange: Exchange = serde_json::from_str(&line)?;
        let candidate = serve(&exchange.product, &exchange.params);
        let compared = compare(exchange, &candidate, &report.patterns);
        report.count(index, compared);
    }
    Ok(report)
}

/// A request as transcripts key it: its product and its parameters' text.
fn request_key(product: &str, params: &Value) -> String {
    format!("{product}\n{params}")
}

/// The requests asked more than once across `transcripts`, each after its header, with how many
/// times: a check serves each of them once.
pub fn shared_requests(
    transcripts: impl IntoIterator<Item = io::Result<Box<dyn BufRead>>>,
) -> io::Result<HashMap<String, usize>> {
    #[derive(Deserialize)]
    struct Request {
        product: String,
        params: Value,
    }
    let mut counts = HashMap::<String, usize>::new();
    let mut line = String::new();
    for transcript in transcripts {
        let mut transcript = transcript?;
        loop {
            line.clear();
            if transcript.read_line(&mut line)? == 0 {
                break;
            }
            if line.trim().is_empty() {
                continue;
            }
            let request: Request = serde_json::from_str(&line)?;
            *counts
                .entry(request_key(&request.product, &request.params))
                .or_default() += 1;
        }
    }
    counts.retain(|_, count| *count > 1);
    Ok(counts)
}

/// [`check_ignoring`] of several transcripts at once, each after its header, their requests asked
/// `threads` at a time, each of the `shared` ones ([`shared_requests`]) once: each transcript's
/// report, and the seconds its requests were served for.
pub fn check_parallel(
    transcripts: Vec<Box<dyn BufRead + Send + '_>>,
    serve: &(dyn Fn(&str, &Value) -> Reply + Sync),
    shared: HashMap<String, usize>,
    ignored: &[Vec<String>],
    threads: NonZeroUsize,
) -> io::Result<Vec<(Report, f64)>> {
    // A shared request's reply, served by its first asker, kept until its last.
    type Shared = (Arc<std::sync::OnceLock<Arc<Reply>>>, usize);
    let shared: Mutex<HashMap<String, Shared>> = Mutex::new(
        shared
            .into_iter()
            .map(|(key, count)| (key, (Default::default(), count)))
            .collect(),
    );
    let serve_once = |product: &str, params: &Value| -> Arc<Reply> {
        let key = request_key(product, params);
        let cell = shared
            .lock()
            .unwrap()
            .get(&key)
            .map(|(cell, _)| Arc::clone(cell));
        let Some(cell) = cell else {
            return Arc::new(serve(product, params));
        };
        let reply = Arc::clone(cell.get_or_init(|| Arc::new(serve(product, params))));
        let mut shared = shared.lock().unwrap();
        if let Some((_, left)) = shared.get_mut(&key) {
            *left -= 1;
            if *left == 0 {
                shared.remove(&key);
            }
        }
        reply
    };
    let count = transcripts.len();
    let reports: Vec<Mutex<(Report, f64)>> = (0..count)
        .map(|_| Mutex::new((Report::ignoring(ignored), 0.0)))
        .collect();
    let (sender, receiver) =
        std::sync::mpsc::sync_channel::<(usize, usize, String)>(threads.get() * 4);
    let receiver = Mutex::new(receiver);
    let failed: Mutex<Option<io::Error>> = Mutex::new(None);
    let stop = AtomicBool::new(false);
    let fail = |error: io::Error| {
        failed.lock().unwrap().get_or_insert(error);
        stop.store(true, Ordering::Relaxed);
    };
    std::thread::scope(|scope| -> io::Result<()> {
        let mut workers = Vec::new();
        for worker in 0..threads.get() {
            let (receiver, reports, fail, stop) = (&receiver, &reports, &fail, &stop);
            workers.push(
                std::thread::Builder::new()
                    .name(format!("served-check-{worker}"))
                    .spawn_scoped(scope, move || {
                        loop {
                            let Ok((transcript, index, line)) = receiver.lock().unwrap().recv()
                            else {
                                break;
                            };
                            if stop.load(Ordering::Relaxed) {
                                continue;
                            }
                            // A failure stops the reading, but the queue is drained to its end
                            // so the reader never waits on a worker that is gone.
                            let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                                || -> io::Result<()> {
                                    let exchange: Exchange = serde_json::from_str(&line)?;
                                    let started = std::time::Instant::now();
                                    let candidate = serve_once(&exchange.product, &exchange.params);
                                    let seconds = started.elapsed().as_secs_f64();
                                    let compared = compare(exchange, &candidate, ignored);
                                    let mut held = reports[transcript]
                                        .lock()
                                        .unwrap_or_else(|error| error.into_inner());
                                    held.0.count(index, compared);
                                    held.1 += seconds;
                                    Ok(())
                                },
                            ));
                            match checked {
                                Ok(Ok(())) => {}
                                Ok(Err(error)) => fail(error),
                                Err(_) => fail(io::Error::other("A served-check request panicked")),
                            }
                        }
                    })?,
            );
        }
        let read = (|| -> io::Result<()> {
            for (transcript, mut lines) in transcripts.into_iter().enumerate() {
                for index in 0.. {
                    if stop.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    let mut line = String::new();
                    if lines.read_line(&mut line)? == 0 {
                        break;
                    }
                    if !line.trim().is_empty() && sender.send((transcript, index, line)).is_err() {
                        return Ok(());
                    }
                }
            }
            Ok(())
        })();
        drop(sender);
        if let Err(error) = read {
            fail(error);
        }
        for worker in workers {
            worker
                .join()
                .map_err(|_| io::Error::other("A served-check worker panicked"))?;
        }
        Ok(())
    })?;
    if let Some(error) = failed.into_inner().unwrap() {
        return Err(error);
    }
    Ok(reports
        .into_iter()
        .map(|report| report.into_inner().unwrap())
        .collect())
}

/// One request compared: what the report counts of it.
struct Compared {
    label: String,
    /// The reference's reply's data: none refused, null pending.
    reference: Option<bool>,
    mismatch: Option<Mismatch>,
}

fn compare(mut exchange: Exchange, candidate: &Reply, patterns: &[Vec<String>]) -> Compared {
    let formed;
    let mut candidate = candidate;
    if let Some(form) = compared_as(&exchange.product) {
        exchange.reply = in_form(form, &exchange.reply);
        formed = in_form(form, candidate);
        candidate = &formed;
    }
    let reference = exchange.reply.data().map(|data| !data.is_null());
    let difference = difference_ignoring(&exchange.reply, candidate, &exchange.unordered, patterns);
    Compared {
        label: label(&exchange.product, &exchange.params),
        reference,
        mismatch: difference.map(|difference| Mismatch {
            product: exchange.product,
            params: exchange.params,
            difference,
        }),
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
    /// The paths not compared.
    pub ignored: Vec<String>,
    #[serde(skip)]
    patterns: Vec<Vec<String>>,
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
    /// The transcript line of `first`.
    #[serde(skip)]
    first_at: usize,
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
    fn ignoring(ignored: &[Vec<String>]) -> Self {
        Self {
            ignored: ignored.iter().map(|path| path.join(".")).collect(),
            patterns: ignored.to_vec(),
            ..Self::default()
        }
    }

    /// Counts the request at transcript line `index`, in any order.
    fn count(&mut self, index: usize, compared: Compared) {
        let tally = self.products.entry(compared.label).or_default();
        self.queries += 1;
        tally.queries += 1;
        match compared.reference {
            None => tally.refused += 1,
            Some(false) => tally.pending += 1,
            Some(true) => tally.answered += 1,
        }
        if let Some(mismatch) = compared.mismatch {
            self.differing += 1;
            tally.differing += 1;
            if tally.first.is_none() || index < tally.first_at {
                tally.first = Some(mismatch);
                tally.first_at = index;
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
    difference_ignoring(reference, candidate, unordered, &[])
}

/// [`difference`], passing over what lies at an `ignored` path.
fn difference_ignoring(
    reference: &Reply,
    candidate: &Reply,
    unordered: &[Vec<String>],
    ignored: &[Vec<String>],
) -> Option<Difference> {
    let side = |reply: &Reply| match reply {
        Reply::Answered(reply) => Side::Value(reply.clone()),
        Reply::Refused(why) => Side::Refused(why.clone()),
    };
    match (reference, candidate) {
        (Reply::Answered(reference), Reply::Answered(candidate)) => {
            first(reference, candidate, &mut Vec::new(), unordered, ignored)
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

/// Whether `path` is one of the `ignored` paths.
fn ignored_at(path: &[Step], ignored: &[Vec<String>]) -> bool {
    ignored.iter().any(|pattern| {
        pattern.len() == path.len()
            && pattern
                .iter()
                .zip(path)
                .all(|(want, step)| want == "*" || matches!(step, Step::Key(key) if key == want))
    })
}

fn first(
    reference: &Value,
    candidate: &Value,
    path: &mut Vec<Step>,
    unordered: &[Vec<String>],
    ignored: &[Vec<String>],
) -> Option<Difference> {
    if ignored_at(path, ignored) {
        return None;
    }
    match (reference, candidate) {
        (Value::Object(held), Value::Object(other)) => {
            let mut keys: Vec<&String> = held.keys().chain(other.keys()).collect();
            keys.sort_unstable();
            keys.dedup();
            for key in keys {
                path.push(Step::Key(key.clone()));
                if ignored_at(path, ignored) {
                    path.pop();
                    continue;
                }
                let found = match (held.get(key), other.get(key)) {
                    (Some(a), Some(b)) => first(a, b, path, unordered, ignored),
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
            multiset(held, other, path, ignored)
        }
        (Value::Array(held), Value::Array(other)) => {
            for at in 0..held.len().max(other.len()) {
                path.push(Step::Index(at));
                if ignored_at(path, ignored) {
                    path.pop();
                    continue;
                }
                let found = match (held.get(at), other.get(at)) {
                    (Some(a), Some(b)) => first(a, b, path, unordered, ignored),
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
fn multiset(
    reference: &[Value],
    candidate: &[Value],
    path: &mut Vec<Step>,
    ignored: &[Vec<String>],
) -> Option<Difference> {
    // An element path matches only `*`, so ignoring one ignores them all.
    path.push(Step::Index(0));
    let every = ignored_at(path, ignored);
    path.pop();
    if every {
        return None;
    }
    // An element's ignored members are left out of what it is counted as.
    let mut key = |item: &Value| {
        path.push(Step::Index(0));
        let text = canonical_at(item, path, ignored);
        path.pop();
        text
    };
    let reference_keys: Vec<String> = reference.iter().map(&mut key).collect();
    let candidate_keys: Vec<String> = candidate.iter().map(&mut key).collect();
    let mut surplus: HashMap<&str, isize> = HashMap::new();
    for item in &candidate_keys {
        *surplus.entry(item).or_default() += 1;
    }
    for (at, item) in reference.iter().enumerate() {
        let count = surplus.entry(&reference_keys[at]).or_default();
        if *count == 0 {
            path.push(Step::Index(at));
            let found = differs(path, Some(item), None);
            path.pop();
            return Some(found);
        }
        *count -= 1;
    }
    for (at, item) in candidate.iter().enumerate() {
        if let Some(count) = surplus.get_mut(candidate_keys[at].as_str())
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

/// `value` at `path` as text, every object's members in key order and `ignored` paths left out.
fn canonical_at(value: &Value, path: &mut Vec<Step>, ignored: &[Vec<String>]) -> String {
    match value {
        Value::Object(members) => {
            let mut keys: Vec<&String> = members.keys().collect();
            keys.sort_unstable();
            let mut fields = Vec::with_capacity(keys.len());
            for key in keys {
                path.push(Step::Key(key.clone()));
                if !ignored_at(path, ignored) {
                    let text = canonical_at(&members[key], path, ignored);
                    fields.push(format!("{}:{text}", Value::from(key.as_str())));
                }
                path.pop();
            }
            format!("{{{}}}", fields.join(","))
        }
        Value::Array(items) => {
            let mut texts = Vec::with_capacity(items.len());
            for (at, item) in items.iter().enumerate() {
                path.push(Step::Index(at));
                if !ignored_at(path, ignored) {
                    texts.push(canonical_at(item, path, ignored));
                }
                path.pop();
            }
            format!("[{}]", texts.join(","))
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
    use replay::columnar_frames::Recording;
    let file = Recording::open_path(path)?;
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
    complete: bool,
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
        let served = Self::open_shared(recording, folder, content)?;
        served.require_family(family)?;
        Ok(served)
    }

    pub fn open_shared(
        recording: &Path,
        folder: &Path,
        content: Option<ContentOptions>,
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
        let complete = source.in_scope(|| {
            let (store, key) = super::files(&source, content.as_ref())?;
            if !store.is_reusable(&key)? {
                return Err(invalid(format!(
                    "{} holds no preparation of this recording under this content",
                    folder.display()
                )));
            }
            io::Result::Ok(
                super::pipeline::restore(&source, content.as_ref(), &cancelled)?.is_some(),
            )
        })?;
        Ok(Self {
            source,
            content,
            complete,
        })
    }

    pub fn require_family(&self, family: &str) -> io::Result<()> {
        if !self.complete
            && !self.source.in_scope(|| {
                super::driver::native::complete(&self.source, self.content.as_ref(), family)
            })?
        {
            return Err(invalid(format!(
                "The store holds an unfinished preparation of {family} of this recording"
            )));
        }
        Ok(())
    }

    pub fn family_bytes(&self) -> io::Result<HashMap<String, u64>> {
        use super::driver::plan::{self, Owner};
        let (store, key) = self
            .source
            .in_scope(|| super::files(&self.source, self.content.as_ref()))?;
        let mut sizes = HashMap::<String, u64>::new();
        if let Some(session) = store.session(&key)? {
            let snapshot = session.snapshot();
            let plan = plan::native();
            for section in snapshot.section_names() {
                let name = match plan.owner(section) {
                    Some(Owner::Family(family)) => family.name(),
                    Some(Owner::Job(name)) => name,
                    _ => continue,
                };
                let bytes = sizes.entry(name.to_owned()).or_default();
                for range in 0..snapshot.range_count(section).unwrap_or(0) {
                    if let Some(range) = snapshot.range_id(section, range)? {
                        *bytes = bytes.saturating_add(range.len());
                    }
                }
            }
        }
        Ok(sizes)
    }

    /// The recording's revision, which requests name.
    pub fn revision(&self) -> &str {
        &self.source.key().revision
    }

    pub fn recording(&self) -> &replay::container::Replay {
        self.source.replay_ref()
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
        let _scope = replay_frame::cpu::scope_named("query", product);
        let routed = crate::products::find(product).filter(|_| !super::query::answers(product));
        // The recording's own executable tables answer it, never the installed game's.
        let answered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.source.in_scope(|| match routed {
                // The product's own route, as the service asks it.
                Some(registered) => {
                    let context = crate::config::LoadContext {
                        content: self.content.clone(),
                    };
                    (registered.load)(&self.source, SOURCE, params.clone(), &context, &cancelled)
                        .map_err(io::Error::other)
                }
                None => {
                    super::query::query(&self.source, &request, self.content.as_ref(), &cancelled)
                }
            })
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
