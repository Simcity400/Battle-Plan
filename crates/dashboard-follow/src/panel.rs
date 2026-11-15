//! Panels at the head: a panel names the tables it reads and makes one value from a frame of rows,
//! its data (a product's reply as JSON) or why it has none at this hour; its host gives it rows at
//! the head at its cadence (at most four times a second while playing, at once when paused or on an
//! anchor), evaluates it off the playhead's threads, and pushes its data only when it differs, as a
//! message of its own kind ([`crate::push::PANEL`]): `PNL1` and JSON
//! `{"panels":[{"id","hour","data","refused"}]}`. A new panel is a [`Panel`] and the name its follow
//! query gives (`{"panel": <kind>, ..}`); nothing else. A panel naming live cells ([`Panel::live`])
//! has them in its data as `live` ([`Live::json`]), each group followed under the id the host gives.

use crate::live::Live;
use replay_data::{Hour, rows::TableId};
use replay_frame::frame::DecodedFrame;
use serde_json::{Value, json};
use std::{collections::BTreeMap, io, sync::Arc};

/// The first bytes of a panel message, beside followed answers' `FOL1`.
pub const MAGIC: &[u8; 4] = b"PNL1";

/// A panel's value at an hour: its data, or why it has none (the request path's refusal).
pub type PanelData = Result<Value, String>;

/// A panel at the head.
pub trait Panel: Send {
    /// The tables its data reads, whole.
    fn tables(&self) -> &[TableId];
    /// Its data from rows at the head holding those tables.
    fn data(&mut self, frame: &DecodedFrame) -> PanelData;
    /// The live cells its last data names ([`crate::live`]): its host follows them by key while
    /// that data stands, and replaces them with the next data that differs.
    fn live(&self) -> Option<&Live> {
        None
    }
}

/// Rows at the head of one playhead, for the panels its host shows.
pub trait PanelRows: Send + Sync {
    /// A reader of `tables` at a panel's cadence; it holds them until dropped.
    fn reader(&self, tables: &[TableId]) -> io::Result<Box<dyn PanelReader>>;
}

/// One panel's rows at the head.
pub trait PanelReader: Send {
    /// The newest frame of its tables; none while one is loading. Never waits; the same `Arc`
    /// while nothing it reads changed.
    fn frame(&self) -> Option<Arc<DecodedFrame>>;
    /// Why its rows stopped coming for good, and the hour that is said at: the panel is refused
    /// by name, never left loading.
    fn stopped(&self) -> Option<(Hour, String)> {
        None
    }
}

fn data_json(id: u32, hour: Hour, data: &PanelData) -> Value {
    match data {
        Ok(data) => json!({ "id": id, "hour": hour, "data": data, "refused": null }),
        Err(refused) => json!({ "id": id, "hour": hour, "data": null, "refused": refused }),
    }
}

/// A message of panels' data.
pub fn message(panels: &[(u32, Hour, &PanelData)]) -> Vec<u8> {
    let body = json!({ "panels": panels.iter().map(|(id, hour, data)| data_json(*id, *hour, data)).collect::<Vec<_>>() });
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(body.to_string().as_bytes());
    bytes
}

fn panels(bytes: &[u8]) -> io::Result<Vec<Value>> {
    let body = bytes.strip_prefix(MAGIC.as_slice()).ok_or_else(|| crate::invalid("A panel message does not start with PNL1"))?;
    let value: Value = serde_json::from_slice(body).map_err(|error| crate::invalid(format!("A panel message is not JSON: {error}")))?;
    value.get("panels").and_then(Value::as_array).cloned().ok_or_else(|| crate::invalid("A panel message names no panels"))
}

/// `older` and `newer` as one message, the newer data of each panel kept; and how many were superseded.
pub fn merge(older: &[u8], newer: &[u8]) -> io::Result<(Vec<u8>, u32)> {
    let mut by_id: BTreeMap<u64, Value> = BTreeMap::new();
    let mut superseded = 0;
    for panel in panels(older)?.into_iter().chain(panels(newer)?) {
        let id = panel.get("id").and_then(Value::as_u64).ok_or_else(|| crate::invalid("A panel names no id"))?;
        superseded += u32::from(by_id.insert(id, panel).is_some());
    }
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(json!({ "panels": by_id.into_values().collect::<Vec<_>>() }).to_string().as_bytes());
    Ok((bytes, superseded))
}
