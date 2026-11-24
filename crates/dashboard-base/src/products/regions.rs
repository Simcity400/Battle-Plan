use serde::Serialize;
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
#[doc = " Why a region reply lists no region at the hour."]
pub enum NoRegions {
    #[doc = " Regions were active, but this widget's own settings leave every one out: `hidden` the user removed, `outside` beyond the chosen theatres."]
    Filtered { hidden: u32, outside: u32 },
    #[doc = " Whether any region was active is not known at the hour."]
    Unread { reason: String },
    #[doc = " The recording shows no active region at the hour."]
    None,
}
