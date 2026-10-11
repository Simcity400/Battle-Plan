//! Technology images load independently of research history and its titles.
use super::{LoadContext, Product, artwork};
use crate::{Artwork, Source, metadata::check_cancel};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io, sync::atomic::AtomicBool};
use ts_rs::TS;

pub struct TechnologyArtwork;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TechnologyArtworkParams {
    pub tokens: Vec<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TechnologyArtworkReply {
    pub items: Vec<Artwork>,
    pub warnings: Vec<String>,
}

#[derive(Default)]
struct State {
    artwork: Option<artwork::Context>,
}

impl Product for TechnologyArtwork {
    const KEY: &'static str = "technology-artwork";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = TechnologyArtworkParams;
    type Reply = TechnologyArtworkReply;

    fn types() -> Vec<String> {
        vec![
            TechnologyArtworkParams::decl(),
            TechnologyArtworkReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.tokens.is_empty()
            || params
                .tokens
                .iter()
                .any(|token| token.is_empty() || token.contains('\0'))
        {
            return Err("Invalid technology artwork tokens");
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
        let state = source.state::<State>();
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        let art = artwork::ensure(
            &mut state.artwork,
            source,
            context.content()?,
            artwork::Kind::Technology,
        )?;
        let (items, warnings) = art.catalogue(
            params
                .tokens
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            cancelled,
        )?;
        Ok(TechnologyArtworkReply { items, warnings })
    }
}
