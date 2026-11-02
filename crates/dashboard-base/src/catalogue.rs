//! The content catalogue (`dashboard-text`) and its
//! loader: one catalogue per source, folders and presentation, held in the source's own
//! cache.

pub use dashboard_text::catalogue::*;

use crate::{
    Source, content,
    presentation::{Presentation, Presented},
    products::ContentOptions,
};
use std::{io, sync::Arc};

/// The recording's catalogue for these folders over `presented`, a mount already resolved
/// outside every cache lock: its verified content with the visual mods mounted over it, the
/// localisation in the user's game language and the client's texture settings.
fn load(
    source: &Source,
    options: &ContentOptions,
    settings: &Settings,
    presented: Arc<Presented>,
) -> io::Result<Catalogue> {
    load_with(
        source,
        options,
        settings,
        || source.content(&options.options),
        || Ok(presented),
    )
}

/// A source's catalogue slot: the shown one ([`Cached`]) or the recording's own.
trait Slot: Default + Send + 'static {
    fn slot(&mut self) -> &mut Cached;
}

impl Slot for Cached {
    fn slot(&mut self) -> &mut Cached {
        self
    }
}

/// The recording's own catalogue slot, apart so neither it nor the catalogue of the visual mods
/// requests show displaces the other.
#[derive(Default)]
struct Recorded(Cached);

impl Slot for Recorded {
    fn slot(&mut self) -> &mut Cached {
        &mut self.0
    }
}

/// Whether `options` read the recording's own catalogue: recorded-only work, or work showing
/// no visual mod, which share it ([`recorded_slot`]).
pub fn reads_recorded(options: &ContentOptions) -> bool {
    options.presentation.is_recorded() || options.presentation.identity().is_empty()
}

/// `change` run on the catalogue slot `options` read, under its lock.
fn with_slot<R>(
    source: &Source,
    options: &ContentOptions,
    change: impl FnOnce(&mut Cached) -> R,
) -> R {
    if reads_recorded(options) {
        change(
            source
                .state::<Recorded>()
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .slot(),
        )
    } else {
        change(
            &mut source
                .state::<Cached>()
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        )
    }
}

/// The catalogue the source's `S` slot keeps for `key`, loaded on first use over the mount
/// `presented` gives. The mount is resolved before the catalogue's cell is entered, and only
/// when nothing is loaded yet, so no cache lock is ever held while a mount is computed.
fn kept_in<S: Slot>(
    key: Binding,
    source: &Source,
    options: &ContentOptions,
    presented: impl FnOnce() -> io::Result<Arc<Presented>>,
) -> io::Result<Arc<Catalogue>> {
    let settings = key.3.clone();
    let result = {
        let state = source.state::<S>();
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        let slot = state.slot();
        if !slot.as_ref().is_some_and(|(cached, _)| *cached == key) {
            *slot = Some((key, Arc::new(CachedResult::default())));
        }
        Arc::clone(&slot.as_ref().expect("catalogue binding retained").1)
    };
    if let Some(loaded) = result.get() {
        return loaded;
    }
    let presented = presented();
    result.get_or_init(|| load(source, options, &settings, presented?))
}

/// The recording's own catalogue for these folders.
fn recorded_slot(source: &Source, options: &ContentOptions) -> io::Result<Arc<Catalogue>> {
    kept_in::<Recorded>(
        binding_with(options, Presentation::default()),
        source,
        options,
        || source.recorded_presented(&options.options),
    )
}

/// [`load`] over the recorded content `resolved` gives and the presentation `presented`
/// mounts.
fn load_with(
    source: &Source,
    options: &ContentOptions,
    settings: &Settings,
    resolved: impl Fn() -> io::Result<Arc<installed_content_core::content::ResolvedContent>>,
    presented: impl FnOnce() -> io::Result<Arc<Presented>>,
) -> io::Result<Catalogue> {
    let mut verification = Vec::new();
    let replay = source.replay();
    content::check_replay(&replay, options.allow_unverified, &mut verification)?;
    let language = &replay.identity.language;
    let recorded_language = language.strip_prefix("l_").unwrap_or(language).to_owned();
    let presented = presented()?;
    let mut warnings = Vec::new();
    let language = match settings {
        Ok((language, _)) => language.clone().map_err(io::Error::other)?,
        Err(why) => {
            warnings.push(format!(
                "Localisation uses the recorded language {recorded_language}: settings.txt is unavailable: {why}"
            ));
            recorded_language.clone()
        }
    };
    let localisation = content::localisation_in(
        source,
        options,
        (
            content::content_key(Some(options)).expect("content options given"),
            options.options.cache_dir.clone(),
            language,
        ),
        &resolved,
    )?;
    let gui_decode = settings
        .as_ref()
        .map(
            |&(_, force_pow2_textures)| installed_content_image::native::Decode {
                force_pow2_textures,
            },
        )
        .map_err(|why| format!("settings.txt is unavailable: {why}"));
    Catalogue::from_content(
        Arc::clone(&presented.content),
        presented.presentation.clone(),
        localisation,
        verification,
        warnings,
        recorded_language,
        gui_decode,
    )
}

