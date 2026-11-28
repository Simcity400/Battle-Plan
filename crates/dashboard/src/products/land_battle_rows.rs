use super::battle_names;
use dashboard_base::{Source, catalogue::{self, OwnRefusals}, products::{self, LoadContext, PanelReader, Product, Registered, at_hour, occurrences, read_ahead::Prepare}, queries};
use dashboard_core::{QueryError, presentation::Admitted};
use dashboard_follow::panel::Panel;
use dashboard_land::{products::land_battles::tail::{self, LandBattlesTail, LandBattlesTailParams, LandBattlesTailReply}, registrations::LAND_BATTLES_TAIL};
use dashboard_world::products::{place_name_runs::{PlacePoint, names_at}, place_names::Name};
use serde_json::Value;
use std::{collections::{BTreeMap, BTreeSet}, io, sync::{Arc, Mutex, PoisonError, atomic::AtomicBool}};

pub(super) const PRODUCT: Registered = Registered {
    content_dependent: true,
    prepare: products::register::<Named>().prepare,
    load,
    panel: Some(PanelReader { open }),
    ..LAND_BATTLES_TAIL
};

fn load(source: &Source, lease: &str, asked: Value, load: &LoadContext, cancelled: &AtomicBool) -> Result<String, QueryError> {
    if asked["rowNames"].as_bool() == Some(true) {
        return (products::register::<Named>().load)(source, lease, asked, load, cancelled);
    }
    let mut content = load.content.clone();
    if let Some(content) = &mut content {
        content.presentation = Admitted::recorded(Arc::clone(content.presentation.binding()));
    }
    (LAND_BATTLES_TAIL.load)(source, lease, asked, &LoadContext { content }, cancelled)
}

fn open(source: &Arc<Source>, load: &LoadContext, asked: &Value) -> Result<Box<dyn Panel>, String> {
    if asked["rowNames"].as_bool() != Some(true) {
        return tail::dated_panel(source, load, asked);
    }
    let page = Mutex::new(Page::default());
    at_hour::dated_product::<Named>(source, load, asked, at_hour::at_hour, |_, reply| at_hour::before(reply.next_change), |_, _| {},
        move |source, params, load, cancelled, _| page.lock().unwrap_or_else(PoisonError::into_inner).read(source, params, load, cancelled))
}

struct Named;

impl Product for Named {
    const KEY: &'static str = LandBattlesTail::KEY;
    const CONTENT_DEPENDENT: bool = true;
    const FIELDS: products::Fields = LandBattlesTail::FIELDS;
    const PREPARE: Option<Prepare<Self::Params>> = Some(|source, params, load| {
        if params.row_names == Some(true) { battle_names::prepare_for_rows(source, branch(params)?, load) } else { Ok(None) }
    });
    type Params = LandBattlesTailParams;
    type Reply = LandBattlesTailReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        LandBattlesTail::validate(params)
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        Page::default().read(source, params, load, cancelled)
    }
}

fn branch(params: &LandBattlesTailParams) -> io::Result<u64> {
    params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))
}

#[derive(Default)]
struct Page {
    prepared: Option<(tail::Prepared, OwnRefusals)>,
    places: BTreeMap<PlacePoint, Name>,
}

impl Page {
    fn read(&mut self, source: &Source, params: &LandBattlesTailParams, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<LandBattlesTailReply> {
        dashboard_base::metadata::check_cancel(cancelled)?;
        let branch = branch(params)?;
        let query = queries::query(source, branch)?;
        let hour = occurrences::cutoff(&query, params.hour)?;
        if self.prepared.is_none() {
            let mut notes = OwnRefusals::default();
            let prepared = catalogue::own_refusals(&mut notes, || tail::prepare(source, &query, load, params, cancelled))?;
            self.prepared = Some((prepared, notes));
        }
        let (prepared, notes) = self.prepared.as_ref().expect("prepared page");
        catalogue::credit(notes);
        let mut reply = tail::reply(prepared, &query.holders, params, hour)?;
        at_hour::count_rows(|| reply.rows.len() as u64);
        if reply.rows.is_empty() { self.places.clear(); return Ok(reply); }
        let titles = battle_names::for_rows(source, branch, load, cancelled).map_err(|reason| reason.to_string());
        dashboard_base::metadata::check_cancel(cancelled)?;
        let point = |row: &tail::LandBattleLogRow| row.state.map(|id| PlacePoint::State(id, row.hour));
        let points: BTreeSet<_> = reply.rows.iter().filter_map(point).collect();
        self.places.retain(|point, _| points.contains(point));
        let needed = points.into_iter().filter(|point| !self.places.contains_key(point)).collect();
        self.places.extend(names_at(source, &query, load, &needed, cancelled)?);
        for row in &mut reply.rows {
            let id = row.key.split_once(':').map_or(row.key.as_str(), |(_, id)| id);
            let title = match &titles {
                Ok(titles) => titles.reading(&format!("battle:{id}")),
                Err(reason) => battle_names::BattleName::missing(reason.clone()),
            };
            row.title = title.name;
            row.title_missing = title.missing;
            if let Some(name) = point(row).and_then(|point| self.places.get(&point)) {
                row.place_name.clone_from(&name.name);
                row.place_missing.clone_from(&name.missing);
            } else {
                row.place_missing = Some("Province state not recorded".into());
            }
        }
        let bytes = self.places.values().map(|place| place.name.as_ref().map_or(0, String::capacity) + place.missing.as_ref().map_or(0, String::capacity)).sum::<usize>();
        if bytes > products::MAX_PRODUCT_REPLY_BYTES { self.places.clear(); }
        products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}
