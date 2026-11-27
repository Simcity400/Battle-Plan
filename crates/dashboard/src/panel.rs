use dashboard_base::{Source, products::{PanelKind, PanelReader, at_hour::{at_hour, interval_panel}}};
use dashboard_core::config::LoadContext;
use dashboard_follow::panel::Panel;
use serde_json::Value;
use std::{collections::BTreeMap, sync::{Arc, LazyLock}};

pub(crate) static READERS: LazyLock<BTreeMap<&'static str, PanelReader>> = LazyLock::new(|| {
    let mut readers = BTreeMap::from([(dashboard_query::series::ranked::PRODUCT, PanelReader { kind: PanelKind::Interval, open: rank_panel })]);
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
    Ok(interval_panel(source.tables(), move |hour, cancelled| {
        let mut request = params.clone();
        at_hour(&mut request, hour);
        let reply = crate::answer::answer(&source, dashboard_query::series::ranked::PRODUCT, &request, &load, cancelled)?;
        let bound = |name: &str| reply[name].as_i64().and_then(|n| i32::try_from(n).ok()).unwrap_or(hour);
        let proven = bound("from")..=bound("until");
        let rows = reply["rows"].as_array().map_or(0, |rows| rows.len() as u64);
        Ok((reply, proven, rows))
    }))
}
