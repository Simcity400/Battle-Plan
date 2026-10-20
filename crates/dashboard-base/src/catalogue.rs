//! The content catalogue ([dashboard-text](../../dashboard-text/README.md)) and its
//! loader: one catalogue per source and folders, held in the source's own cache.

pub use dashboard_text::catalogue::*;

use crate::{Source, content, products::ContentOptions};
use std::{io, sync::Arc};

/// The recording's catalogue for these folders: its verified content, the
/// localisation in the user's game language and the client's texture settings.
fn load(source: &Source, options: &ContentOptions) -> io::Result<Catalogue> {
    load_with(source, options, || source.content(&options.options))
}

/// [`load`] over the content `resolved` gives.
fn load_with(
    source: &Source,
    options: &ContentOptions,
    resolved: impl Fn() -> io::Result<Arc<installed_content_core::content::ResolvedContent>>,
) -> io::Result<Catalogue> {
    let mut verification = Vec::new();
    let replay = source.replay();
    content::check_replay(&replay, options.allow_unverified, &mut verification)?;
    let language = &replay.identity.language;
    let recorded_language = language.strip_prefix("l_").unwrap_or(language).to_owned();
    let content = resolved()?;
    let mut warnings = Vec::new();
    let settings = installed_content_settings::settings::read_keys(&options.options.user_dir);
    let language = match &settings {
        Ok(settings) => settings.language()?,
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
        .map(|settings| settings.texture_decode())
        .map_err(|why| format!("settings.txt is unavailable: {why}"));
    Catalogue::from_content(
        content,
        localisation,
        verification,
        warnings,
        recorded_language,
        gui_decode,
    )
}

pub type Binding = (content::ContentKey, std::path::PathBuf);
type Cached = Option<(Binding, Arc<CachedResult<Catalogue>>)>;

pub fn binding(options: &ContentOptions) -> Binding {
    (
        content::content_key(Some(options)).expect("content options given"),
        options.options.cache_dir.clone(),
    )
}

pub fn binding_matches(key: &Binding, options: &ContentOptions) -> bool {
    key.0.0 == options.options.game_dir
        && key.0.1 == options.options.user_dir
        && key.0.2 == options.allow_unverified
        && key.1 == options.options.cache_dir
}

/// The shared catalogue for these folders, loaded on first use and replaced when
/// the folders, the cache folder or the unverified-content choice change.
pub fn catalogue(source: &Source, options: &ContentOptions) -> io::Result<Arc<Catalogue>> {
    let state = source.state::<Cached>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    if !state
        .as_ref()
        .is_some_and(|(cached, _)| binding_matches(cached, options))
    {
        *state = Some((binding(options), Arc::new(CachedResult::default())));
    }
    let result = Arc::clone(&state.as_ref().expect("catalogue binding retained").1);
    drop(state);
    result.get_or_init(|| load(source, options))
}

/// The catalogue for these folders only when reading it binds nothing a request did not: the
/// source's content must already be resolved for them ([`Source::bound_content`]), and its
/// catalogue bound to them or to none. Otherwise `None`, having read and published nothing.
/// Work done ahead of a request uses it, so content a request has since replaced is never
/// bound again for its replies to report.
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
    let state = source.state::<Cached>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    let result = match state.as_ref() {
        Some((cached, result)) if binding_matches(cached, options) => Arc::clone(result),
        Some(_) => return None,
        None => {
            let result = Arc::new(CachedResult::default());
            *state = Some((binding(options), Arc::clone(&result)));
            result
        }
    };
    drop(state);
    before_load();
    Some(result.get_or_init(|| load_with(source, options, || Ok(Arc::clone(&content)))))
}

#[cfg(any(test, feature = "test-support"))]
pub fn set_test_catalogue(source: &Source, options: &ContentOptions, catalogue: Arc<Catalogue>) {
    let result = CachedResult::loaded(catalogue);
    *source.state::<Cached>().lock().unwrap() = Some((binding(options), Arc::new(result)));
}

#[cfg(any(test, feature = "test-support"))]
pub fn catalogue_loads(source: &Source, options: &ContentOptions) -> usize {
    source
        .state::<Cached>()
        .lock()
        .unwrap()
        .as_ref()
        .filter(|(key, _)| binding_matches(key, options))
        .map_or(0, |(_, result)| result.loads())
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
            if unreadable {
                std::fs::remove_dir(&settings).unwrap();
            }
            std::fs::write(&settings, "language=l_english force_pow2_textures=yes").unwrap();
            let cached = crate::catalogue::catalogue(&source, &options).unwrap();
            assert!(Arc::ptr_eq(&catalogue, &cached));
            assert!(cached.gui_decode.is_err());
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
