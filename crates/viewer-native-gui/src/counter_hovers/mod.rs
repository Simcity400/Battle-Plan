pub mod gui_authored;
pub mod land;
pub mod task_force;
pub mod task_force_port;
pub mod unit_hits;
use viewer_foundation::tooltip_text::Tooltip;

pub fn refuse(out: &mut Tooltip, line: &str, reason: impl Into<String>) {
    *out.refused
        .entry(format!("counter {line}: {}", reason.into()))
        .or_default() += 1;
}
