use dashboard_base::products::{CountryParams, LoadContext, Product, artwork};
use super::focus;
use dashboard_core::dto::{Artwork, FocusDatabase};
use crate::products::focus_dto::FocusHistory;
use dashboard_base::Source;
use serde::Serialize;
use std::{collections::BTreeSet, io, sync::atomic::AtomicBool};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct NationalFocus;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FocusReply {
    pub history: FocusHistory,
    #[doc = " Hours of the branch the country's focus was not known, with why."]
    pub missing: Vec<focus::FocusMissing>,
    #[doc = " The national focuses' (completed and current) authored durations."]
    pub artwork: Vec<Artwork>,
    #[doc = " The national focuses' country-independent text only; scoped titles"]
    #[doc = " still require dated reads."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub static_text: Option<Vec<Artwork>>,
    #[doc = " The continuous focuses' country-independent text and authored"]
    #[doc = " durations, looked up in the continuous database."]
    pub continuous_static_text: Vec<Artwork>,
    pub warnings: Vec<String>,
}

#[derive(Default)]
pub struct State {
    national: Option<artwork::Context>,
    continuous: Option<artwork::Context>,
}

impl Product for NationalFocus {
    const KEY: &'static str = "national-focus";
    const CONTENT_DEPENDENT: bool = true;
    type Params = CountryParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country];
    type Reply = FocusReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![CountryParams::decl(), FocusReply::decl(), focus::FocusMissing::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let content = context.content()?;
        let fold = focus::fold(source, params.branch(), &params.country, cancelled)?;
        let state = source.state::<State>();
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        let State { national, continuous } = &mut *state;
        artwork::ensure(national, source, content, artwork::Kind::Focus(FocusDatabase::National))?;
        artwork::ensure(continuous, source, content, artwork::Kind::Focus(FocusDatabase::Continuous))?;
        let national = national.as_mut().expect("national focus context prepared");
        let continuous = continuous.as_mut().expect("continuous focus context prepared");
        let national_tokens = fold
            .segments
            .iter()
            .flat_map(|segment| segment.order.completed.iter().map(|entry| entry.token.as_str()).chain(segment.order.current.as_deref()))
            .collect::<BTreeSet<_>>();
        let continuous_tokens = fold.segments.iter().filter_map(|segment| segment.order.continuous.as_deref()).collect::<BTreeSet<_>>();
        let (static_text, _) = national.text_catalogue(national_tokens.clone(), cancelled)?;
        let (continuous_static_text, _) = continuous.text_catalogue(continuous_tokens, cancelled)?;
        let (artwork, warnings) = national.focus_metadata(national_tokens, cancelled)?;
        let reply = FocusReply { history: fold.history(), missing: fold.missing.clone(), artwork, static_text: Some(static_text), continuous_static_text, warnings };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}