/// What a catalogue, and every cache derived from one, is bound to: the folders, the
/// unverified-content choice, the cache folder, the presentation the request's artwork shows
/// (`ContentOptions::presentation`) and the game settings it reads ([`Settings`]).
pub type Binding = (
    content::ContentKey,
    std::path::PathBuf,
    Presentation,
    Settings,
);
type Cached = Option<(Binding, Arc<CachedResult<Catalogue>>)>;

/// What a catalogue reads from the user folder's `settings.txt`: the game language (or why it is
/// unusable) and `force_pow2_textures`; or why the file cannot be read.
pub type Settings = Result<(Result<String, String>, bool), String>;

/// The [`Settings`] the user folder of `options` holds now.
fn settings(options: &ContentOptions) -> Settings {
    installed_content_settings::settings::read_keys(&options.options.user_dir)
        .map(|keys| {
            (
                keys.language().map_err(|why| why.to_string()),
                keys.force_pow2_textures(),
            )
        })
        .map_err(|why| why.to_string())
}

/// The binding of the catalogue [`catalogue`] gives for these options.
pub fn binding(options: &ContentOptions) -> Binding {
    binding_with(options, options.presentation.identity().clone())
}

fn binding_with(options: &ContentOptions, presentation: Presentation) -> Binding {
    (
        content::content_key(Some(options)).expect("content options given"),
        options.options.cache_dir.clone(),
        presentation,
        settings(options),
    )
}

/// What a cache of recorded facts alone (no artwork, interface or game-look text) is bound to:
/// [`Binding`] without the presentation, which cannot change a recorded fact.
pub type RecordedBinding = (content::ContentKey, std::path::PathBuf);

pub fn recorded_binding(options: &ContentOptions) -> RecordedBinding {
    (
        content::content_key(Some(options)).expect("content options given"),
        options.options.cache_dir.clone(),
    )
}

pub fn binding_matches(key: &Binding, options: &ContentOptions) -> bool {
    *key == binding(options)
}

/// The shared catalogue for these options (folders and presentation), loaded on first use and
/// replaced when the folders, the cache folder, the unverified-content choice or the
/// presentation change. A request admitted under a presentation the source has since left
/// gets a catalogue of its own, which displaces nothing.
///
/// Recorded-only work (preparation) gets the recording's own catalogue and never mounts a
/// visual mod. A catalogue showing visual mods keeps the recording's own
/// ([`Catalogue::recorded`]), which art the mods cannot draw is drawn from ([`draw`]).
pub fn catalogue(source: &Source, options: &ContentOptions) -> io::Result<Arc<Catalogue>> {
    // Without a visual mod the recording's own catalogue is the one shown: recorded-only work
    // and requests share it, so caches built from it serve both.
    if reads_recorded(options) {
        return recorded_slot(source, options);
    }
    let shown = if source.still_admitted(options) {
        kept_in::<Cached>(binding(options), source, options, || {
            source.presented_for(options)
        })?
    } else {
        Arc::new(load(
            source,
            options,
            &settings(options),
            source.presented_for(options)?,
        )?)
    };
    keeping_recorded(source, options, shown)
}

/// `shown`, a catalogue showing visual mods, keeping the recording's own catalogue for these
/// folders ([`Catalogue::recorded`]), which [`draw`] draws what the mods cannot from.
fn keeping_recorded(
    source: &Source,
    options: &ContentOptions,
    shown: Arc<Catalogue>,
) -> io::Result<Arc<Catalogue>> {
    if shown.recorded().is_none() {
        shown.keep_recorded(recorded_slot(source, options)?);
    }
    Ok(shown)
}

