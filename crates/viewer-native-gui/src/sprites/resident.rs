
use super::{BatchWindow, WindowKey};
use std::sync::Mutex;

pub(super) fn every() -> Option<u64> {
    static EVERY: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *EVERY.get_or_init(|| {
        let value = std::env::var_os("BATTLEPLAN_VIEWER_CHECK_OVERLAY").or_else(|| std::env::var_os("HOI4_VIEWER_CHECK_OVERLAY"))?;
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Run {
    pub native: Option<usize>,
    pub composite: Option<usize>,
    pub texture: usize,
    pub scissor: Option<[u32; 4]>,
    pub range: std::ops::Range<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Draw {
    Plain {
        window: Option<WindowKey>,
        page: Option<usize>,
        texture: usize,
        scissor: Option<[u32; 4]>,
        vertices: Vec<u8>,
    },
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

pub(super) fn note_equal(record: &Record, written: u64) {
    eprintln!(
        "CHECK sprite resident equal: {} draws, {} runs, {} fade pages, {written} bytes written",
        record.draws.len(),
        record.runs.len(),
        record.extents.len()
    );
}

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

pub(super) fn owners(windows: &[BatchWindow], quads: usize) -> Vec<Option<WindowKey>> {
    let mut owners = vec![None; quads];
    for window in windows {
        for owner in &mut owners[window.quads()] {
            *owner = Some(window.key);
        }
    }
    owners
}
