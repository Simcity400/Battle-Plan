//! What an upload left resident on the GPU, and the check that it is what a whole rebuild would
//! leave (`BATTLE_PLAN_VIEWER_CHECK_OVERLAY`, every Nth upload).
//!
//! The pass keeps windows' bytes between frames, so a quad comparison is no longer enough: a
//! transform written into the wrong range, or not written at all, leaves the quads equal and the
//! pixels wrong. The record is therefore the bytes every draw reads, in the order the frame
//! encodes them, with the window each belongs to, and the runs that encode them. A difference
//! names the window and the range.
//!
//! A frame in which nothing the batch draws changed must write no bytes at all; that is checked
//! on every upload, not only a checked one.

use super::{BatchWindow, WindowKey};
use std::sync::Mutex;

/// How many uploads apart the check runs, from `BATTLE_PLAN_VIEWER_CHECK_OVERLAY`: its value when that
/// is a number, else every upload of a gate run.
pub(super) fn every() -> Option<u64> {
    static EVERY: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *EVERY.get_or_init(|| {
        let value = std::env::var_os("BATTLE_PLAN_VIEWER_CHECK_OVERLAY").or_else(|| std::env::var_os("HOI4_VIEWER_CHECK_OVERLAY"))?;
        Some(
            value
                .to_str()
                .and_then(|value| value.trim().parse::<u64>().ok())
                .filter(|every| *every > 0)
                .unwrap_or(1),
        )
    })
}

static FOUND: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// What the check found since the last call. The frame fails on anything here.
pub fn failures() -> Vec<String> {
    std::mem::take(&mut *FOUND.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
}

fn fail(message: String) {
    eprintln!("CHECK sprite resident DIFFERS: {message}");
    FOUND
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(message);
}

/// One run of the encoded draw list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Run {
    pub native: Option<usize>,
    pub composite: Option<usize>,
    pub texture: usize,
    pub scissor: Option<[u32; 4]>,
    pub range: std::ops::Range<u32>,
}

/// One draw an upload prepared, with the bytes it reads.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Draw {
    /// A snapped quad, as its six vertices were written.
    Plain {
        window: Option<WindowKey>,
        page: Option<usize>,
        texture: usize,
        scissor: Option<[u32; 4]>,
        vertices: Vec<u8>,
    },
    /// A compiled GUI draw: buffer zero as it binds it, which is where a window's transform
    /// stands, then its animation records, its geometry, its textures and its scissor.
    Native {
        window: Option<WindowKey>,
        page: Option<usize>,
        path: String,
        constants: Vec<u8>,
        extra: Vec<(u32, Vec<u8>)>,
        vertices: Vec<u8>,
        textures: Vec<(u32, Option<usize>)>,
        scissor: Option<[u32; 4]>,
        batched: bool,
    },
    /// A fading icon's page cell composited over the frame.
    Composite { page: usize, vertices: Vec<u8> },
}

impl Draw {
    fn window(&self) -> Option<WindowKey> {
        match self {
            Self::Plain { window, .. } | Self::Native { window, .. } => *window,
            Self::Composite { .. } => None,
        }
    }

    fn name(&self) -> String {
        let window = match self.window() {
            Some(key) => format!("{key}"),
            None => "no window".to_owned(),
        };
        match self {
            Self::Plain { texture, .. } => format!("snapped quad of texture {texture} ({window})"),
            Self::Native { path, .. } => format!("GUI draw {path:?} ({window})"),
            Self::Composite { page, .. } => format!("fade composite of page {page}"),
        }
    }
}

/// What one upload left resident.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Record {
    pub draws: Vec<Draw>,
    pub runs: Vec<Run>,
    pub pages: Vec<Vec<Run>>,
    pub extents: Vec<[u32; 2]>,
}

impl Record {
    pub(super) fn clear(&mut self) {
        self.draws.clear();
        self.runs.clear();
        self.pages.clear();
        self.extents.clear();
    }
}

/// The first way `held` is not what `fresh` holds, named by window and range.
pub(super) fn difference(held: &Record, fresh: &Record) -> Option<String> {
    for (at, (a, b)) in held.draws.iter().zip(&fresh.draws).enumerate() {
        if a == b {
            continue;
        }
        if let (
            Draw::Native {
                constants: held_constants,
                extra: held_extra,
                ..
            },
            Draw::Native {
                constants: fresh_constants,
                extra: fresh_extra,
                ..
            },
        ) = (a, b)
            && (held_constants != fresh_constants || held_extra != fresh_extra)
        {
            let byte = held_constants
                .iter()
                .zip(fresh_constants)
                .position(|(a, b)| a != b);
            return Some(format!(
                "draw {at}, {}: its constants are resident as {held_constants:?} where a whole \
                 rebuild writes {fresh_constants:?}{}",
                a.name(),
                match byte {
                    Some(byte) if byte < 64 =>
                        format!(" (byte {byte}, inside the window's transform)"),
                    Some(byte) => format!(" (byte {byte})"),
                    None => String::new(),
                }
            ));
        }
        return Some(format!(
            "draw {at} is resident as {} where a whole rebuild draws {}",
            a.name(),
            b.name()
        ));
    }
    if held.draws.len() != fresh.draws.len() {
        let (longer, shorter) = if held.draws.len() > fresh.draws.len() {
            (held, fresh)
        } else {
            (fresh, held)
        };
        return Some(format!(
            "{} draws are resident where a whole rebuild makes {}; the first unmatched is {}",
            held.draws.len(),
            fresh.draws.len(),
            longer.draws[shorter.draws.len()].name()
        ));
    }
    if held.runs != fresh.runs {
        let at = held
            .runs
            .iter()
            .zip(&fresh.runs)
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| held.runs.len().min(fresh.runs.len()));
        return Some(format!(
            "the draw list differs at run {at}: {} runs resident, {} rebuilt",
            held.runs.len(),
            fresh.runs.len()
        ));
    }
    if held.extents != fresh.extents || held.pages != fresh.pages {
        return Some(format!(
            "the fade pages differ: {:?} resident, {:?} rebuilt",
            held.extents, fresh.extents
        ));
    }
    None
}

/// A gate run's line for a check that found nothing, so a run can be seen to have checked.
pub(super) fn note_equal(record: &Record, written: u64) {
    eprintln!(
        "CHECK sprite resident equal: {} draws, {} runs, {} fade pages, {written} bytes written",
        record.draws.len(),
        record.runs.len(),
        record.extents.len()
    );
}

/// A frame in which no window and no loose quad changed must write nothing.
pub(super) fn check_quiet(written: u64) {
    if written != 0 {
        fail(format!(
            "{written} bytes were written on a frame in which nothing the batch draws changed"
        ));
    }
}

pub(super) fn check(held: &Record, fresh: &Record, written: u64) {
    match difference(held, fresh) {
        Some(found) => fail(found),
        None => note_equal(fresh, written),
    }
}

/// Which window each quad of a batch belongs to, by its key.
pub(super) fn owners(windows: &[BatchWindow], quads: usize) -> Vec<Option<WindowKey>> {
    let mut owners = vec![None; quads];
    for window in windows {
        for owner in &mut owners[window.quads()] {
            *owner = Some(window.key);
        }
    }
    owners
}
