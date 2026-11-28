use super::battle_names;
use dashboard_base::{Source, catalogue::{self, OwnRefusals}, products::{self, LoadContext, PanelReader, Product, Registered, at_hour, occurrences, read_ahead::Prepare}, queries};
use dashboard_core::QueryError;
use dashboard_follow::panel::Panel;
use dashboard_naval::{
    products::naval::{sinking_details::ShipSinkingBattle, sunk::tail::{self, ShipsSunkTail, ShipsSunkTailParams, ShipsSunkTailReply}},
    registrations::NAVAL_SUNK_TAIL,
};
use serde_json::Value;
use std::{io, sync::{Arc, Mutex, PoisonError, atomic::AtomicBool}};

pub(super) const PRODUCT: Registered = Registered {
    prepare: products::register::<Named>().prepare,
    load,
    panel: Some(PanelReader { open }),
    ..NAVAL_SUNK_TAIL
};

fn load(source: &Source, lease: &str, asked: Value, load: &LoadContext, cancelled: &AtomicBool) -> Result<String, QueryError> {
    if asked["rowNames"].as_bool() == Some(true) {
        return (products::register::<Named>().load)(source, lease, asked, load, cancelled);
    }
    (NAVAL_SUNK_TAIL.load)(source, lease, asked, load, cancelled)
}

fn open(source: &Arc<Source>, load: &LoadContext, asked: &Value) -> Result<Box<dyn Panel>, String> {
    if asked["rowNames"].as_bool() != Some(true) {
        return tail::dated_panel(source, load, asked);
    }
    let prepared = Mutex::new(None);
    at_hour::dated_product::<Named>(source, load, asked, at_hour::at_hour, |_, reply| at_hour::before(reply.next_change), |_, _| {},
        move |source, params, load, cancelled, _| {
            dashboard_base::metadata::check_cancel(cancelled)?;
            let branch = branch(params)?;
            let query = queries::query(source, branch)?;
            let hour = occurrences::cutoff(&query, params.hour)?;
            let mut prepared = prepared.lock().unwrap_or_else(PoisonError::into_inner);
            if prepared.is_none() {
                let mut notes = OwnRefusals::default();
                let ready = catalogue::own_refusals(&mut notes, || tail::prepare(source, &query, load, params, cancelled))?;
                *prepared = Some((ready, notes));
            }
            let (prepared, notes) = prepared.as_ref().expect("prepared ships sunk page");
            catalogue::credit(notes);
            let mut reply = tail::reply(prepared, &query.holders, params, hour)?;
            at_hour::count_rows(|| reply.rows.len() as u64);
            attach_titles(source, branch, load, cancelled, &mut reply)?;
            Ok(reply)
        })
}

struct Named;

impl Product for Named {
    const KEY: &'static str = ShipsSunkTail::KEY;
    const CONTENT_DEPENDENT: bool = ShipsSunkTail::CONTENT_DEPENDENT;
    const FIELDS: products::Fields = ShipsSunkTail::FIELDS;
    const PREPARE: Option<Prepare<Self::Params>> = Some(|source, params, load| {
        if params.row_names == Some(true) { battle_names::prepare_for_rows(source, branch(params)?, load) } else { Ok(None) }
    });
    type Params = ShipsSunkTailParams;
    type Reply = ShipsSunkTailReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        ShipsSunkTail::validate(params)
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let mut reply = ShipsSunkTail::load(source, params, load, cancelled)?;
        attach_titles(source, branch(params)?, load, cancelled, &mut reply)?;
        Ok(reply)
    }
}

fn branch(params: &ShipsSunkTailParams) -> io::Result<u64> {
    params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))
}

fn attach_titles(source: &Source, branch: u64, load: &LoadContext, cancelled: &AtomicBool, reply: &mut ShipsSunkTailReply) -> io::Result<()> {
    let mut linked = reply.rows.iter_mut().filter_map(|row| match &row.battle {
        ShipSinkingBattle::Linked { id, .. } => Some((id, &mut row.title, &mut row.title_missing)),
        ShipSinkingBattle::NotRecorded { .. } => None,
    }).peekable();
    if linked.peek().is_none() {
        return Ok(());
    }
    let titles = battle_names::for_rows(source, branch, load, cancelled).map_err(|reason| reason.to_string());
    dashboard_base::metadata::check_cancel(cancelled)?;
    for (id, name, missing) in linked {
        let title = match &titles {
            Ok(titles) => titles.reading(id),
            Err(reason) => battle_names::BattleName::missing(reason.clone()),
        };
        *name = title.name;
        *missing = title.missing;
    }
    products::check_reply_budget(reply)
}
