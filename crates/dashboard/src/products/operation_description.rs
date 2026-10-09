//! Exact recorded operation text, loaded in byte pages only when requested.
use super::{LoadContext, Product};
use crate::{Source, metadata::validate_campaign};
use replay::{
    container::Replay,
    frame::FrameDecoder,
    rows::{TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

pub struct OperationDescription;
/// Transport page size, independent of the native description's full length.
pub const DESCRIPTION_PAGE_BYTES: u32 = 32 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationDescriptionRef {
    /// Supplying daily read or completion drain hour, never an inferred event date.
    pub hour: i32,
    pub id: String,
    pub owner: u32,
    /// Some identifies an immutable completion event; None a visible-instance read.
    pub sequence: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationDescriptionParams {
    pub branch_id: String,
    pub description: OperationDescriptionRef,
    pub offset: String,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OperationDescriptionReply {
    pub description: OperationDescriptionRef,
    pub recorded_language: String,
    pub offset: String,
    pub total_bytes: String,
    pub next_offset: Option<String>,
    pub bytes: Vec<u8>,
    pub unavailable: Option<String>,
    pub warnings: Vec<String>,
}

fn chunk(
    params: &OperationDescriptionParams,
    language: &str,
    bytes: &[u8],
    required: u64,
    refusal: Option<&str>,
) -> io::Result<OperationDescriptionReply> {
    let offset = params
        .offset
        .parse::<u64>()
        .map_err(|error| crate::invalid(error.to_string()))?;
    if offset > required {
        return Err(crate::invalid(
            "Operation description offset exceeds its native length",
        ));
    }
    let (page, next) = if refusal.is_some() {
        (Vec::new(), None)
    } else {
        if bytes.len() as u64 != required {
            return Err(crate::invalid(
                "Recorded operation description length differs from its native length",
            ));
        }
        let start = usize::try_from(offset).map_err(|error| crate::invalid(error.to_string()))?;
        let end = start
            .saturating_add(DESCRIPTION_PAGE_BYTES as usize)
            .min(bytes.len());
        (
            bytes[start..end].to_vec(),
            (end < bytes.len()).then(|| end.to_string()),
        )
    };
    Ok(OperationDescriptionReply {
        description: params.description.clone(),
        recorded_language: language.to_owned(),
        offset: params.offset.clone(),
        total_bytes: required.to_string(),
        next_offset: next,
        bytes: page,
        unavailable: refusal.map(str::to_owned),
        warnings: Vec::new(),
    })
}
/// Repeated immutable event keys are checked against the first stored read,
/// without retaining every branch description in a widget cache.
pub(super) fn completions_match(
    replay: &Replay,
    hour: i32,
    incoming: &[&replay::rows::OperationCompletionRow],
    branch: u64,
) -> io::Result<bool> {
    let selected = [TableId::Campaign, TableId::OperationCompletions];
    let block = replay
        .index
        .iter()
        .find(|block| block.branch == branch && block.first_hour <= hour && hour <= block.last_hour)
        .ok_or_else(|| crate::invalid("Completion witness hour missing"))?;
    let mut decoder = FrameDecoder::for_tables(&selected);
    for frame in replay.read_block_selected(block, &selected)? {
        if frame.hour > hour {
            break;
        }
        let decoded = decoder.advance(&frame)?;
        validate_campaign(decoded, replay)?;
        if frame.hour == hour {
            let by_key: std::collections::HashMap<_, _> = decoded
                .complete::<tables::OperationCompletions>()?
                .iter()
                .map(|row| ((row.owner, row.sequence), row))
                .collect();
            return Ok(incoming
                .iter()
                .all(|row| by_key.get(&(row.owner, row.sequence)).copied() == Some(*row)));
        }
    }
    Err(crate::invalid("Completion witness read missing"))
}

fn load(
    replay: &Replay,
    params: &OperationDescriptionParams,
    cancelled: &AtomicBool,
) -> io::Result<OperationDescriptionReply> {
    let branch = params
        .branch_id
        .parse::<u64>()
        .map_err(|error| crate::invalid(error.to_string()))?;
    let reference = &params.description;
    let id = reference
        .id
        .parse::<u64>()
        .map_err(|error| crate::invalid(error.to_string()))?;
    let sequence = reference
        .sequence
        .as_ref()
        .map(|s| s.parse::<u64>())
        .transpose()
        .map_err(|error| crate::invalid(error.to_string()))?;
    let table = if sequence.is_some() {
        TableId::OperationCompletions
    } else {
        TableId::Operations
    };
    let selected = [TableId::Campaign, table];
    let block = replay
        .index
        .iter()
        .find(|block| {
            block.branch == branch
                && block.first_hour <= reference.hour
                && reference.hour <= block.last_hour
        })
        .ok_or_else(|| crate::invalid("Operation description supplying hour was not recorded"))?;
    let mut decoder = FrameDecoder::for_tables(&selected);
    for frame in replay.read_block_selected(block, &selected)? {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Operation description request cancelled",
            ));
        }
        if frame.hour > reference.hour {
            break;
        }
        let decoded = decoder.advance(&frame)?;
        validate_campaign(decoded, replay)?;
        if frame.hour != reference.hour {
            continue;
        }
        let language = replay
            .identity
            .language
            .strip_prefix("l_")
            .unwrap_or(&replay.identity.language);
        if let Some(sequence) = sequence {
            let row = decoded.complete::<tables::OperationCompletions>()?.iter()
                .find(|row| row.owner == reference.owner && row.sequence == sequence && row.id == id)
                .ok_or_else(|| crate::invalid("Operation completion description identity was not recorded at its drain hour"))?;
            return chunk(
                params,
                language,
                &row.effect_description,
                row.description_required,
                row.description_refusal(),
            );
        }
        let row = decoded
            .complete::<tables::Operations>()?
            .iter()
            .find(|row| row.owner == reference.owner && row.id == id)
            .ok_or_else(|| {
                crate::invalid(
                    "Visible operation description identity was not recorded at its read hour",
                )
            })?;
        let bytes = row
            .effect_description
            .require("Operations.effect_description")
            .map_err(crate::invalid)?;
        return chunk(params, language, bytes, bytes.len() as u64, None);
    }
    Err(crate::invalid(
        "Operation description supplying hour was not recorded",
    ))
}
impl Product for OperationDescription {
    const KEY: &'static str = "operation-description";
    const CONTENT_DEPENDENT: bool = false;
    type Params = OperationDescriptionParams;
    type Reply = OperationDescriptionReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid operation description branch")?;
        params
            .description
            .id
            .parse::<u64>()
            .map_err(|_| "Invalid operation description handle")?;
        if let Some(sequence) = &params.description.sequence {
            sequence
                .parse::<u64>()
                .map_err(|_| "Invalid operation completion sequence")?;
        }
        params
            .offset
            .parse::<u64>()
            .map_err(|_| "Invalid operation description offset")?;
        Ok(())
    }
    fn types() -> Vec<String> {
        vec![
            OperationDescriptionRef::decl(),
            OperationDescriptionParams::decl(),
            OperationDescriptionReply::decl(),
        ]
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        _: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        load(&source.replay(), params, cancelled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_description_pages_keep_invalid_utf8_and_native_refusals_distinct_from_empty() {
        let mut params = OperationDescriptionParams {
            branch_id: "1".into(),
            description: OperationDescriptionRef {
                hour: -3,
                id: u64::MAX.to_string(),
                owner: 5_000,
                sequence: Some("27".into()),
            },
            offset: "0".into(),
        };
        let exact = vec![0xff; DESCRIPTION_PAGE_BYTES as usize + 17];
        let first = chunk(
            &params,
            "recorded language",
            &exact,
            exact.len() as u64,
            None,
        )
        .unwrap();
        assert_eq!(first.bytes, exact[..DESCRIPTION_PAGE_BYTES as usize]);
        params.offset = first.next_offset.unwrap();
        let second = chunk(
            &params,
            "recorded language",
            &exact,
            exact.len() as u64,
            None,
        )
        .unwrap();
        assert_eq!(second.bytes, exact[DESCRIPTION_PAGE_BYTES as usize..]);
        assert!(second.next_offset.is_none());
        params.offset = "0".into();
        let refused = chunk(
            &params,
            "recorded language",
            &[],
            1_000_000,
            Some("native bytes unavailable"),
        )
        .unwrap();
        assert_eq!(refused.total_bytes, "1000000");
        assert!(refused.unavailable.is_some());
        assert!(
            chunk(&params, "recorded language", &[], 0, None)
                .unwrap()
                .unavailable
                .is_none()
        );
    }
}
