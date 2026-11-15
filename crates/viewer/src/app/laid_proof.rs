//! The equality gate of the overlay laid out on its tables' worker (`--laid-proof N` on a paced
//! `--playback-cost` run): an overlay the worker laid out and the frame installs is compared, quad
//! for quad with the clock set aside, with the same hour laid out in place on the frame thread as
//! a frame without the worker lays it out, from the same rows, tables, camera, view and counter
//! layers' products. Both are drawn on presentations of their own, with no window history and no
//! unit animation, so a difference is a difference of the layouts.

use super::OverlayKey;
use crate::document::{Hour, LeftToLayers, OverlayLayers, OverlayRows, OverlayTables, PreparedOverlay};
use crate::map_views::{CounterView, Presentation};
use replay_frame::frame::DecodedFrame;
use serde_json::json;
use std::sync::Arc;
use viewer_counters::map::counters::CounterReport;
use viewer_foundation::map::sprites::{Batch, Quad};

/// The gate's tally over a run.
#[derive(Debug)]
pub(crate) struct LaidProof {
    every: u32,
    installs: u32,
    /// The hour of the last installed layout.
    last: Option<i32>,
    compared: u32,
    /// Layouts not compared: the lazily decoded art changed between the worker's layout and the
    /// gate's, so the two would draw a flag or portrait at two versions (the frame lays such a
    /// layout out again itself, [`PreparedOverlay::is_stale`]).
    art_moved: u32,
    differing: Vec<String>,
}

/// What one installed layout is compared with.
pub(super) struct Check<'a> {
    pub content: &'a crate::document::Content,
    pub rosters: Arc<crate::playback::AirRosters>,
    pub rows: &'a Arc<DecodedFrame>,
    pub left: LeftToLayers,
    /// The map's layers' products the worker laid in.
    pub lay_ins: &'a super::LayIns,
    pub tables: &'a OverlayTables,
    pub key: &'a OverlayKey,
    /// The hour at the head when the frame installs it.
    pub frame_hour: i32,
    /// The view's counter layers' products when the worker was asked.
    pub asked: Option<&'a Arc<Presentation>>,
    pub view: &'a CounterView,
    pub layers: &'a OverlayLayers,
    pub time: f32,
}

impl LaidProof {
    pub(crate) fn new(every: u32) -> Self {
        Self { every: every.max(1), installs: 0, last: None, compared: 0, art_moved: 0, differing: Vec::new() }
    }

    /// Whether the layout of `hour` the frame installs now is compared: the first, every `every`th,
    /// and any whose hour does not follow the last installed (a seek, a jump of the pace).
    pub(super) fn due(&mut self, hour: i32) -> bool {
        self.installs += 1;
        let jump = self.last.is_none_or(|last| hour != last && hour != last + 1);
        self.last = Some(hour);
        jump || self.installs % self.every == 0
    }

    /// Compares `laid`, about to be installed, with the same hour laid out in place.
    pub(super) fn check(&mut self, laid: &mut PreparedOverlay, check: Check<'_>) {
        let hour = Hour::new(check.content, check.rows, 0.0).with_air_rosters(Some(Arc::clone(&check.rosters)));
        if laid.is_stale(&hour) {
            self.art_moved += 1;
            return;
        }
        let mut found = Vec::new();
        Self::compare(laid, check, &hour, &mut found);
        // A texture the worker's layout queued may decode while the gate lays the hour out
        // again: its version moved during the comparison, which is not a difference of layouts.
        if laid.is_stale(&hour) {
            self.art_moved += 1;
            return;
        }
        self.compared += 1;
        self.differing.extend(found);
    }

