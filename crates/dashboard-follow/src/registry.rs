
use crate::{
    Follow,
    head::HeadOf,
    panel::Panel,
    parse,
    query::{Identity, Query},
    table_name,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    pub seek: u32,
    pub frame_rate: f64,
    pub hour: i32,
    pub position: f64,
    pub timestamps: Option<(f64, f64)>,
    pub playing: bool,
    pub speed: f64,
    pub sought: bool,
}

pub trait Hosted: Send + Sync {
    fn follow(&self) -> &Follow;
    fn lacks(&self, table: usize) -> Option<String>;
    fn anchor(&self, anchor: Anchor);
    fn panel(&self, id: u32, panel: Box<dyn Panel>, lanes: u32) -> Result<(), String> {
        let _ = (id, panel, lanes);
        Err("This playhead shows no panels".into())
    }
    fn unpanel(&self, _id: u32) {}
    fn serve(&self, session: u64) { self.follow().serve(session); }
}

pub trait Playheads: Send + Sync {
    fn open(&self, path: &Path, branch: u64, clock: i32) -> Result<Arc<dyn Hosted>, String>;
    fn credit(&self, _session: u64, _released: u64) {}
    fn restart(&self) -> u64 { 0 }
}

struct Followed {
    path: PathBuf,
    branch: u64,
    clock: i32,
    host: Arc<dyn Hosted>,
    panel: bool,
}

impl Followed {
    fn release(&self, id: u32) -> io::Result<()> {
        if self.panel {
            self.host.unpanel(id);
            Ok(())
        } else {
            self.host.follow().unfollow(id)
        }
    }
}

enum Made {
    Panel(Box<dyn Panel>),
    Query(Query),
}

pub type PanelOf<'a> = dyn FnOnce(&Value) -> Result<Box<dyn Panel>, String> + 'a;

pub struct Registry {
    playheads: Option<Arc<dyn Playheads>>,
    followed: Mutex<Registered>,
}

#[derive(Default)]
struct Registered {
    session: Option<String>,
    epoch: u64,
    held: HashMap<u32, Followed>,
}

impl Registered {
    fn current(&self, params: &Value) -> bool {
        params.get("session").and_then(Value::as_str).is_none_or(|asked| self.session.as_deref() == Some(asked))
    }
}

const RETIRED: &str = "This page was replaced by a newer one; its follows ended with it";

fn lock<T>(held: &Mutex<T>) -> MutexGuard<'_, T> {
    held.lock().unwrap_or_else(PoisonError::into_inner)
}

static REGISTRY: OnceLock<Registry> = OnceLock::new();

pub fn install(playheads: Arc<dyn Playheads>) -> bool {
    REGISTRY.set(Registry::new(Some(playheads))).is_ok()
}

pub fn registry() -> &'static Registry {
    REGISTRY.get_or_init(|| Registry::new(None))
}

pub const PRODUCTS: &[&str] = &["follow", "unfollow", "follow-clock", "follow-close"];

fn unavailable(reason: impl Into<String>) -> Value {
    json!({ "available": false, "reason": reason.into() })
}

impl Registry {
    pub fn new(playheads: Option<Arc<dyn Playheads>>) -> Self {
        Self { playheads, followed: Mutex::default() }
    }

    pub fn answer(&self, path: &Path, product: &str, params: &Value, identity: impl FnOnce(u64) -> io::Result<Identity>, panel: Box<PanelOf<'_>>) -> io::Result<Value> {
        self.answer_heads(path, product, params, identity, panel, Box::new(|_| None))
    }

