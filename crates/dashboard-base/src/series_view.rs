//! A time-series view: the hours a chart shows and its plot width. A reply holds the samples
//! it shows, and where they are denser than its width, each line as a summary on the shared
//! chart grid ([`crate::prepared::series::summary_bins`]) that draws every sample exactly.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SeriesView {
    pub from: f64,
    pub to: f64,
    /// The chart's plot width in pixels.
    pub width: u32,
}

impl SeriesView {
    pub fn valid(&self) -> bool {
        self.from.is_finite() && self.to.is_finite() && self.from <= self.to && self.width > 0
    }

    /// The whole hours it shows.
    pub fn hours(&self) -> (i32, i32) {
        let hour = |value: f64| value.floor().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
        (hour(self.from), hour(self.to))
    }

    /// Whether a sample holding `first..=last` shows in it.
    pub fn shows(&self, first: i32, last: i32) -> bool {
        let (from, to) = self.hours();
        last >= from && first <= to
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_view_shows_the_samples_that_overlap_its_hours() {
        let view = SeriesView { from: 10.5, to: 20.0, width: 4 };
        assert!(view.valid());
        assert_eq!(view.hours(), (10, 20));
        assert!(view.shows(0, 10) && view.shows(20, 30) && view.shows(12, 14));
        assert!(!view.shows(0, 9) && !view.shows(21, 30));
        assert!(!SeriesView { from: 2.0, to: 1.0, width: 4 }.valid());
        assert!(!SeriesView { from: 1.0, to: 2.0, width: 0 }.valid());
    }
}