/// The catalogue of these folders drawing the recording's own artwork whatever visual mods the
/// request shows: what preparation reads, so the prepared store keeps payloads by recorded
/// content alone, and what a reply's art falls back to ([`present`]). It credits nothing.
/// Without visual mods it is [`catalogue`]'s.
pub fn recorded_catalogue(source: &Source, options: &ContentOptions) -> io::Result<Arc<Catalogue>> {
    if options.presentation.is_empty() {
        return catalogue(source, options);
    }
    recorded_slot(source, options)
}

/// Art a reply draws from the operands a prepared payload kept, as the request's selection shows
/// it. `draw` is the whole operation, run against one catalogue: acquiring its GUI selector,
/// choosing among candidates by what is declared, looking the sprite up and rendering it. It runs
/// against the request's `admitted` catalogue first, as one transaction: strictly, with no
/// loader falling back to the recording ([`installed_content_core::content::strict`]), and isolated, its
/// notes kept from the reply ([`capture_isolated`]). When it draws, its notes are published;
/// when any stage fails, they are dropped and the whole operation runs again against the
/// recording's own catalogue, so the recording's own choice is made again, and the reply names
/// the fallback once, as `what` with the stage that failed (decision 5). When the recording
/// cannot draw it either, the recording's failure is the result: the art is unavailable, as it
/// always was. Everything runs in one capture, so the memo names its notes on every reply that
/// shows it. Inside another transaction's strict attempt it draws `admitted` alone, strictly:
/// falling back to the recording is that transaction's.
pub fn present<T, E: std::fmt::Display>(
    what: &str,
    admitted: &Catalogue,
    recorded: &Catalogue,
    draw: impl Fn(&Catalogue) -> Result<T, E>,
) -> Drawn<Result<T, E>> {
    #[cfg(any(test, feature = "test-support"))]
    PRESENTED.with(|count| count.set(count.get() + 1));
    Drawn::new(|| {
        if std::ptr::eq(admitted, recorded) || installed_content_core::content::is_strict() {
            return draw(admitted);
        }
        let (shown, noted) =
            capture_isolated(|| installed_content_core::content::strict(|| draw(admitted)));
        match shown {
            Ok(value) => {
                credit(&noted);
                Ok(value)
            }
            Err(reason) => {
                let drawn = draw(recorded);
                if drawn.is_ok() {
                    installed_content_core::content::credit_overlay_refusals(
                        &std::collections::BTreeSet::from([format!("{what} ({reason})")]),
                    );
                }
                drawn
            }
        }
    })
}

/// Art a reply draws when it answers, through `shown`, the catalogue its request shows
/// ([`catalogue`]): one transaction ([`present`]) against `shown` and then, when any stage of
/// `operation` fails there, against the recording's own catalogue `shown` keeps
/// ([`Catalogue::recorded`]), named as `what` gives it. `operation` is the whole draw, every
/// choice that depends on what the catalogue declares included. A catalogue showing no visual
/// mod draws it once, as it is. A memo keeping the result keeps it per artwork mode
/// (`installed_content_core::content::ByMode`) when another transaction's strict attempt can reach it.
pub fn draw<T, E: std::fmt::Display>(
    what: impl FnOnce() -> String,
    shown: &Catalogue,
    operation: impl Fn(&Catalogue) -> Result<T, E>,
) -> Drawn<Result<T, E>> {
    match shown.recorded() {
        Some(recorded) => present(&what(), shown, recorded, operation),
        None => Drawn::new(|| operation(shown)),
    }
}

#[cfg(test)]
mod present_tests {
    use super::*;
    use crate::test_fixtures::visual_mods::{Machine, RED, fallback_line, png, select};

