
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

const VERSION: u32 = 1;


#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Reply {
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Header {
    pub served_check: u32,
    pub family: String,
    pub revision: String,
    pub branches: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Exchange {
    pub product: String,
    pub params: Value,
    pub reply: Reply,
    #[serde(default)]
    pub unordered: Vec<Vec<String>>,
}

pub fn read_header(transcript: &mut dyn BufRead) -> io::Result<Header> {
    let mut line = String::new();
    transcript.read_line(&mut line)?;
    let header: Header = serde_json::from_str(&line)?;
    if header.served_check != VERSION {
        return Err(invalid(format!("The transcript's layout {} is not this check's {VERSION}", header.served_check)));
    }
    Ok(header)
}

fn label(product: &str, params: &Value) -> String {
    match params.get("family").and_then(|family| family.get("kind")).and_then(Value::as_str) {
        Some(kind) => format!("{product}:{kind}"),
        None => product.to_owned(),
    }
}

pub fn warm_catalogue(
    source: &dashboard_base::Source,
    content: &dashboard_core::config::ContentOptions,
) -> io::Result<f64> {
    let started = Instant::now();
    source.in_scope(|| dashboard_base::content::warm_checked(source, content))?;
    Ok(started.elapsed().as_secs_f64() * 1e3)
}

pub fn check(
    transcript: &mut dyn BufRead,
    asked: &dyn Fn(&str, &Value) -> bool,
    serve: &mut dyn FnMut(&str, &Value) -> Reply,
    reset: &mut dyn FnMut(&Value) -> io::Result<f64>,
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
        let built = reset(&exchange.params)?;
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

struct Served {
    reply: Reply,
    seconds: f64,
    storage: f64,
    tiles: Option<Tiles>,
}

#[derive(Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Tiles {
    bin_hours: i64,
    first: i64,
    last: i64,
    requests: usize,
    each: Vec<TileTime>,
}

#[derive(Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct TileTime {
    index: i64,
    keys: usize,
    milliseconds: f64,
    storage_milliseconds: f64,
}

fn timed(serve: &mut dyn FnMut(&str, &Value) -> Reply, product: &str, params: &Value) -> Served {
    dashboard_query::cost::take();
    let started = Instant::now();
    let reply = serve(product, params);
    let seconds = started.elapsed().as_secs_f64();
    Served { reply, seconds, storage: dashboard_query::cost::take() as f64 / 1e6, tiles: None }
}

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

#[derive(Debug, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub queries: usize,
    pub differing: usize,
    pub skipped: usize,
    pub ignored: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache: Option<replay_tools::cold_cache::Probe>,
    #[serde(skip)]
    patterns: Vec<Vec<String>>,
    pub products: BTreeMap<String, Tally>,
    pub slowest: Vec<Slow>,
}

const SLOWEST: usize = 10;

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Slow {
    pub product: String,
    pub params: Value,
    pub milliseconds: f64,
}

#[derive(Debug, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tally {
    pub queries: usize,
    pub answered: usize,
    pub pending: usize,
    pub refused: usize,
    pub differing: usize,
    pub tile_requests: usize,
    pub first: Option<Mismatch>,
    #[serde(skip)]
    first_at: usize,
    pub p50_milliseconds: Option<f64>,
    pub p95_milliseconds: Option<f64>,
    pub max_milliseconds: Option<f64>,
    #[serde(skip)]
    milliseconds: Vec<f64>,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mismatch {
    pub product: String,
    pub params: Value,
    pub difference: Difference,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Difference {
    pub path: String,
    pub reference: Side,
    pub candidate: Side,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Side {
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

pub fn difference(
    reference: &Reply,
    candidate: &Reply,
    unordered: &[Vec<String>],
) -> Option<Difference> {
    difference_ignoring(reference, candidate, unordered, &[])
}

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

fn multiset(
    reference: &[Value],
    candidate: &[Value],
    path: &mut Vec<Step>,
    ignored: &[Vec<String>],
) -> Option<Difference> {
    path.push(Step::Index(0));
    let every = ignored_at(path, ignored);
    path.pop();
    if every {
        return None;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_surfaces_reset_open_failures() {
        let exchange = json!({
            "product": "clock",
            "params": {},
            "reply": { "answered": { "data": 1 } }
        });
        let body = format!("{exchange}\n");
        let mut cursor = io::Cursor::new(body.as_bytes());
        let err = check(
            &mut cursor,
            &|_, _| true,
            &mut |_, _| Reply::Answered(json!({ "data": 1 })),
            &mut |_| Err(io::Error::new(io::ErrorKind::NotFound, "recording vanished")),
            &[],
            None,
        )
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn warm_catalogue_surfaces_content_load_failure_instead_of_timing() {
        let root = std::env::temp_dir().join(format!("reply-check-warm-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let (source, context) = dashboard_base::test_fixtures::source_with_content(
            vec![dashboard_base::test_fixtures::source(1, 0, Some(1))],
            &root,
        );
        let mut bad = context.content.unwrap();
        bad.allow_unverified = false;
        let err = warm_catalogue(&source, &bad).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("unverified content"), "{err}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