    pub fn answer_heads(&self, path: &Path, product: &str, params: &Value, identity: impl FnOnce(u64) -> io::Result<Identity>, panel: Box<PanelOf<'_>>,
        head: Box<HeadOf<'_>>) -> io::Result<Value> {
        match product {
            "follow" => {
                let id = id(params)?;
                let branch = branch(params)?;
                let clock = clock(params)?;
                let lanes = lanes(params, id)?;
                if !lock(&self.followed).current(params) { return Ok(unavailable(RETIRED)); }
                let Some(playheads) = &self.playheads else {
                    return Ok(unavailable("Values at the head are followed only in the map viewer's process, which has no playhead here"));
                };
                let host = match playheads.open(path, branch, clock) {
                    Ok(host) => host,
                    Err(reason) => return Ok(unavailable(reason)),
                };
                let asked = params.get("query").unwrap_or(&Value::Null);
                if let Some(other) = other_branch(asked, branch) {
                    return Ok(unavailable(format!("The followed query names branch {other}, not the followed branch {branch}")));
                }
                let made = if asked.get("panel").is_some() {
                    match panel(asked) {
                        Err(reason) => return Ok(unavailable(reason)),
                        Ok(shown) => Made::Panel(shown),
                    }
                } else {
                    let query: Query = match head(asked) {
                        Some(Ok(value)) => Query::Head(value),
                        Some(Err(reason)) => return Ok(unavailable(reason)),
                        None => match parse::parse(asked, &identity(branch)?) {
                            Ok(query) => query,
                            Err(error) => return Ok(unavailable(error.to_string())),
                        },
                    };
                    if let Some(reason) = (!matches!(query, Query::Head(_))).then(|| host.lacks(query.table())).flatten() {
                        return Ok(unavailable(reason));
                    }
                    Made::Query(query)
                };
                let mut followed = lock(&self.followed);
                if !followed.current(params) { return Ok(unavailable(RETIRED)); }
                host.serve(followed.epoch);
                let is_panel = matches!(made, Made::Panel(_));
                let registered = match made {
                    Made::Panel(shown) => host.panel(id, shown, lanes),
                    Made::Query(query) => host.follow().follow(id, query).map_err(|error| error.to_string()),
                };
                if let Err(reason) = registered { return Ok(unavailable(reason)); }
                let old = followed.held.insert(id, Followed { path: path.to_path_buf(), branch, clock, host: Arc::clone(&host), panel: is_panel });
                if let Some(old) = old && (!Arc::ptr_eq(&old.host, &host) || old.panel != is_panel) {
                    old.release(id)?;
                }
                Ok(json!({"available":true, "route":host.follow().route()}))
            }
            "unfollow" => {
                let id = id(params)?;
                let mut followed = lock(&self.followed);
                if !followed.current(params) { return Ok(json!({ "available": true })); }
                if let Some(held) = followed.held.remove(&id) {
                    held.release(id)?;
                }
                Ok(json!({ "available": true }))
            }
            "follow-clock" => {
                let branch = branch(params)?;
                let clock = clock(params)?;
                let number = |name: &str| params.get(name).and_then(Value::as_f64).ok_or_else(|| crate::invalid(format!("A clock anchor names no {name}")));
                let anchor = Anchor {
                    frame_rate: params["frameRate"].as_f64().filter(|rate| rate.is_finite() && *rate > 0.0).ok_or_else(|| crate::invalid("A clock anchor needs a positive frame rate"))?,
                    seek: params["seek"].as_u64().and_then(|s| s.try_into().ok()).ok_or_else(|| crate::invalid("A clock anchor names no seek"))?,
                    hour: number("hour")?.floor() as i32,
                    position: number("hour")?,
                    timestamps: params["composedAt"].as_f64().zip(params["sentAt"].as_f64()),
                    playing: params.get("playing").and_then(Value::as_bool).unwrap_or(false),
                    speed: number("speed")?,
                    sought: params.get("sought").and_then(Value::as_bool).unwrap_or(false),
                };
                let followed = lock(&self.followed);
                if !followed.current(params) { return Ok(unavailable(RETIRED)); }
                let Some(held) = followed.held.values().find(|held| held.path == path && held.branch == branch && held.clock == clock) else { return Ok(json!({"available":false})); };
                held.host.anchor(anchor);
                Ok(json!({"available":true}))
            }
            "follow-close" => {
                self.close(path);
                Ok(json!({ "available": true }))
            }
            other => Err(crate::invalid(format!("Follow does not answer {other}"))),
        }
    }

    pub fn credit(&self, session: u64, released: u64) {
        if let Some(playheads) = &self.playheads { playheads.credit(session, released); }
    }

    pub fn restart(&self, session: &str) -> u64 {
        let mut followed = lock(&self.followed);
        followed.session = Some(session.to_owned());
        followed.epoch = self.playheads.as_ref().map_or(0, |playheads| playheads.restart());
        let epoch = followed.epoch;
        for (id, held) in followed.held.drain() {
            let _ = held.release(id);
            held.host.serve(epoch);
        }
        epoch
    }

    pub fn close(&self, path: &Path) {
        let gone: Vec<(u32, Followed)> = {
            let mut followed = lock(&self.followed);
            let ids: Vec<u32> = followed.held.iter().filter(|(_, held)| held.path == path).map(|(id, _)| *id).collect();
            ids.into_iter().filter_map(|id| followed.held.remove(&id).map(|held| (id, held))).collect()
        };
        for (id, held) in gone {
            let _ = held.release(id);
        }
    }

    pub fn followed(&self) -> usize {
        lock(&self.followed).held.len()
    }
}

fn id(params: &Value) -> io::Result<u32> {
    let id = params.get("id").and_then(Value::as_u64).and_then(|id| u32::try_from(id).ok()).ok_or_else(|| crate::invalid("A follow request names no id"))?;
    if id & crate::live::CELLS != 0 {
        return Err(crate::invalid("A follow request's id is a panel's live cells' (its high bit is the host's)"));
    }
    Ok(id)
}

fn lanes(params: &Value, id: u32) -> io::Result<u32> {
    let lanes = match params.get("lanes") {
        None | Some(Value::Null) => 1,
        Some(lanes) => lanes.as_u64().and_then(|lanes| u32::try_from(lanes).ok()).filter(|&lanes| lanes >= 1).ok_or_else(|| crate::invalid("A follow request's lanes are one or more"))?,
    };
    match params.get("lane").and_then(Value::as_u64) {
        Some(lane) if u64::from(id % lanes) != lane => Err(crate::invalid(format!("Follow {id} is not of lane {lane} among {lanes}"))),
        _ => Ok(lanes),
    }
}

fn clock(params: &Value) -> io::Result<i32> {
    params["clock"].as_i64().and_then(|clock| i32::try_from(clock).ok()).ok_or_else(|| crate::invalid("A follow request's clock is not an hour"))
}

fn other_branch(asked: &Value, branch: u64) -> Option<String> {
    match asked {
        Value::Object(fields) => fields.iter().find_map(|(name, value)| match (name.as_str(), value) {
            ("branchId", Value::String(id)) if id.parse::<u64>().ok() == Some(branch) => None,
            ("branchId", other) => Some(other.as_str().map_or_else(|| other.to_string(), str::to_owned)),
            _ => other_branch(value, branch),
        }),
        Value::Array(items) => items.iter().find_map(|item| other_branch(item, branch)),
        _ => None,
    }
}

fn branch(params: &Value) -> io::Result<u64> {
    params.get("branchId").and_then(Value::as_str).and_then(|id| id.parse().ok()).ok_or_else(|| crate::invalid("A follow request names no branch"))
}

pub fn lacking(table: usize, held: bool) -> Option<String> {
    (!held).then(|| format!("{} is not in this playhead's files (a derived table, until format 12)", table_name(table)))
}