    /// The admitted attempt runs strictly and keeps its notes to itself: a failed one leaves no
    /// note of its own on the reply or the memo, only the fallback named once; a drawn one
    /// publishes its notes.
    #[test]
    fn present_is_one_transaction() {
        let machine = Machine::new("present_transaction");
        machine.write("gfx/icon.png", &png(RED));
        machine.write_mod("art", "gfx/icon.png", &png(RED));
        let (source, mut options) = machine.open();
        select(&source, &mut options, &["art"]);
        let (admitted, recorded) = presenting(&source, Some(&options)).unwrap().unwrap();
        let note = |text: &str| {
            installed_content_core::content::credit_overlay_refusals(&std::collections::BTreeSet::from([
                text.to_owned(),
            ]))
        };
        let (drawn, line) = fallback_line(|| {
            present("the icon", &admitted, &recorded, |cat| {
                if std::ptr::eq(cat, &*admitted) {
                    assert!(installed_content_core::content::is_strict());
                    note("discarded selector note");
                    Err("the mod's sprite does not load".to_owned())
                } else {
                    assert!(!installed_content_core::content::is_strict());
                    Ok::<_, String>(7)
                }
            })
        });
        assert_eq!(*drawn.get(), Ok::<_, String>(7));
        for line in [line, fallback_line(|| drawn.get().clone()).1] {
            let line = line.unwrap();
            assert!(
                line.contains("the icon (the mod's sprite does not load)"),
                "{line}"
            );
            assert!(!line.contains("discarded"), "{line}");
        }
        let (drawn, line) = fallback_line(|| {
            present("the icon", &admitted, &recorded, |_| {
                note("kept selector note");
                Ok::<_, String>(8)
            })
        });
        assert_eq!(*drawn.get(), Ok::<_, String>(8));
        assert!(line.is_some_and(|line| line.contains("kept selector note")));
    }

    /// A catalogue showing visual mods keeps the recording's own, which [`draw`] falls back to as
    /// a whole; without a selection the catalogue is the recording's, keeps none, and draws once.
    #[test]
    fn draw_falls_back_through_the_recorded_catalogue_the_shown_one_keeps() {
        let machine = Machine::new("draw_keeps_recorded");
        machine.write("gfx/icon.png", &png(RED));
        machine.write_mod("art", "gfx/icon.png", &png(RED));
        let (source, mut options) = machine.open();
        select(&source, &mut options, &[]);
        let plain = catalogue(&source, &options).unwrap();
        assert!(plain.recorded().is_none());
        let attempts = std::cell::Cell::new(0);
        let (drawn, line) = fallback_line(|| {
            draw(
                || unreachable!("nothing is named"),
                &plain,
                |cat| {
                    attempts.set(attempts.get() + 1);
                    assert!(!installed_content_core::content::is_strict());
                    assert!(std::ptr::eq(cat, &*plain));
                    Ok::<_, String>(1)
                },
            )
        });
        assert_eq!(
            (drawn.get().clone(), attempts.get(), line),
            (Ok::<_, String>(1), 1, None)
        );
        select(&source, &mut options, &["art"]);
        let shown = catalogue(&source, &options).unwrap();
        let recorded = shown
            .recorded()
            .expect("the recording's own catalogue kept");
        assert!(Arc::ptr_eq(
            recorded,
            &recorded_catalogue(&source, &options).unwrap()
        ));
        let (drawn, line) = fallback_line(|| {
            draw(
                || "the icon".to_owned(),
                &shown,
                |cat| {
                    if std::ptr::eq(cat, &*shown) {
                        Err("the mod's sprite does not load".to_owned())
                    } else {
                        Ok::<_, String>(2)
                    }
                },
            )
        });
        assert_eq!(*drawn.get(), Ok::<_, String>(2));
        assert!(
            line.is_some_and(|line| line.contains("the icon (the mod's sprite does not load)"))
        );
    }

