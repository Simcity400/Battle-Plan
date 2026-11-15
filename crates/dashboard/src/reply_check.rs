//! The reply check: every request of a reference transcript (recorded from the old preparation by
//! main's `served-check record`) asked again of this build, its reply compared with the kept one
//! member by member ([`check`]). Transcripts are the only reference for whether a moved product
//! answers what users were served.
//!
//! A chart this build serves as tiles (`series-tiles`) is answered as the UI draws it: the view
//! assembled from the tiles covering it ([`dashboard_query::tile::view`]), timed as the sum of its
//! tile requests, which the kept exchange counts.

use dashboard_query::tile::{self, SeriesTileReply, view};
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    io::{self, BufRead},
    time::Instant,
};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

/// The transcript's layout.
const VERSION: u32 = 1;


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

/// The transcript's first line.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Header {
    pub served_check: u32,
    pub family: String,
    pub revision: String,
    pub branches: Value,
}

/// One request and the reference's reply, as the transcript keeps it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Exchange {
    pub product: String,
    pub params: Value,
    pub reply: Reply,
    #[serde(default)]
    pub unordered: Vec<Vec<String>>,
}

/// The transcript's header, read from its first line.
pub fn read_header(transcript: &mut dyn BufRead) -> io::Result<Header> {
    let mut line = String::new();
    transcript.read_line(&mut line)?;
    let header: Header = serde_json::from_str(&line)?;
    if header.served_check != VERSION {
        return Err(invalid(format!("The transcript's layout {} is not this check's {VERSION}", header.served_check)));
    }
    Ok(header)
}

/// What a report counts a request under: its product, and the series family kind it routes by.
fn label(product: &str, params: &Value) -> String {
    match params.get("family").and_then(|family| family.get("kind")).and_then(Value::as_str) {
        Some(kind) => format!("{product}:{kind}"),
        None => product.to_owned(),
    }
}

/// Asks `serve` every request of the transcript after its header that `asked` selects,
/// comparing each reply with the reference's, not comparing what lies at an `ignored` path.
pub fn check(
    transcript: &mut dyn BufRead,
    asked: &dyn Fn(&str, &Value) -> bool,
    serve: &mut dyn FnMut(&str, &Value) -> Reply,
    reset: &mut dyn FnMut(&Value) -> f64,
    ignored: &[Vec<String>],
    mut kept: Option<&mut dyn io::Write>,
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
        if !asked(&exchange.product, &exchange.params) {
            report.skipped += 1;
            continue;
        }
        let built = reset(&exchange.params);
        let served = match exchange.product.as_str() {
            "series-summary" => chart(serve, &exchange.params),
            product => timed(serve, product, &exchange.params),
        };
        let reference = exchange.reply.data().map(|data| !data.is_null());
        let difference = difference_ignoring(&exchange.reply, &served.reply, &exchange.unordered, &report.patterns);
        if let Some(kept) = kept.as_mut() {
            let mut line = json!({ "product": exchange.product, "params": exchange.params, "milliseconds": served.seconds * 1e3, "storageMilliseconds": served.storage, "indexMilliseconds": built,
                "reference": exchange.reply, "candidate": served.reply });
            if let Some(tiles) = &served.tiles {
                line["tiles"] = serde_json::to_value(tiles)?;
            }
            serde_json::to_writer(&mut **kept, &line)?;
            kept.write_all(b"\n")?;
        }
        report.count(
            index,
            Compared {
                label: label(&exchange.product, &exchange.params),
                product: exchange.product.clone(),
                params: exchange.params.clone(),
                reference,
                tiles: served.tiles.as_ref().map_or(0, |tiles| tiles.requests),
                mismatch: difference.map(|difference| Mismatch { product: exchange.product, params: exchange.params, difference }),
            },
            served.seconds,
        );
    }
    report.time();
    Ok(report)
}

struct Compared {
    label: String,
    product: String,
    params: Value,
    reference: Option<bool>,
    tiles: usize,
    mismatch: Option<Mismatch>,
}

/// The candidate's reply to one exchange, and what serving it cost.
struct Served {
    reply: Reply,
    /// Wall time of the requests asked, serialisation included.
    seconds: f64,
    /// Storage time of those requests, milliseconds.
    storage: f64,
    tiles: Option<Tiles>,
}

/// The tile requests a chart was answered by.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Tiles {
    bin_hours: i64,
    first: i64,
    last: i64,
    requests: usize,
    each: Vec<TileTime>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TileTime {
    index: i64,
    keys: usize,
    milliseconds: f64,
    storage_milliseconds: f64,
}

/// `product` asked `params` of the candidate once, timed as the service serves it.
fn timed(serve: &mut dyn FnMut(&str, &Value) -> Reply, product: &str, params: &Value) -> Served {
    dashboard_query::cost::take();
    let started = Instant::now();
    let reply = serve(product, params);
    let seconds = started.elapsed().as_secs_f64();
    Served { reply, seconds, storage: dashboard_query::cost::take() as f64 / 1e6, tiles: None }
}

