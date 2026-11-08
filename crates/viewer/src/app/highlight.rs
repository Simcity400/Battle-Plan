//! What the drawn frame highlights and which map tooltip it painted; provinces take no highlight.

use super::Viewer;
use std::fmt;
use viewer_foundation::unit_handle::Unit;

/// What the pointer highlights in the frame drawn, displayed as `<kind>:<id>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HoverTarget {
    Counter(Unit),
    /// A map GUI button by its element path.
    Button(String),
}

impl fmt::Display for HoverTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Counter(Unit::Division(id)) => write!(f, "division:{id}"),
            Self::Counter(Unit::RailwayGun(id)) => write!(f, "railway_gun:{id}"),
            Self::Counter(Unit::TaskForce(id)) => write!(f, "task_force:{id}"),
            Self::Button(path) => write!(f, "button:{path}"),
        }
    }
}

/// The map hover whose tooltip the drawn frame shows, displayed as `<kind>:<id>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TooltipTarget {
    Province(u32),
    Counter(Unit),
}

impl fmt::Display for TooltipTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Province(id) => write!(f, "province:{id}"),
            Self::Counter(unit) => HoverTarget::Counter(*unit).fmt(f),
        }
    }
}

impl Viewer {
    /// The province or counter whose map tooltip the last frame painted.
    pub(crate) fn shown_tooltip(&self) -> Option<TooltipTarget> {
        self.shown_tooltip.clone()
    }

    /// The element the last frame drew highlighted, topmost first.
    pub(crate) fn highlighted(&self) -> Option<HoverTarget> {
        let picker = (!self.picker.hidden)
            .then(|| self.picker.picker.highlighted())
            .flatten();
        picker
            .or_else(|| self.combat.highlighted())
            .map(|path| HoverTarget::Button(path.to_owned()))
            .or_else(|| self.counter_report.pointed.map(HoverTarget::Counter))
    }
}