    /// A transaction reached inside another's strict attempt draws the selection alone,
    /// strictly, and names nothing: when the outer attempt fails, only the outer fallback is
    /// named, and the outer operation, not the inner one, makes the recording's choice again.
    #[test]
    fn a_nested_transaction_leaves_falling_back_to_the_outer_one() {
        let machine = Machine::new("nested_present");
        machine.write("gfx/icon.png", &png(RED));
        machine.write_mod("art", "gfx/icon.png", &png(RED));
        let (source, mut options) = machine.open();
        select(&source, &mut options, &["art"]);
        let shown = catalogue(&source, &options).unwrap();
        let inner = |cat: &Catalogue| {
            draw(
                || "the inner icon".to_owned(),
                cat,
                |drawing| {
                    if std::ptr::eq(drawing, &*shown) {
                        assert!(installed_content_core::content::is_strict());
                        Err("the mod's inner sprite does not load".to_owned())
                    } else {
                        Ok::<_, String>("recorded inner")
                    }
                },
            )
            .get()
            .clone()
        };
        let (drawn, line) = fallback_line(|| {
            draw(
                || "the outer icon".to_owned(),
                &shown,
                |cat| {
                    let inner = inner(cat)?;
                    Ok::<_, String>(if std::ptr::eq(cat, &*shown) {
                        "shown"
                    } else {
                        inner
                    })
                },
            )
        });
        assert_eq!(
            *drawn.get(),
            Ok::<_, String>("recorded inner"),
            "the outer operation's fallback"
        );
        let line = line.unwrap();
        assert!(
            line.contains("the outer icon (the mod's inner sprite does not load)"),
            "{line}"
        );
        assert!(!line.contains("the inner icon"), "{line}");
    }
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static PRESENTED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many art operations [`present`] has drawn on this thread: what a test counts a reply's
/// renders by, memo hits drawing none.
#[cfg(any(test, feature = "test-support"))]
pub fn presented() -> usize {
    PRESENTED.with(std::cell::Cell::get)
}

/// What a family's redraw keeps for one selection: the request's admitted catalogue, the
/// recording's own, and each key's art drawn by [`present`]; one per kind of catalogue
/// ([`ByCatalogue`]), replaced when either catalogue is.
pub struct Redrawn<K, T> {
    pub admitted: Arc<Catalogue>,
    pub recorded: Arc<Catalogue>,
    pub drawn: std::collections::BTreeMap<K, Drawn<Result<T, String>>>,
}

impl<K: Ord, T> Redrawn<K, T> {
    /// The memo `slot` keeps for these catalogues, bound anew for others.
    pub fn bind<'a>(
        slot: &'a mut Option<Self>,
        admitted: &Arc<Catalogue>,
        recorded: &Arc<Catalogue>,
    ) -> &'a mut Self {
        if !slot.as_ref().is_some_and(|held| {
            Arc::ptr_eq(&held.admitted, admitted) && Arc::ptr_eq(&held.recorded, recorded)
        }) {
            *slot = Some(Self {
                admitted: Arc::clone(admitted),
                recorded: Arc::clone(recorded),
                drawn: std::collections::BTreeMap::new(),
            });
        }
        slot.as_mut().expect("presented art bound")
    }

    /// `key`'s art, drawn by [`present`] on first use; its notes are credited on every use.
    pub fn get(
        &mut self,
        key: K,
        what: impl FnOnce() -> String,
        draw: impl Fn(&Catalogue) -> Result<T, String>,
    ) -> &Result<T, String> {
        let (admitted, recorded) = (&self.admitted, &self.recorded);
        self.drawn
            .entry(key)
            .or_insert_with(|| present(&what(), admitted, recorded, draw))
            .get()
    }
}

/// The catalogues a redraw for `options` reads: the request's admitted one and the recording's
/// own, or `None` when the request shows no visual mods (its stored art is already the one it
/// shows).
pub fn presenting(
    source: &Source,
    options: Option<&ContentOptions>,
) -> io::Result<Option<(Arc<Catalogue>, Arc<Catalogue>)>> {
    let Some(options) = options.filter(|options| options.presentation.shows_mods()) else {
        return Ok(None);
    };
    Ok(Some((
        catalogue(source, options)?,
        recorded_catalogue(source, options)?,
    )))
}

/// A source cache built from one catalogue at a time, kept once for the recording's own
/// catalogue (what preparation reads, and requests while no visual mod is shown) and once for
/// the catalogue showing the source's visual mods: preparation and requests showing visual
/// mods each keep their entry, under the cache's own retention rules, instead of displacing
/// each other's.
#[derive(Default)]
pub struct ByCatalogue<T> {
    recorded: T,
    shown: T,
}

impl<T> ByCatalogue<T> {
    /// The cache kept for catalogues of `catalogue`'s kind.
    pub fn of(&mut self, catalogue: &Catalogue) -> &mut T {
        self.of_optional(Some(catalogue))
    }

    /// [`Self::of`] for a cache that may be built from no catalogue, which reads as the
    /// recording's.
    pub fn of_optional(&mut self, catalogue: Option<&Catalogue>) -> &mut T {
        if catalogue.is_none_or(|catalogue| catalogue.presentation.is_empty()) {
            &mut self.recorded
        } else {
            &mut self.shown
        }
    }

    /// Both caches, the recording's first.
    #[cfg(any(test, feature = "test-support"))]
    pub fn both(&self) -> [&T; 2] {
        [&self.recorded, &self.shown]
    }
}

