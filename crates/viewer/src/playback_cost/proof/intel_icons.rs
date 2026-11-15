//! The proof tool's line for the intel icons layer (`--layers intel_icons --map-mode operatives`):
//! the icons evaluated over rows at the head against today's `intel_icons::icons` over today's frame
//! of the same hour, and against the same evaluation of the newest rows the layer holds.

use super::Compared;
use crate::{
    document::Document,
    layers::{
        Layer,
        intel_icons::{Evaluate, Icons},
    },
    playhead_host::{MapSide, mirror::Mirrors, rows::Cadence},
};
use replay_frame::frame::{DecodedFrame, TableView};
use serde_json::{Value, json};
use std::{io, sync::Arc};
use viewer_raids::map::intel_icons::{self, IntelIcons};

pub(super) fn make(document: &Document) -> Box<dyn Layer> {
    let mut layer = Icons::new(Arc::clone(&document.content) as Arc<dyn Evaluate>, None, Cadence::PerSecond(4));
    layer.drive();
    Box::new(layer)
}

fn of(layer: &dyn Layer) -> &Icons {
    layer.any().downcast_ref::<Icons>().expect("the intel icons line's layer")
}

/// The proof's playhead, which the layer reads rows of; given before every comparison.
pub(super) fn give_head(side: &MapSide, document: &Document) {
    for layer in side.parts().map_or(&[][..], |(_, layers)| layers) {
        if let Some(icons) = layer.any().downcast_ref::<Icons>() {
            icons.set_head(side.entry(), crate::playhead_host::rows::Recording { branch: document.playback.branch() });
        }
    }
}

pub(super) fn marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    let stats = of(layer).stats();
    vec![("evaluations", stats.evaluations.values().sum()), ("rows skipped", stats.skipped), ("discarded learning", stats.learning)]
}

/// The tables the rows at the head refuse because a text cell's string was never told (the known
/// recorder-side gap of these unit files).
fn untold(frame: &DecodedFrame) -> Vec<String> {
    frame.tables.iter().filter(|view| !view.is_not_declared()).filter_map(|view| match &**view {
        TableView::Unavailable { reason } if reason.contains("rows at the head could not be made") => Some(reason.clone()),
        _ => None,
    }).collect()
}

fn entries(icons: &IntelIcons) -> u64 {
    (icons.states.len() + icons.states.iter().map(|state| state.entries.len()).sum::<usize>() + icons.report.reasons.len() + 1) as u64
}

fn first_difference(today: &IntelIcons, ours: &IntelIcons) -> String {
    if today.built_for != ours.built_for {
        return format!("built for: today {:?}, the layer {:?}", today.built_for, ours.built_for);
    }
    if let Some((a, b)) = today.states.iter().zip(&ours.states).find(|(a, b)| a != b) {
        return format!("state {}: today {a:?}, the layer {b:?}", a.state);
    }
    if today.states.len() != ours.states.len() {
        return format!("states: today {}, the layer {}", today.states.len(), ours.states.len());
    }
    format!("report: today {:?}, the layer {:?}", today.report, ours.report)
}

/// The layer's icons against today's at the document's hour, for the layer's viewing choice. A
/// sample whose icons are not of the hour (a table learned joins later) is skipped, said so.
pub(super) fn today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let layer = of(layer);
    let frame = document.frame()?;
    let Some((icons, hour)) = layer.icons() else {
        return Ok(Compared { notes: json!({"skipped": "no icons yet: their tables are being learned or loading", "tables": layer.tables().len()}), ..Compared::default() });
    };
    if *hour != frame.hour {
        return Ok(Compared { notes: json!({"skipped": format!("the icons are of hour {hour}")}), ..Compared::default() });
    }
    let choice = viewer_foundation::map::perspective::Choice::default();
    let ours = document.vision(&choice)?.ours();
    let content = &document.content;
    let wanted = intel_icons::icons(&content.icons.intel, &content.icons.raids, frame, ours, &content.names.localisation, &content.raid_hover.recorded_language);
    let mut compared = Compared { entries: entries(&wanted), ..Compared::default() };
    let refused = layer.newest_rows().filter(|rows| rows.hour == *hour).map(|rows| untold(&rows)).unwrap_or_default();
    let notes = json!({"states": wanted.states.len(), "entries": wanted.report.entries, "reasons": wanted.report.reasons.len(), "tables": layer.tables().len()});
    if wanted != **icons {
        let first = first_difference(&wanted, icons);
        if refused.is_empty() {
            compared.differing = 1;
            compared.first = Some(first);
        } else {
            compared.notes = json!({"known_exception": first, "refused_at_the_head": refused, "detail": notes});
            return Ok(compared);
        }
    }
    compared.notes = notes;
    Ok(compared)
}

/// The icons drawn against the same evaluation of the newest rows the layer holds: the result
/// installed is that of the rows it last took.
pub(super) fn fresh(layer: &dyn Layer, _mirrors: &Mirrors, _upstream: &crate::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = of(layer);
    let (Some((icons, hour)), Some(rows)) = (layer.icons(), layer.newest_rows()) else {
        return Ok(Compared { notes: json!({"skipped": "no icons or no rows yet"}), ..Compared::default() });
    };
    if rows.hour != *hour {
        return Ok(Compared { notes: json!({"skipped": format!("the rows are at {}, the icons of {hour}", rows.hour)}), ..Compared::default() });
    }
    let again = layer.evaluate(&rows);
    let differing = u64::from(again != **icons);
    Ok(Compared { entries: entries(&again), differing, first: (differing > 0).then(|| first_difference(&again, icons)), notes: Value::Null })
}

/// What the layer measured and what its rows hold, for the summary.
pub(super) fn summary(side: &MapSide) -> Value {
    crate::layers::intel_icons::summary(side)
}
