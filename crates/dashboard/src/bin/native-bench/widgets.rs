use dashboard::panel::dated_panel_of;
use dashboard_base::{Source, products::at_hour};
use dashboard_core::config::LoadContext;
use dashboard_follow::{Follow, Identity, panel::{ExactReader, ProductReader, Panel, PanelData, ProductStep, product::Delivery}};
use playhead::{Declared, Hook, Lanes, Step};
use serde_json::{Value, json};
use std::{io, sync::{Arc, Mutex, atomic::AtomicBool}};

pub struct Packet {
    pub bytes: Vec<u8>,
    pub through: i32,
    pub data: bool,
    pub refused: bool,
    pub rows: u64,
    pub cold_rows: Option<u64>,
    pub mismatch: Option<String>,
    pub lanes: Vec<Declared>,
}

pub struct Widget {
    pub name: &'static str,
    pub declared: Vec<Declared>,
    reader: Reader,
}

enum Reader {
    Panel { read: ProductReader, cold: ExactReader, delivery: Delivery, last_bytes: Vec<u8>, last_through: i32 },
    Ranking { hook: Hook, result: Arc<Mutex<Option<(u64, dashboard_follow::Message)>>>, query: dashboard_follow::Query, previous: Vec<u8> },
}

impl Widget {
    pub fn step(&mut self, id: u32, step: &Step, lanes: &Lanes<'_>, audit: Option<&Arc<playhead::pages::WorldPages>>) -> Packet {
        match &mut self.reader {
            Reader::Panel { read, cold, delivery, last_bytes, last_through } => {
                let token = AtomicBool::new(false);
                let delivered = *last_through != i32::MIN;
                let run = || {
                    let reading = read(step.last(), step.reset(), &token, Some(ProductStep { step, lanes, held: None, wake: None }));
                    if reading.pending && !delivered { read(step.last(), true, &token, None) } else { reading }
                };
                let (reading, extra) = if audit.is_some() { at_hour::counted(run) } else { (run(), 0) };
                if reading.pending {
                    return Packet { bytes: last_bytes.clone(), through: *last_through, data: false, refused: false, rows: reading.rows + extra, cold_rows: None, mismatch: None, lanes: reading.lanes };
                }
                let (answer, rows, read_lanes, fresh) = (reading.value, reading.rows, reading.lanes, reading.fresh);
                let record = delivery.packet((0, 1, step.epoch() as u32), id, step.last(), step.reset(), &answer);
                let mut cold_rows = None;
                let mut mismatch = None;
                if audit.is_some() {
                    let (expected, count) = at_hour::counted(|| cold(step.last(), &token));
                    cold_rows = Some(count);
                    let (expected, shown) = (shown(&expected, fresh), shown(&answer, fresh));
                    if expected.is_err() != shown.is_err() || hash(&expected) != hash(&shown) {
                        mismatch = Some(match (&expected, &shown) {
                            (Ok(left), Ok(right)) => difference("", left, right).unwrap_or_else(|| "payload bytes differ".into()),
                            _ => format!("refusals {:?} / {:?}", expected.as_ref().err(), shown.as_ref().err()),
                        });
                    }
                }
                last_bytes.clone_from(&record.bytes);
                *last_through = step.last();
                Packet { bytes: record.bytes, through: step.last(), data: record.changed, refused: record.refused, rows: rows + extra, cold_rows, mismatch, lanes: read_lanes }
            }
            Reader::Ranking { hook, result, query, previous } => {
                hook(step, lanes);
                let (rows, msg) = result.lock().unwrap().take().expect("one dated ranking confirmation per step");
                let decoded = dashboard_follow::decode(&msg.bytes).expect("our ranking message");
                let refused = decoded.answers.first().is_some_and(|(_, _, body)| body.first() == Some(&0));
                let mut mismatch = None;
                let cold_rows = audit.map(|pages| {
                    let d = &self.declared[0];
                    let mut lane = playhead::lane::Lane::new(pages.clone(), 0, d.table, &d.columns, d.keyed).unwrap();
                    lane.seek(step.last()).unwrap();
                    let expected = dashboard_follow::answer_at(query, &lane).unwrap();
                    if let Some((_, _, bytes)) = decoded.answers.first() { previous.clear(); previous.extend_from_slice(bytes); }
                    if decoded.hour != step.last() || blake3::hash(previous) != blake3::hash(&expected) { mismatch = Some("ranking body or confirmation differs".into()); }
                    lane.cells().rows().count() as u64
                });
                Packet { through: decoded.hour, data: msg.answers != 0, refused, bytes: msg.bytes, rows, cold_rows, mismatch, lanes: Vec::new() }
            }
        }
    }
}

