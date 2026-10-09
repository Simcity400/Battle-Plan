//! Hour-specific focus presentation; history consumers share the native selector.
use super::{LoadContext, Product, artwork, validate_tag};
use crate::{
    Artwork, Source,
    metadata::{check_cancel, validate_campaign},
};
use replay::{frame::FrameDecoder, rows::TableId};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io, sync::atomic::AtomicBool};
use ts_rs::TS;

pub struct FocusArtwork;
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FocusArtworkParams {
    pub branch_id: String,
    pub hour: i32,
    pub tag: String,
    pub tokens: Vec<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DatedFocusArtwork {
    pub artwork: Artwork,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FocusArtworkReply {
    pub branch_id: String,
    pub hour: i32,
    pub tag: String,
    pub items: Vec<DatedFocusArtwork>,
    pub warnings: Vec<String>,
}
#[derive(Default)]
struct State {
    artwork: Option<artwork::Context>,
}

impl Product for FocusArtwork {
    const KEY: &'static str = "focus-artwork";
    const CONTENT_DEPENDENT: bool = true;
    type Params = FocusArtworkParams;
    type Reply = FocusArtworkReply;
    fn types() -> Vec<String> {
        vec![
            FocusArtworkParams::decl(),
            DatedFocusArtwork::decl(),
            FocusArtworkReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate_tag(&params.tag)?;
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid focus artwork branch")?;
        if params.tokens.is_empty()
            || params
                .tokens
                .iter()
                .any(|token| token.is_empty() || token.contains('\0'))
        {
            return Err("Invalid focus artwork tokens");
        }
        Ok(())
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        check_cancel(cancelled)?;
        let branch = params.branch_id.parse::<u64>().expect("validated branch");
        let replay = source.replay();
        let block = replay
            .index
            .iter()
            .find(|block| {
                block.branch == branch
                    && block.first_hour <= params.hour
                    && params.hour <= block.last_hour
            })
            .ok_or_else(|| io::Error::other("Focus artwork hour was not recorded"))?;
        let state = source.state::<State>();
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        let art = artwork::ensure(
            &mut state.artwork,
            source,
            context.content()?,
            artwork::Kind::Focus,
        )?;
        let mut selected =
            BTreeSet::from([TableId::Campaign, TableId::Countries, TableId::CountryLinks]);
        if art.requested_focus_needs_frame(&params.tokens) {
            selected.extend(crate::scripted_text::TABLES.iter().copied());
            selected.insert(TableId::Clock);
        }
        let selected: Vec<_> = selected.into_iter().collect();
        let mut decoder = FrameDecoder::for_tables(&selected);
        for frame in replay.read_block_selected(block, &selected)? {
            check_cancel(cancelled)?;
            if frame.hour > params.hour {
                break;
            }
            let decoded = decoder.advance(&frame)?;
            validate_campaign(decoded, &replay)?;
            if frame.hour != params.hour {
                continue;
            }
            let tokens: BTreeSet<_> = params.tokens.iter().map(String::as_str).collect();
            let requests: Vec<_> = tokens
                .into_iter()
                .map(|token| (token, params.tag.as_str()))
                .collect();
            let warnings = art.standing_warnings();
            return art.focus_tree_at(&requests, decoded, |items| {
                reply_within(
                    params,
                    warnings,
                    items,
                    cancelled,
                    super::MAX_PRODUCT_REPLY_BYTES,
                )
            });
        }
        Err(io::Error::other("Focus artwork hour was not recorded"))
    }
}

/// Measure the actual escaped JSON before retaining each lazy artwork item.
fn reply_within(
    params: &FocusArtworkParams,
    warnings: Vec<String>,
    mut items: impl Iterator<Item = artwork::ResolvedArt>,
    cancelled: &AtomicBool,
    budget: usize,
) -> io::Result<FocusArtworkReply> {
    let mut reply = FocusArtworkReply {
        branch_id: params.branch_id.clone(),
        hour: params.hour,
        tag: params.tag.clone(),
        items: Vec::new(),
        warnings,
    };
    let refused = || {
        io::Error::other(format!(
            "Focus artwork exceeds the transport reply budget of {budget} bytes; no tokens were omitted"
        ))
    };
    let mut bytes = crate::transport::json_len(&reply, budget).ok_or_else(refused)?;
    loop {
        check_cancel(cancelled)?;
        let Some((artwork, reason)) = items.next() else {
            break;
        };
        let item = DatedFocusArtwork { artwork, reason };
        let separator = usize::from(!reply.items.is_empty());
        let remaining = budget.saturating_sub(bytes).saturating_sub(separator);
        let size = crate::transport::json_len(&item, remaining).ok_or_else(refused)?;
        bytes += separator + size;
        reply.items.push(item);
    }
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn params() -> FocusArtworkParams {
        FocusArtworkParams {
            branch_id: "1".into(),
            hour: 10,
            tag: "synthetic".into(),
            tokens: vec!["focus".into()],
        }
    }
    fn item(icon: Option<String>) -> artwork::ResolvedArt {
        (
            Artwork {
                token: "focus".into(),
                name: Some("A \"quoted\" focus".into()),
                description: None,
                base_duration_days: None,
                icon,
            },
            None,
        )
    }

    #[test]
    fn large_token_requests_stop_artwork_generation_at_the_serialized_budget() {
        let mut params = params();
        params.tokens = (0..5_000).map(|index| format!("focus-{index}")).collect();
        assert!(FocusArtwork::validate(&params).is_ok());
        let produced = Cell::new(0);
        let items = params.tokens.iter().map(|token| {
            produced.set(produced.get() + 1);
            let mut resolved = item(Some(format!("data:image/png;base64,{}", "A".repeat(512))));
            resolved.0.token.clone_from(token);
            resolved
        });
        let error =
            reply_within(&params, Vec::new(), items, &AtomicBool::new(false), 1024).unwrap_err();
        assert!(error.to_string().contains("transport reply budget"));
        assert!(
            produced.get() < 5_000,
            "rejection must stop before cloning all requested icons"
        );
    }

    #[test]
    fn reply_admission_charges_escaped_strings_and_item_separators_exactly() {
        let params = params();
        let items = || [item(None), item(None)].into_iter();
        let cancelled = AtomicBool::new(false);
        let reply = reply_within(
            &params,
            vec!["A \"quoted\" warning".into()],
            items(),
            &cancelled,
            super::super::MAX_PRODUCT_REPLY_BYTES,
        )
        .unwrap();
        let size = crate::transport::json_len(&reply, usize::MAX).unwrap();
        assert!(reply_within(&params, reply.warnings.clone(), items(), &cancelled, size).is_ok());
        assert!(
            reply_within(
                &params,
                reply.warnings.clone(),
                items(),
                &cancelled,
                size - 1
            )
            .is_err()
        );
    }
}
