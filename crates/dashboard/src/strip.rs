use base64::Engine as _;
use dashboard_base::{
    Source,
    products::{LoadContext, MAX_PRODUCT_REPLY_BYTES, over_budget},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub(super) mod cache;
pub mod patch;

pub const PRODUCT: &str = "strip";
const VERSION: u8 = 1;
const RAW_LIMIT: usize = (MAX_PRODUCT_REPLY_BYTES - "{\"data\":\"\"}".len()) / 4 * 3;

#[derive(Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(deny_unknown_fields)]
pub struct StripParams {
    pub product: String,
    #[cfg_attr(
        any(test, feature = "export-types"),
        ts(type = "Array<Record<string, unknown>>")
    )]
    pub asks: Vec<Value>,
    pub whole: bool,
}

#[derive(Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
pub struct StripReply {
    pub data: String,
}

impl StripReply {
    fn from_bytes(bytes: &[u8]) -> Self {
        Self { data: base64::engine::general_purpose::STANDARD.encode(bytes) }
    }
}

pub fn answer(
    source: &Arc<Source>,
    params: &Value,
    load: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<StripReply> {
    let params = request(params)?;
    Ok(StripReply::from_bytes(&compute(source, &params, load, cancelled)?))
}

fn request(params: &Value) -> io::Result<StripParams> {
    let params = StripParams::deserialize(params)
        .map_err(|error| dashboard_core::refused(format!("Invalid strip request: {error}")))?;
    if params.product == PRODUCT {
        return Err(dashboard_core::refused("A strip cannot answer strip"));
    }
    if crate::answer::follows(&params.product) {
        return Err(dashboard_core::refused(format!(
            "A strip cannot answer the {} command",
            params.product
        )));
    }
    if !(1..=17).contains(&params.asks.len()) || params.asks.iter().any(|ask| !ask.is_object()) {
        return Err(dashboard_core::refused(
            "A strip names 1 to 17 parameter objects in hour order",
        ));
    }
    Ok(params)
}

fn compute(
    source: &Arc<Source>,
    params: &StripParams,
    load: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Vec<u8>> {
    dashboard_base::products::at_hour::with_strip_pool(source, || {
        build(params.asks.len(), params.whole, cancelled, |index| {
            state(crate::answer::answer(
                source,
                &params.product,
                &params.asks[index],
                load,
                cancelled,
            ))
        })
    })
}

fn state(result: io::Result<Value>) -> io::Result<Value> {
    match result {
        Ok(answer) => Ok(Value::Object(serde_json::Map::from_iter([(
            "answer".into(),
            answer,
        )]))),
        Err(error) => {
            let reason = dashboard_core::refusal(&error).or_else(|| {
                match error
                    .get_ref()?
                    .downcast_ref::<dashboard_core::QueryError>()?
                {
                    dashboard_core::QueryError::Refused { text } => Some(text.as_str()),
                    _ => None,
                }
            });
            match reason {
                Some(reason) => Ok(json!({"refused":reason})),
                None => Err(error),
            }
        }
    }
}

fn build(
    count: usize,
    whole: bool,
    cancelled: &AtomicBool,
    mut answer: impl FnMut(usize) -> io::Result<Value>,
) -> io::Result<Vec<u8>> {
    let mut bytes = vec![b'B', b'P', b'S', b'T', VERSION, u8::from(whole), count as u8, 0];
    let mut previous = None;
    for index in 0..count {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let current = answer(index)?;
        dashboard_text::metadata::check_cancel(cancelled)?;
        match previous {
            Some(previous) => {
                append(
                    &mut bytes,
                    patch::encode(&patch::diff(&previous, &current))?,
                )?;
                append(
                    &mut bytes,
                    patch::encode(&patch::diff(&current, &previous))?,
                )?;
            }
            None if whole => append(
                &mut bytes,
                rmp_serde::to_vec(&current).map_err(io::Error::other)?,
            )?,
            None => {}
        }
        previous = Some(current);
    }
    Ok(bytes)
}

fn append(bytes: &mut Vec<u8>, part: Vec<u8>) -> io::Result<()> {
    if bytes.len().saturating_add(4).saturating_add(part.len()) > RAW_LIMIT {
        return Err(over_budget(format!(
            "This product reply exceeds the transport reply budget of {MAX_PRODUCT_REPLY_BYTES} bytes"
        )));
    }
    bytes.extend_from_slice(&(part.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&part);
    Ok(())
}

#[cfg(test)]
mod tests;