/// The catalogue for these options only when reading it binds nothing a request did not: the
/// source's content must already be resolved for them ([`Source::bound_content`]), they must
/// show the source's presentation now, and its catalogue must be bound to them or to none; the
/// presentation is mounted over the bound content if no request has yet
/// ([`Source::bound_presented_for`]). Otherwise `None`, having read and published
/// nothing. Work done ahead of a request uses it, so content or a presentation a request has
/// since replaced is never bound again for its replies to report.
pub fn bound_catalogue(
    source: &Source,
    options: &ContentOptions,
) -> Option<io::Result<Arc<Catalogue>>> {
    bound_catalogue_with(source, options, || {})
}

/// [`bound_catalogue`], running `before_load` once the binding is decided and before the
/// catalogue is read (a test holds the read there).
pub(crate) fn bound_catalogue_with(
    source: &Source,
    options: &ContentOptions,
    before_load: impl FnOnce(),
) -> Option<io::Result<Arc<Catalogue>>> {
    let content = source.bound_content(&options.options)?;
    // The binding's mount, computed outside the catalogue's slot, and only while `options` still
    // hold the source's binding.
    let presented = source.bound_presented_for(options)?;
    let key = binding(options);
    let settings = key.3.clone();
    let result = with_slot(source, options, |slot| {
        // A catalogue a request bound to anything else is left alone.
        if slot.as_ref().is_some_and(|(cached, _)| *cached != key) {
            return None;
        }
        Some(match slot.as_ref() {
            Some((_, result)) => Arc::clone(result),
            None => {
                let result = Arc::new(CachedResult::default());
                *slot = Some((key, Arc::clone(&result)));
                result
            }
        })
    })?;
    before_load();
    let loaded = result.get_or_init(|| {
        load_with(
            source,
            options,
            &settings,
            || Ok(Arc::clone(&content)),
            || Ok(Arc::clone(&presented)),
        )
    });
    Some(if reads_recorded(options) {
        loaded
    } else {
        loaded.and_then(|shown| keeping_recorded(source, options, shown))
    })
}

#[cfg(any(test, feature = "test-support"))]
pub fn set_test_catalogue(source: &Source, options: &ContentOptions, catalogue: Arc<Catalogue>) {
    let result = Arc::new(CachedResult::loaded(catalogue));
    with_slot(source, options, |slot| {
        *slot = Some((binding(options), result))
    });
}

#[cfg(any(test, feature = "test-support"))]
pub fn catalogue_loads(source: &Source, options: &ContentOptions) -> usize {
    with_slot(source, options, |slot| {
        slot.as_ref()
            .filter(|(key, _)| binding_matches(key, options))
            .map_or(0, |(_, result)| result.loads())
    })
}

#[cfg(test)]
mod parity_tests;

#[cfg(test)]
mod loader_tests {
    use super::*;

