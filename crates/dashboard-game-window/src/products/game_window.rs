//! `game-window`: one of the game's own windows for an entity at a replay hour, laid out from
//! the recording's content stack and drawn read-only ([`crate::scene::Scene`]).
//!
//! Each window has a binder that reads the hour's recorded state and makes the calls the
//! game's code makes on the window. Slots the binder cannot fill from the recording are named
//! in [`GameWindowReply::slots`]; their controls keep what the game's construction leaves, or
//! are hidden where the authored text is a placeholder, never filled with an invented value.

use std::{
    collections::BTreeMap,
    io,
    sync::atomic::{AtomicBool, Ordering},
};

use dashboard_base::{
    Source, catalogue,
    metadata::check_cancel,
    products::{ContentOptions, LoadContext, Product, check_reply_budget},
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{invalid, scene::Scene, session};

pub struct GameWindow;

/// Which window, for which entity.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum GameWindowTarget {
    /// The policy chooser of a military-industrial organisation, by its reference
    /// (`IndustrialOrganisations.organisation`).
    #[serde(rename_all = "camelCase")]
    MioPolicies { organisation: String },
    /// The division designer of a template, by its full handle (`DivisionTemplates.template`),
    /// as its roster owner sees it.
    #[serde(rename_all = "camelCase")]
    DivisionDesigner { template: String },
}

/// A list's scroll offset in interface pixels, by its element path ([`crate::scene::SceneList`]).
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListScroll {
    pub path: String,
    pub offset: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GameWindowParams {
    pub branch_id: String,
    pub hour: i32,
    pub window: GameWindowTarget,
    #[serde(default)]
    #[ts(optional)]
    pub scroll: Option<Vec<ListScroll>>,
}

/// Whether a window's slot shows recorded state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SlotState {
    /// Filled from the recording.
    Bound,
    /// Not filled: the recording or the dashboard does not supply it yet.
    Unbound,
    /// Not filled because its inputs refused, for the reason given.
    Refused,
}

/// A named part of a window and whether it shows recorded state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WindowSlot {
    pub name: String,
    pub state: SlotState,
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GameWindowReply {
    pub warnings: Vec<String>,
    /// The window's title as plain text, for the app's own heading and accessibility.
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The hour the bound state was read at, when it was sampled before the requested hour.
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_hour: Option<i32>,
    /// The drawn window; absent when it is refused.
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scene: Option<Scene>,
    /// Why the window is not drawn, each reason counted.
    pub refused: Vec<String>,
    pub slots: Vec<WindowSlot>,
}

/// What a binder answers: the scene or why it is refused, and its slots.
pub struct Bound {
    pub title: Option<String>,
    pub read_hour: Option<i32>,
    pub scene: Result<crate::compose::Laid, Vec<String>>,
    pub slots: Vec<WindowSlot>,
    pub warnings: Vec<String>,
}

impl Product for GameWindow {
    const KEY: &'static str = "game-window";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = GameWindowParams;
    type Reply = GameWindowReply;

