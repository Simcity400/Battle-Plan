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

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
#[doc = " Regions active at the hour that a reply does not list: past its top, or removed by the user."]
pub struct ActiveRegions {
    pub ids: Vec<u32>,
    #[doc = " Why the active regions at the hour are not all known; `ids` then holds only the witnessed ones."]
    pub missing: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Automatic {
    pub listed: Vec<u32>,
    pub active_unlisted: ActiveRegions,
    pub hidden: u32,
}

pub fn automatic(ranked: impl IntoIterator<Item = u32>, hidden: &std::collections::HashSet<u32>, top: usize, missing: Option<String>) -> Automatic {
    let mut out = Automatic { active_unlisted: ActiveRegions { ids: Vec::new(), missing }, ..Automatic::default() };
    for id in ranked {
        if hidden.contains(&id) {
            out.hidden += 1;
            out.active_unlisted.ids.push(id);
        } else if out.listed.len() < top {
            out.listed.push(id);
        } else {
            out.active_unlisted.ids.push(id);
        }
    }
    out
}

pub fn no_regions(listed: bool, missing: Option<&str>, hidden: u32, outside: u32) -> Option<NoRegions> {
    if listed {
        return None;
    }
    Some(match missing {
        Some(reason) => NoRegions::Unread { reason: reason.to_owned() },
        None if hidden + outside > 0 => NoRegions::Filtered { hidden, outside },
        None => NoRegions::None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_automatic_set_lists_the_top_and_answers_the_rest_of_the_active_regions_by_id() {
        let chosen = automatic([5, 3, 9, 1], &[3].into(), 2, None);
        assert_eq!(chosen.listed, [5, 9], "a removed region gives its place to the next");
        assert_eq!(chosen.active_unlisted.ids, [3, 1], "removed and past the top: still active now");
        assert_eq!(no_regions(false, None, chosen.hidden, 0), Some(NoRegions::Filtered { hidden: 1, outside: 0 }));
        assert_eq!(no_regions(false, Some("unread"), 1, 0), Some(NoRegions::Unread { reason: "unread".into() }), "an unknown hour is said before what was removed");
        assert_eq!(no_regions(false, None, 0, 0), Some(NoRegions::None));
        assert_eq!(no_regions(true, Some("unread"), 1, 0), None);
    }
}
