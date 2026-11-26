
use super::{EventFeedKind, EventFeedRow, row::FeedEvent};
use serde::{Deserialize, Serialize};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum List {
    EventChoices,
    Decisions,
}

impl List {
    pub fn openers(self) -> &'static [EventFeedKind] {
        use EventFeedKind::*;
        match self {
            Self::EventChoices => &[ScriptEventFired, NewsEventFired],
            Self::Decisions => &[DecisionSelected, DecisionActivated],
        }
    }

    pub fn continuing(self) -> &'static [EventFeedKind] {
        use EventFeedKind::*;
        match self {
            Self::EventChoices => &[EventOptionSelected],
            Self::Decisions => &[
                DecisionActivated,
                DecisionDurationChanged,
                DecisionRemovalEffect,
                DecisionCancellationEffect,
                DecisionCompletionEffect,
                DecisionTimeoutEffect,
                DecisionCompleted,
                DecisionCancelled,
                DecisionTimedOut,
                DecisionRemoved,
            ],
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ScriptCurrency {
    PoliticalPower,
    CommandPower,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ScriptCost {
    pub currency: ScriptCurrency,
    #[doc = " The exact fixed5 balance change as a decimal string; negative when spent."]
    pub amount: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ScriptEdge {
    pub row: EventFeedRow,
    #[doc = " The executed option's signed native ordinal."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional = nullable))]
    pub option: Option<i32>,
    #[doc = " The writer's lifecycle result, where its boundary supplies one."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional = nullable))]
    pub lifecycle: Option<i32>,
    #[doc = " The writer's duration result in days."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional = nullable))]
    pub days: Option<i32>,
    #[doc = " The power the execution was proven to charge; absent when not proven."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional = nullable))]
    pub costs: Option<Vec<ScriptCost>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ScriptOccurrence {
    pub id: String,
    #[doc = " Whether the first edge opened the card; false for a continuing row whose opening was not"]
    #[doc = " recorded or joined."]
    pub opened: bool,
    pub edges: Vec<ScriptEdge>,
}

pub(crate) fn edge(event: &FeedEvent, row: EventFeedRow) -> ScriptEdge {
    let costs = event.costs.and_then(|[political, command]| {
        (political.is_some() || command.is_some()).then(|| {
            [(ScriptCurrency::PoliticalPower, political), (ScriptCurrency::CommandPower, command)]
                .into_iter()
                .filter_map(|(currency, amount)| amount.filter(|amount| *amount != 0).map(|amount| ScriptCost { currency, amount: amount.to_string() }))
                .collect()
        })
    });
    ScriptEdge { row, option: event.option, lifecycle: event.lifecycle, days: event.days, costs }
}
