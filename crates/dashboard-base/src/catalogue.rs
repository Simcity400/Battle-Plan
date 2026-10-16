//! The content catalogue ([dashboard-text](../../dashboard-text/README.md)) and its
//! loader: one catalogue per source and folders, held in the source's own cache.

pub use dashboard_text::catalogue::*;

use crate::{Source, content, products::ContentOptions};
use std::{io, sync::Arc};

/// The recording's catalogue for these folders: its verified content, the
/// localisation in the user's game language and the client's texture settings.
fn load(source: &Source, options: &ContentOptions) -> io::Result<Catalogue> {
    let mut verification = Vec::new();
    let replay = source.replay();
    content::check_replay(&replay, options.allow_unverified, &mut verification)?;
    let language = &replay.identity.language;
    let recorded_language = language.strip_prefix("l_").unwrap_or(language).to_owned();
    let content = source.content(&options.options)?;
    let mut warnings = Vec::new();
    let settings = installed_content::settings::read_keys(&options.options.user_dir);
    let language = match &settings {
        Ok(settings) => settings.language()?,
        Err(why) => {
            warnings.push(format!(
                "Localisation uses the recorded language {recorded_language}: settings.txt is unavailable: {why}"
            ));
            recorded_language.clone()
        }
    };
    let localisation = content::localisation_with_key(
        source,
        options,
        (
            content::content_key(Some(options)).expect("content options given"),
            options.options.cache_dir.clone(),
            language,
        ),
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
        engine::executable_tables::initialize_for_tests()
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
}