/// A chart request answered as the UI draws it: from the tiles covering its view when the build
/// serves it so, else as asked (a ranking, whose members change bin by bin). A chart costs its
/// tiles: their requests' times summed; assembling them is the UI's work and is not timed.
fn chart(serve: &mut dyn FnMut(&str, &Value) -> Reply, params: &Value) -> Served {
    let Ok((asked, requests)) = view::requests(params) else {
        return timed(serve, "series-summary", params);
    };
    if requests.is_empty() {
        let as_asked = timed(serve, "series-summary", params);
        if matches!(as_asked.reply, Reply::Answered(_)) {
            return as_asked;
        }
    }
    let mut tiles = Tiles { bin_hours: asked.bin_hours(), first: *asked.tiles().start(), last: *asked.tiles().end(), requests: 0, each: Vec::new() };
    let (mut seconds, mut storage, mut replies) = (0.0, 0.0, Vec::with_capacity(requests.len()));
    for request in &requests {
        let served = timed(serve, tile::PRODUCT, request);
        seconds += served.seconds;
        storage += served.storage;
        tiles.requests += 1;
        tiles.each.push(TileTime {
            index: request["index"].as_i64().unwrap_or_default(),
            keys: request["tags"].as_array().map_or(0, Vec::len),
            milliseconds: served.seconds * 1e3,
            storage_milliseconds: served.storage,
        });
        let refused = match served.reply {
            Reply::Answered(reply) => match serde_json::from_value::<SeriesTileReply>(reply.get("data").cloned().unwrap_or(Value::Null)) {
                Ok(reply) => {
                    replies.push(reply);
                    continue;
                }
                Err(error) => format!("The tile reply is malformed: {error}"),
            },
            Reply::Refused(why) => why,
        };
        if replies.is_empty() {
            let as_asked = timed(serve, "series-summary", params);
            if matches!(as_asked.reply, Reply::Answered(_)) {
                return as_asked;
            }
        }
        return Served { reply: Reply::Refused(refused), seconds, storage, tiles: Some(tiles) };
    }
    let reply = match view::assemble(asked, &replies).and_then(|view| Ok(serde_json::to_value(view)?)) {
        Ok(view) => Reply::Answered(json!({ "data": view })),
        Err(error) => Reply::Refused(format!("The chart's tiles do not assemble: {error}")),
    };
    Served { reply, seconds, storage, tiles: Some(tiles) }
}

/// How the candidate's replies compare with the reference's.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub queries: usize,
    pub differing: usize,
    /// Requests of products this build does not answer from the query layer yet.
    pub skipped: usize,
    /// The paths not compared.
    pub ignored: Vec<String>,
    #[serde(skip)]
    patterns: Vec<Vec<String>>,
    /// By product, or by product and the series family kind it routes by.
    pub products: BTreeMap<String, Tally>,
    /// The slowest requests the candidate served, slowest first.
    pub slowest: Vec<Slow>,
}

/// How many of the slowest requests a report names.
const SLOWEST: usize = 10;

/// A request the candidate served, and how long it took.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Slow {
    pub product: String,
    pub params: Value,
    pub milliseconds: f64,
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
    /// Tile requests its charts were answered by.
    pub tile_requests: usize,
    /// Its first request whose replies differ, and where.
    pub first: Option<Mismatch>,
    /// The transcript line of `first`.
    #[serde(skip)]
    first_at: usize,
    /// Serve times in milliseconds: median, 95th percentile and slowest.
    pub p50_milliseconds: Option<f64>,
    pub p95_milliseconds: Option<f64>,
    pub max_milliseconds: Option<f64>,
    #[serde(skip)]
    milliseconds: Vec<f64>,
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

    /// Counts the request at transcript line `index`, in any order, which took `seconds`.
    fn count(&mut self, index: usize, compared: Compared, seconds: f64) {
        let milliseconds = seconds * 1e3;
        let tally = self.products.entry(compared.label).or_default();
        self.queries += 1;
        tally.queries += 1;
        tally.tile_requests += compared.tiles;
        tally.milliseconds.push(milliseconds);
        if self.slowest.len() < SLOWEST
            || self
                .slowest
                .last()
                .is_some_and(|slow| slow.milliseconds < milliseconds)
        {
            let at = self
                .slowest
                .partition_point(|slow| slow.milliseconds >= milliseconds);
            self.slowest.insert(
                at,
                Slow {
                    product: compared.product,
                    params: compared.params,
                    milliseconds,
                },
            );
            self.slowest.truncate(SLOWEST);
        }
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

    /// Each product's serve times, from the requests counted.
    fn time(&mut self) {
        for tally in self.products.values_mut() {
            let times = &mut tally.milliseconds;
            times.sort_unstable_by(f64::total_cmp);
            let at = |share: f64| {
                times
                    .get(((times.len() as f64 * share).ceil() as usize).saturating_sub(1))
                    .copied()
            };
            tally.p50_milliseconds = at(0.5);
            tally.p95_milliseconds = at(0.95);
            tally.max_milliseconds = times.last().copied();
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
