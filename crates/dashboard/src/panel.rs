use dashboard_base::{Source, products::{PanelReader, at_hour::{at_hour, interval_panel}}};
use dashboard_core::config::LoadContext;
use dashboard_follow::panel::{AtHour, ExactReader, Panel};
use dashboard_query::series::{self, ranked};
use serde_json::Value;
use std::{collections::{BTreeMap, HashMap}, io, sync::{Arc, LazyLock, Mutex, PoisonError, atomic::AtomicBool}};

pub(crate) static READERS: LazyLock<BTreeMap<&'static str, PanelReader>> = LazyLock::new(|| {
    let mut readers = BTreeMap::from([(dashboard_query::series::ranked::PRODUCT, PanelReader { open: rank_panel })]);
    for product in crate::products::PRODUCTS.iter() {
        if let Some(reader) = product.panel {
            assert!(readers.insert(product.key, reader).is_none(), "Duplicate panel {}", product.key);
        }
    }
    readers
});

pub fn open(source: &Arc<Source>, load: &LoadContext, asked: &Value) -> Result<Box<dyn Panel>, String> {
    let key = &asked["panel"];
    let reader = key.as_str().and_then(|key| READERS.get(key)).ok_or_else(|| format!("No panel {key}"))?;
    (reader.open)(source, load, asked)
}

#[cfg(test)]
mod registry_tests;

fn rank_panel(source: &Arc<Source>, load: &LoadContext, asked: &Value) -> Result<Box<dyn Panel>, String> {
    let source = Arc::clone(source);
    let load = LoadContext { content: load.content.clone() };
    let mut params = asked.clone();
    params.as_object_mut().ok_or("Expected rank parameters")?.remove("panel");
    let retains_runs = params["family"]["kind"] == "country-game-scores";
    let cold: Option<ExactReader> = retains_runs.then(|| {
        let (source, params) = (source.clone(), params.clone());
        let load = LoadContext { content: load.content.clone() };
        Arc::new(move |hour, cancelled: &AtomicBool| {
            let mut request = params.clone();
            at_hour(&mut request, hour);
            engine_executable::executable_tables::scope(source.tables(), || {
                crate::answer::answer(&source, ranked::PRODUCT, &request, &load, cancelled).map_err(|error| error.to_string())
            })
        }) as ExactReader
    });
    let panel = interval_panel(source.tables(), move |hour, cancelled| {
        let mut request = params.clone();
        at_hour(&mut request, hour);
        let reply = if retains_runs {
            followed_rank(&source, &request, &load, cancelled)?
        } else {
            crate::answer::answer(&source, ranked::PRODUCT, &request, &load, cancelled)?
        };
        let bound = |name: &str| reply[name].as_i64().and_then(|n| i32::try_from(n).ok()).unwrap_or(hour);
        let proven = bound("from")..=bound("until");
        let rows = reply["rows"].as_array().map_or(0, |rows| rows.len() as u64);
        Ok((reply, proven, rows))
    });
    match cold {
        Some(cold) => {
            let advance = Arc::clone(&panel.exact().ok_or("The rank panel has no at-hour reader")?.advance);
            Ok(Box::new(AtHour { advance, read: cold.clone(), cold }))
        }
        None => Ok(panel),
    }
}

type RankBlocks = HashMap<u64, Arc<Mutex<ranked::Retained>>>;

fn followed_rank(source: &Arc<Source>, request: &Value, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Value> {
    let check_cancel = || dashboard_text::metadata::check_cancel(cancelled);
    check_cancel()?;
    let branch = dashboard_query::branch(request)?;
    let query = dashboard_base::queries::query(source, branch)?;
    let groups = request.get("group").is_some_and(|group| !group.is_null())
        .then(|| dashboard_world::products::group_members::GroupsAt::for_query(source, &query, load, cancelled)).transpose()?;
    let resolve = |group: &ranked::GroupSelection, hour| {
        groups.as_ref().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Group membership is not read for this request"))?
            .slots_at(group, &query.holders, hour)
    };
    let state = source.state::<RankBlocks>();
    let cached = Arc::clone(state.lock().unwrap_or_else(PoisonError::into_inner).entry(branch).or_default());
    series::answer_ranked_retained(&query, request, &resolve, &cached, &check_cancel)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("No declared series answers {}", ranked::PRODUCT)))
}