    /// The two layouts compared; what differs in `found`.
    fn compare(laid: &mut PreparedOverlay, check: Check<'_>, hour: &Hour<'_>, found: &mut Vec<String>) {
        let Check { rows, left, lay_ins, tables, key, frame_hour, asked, view, layers, time, .. } = check;
        let hours = format!("the layout stands for hour {} (rows of hour {}), the frame's head is at hour {frame_hour}", key.hour, rows.hour);
        let Some(asked) = asked else {
            found.push(format!("the worker was asked before the gate held the view's products: {hours}"));
            return;
        };
        let lent = asked.counters.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).layered().identity();
        if lent != key.layered {
            found.push(format!("the worker was lent counter layers' products {lent:?} while its key names {:?}: {hours}", key.layered));
        }
        let scratch = |products: Option<&Presentation>| {
            let presentation = view.presentation.snapshot();
            if let Some(from) = products {
                from.lend_layered(&presentation);
            }
            CounterView { presentation: Arc::new(presentation), ..view.clone() }
        };
        // In place, as the frame thread lays an hour out without the worker: the view's products
        // when the worker was asked, on a memory of its own.
        let read = OverlayRows::at_head(Some(Arc::clone(rows))).leaving(left);
        let fresh = hour.prepare_overlay_at(key.camera, key.pixels, key.ui_scale, tables, &scratch(Some(asked)), &Default::default(), &read).map(|mut fresh| {
            lay_ins.lay(&mut fresh, hour, key.camera, key.pixels, key.ui_scale, tables.ours());
            fresh
        });
        let draw = |prepared: &mut PreparedOverlay| {
            let (mut batch, mut report) = (Batch::default(), CounterReport::default());
            prepared
                .draw_into(hour, key.camera, key.pixels, key.ui_scale, layers, time, &Default::default(), &mut batch, &mut report)
                .map(|()| (batch, report, prepared.label_report().clone()))
        };
        laid.set_counter_view(scratch(None));
        let laid_drawn = draw(laid);
        laid.set_counter_view(view.clone());
        let missed = read.missed();
        let fresh_drawn = match fresh {
            Ok(mut fresh) => draw(&mut fresh),
            Err(error) => {
                found.push(format!("the in-place layout failed: {error}: {hours}"));
                return;
            }
        };
        if !missed.is_empty() {
            found.push(format!("the in-place layout read tables its rows do not hold: {missed:?}: {hours}"));
        }
        match (laid_drawn, fresh_drawn) {
            (Ok((laid, laid_report, laid_labels)), Ok((fresh, fresh_report, fresh_labels))) => {
                if let Some(difference) = first_difference(&clockless(&laid), &clockless(&fresh)) {
                    found.push(format!("{difference}: {hours}"));
                }
                if laid_report != fresh_report {
                    found.push(format!("the counters' reports differ: {hours}"));
                }
                if laid_labels != fresh_labels {
                    found.push(format!("the labels' reports differ: {hours}"));
                }
            }
            (laid, fresh) => {
                if laid.is_err() != fresh.is_err() {
                    found.push(format!("one layout draws and the other is refused (worker: {}, in place: {}): {hours}", laid.is_ok(), fresh.is_ok()));
                }
            }
        }
    }

    /// The tally for the run's report, and the run's failure when a layout differed or none was
    /// compared.
    pub(crate) fn finish(&self) -> (serde_json::Value, Option<String>) {
        let tally = json!({
            "installs": self.installs,
            "compared": self.compared,
            "art_moved": self.art_moved,
            "differing": self.differing,
            "scope": "Each overlay the tables' worker laid out and the frame installed (the first, every Nth, and every one not of the hour after the last installed) against the same hour laid out in place on the frame thread from the same rows, tables, camera, view and counter layers' products, both drawn on presentations with no window history and no unit animation, quad for quad with each draw's Time and time since its restart set aside, with both reports.",
        });
        let failure = if self.compared == 0 {
            Some("the laid-out overlay's gate compared nothing: the worker installed no layout".to_owned())
        } else if !self.differing.is_empty() {
            Some(format!("the overlay the worker laid out differs from the one laid out in place in {} of {} compared: first, {}", self.differing.len(), self.compared, self.differing[0]))
        } else {
            None
        };
        (tally, failure)
    }
}

/// `batch` with the frame's clock set aside: each draw's `Time` constant and its time since its
/// state's restart, which only a program that has not settled reads.
pub(crate) fn clockless(batch: &Batch) -> Vec<Quad> {
    use viewer_foundation::map::native_shader::draw::SPRITE_TIME;
    let mut quads = batch.quads.clone();
    for quad in &mut quads {
        if let Some(exact) = &mut quad.exact {
            exact.inputs[0] = 0.;
            if let Some(native) = &mut exact.native {
                let native = Arc::make_mut(native);
                if let Some(time) = native.constants.get_mut(SPRITE_TIME as usize / 4) {
                    *time = 0.;
                }
            }
        }
    }
    quads
}

/// The first quad that differs, named by the window part that drew it.
pub(crate) fn first_difference(laid: &[Quad], fresh: &[Quad]) -> Option<String> {
    let name = |quad: &Quad| {
        quad.exact
            .as_ref()
            .and_then(|exact| exact.native.as_ref())
            .map_or_else(|| format!("a quad of texture {}", quad.texture), |native| native.path.clone())
    };
    if let Some(at) = laid.iter().zip(fresh).position(|(a, b)| a != b) {
        return Some(format!("quad {at} differs: {} (worker) against {} (in place)", name(&laid[at]), name(&fresh[at])));
    }
    (laid.len() != fresh.len()).then(|| {
        let longer = if laid.len() > fresh.len() { &laid[fresh.len()] } else { &fresh[laid.len()] };
        format!("the worker drew {} quads and the frame {}; the first unmatched is {}", laid.len(), fresh.len(), name(longer))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(texture: usize, layer: u64) -> Quad {
        Quad {
            texture,
            rect: [0.; 4],
            uv: [0.; 4],
            color: [1.; 4],
            rotation: 0.,
            layer,
            shade: 0.,
            exact: None,
            anchor: None,
            present: None,
        }
    }

    #[test]
    fn the_gate_names_the_first_differing_quad_and_fails_on_any_difference_or_none_compared() {
        assert_eq!(first_difference(&[quad(1, 0), quad(2, 0)], &[quad(1, 0), quad(2, 0)]), None);
        let differs = first_difference(&[quad(1, 0), quad(2, 0)], &[quad(1, 0), quad(2, 5)]).unwrap();
        assert!(differs.starts_with("quad 1 differs: a quad of texture 2"), "{differs}");
        let shorter = first_difference(&[quad(1, 0)], &[quad(1, 0), quad(3, 0)]).unwrap();
        assert!(shorter.contains("the worker drew 1 quads and the frame 2") && shorter.contains("texture 3"), "{shorter}");
        let mut proof = LaidProof::new(3);
        assert!(proof.finish().1.is_some(), "nothing compared is a failure");
        // The first, the third, and a jump of hours are due; the hour after the last is not.
        let due: Vec<bool> = [10, 11, 12, 13, 20, 21].into_iter().map(|hour| proof.due(hour)).collect();
        assert_eq!(due, [true, false, true, false, true, true]);
        proof.compared = 1;
        assert!(proof.finish().1.is_none());
        proof.differing.push("quad 0 differs".into());
        assert!(proof.finish().1.is_some());
    }
}
