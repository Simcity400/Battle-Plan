//! Concrete drawn counter and selection observations shared by producers and hover text.
pub mod gui_authored;
pub mod land;
pub mod task_force;
pub mod task_force_port;
pub mod unit_hits;
use crate::tooltip_text::Tooltip;

pub fn refuse(out: &mut Tooltip, line: &str, reason: impl Into<String>) {
    *out.refused
        .entry(format!("counter {line}: {}", reason.into()))
        .or_default() += 1;
}