    fn types() -> Vec<String> {
        vec![
            GameWindowParams::decl(),
            GameWindowTarget::decl(),
            ListScroll::decl(),
            GameWindowReply::decl(),
            SlotState::decl(),
            WindowSlot::decl(),
            Scene::decl(),
            crate::scene::SceneHover::decl(),
            crate::scene::SceneList::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid game window branch");
        }
        if params.hour < 0 {
            return Err("Invalid game window hour");
        }
        match &params.window {
            GameWindowTarget::MioPolicies { organisation } => {
                if organisation.parse::<u64>().map_or(true, |id| id == 0) {
                    return Err("Invalid military-industrial organisation");
                }
            }
            GameWindowTarget::DivisionDesigner { template } => {
                if template.parse::<u64>().map_or(true, |id| id == 0) {
                    return Err("Invalid division template");
                }
            }
        }
        if params
            .scroll
            .iter()
            .flatten()
            .any(|scroll| scroll.path.is_empty() || scroll.offset < 0)
        {
            return Err("Invalid game window scroll");
        }
        Ok(())
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        check_cancel(cancelled)?;
        let options = context.content()?;
        let (catalogue, shared) = open(source, options, session::Opener::Request, cancelled)?;
        let shared = shared.expect("a request always has its session");
        let branch = params.branch_id.parse().expect("validated branch");
        let scroll: BTreeMap<String, i32> = params
            .scroll
            .iter()
            .flatten()
            .map(|scroll| (scroll.path.clone(), scroll.offset))
            .collect();
        let bound = match &params.window {
            GameWindowTarget::MioPolicies { organisation } => crate::mio::bind(
                source,
                &catalogue,
                &shared,
                branch,
                params.hour,
                organisation.parse().expect("validated organisation"),
                &scroll,
                cancelled,
            )?,
            GameWindowTarget::DivisionDesigner { template } => crate::designer::bind(
                source,
                &catalogue,
                &shared,
                branch,
                params.hour,
                template.parse().expect("validated template"),
                nato(options)?,
                &scroll,
                cancelled,
            )?,
        };
        let mut session = shared.lock().unwrap_or_else(|error| error.into_inner());
        let session = &mut *session;
        let painted = bound.scene.and_then(|laid| {
            laid.paint(crate::compose::Canvas {
                sheets: &session.gui,
                pages: &session.images,
                painter: &mut session.painter,
            })
        });
        let (scene, refused) = match painted {
            Ok(scene) => (Some(scene), Vec::new()),
            Err(refused) => (None, refused),
        };
        let mut warnings = bound.warnings;
        warnings.extend(session.notes.iter().cloned());
        warnings.extend(session.gui.uncovered_note("Game window"));
        let reply = GameWindowReply {
            warnings,
            title: bound.title,
            read_hour: bound.read_hour,
            scene,
            refused,
            slots: bound.slots,
        };
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

/// Whether the viewer's game shows NATO unit symbols (its `settings.txt`), which selects the
/// unit glyphs' frame.
fn nato(options: &ContentOptions) -> io::Result<bool> {
    let keys = installed_content_settings::settings::read_keys(&options.options.user_dir)?;
    installed_content_settings::settings::Counters::read(&keys)
        .map(|counters| counters.nato == 1)
        .map_err(invalid)
}

/// The stack identity of the content `options` bind: their folders and presentation.
fn stack(options: &ContentOptions) -> String {
    format!("{:?}", catalogue::binding(options))
}

/// The source's catalogue and the artwork session its windows draw from, opened on first use;
/// the session is `None` when `opener` is the read-ahead and a request has since read other
/// content ([`session::session`]).
#[allow(clippy::type_complexity)]
pub fn open(
    source: &Source,
    options: &ContentOptions,
    opener: session::Opener,
    cancelled: &AtomicBool,
) -> io::Result<(
    std::sync::Arc<catalogue::Catalogue>,
    Option<std::sync::Arc<std::sync::Mutex<session::Session>>>,
)> {
    let catalogue = catalogue::catalogue(source, options)?;
    let shared = session_of(source, options, &catalogue, opener, Some(cancelled))?;
    Ok((catalogue, shared))
}

/// The artwork session of `catalogue`'s stack ([`open`]).
fn session_of(
    source: &Source,
    options: &ContentOptions,
    catalogue: &catalogue::Catalogue,
    opener: session::Opener,
    cancelled: Option<&AtomicBool>,
) -> io::Result<Option<std::sync::Arc<std::sync::Mutex<session::Session>>>> {
    let viewport = session::viewport(&options.options.user_dir).map_err(|why| {
        invalid(format!(
            "Game windows need the game's interface size: {why}"
        ))
    })?;
    let decode = catalogue.gui_decode.clone().map_err(|why| {
        invalid(format!(
            "Game windows need the game's texture settings: {why}"
        ))
    })?;
    // A request admitted under content the source's latest work is no longer bound to (other
    // folders, another unverified-content choice or other visual mods), or which was
    // cancelled, answers alone; this is decided under the sessions' lock.
    session::session(
        &source.state::<session::Sessions>(),
        &stack(options),
        &catalogue.content.layers,
        &catalogue.localisation.language,
        decode,
        viewport,
        opener,
        &|| still_current(source, options, cancelled),
    )
}

/// Whether a request admitted under `options` still holds what its source's latest work is
/// bound to (`Source::still_admitted`), and was not cancelled.
pub(crate) fn still_current(
    source: &Source,
    options: &ContentOptions,
    cancelled: Option<&AtomicBool>,
) -> bool {
    source.still_admitted(options)
        && !cancelled.is_some_and(|cancelled| cancelled.load(Ordering::Acquire))
}

/// Whether a source's read-ahead has been started: one per source, however often it opens.
#[derive(Default)]
struct ReadAhead(bool);

/// Marks the read-ahead started; false when it already was.
fn claim(state: &std::sync::Mutex<ReadAhead>) -> bool {
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    !std::mem::replace(&mut state.0, true)
}

/// Reads ahead what a source's first window reads, on a thread of its own below the priority of
/// the work the user waits for ([`job_priority`]): the catalogue, the artwork session (every
/// template and resource registry of the stack) and the policies' definitions, each the step a
/// window takes, so it shares their caches. One runs per source. It holds the source only while
/// a step runs: once the source is dropped, or a window's request has read other content, no
/// further step starts, and its session is never published over a newer request's. A failure
/// is left for the window to report.
pub fn warm_in_background(source: &std::sync::Arc<Source>, mut options: ContentOptions) {
    if !claim(&source.state::<ReadAhead>()) {
        return;
    }
    let stack = stack(&options);
    let source = std::sync::Arc::downgrade(source);
    let _ = std::thread::Builder::new()
        .name("game window read-ahead".into())
        .spawn(move || {
            let ahead = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
            job_priority::JobPriority::run(ahead, move || {
                // The read-ahead's entry: its mount is resolved before any stage takes a lock.
                if let Some(source) = source.upgrade() {
                    source.resolve_bound(&mut options);
                }
                // Every stage reads the catalogue only over content already bound for these
                // folders ([`catalogue::bound_catalogue`]): an obsolete read-ahead binds
                // nothing a reply would report.
                let bound = |source: &Source| catalogue::bound_catalogue(source, &options)?.ok();
                let catalogue = |source: &Source| bound(source).is_some();
                let session = |source: &Source| {
                    bound(source).is_some_and(|catalogue| {
                        session_of(
                            source,
                            &options,
                            &catalogue,
                            session::Opener::ReadAhead,
                            None,
                        )
                        .is_ok_and(|session| session.is_some())
                    })
                };
                let policies = |source: &Source| {
                    bound(source).is_some_and(|catalogue| {
                        crate::mio::warm(&catalogue.content.layers);
                        true
                    })
                };
                let steps: [&dyn Fn(&Source) -> bool; 3] = [&catalogue, &session, &policies];
                stages(&steps, || {
                    source.upgrade().filter(|source| {
                        session::is_current(&source.state::<session::Sessions>(), &stack)
                    })
                });
            });
        });
}

/// Runs each stage on what `live` gives while it gives one, stopping at the first that fails.
/// `live` is asked before each stage and its answer dropped after it, so a stage never starts
/// for what was dropped meanwhile.
pub(crate) fn stages<T>(
    stages: &[&dyn Fn(&T) -> bool],
    live: impl Fn() -> Option<std::sync::Arc<T>>,
) {
    for stage in stages {
        let Some(held) = live() else {
            return;
        };
        if !stage(&held) {
            return;
        }
    }
}

#[cfg(test)]
mod read_ahead_tests {
    use std::sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    /// A read-ahead starts once per source.
    #[test]
    fn a_source_reads_ahead_once() {
        let state = Mutex::new(super::ReadAhead::default());
        assert!(super::claim(&state));
        assert!(!super::claim(&state));
    }

    /// Once what the stages read is dropped during a stage, no later stage starts; the stages
    /// hold it only while one runs.
    #[test]
    fn no_stage_starts_for_a_dropped_source() {
        let source = Arc::new(());
        let weak = Arc::downgrade(&source);
        let (entered, release) = (Arc::new(Barrier::new(2)), Arc::new(Barrier::new(2)));
        let later = Arc::new(AtomicUsize::new(0));
        let runner = {
            let (entered, release, later) = (
                Arc::clone(&entered),
                Arc::clone(&release),
                Arc::clone(&later),
            );
            std::thread::spawn(move || {
                let first = |_: &()| {
                    entered.wait();
                    release.wait();
                    true
                };
                let second = |_: &()| {
                    later.fetch_add(1, Ordering::SeqCst);
                    true
                };
                let steps: [&dyn Fn(&()) -> bool; 2] = [&first, &second];
                super::stages(&steps, || weak.upgrade());
            })
        };
        entered.wait();
        drop(source);
        release.wait();
        runner.join().unwrap();
        assert_eq!(later.load(Ordering::SeqCst), 0);
        // While it lives, every stage runs.
        let source = Arc::new(());
        let weak = Arc::downgrade(&source);
        let ran = AtomicUsize::new(0);
        let stage = |_: &()| ran.fetch_add(1, Ordering::SeqCst) < 10;
        let steps: [&dyn Fn(&()) -> bool; 3] = [&stage, &stage, &stage];
        super::stages(&steps, || weak.upgrade());
        assert_eq!(ran.load(Ordering::SeqCst), 3);
    }
}
