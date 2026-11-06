//! One country or unit at one recorded hour, answered from the source's shared
//! frame cache and kept in its observation cache: the reached-hour pattern of
//! the Army breakdown's template roster and the Navy size drill, for widgets
//! that show retained state at the playback date.
use dashboard_base::{catalogue, prepared::query::observations, source::Source};
use dashboard_core::config::LoadContext;
use dashboard_text::catalogue::Catalogue;
use serde::{Serialize, de::DeserializeOwned};
use std::{io, sync::atomic::AtomicBool};

/// What a reply's names, artwork and refusals depend on besides the recording
/// and its content files: the folders, the cache folder and the choice to
/// allow unverified content, and the localisation language, recorded language
/// and texture settings of the catalogue they load. A reply kept under one is
/// never answered under another.
#[derive(Debug, PartialEq, Serialize)]
pub struct Presentation {
    binding: Option<catalogue::Binding>,
    language: Option<String>,
    recorded_language: Option<String>,
    textures: Option<String>,
}

impl Presentation {
    pub fn of(binding: Option<catalogue::Binding>, cat: Option<&Catalogue>) -> Self {
        Self {
            binding,
            language: cat.map(|cat| cat.localisation.language.clone()),
            recorded_language: cat.map(|cat| cat.recorded_language.clone()),
            textures: cat.map(|cat| format!("{:?}", cat.gui_decode)),
        }
    }

    /// The presentation a load under `context` answers with: its folders'
    /// shared catalogue, when it loads.
    pub fn current(source: &Source, context: &LoadContext) -> Self {
        let options = context.content.as_ref();
        let cat = options.and_then(|options| catalogue::catalogue(source, options).ok());
        Self::of(options.map(catalogue::binding), cat.as_deref())
    }
}

/// The reply `compute` makes for `params` at `hour` of `branch`, kept in the
/// observation cache of the block holding that hour under the source's
/// recording and content binding and the load's [`Presentation`]. A source
/// without a prepared-store binding (an in-memory recording) answers without
/// keeping it.
#[allow(clippy::too_many_arguments)]
pub fn cached<R: Serialize + DeserializeOwned>(
    source: &Source,
    context: &LoadContext,
    product: &'static str,
    branch: u64,
    hour: i32,
    params: &impl Serialize,
    cancelled: &AtomicBool,
    compute: impl FnOnce() -> io::Result<R>,
) -> io::Result<R> {
    let block = source
        .replay_ref()
        .index
        .iter()
        .find(|block| block.branch == branch && block.first_hour <= hour && hour <= block.last_hour);
    let (Some(block), Ok((_, key))) = (
        block,
        dashboard_base::prepared::files(source, context.content.as_ref()),
    ) else {
        return compute();
    };
    let value = observations::scope(source, product, Some(&key))
        .get(
            block,
            &(hour, params, Presentation::current(source, context)),
            cancelled,
            || serde_json::to_value(compute()?).map_err(io::Error::other),
        )?;
    serde_json::from_value(value.as_ref().clone()).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::products::ContentOptions;

    fn options(allow_unverified: bool) -> ContentOptions {
        ContentOptions {
            options: installed_content_core::ResolveOptions {
                game_dir: "game".into(),
                user_dir: "user".into(),
                cache_dir: "cache".into(),
            },
            allow_unverified,
            presentation: Default::default(),
        }
    }

    #[test]
    fn a_reply_kept_under_one_presentation_is_never_answered_under_another() {
        let (english, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[]);
        let verified = Presentation::of(Some(catalogue::binding(&options(false))), Some(&english));
        assert_eq!(verified, Presentation::of(Some(catalogue::binding(&options(false))), Some(&english)));
        assert_ne!(verified, Presentation::of(Some(catalogue::binding(&options(true))), Some(&english)),
            "allowing unverified content");
        let mut cache = options(false);
        cache.options.cache_dir = "elsewhere".into();
        assert_ne!(verified, Presentation::of(Some(catalogue::binding(&cache)), Some(&english)), "another cache folder");
        let (mut french, _french) = dashboard_text::catalogue::tests::folder_catalogue(&[]);
        std::sync::Arc::get_mut(&mut french.localisation).unwrap().language = "french".into();
        assert_ne!(verified, Presentation::of(Some(catalogue::binding(&options(false))), Some(&french)), "another language");
        let (mut recorded, _recorded) = dashboard_text::catalogue::tests::folder_catalogue(&[]);
        recorded.recorded_language = "german".into();
        assert_ne!(verified, Presentation::of(Some(catalogue::binding(&options(false))), Some(&recorded)), "another recorded language");
        let (mut textures, _textures) = dashboard_text::catalogue::tests::folder_catalogue(&[]);
        textures.gui_decode = Err("settings.txt is unavailable".into());
        assert_ne!(verified, Presentation::of(Some(catalogue::binding(&options(false))), Some(&textures)), "other texture settings");
        assert_ne!(verified, Presentation::of(Some(catalogue::binding(&options(false))), None), "an unloaded catalogue");
        let mut styled = options(false);
        styled.presentation = dashboard_core::presentation::Admitted::new(dashboard_core::presentation::Presentation { visual_mods: vec![installed_content_core::content::visual::Choice { id: "mod/art.mod".into(), name: "Art".into() }], revision: String::new() });
        assert_ne!(verified, Presentation::of(Some(catalogue::binding(&styled)), Some(&english)), "other visual mods");
    }
}