    #[test]
    fn missing_or_unreadable_settings_only_refuse_catalogue_artwork() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use crate::test_fixtures::{CAMPAIGN, records, source};
        use replay::{container::Replay, spool::Record};
        use std::sync::atomic::AtomicBool;
        let (_, folder) = crate::catalogue::tests::folder_catalogue(&[
            ("hoi4.exe", "test"),
            (
                "user/settings.txt",
                "language=l_english force_pow2_textures=no",
            ),
            (
                "user/dlc_load.json",
                r#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
            ),
            (
                "localisation/a_l_english.yml",
                "\u{feff}l_english:\n NAME:0 \"Recorded name\"\n",
            ),
        ]);
        let (options, identity) = crate::test_fixtures::content_identity(&folder.0);
        let settings = options.options.user_dir.join("settings.txt");
        std::fs::remove_file(&settings).unwrap();
        for unreadable in [false, true] {
            if unreadable {
                std::fs::create_dir(&settings).unwrap();
            }
            let mut input = records(1, vec![source(1, 100, Some(1))]);
            let Record::SessionStart(session) = &mut input[0] else {
                unreachable!()
            };
            session.identity = identity.clone();
            let source = crate::Source::from_replay(
                crate::SourceKey {
                    source_id: "settings-isolation".into(),
                    revision: unreadable.to_string(),
                },
                "memory.battleplan".into(),
                Arc::new(Replay::from_records(CAMPAIGN.into(), &[input]).unwrap()),
                &AtomicBool::new(false),
            )
            .unwrap();
            let catalogue = crate::catalogue::catalogue(&source, &options).unwrap();
            assert_eq!(
                catalogue
                    .localised("NAME", crate::catalogue::View::Plain)
                    .unwrap(),
                "Recorded name"
            );
            assert!(
                catalogue
                    .gui_decode
                    .as_ref()
                    .unwrap_err()
                    .contains("settings.txt")
            );
            assert!(
                catalogue
                    .warnings
                    .iter()
                    .any(|why| why.contains("recorded language"))
            );
            let cached = crate::catalogue::catalogue(&source, &options).unwrap();
            assert!(Arc::ptr_eq(&catalogue, &cached));
            if unreadable {
                std::fs::remove_dir(&settings).unwrap();
            }
            // Settings that change since are read again, with the catalogue bound to them.
            std::fs::write(&settings, "language=l_english force_pow2_textures=yes").unwrap();
            let changed = crate::catalogue::catalogue(&source, &options).unwrap();
            assert!(!Arc::ptr_eq(&catalogue, &changed));
            assert!(
                changed
                    .gui_decode
                    .as_ref()
                    .is_ok_and(|decode| decode.force_pow2_textures)
            );
            assert!(
                changed
                    .warnings
                    .iter()
                    .all(|why| !why.contains("recorded language"))
            );
            std::fs::remove_file(&settings).unwrap();
        }
    }
    /// A read-ahead of the content a source opened with is held inside its catalogue load
    /// while a request binds newer content. It binds nothing back: the source's content, the
    /// catalogue every request gets and the warnings every reply reports stay the newer
    /// content's. Without content bound for its folders it reads nothing at all.
    #[test]
    fn a_read_ahead_never_rebinds_content_a_request_replaced() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use crate::test_fixtures::{CAMPAIGN, records, source};
        use replay::{container::Replay, spool::Record};
        use std::sync::{Barrier, atomic::AtomicBool};
        let files: &[(&str, &str)] = &[
            ("hoi4.exe", "test"),
            (
                "user/settings.txt",
                "language=l_english force_pow2_textures=no",
            ),
            (
                "user/dlc_load.json",
                r#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
            ),
            (
                "localisation/a_l_english.yml",
                "\u{feff}l_english:\n NAME:0 \"Recorded name\"\n",
            ),
        ];
        let (_, old_folder) = crate::catalogue::tests::folder_catalogue(files);
        let (_, new_folder) = crate::catalogue::tests::folder_catalogue(files);
        let (old, identity) = crate::test_fixtures::content_identity(&old_folder.0);
        let (new, _) = crate::test_fixtures::content_identity(&new_folder.0);
        let mut input = records(1, vec![source(1, 100, Some(1))]);
        let Record::SessionStart(session) = &mut input[0] else {
            unreachable!()
        };
        session.identity = identity;
        let source = crate::Source::from_replay(
            crate::SourceKey {
                source_id: "read-ahead".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            Arc::new(Replay::from_records(CAMPAIGN.into(), &[input]).unwrap()),
            &AtomicBool::new(false),
        )
        .unwrap();
        // Nothing bound yet: the read-ahead reads and publishes nothing.
        assert!(bound_catalogue(&source, &old).is_none());
        assert!(source.bound_content(&old.options).is_none());
        // The source opened with the old content bound.
        source.content(&old.options).unwrap();
        let (entered, release) = (Barrier::new(2), Barrier::new(2));
        let (requested, warnings_before) = std::thread::scope(|scope| {
            let ahead = scope.spawn(|| {
                bound_catalogue_with(&source, &old, || {
                    entered.wait();
                    release.wait();
                })
            });
            entered.wait();
            // A request of the newer content, while the read-ahead is inside its load.
            let requested = catalogue(&source, &new).unwrap();
            let mut warnings = Vec::new();
            source.append_content_warnings(&mut warnings);
            release.wait();
            // The read-ahead read the old catalogue for itself, publishing nothing.
            ahead.join().unwrap().unwrap().unwrap();
            (requested, warnings)
        });
        assert!(source.bound_content(&new.options).is_some());
        assert!(
            source.bound_content(&old.options).is_none(),
            "the old content is not bound back"
        );
        assert!(
            Arc::ptr_eq(&requested, &catalogue(&source, &new).unwrap()),
            "requests keep the newer catalogue"
        );
        let mut warnings = Vec::new();
        source.append_content_warnings(&mut warnings);
        assert_eq!(
            warnings, warnings_before,
            "replies report the newer content's reads"
        );
        // And once the newer content is bound, a read-ahead of the old one reads nothing.
        assert!(bound_catalogue(&source, &old).is_none());
    }
}