fn shown(data: &PanelData, values: bool) -> PanelData {
    data.clone().map(|mut reply| {
        for group in reply["live"]["groups"].as_array_mut().into_iter().flatten() {
            if !values && let Some(group) = group.as_object_mut() { group.remove("values"); }
        }
        reply
    })
}

fn hash(data: &PanelData) -> blake3::Hash {
    match data {
        Ok(value) => blake3::hash(&rmp_serde::to_vec(value).unwrap()),
        Err(reason) => blake3::hash(reason.as_bytes()),
    }
}

fn difference(path: &str, a: &Value, b: &Value) -> Option<String> {
    if a == b { return None; }
    match (a,b) {
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => a.iter().zip(b).enumerate().find_map(|(i,(a,b))| difference(&format!("{path}[{i}]"),a,b)),
        (Value::Object(a), Value::Object(b)) if a.keys().eq(b.keys()) => a.iter().find_map(|(key,v)| difference(&format!("{path}.{key}"),v,&b[key])),
        _ => Some(format!("{path}: exact {} / advanced {}", short(a), short(b))),
    }
}

fn short(value: &Value) -> String { value.to_string().chars().take(160).collect() }

fn panel(name: &'static str, panel: Result<Box<dyn Panel>, String>) -> io::Result<Widget> {
    let panel = panel.map_err(io::Error::other)?;
    let exact = panel.exact().expect("complete at-hour reader");
    let (read, cold) = (exact.advance.clone(), exact.cold.clone());
    Ok(Widget { name, declared: Vec::new(), reader: Reader::Panel { read, cold, delivery: Delivery::default(), last_bytes: Vec::new(), last_through: i32::MIN } })
}

fn rank(identity: &Identity, name: &'static str, product: &str, mut params: Value) -> io::Result<Widget> {
    params["top"] = json!(20);
    let query = dashboard_follow::parse(&json!({ "product": product, "params": params }), identity)?;
    let follow = Follow::new(0);
    follow.follow(1, query.clone())?;
    let result = Arc::new(Mutex::new(None));
    let send = Arc::clone(&result);
    let hook = follow.hook(Box::new(move |stats, message| {
        *send.lock().unwrap() = message.map(|m| (stats.rows_read, m));
    }));
    Ok(Widget { name, declared: follow.declared()?, reader: Reader::Ranking { hook, result, query, previous: Vec::new() } })
}

pub fn page(source: &Source, load: &LoadContext, identity: &Identity, tag: &str) -> io::Result<Vec<Widget>> {
    let product = |name: &'static str, mut params: Value| {
        params["panel"] = name.into();
        panel(name, dated_panel_of(source, load, &params).expect("registered dated product"))
    };
    let branch = identity.timeline.id.to_string();
    let owner = json!({ "branchId": branch, "country": tag });
    let world = json!({ "branchId": branch });
    Ok(vec![
        product("occupied-states", world.clone())?,
        product("special-project-details", owner.clone())?,
        product("special-projects", owner.clone())?,
        product("doctrine-mastery", owner.clone())?,
        product("research-slots", owner.clone())?,
        product("game-score", world.clone())?,
        product("occupation-reserve-manpower", world.clone())?,
        product("lend-lease-balance", world)?,
        product("agency-comparison", json!({ "branchId": branch, "includeUncreated": true }))?,
        product("production-lines-at", owner)?,
        product("territory-share", json!({ "branchId": branch, "measure": "ownedStates" }))?,
        rank(identity, "army-size", "army-size", json!({ "measure": "strength", "countFor": "owner" }))?,
        rank(identity, "air-force-totals", "series-state", json!({ "family": { "kind": "air-force-totals", "measure": "rosterPlanes" } }))?,
        rank(identity, "casualty-totals", "series-state", json!({ "family": { "kind": "casualty-totals", "measure": "dealtAndTaken" } }))?,
        rank(identity, "country-factories", "series-state", json!({ "family": { "kind": "country-factories", "measure": "free7" } }))?,
        rank(identity, "production-country-totals", "series-state", json!({ "family": { "kind": "production-country-totals", "measure": "spent" } }))?,
    ])
}
