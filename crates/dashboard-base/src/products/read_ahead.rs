//! The content many pages' first replies wait for, read ahead once a source opens.
use super::{ContentOptions, LoadContext, politics_content, production_lines};
use crate::{
    catalogue::{self, Catalogue},
    source::Source,
};
use std::sync::{Arc, PoisonError, Weak, atomic::Ordering};

/// Whether a source's page content read-ahead has been started: one per source, however often
/// it opens.
#[derive(Default)]
struct ReadAhead(bool);

/// Reads ahead, on a thread of its own, the content pages' first replies wait for ([`read`]), on
/// half the processors ([`job_priority::confined`]): the page shown answers on the others, and a
/// product asking for what it reads waits for the same read, held at that product's priority.
pub fn warm_in_background(source: &Arc<Source>, options: ContentOptions) {
    {
        let state = source.state::<ReadAhead>();
        let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
        if std::mem::replace(&mut state.0, true) {
            return;
        }
    }
    let source = Arc::downgrade(source);
    let _ = std::thread::Builder::new()
        .name("page content read-ahead".into())
        .spawn(move || job_priority::confined(|| read(&source, options)));
}

/// Reads the native GUI registry every country flag's shield is drawn with ([`warm_guis`]), then
/// the politics content (the event feed, politics and special projects), then the effect description
/// content (doctrine mastery, special projects and politics), each through the cache its products
/// read, then the rest the catalogue reads lazily, which prepared answers kept on
/// disk wait for ([`Catalogue::read_readers`]), then production lines' icon content where the
/// store key stamps its inputs ([`production_lines::warm_icons`]), over the catalogue already
/// bound for `options`: a product asking meanwhile waits for the same read, and a cache a request
/// kept for another catalogue is left as it is.
/// It holds the source only while a step runs, and no step starts once the source is dropped or
/// `options`' binding is retired or replaced. A failure is left for the products to report.
fn read(source: &Weak<Source>, mut options: ContentOptions) {
    let retired = |options: &ContentOptions| options.presentation.cancel().load(Ordering::Acquire);
    if retired(&options) {
        return;
    }
    if let Some(source) = source.upgrade() {
        source.resolve_bound(&mut options);
    }
    let steps: [fn(&Source, &ContentOptions, Arc<Catalogue>); 5] = [
        |_, _, catalogue| warm_guis(&catalogue),
        |source, _, catalogue| drop(politics_content::warm(source, catalogue)),
        |_, _, catalogue| {
            if let Ok(content) = catalogue.scripted_content() {
                drop(content.effects());
            }
        },
        |_, _, catalogue| catalogue.read_readers(),
        |source, options, _| {
            production_lines::warm_icons(
                source,
                &LoadContext {
                    content: Some(options.clone()),
                },
            );
        },
    ];
    for step in steps {
        let Some(source) = source.upgrade() else {
            return;
        };
        if retired(&options) {
            return;
        }
        let Some(Ok(catalogue)) = catalogue::bound_catalogue(&source, &options) else {
            return;
        };
        if retired(&options) {
            return;
        }
        step(&source, &options, catalogue);
    }
}

/// Reads `catalogue`'s native GUI registry in the mode a flag's shield reads it: strictly while
/// visual mods are shown, as every artwork transaction under them reads.
pub(crate) fn warm_guis(catalogue: &Catalogue) {
    if catalogue.content.layers.presentation().shows_visual_mods() {
        installed_content_core::content::strict(|| drop(catalogue.guis()));
    } else {
        drop(catalogue.guis());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{presentation::Presentation, source::Work};

    /// A source bound to a content folder, admitted for work ahead of a request as an open
    /// admits it, its content resolved as the open resolves it.
    fn opened() -> (Arc<Source>, ContentOptions, catalogue::tests::Folder) {
        opened_with(&[])
    }

    /// [`opened`] over `files` besides its own.
    fn opened_with(
        files: &[(&str, &str)],
    ) -> (Arc<Source>, ContentOptions, catalogue::tests::Folder) {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let mut all = vec![
            (
                "common/defines/work.lua",
                "NDefines={NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}},\
                 NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
            ),
            ("common/ideas/work.txt", "ideas={country={mod_a={}}}"),
        ];
        all.extend_from_slice(files);
        let (_catalogue, folder) = catalogue::tests::folder_catalogue(&all);
        let frame = crate::test_fixtures::source(1, 100, Some(1));
        let (source, context) = crate::test_fixtures::source_with_content(vec![frame], &folder.0);
        let source = Arc::new(source);
        let content = context.content.unwrap();
        source.bind(
            Some((&content.options, content.allow_unverified)),
            Presentation::default(),
        );
        let options = source.admit(Work::Ahead).unwrap();
        source.content(&options.options).unwrap();
        (source, options, folder)
    }

    #[test]
    fn an_opened_sources_page_content_is_read_ahead_for_its_products() {
        let (source, options, _folder) = opened();
        read(&Arc::downgrade(&source), options.clone());
        let catalogue = catalogue::bound_catalogue(&source, &options)
            .unwrap()
            .unwrap();
        let warmed = politics_content::held(&source, &catalogue).expect("read ahead");
        // The product's own read takes what was read ahead.
        let read = politics_content::politics_content(&source, &options).unwrap();
        assert!(Arc::ptr_eq(&warmed, &read));
        assert_eq!(politics_content::content_loads(&source), 1);
        assert!(catalogue.guis_loaded());
        assert!(catalogue.scripted_content().unwrap().effects_read());
        assert!(catalogue.readers_read());
    }

    /// Icon content that reads a file outside the recorded coverage is left for a product to ask
    /// for: every reply reports such a read.
    #[test]
    fn icon_content_reading_outside_coverage_is_not_read_ahead() {
        let (source, options, _folder) = opened_with(&[
            (
                "map/default.map",
                "definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp continent=music/continent.txt",
            ),
            ("map/definition.csv", "0;0;0;0;land;false;unknown;0\n"),
            ("map/music/continent.txt", "continents={synthetic_continent}"),
        ]);
        read(&Arc::downgrade(&source), options.clone());
        assert!(!source.reads_uncovered());
        drop(production_lines::preparation_icons(
            &source,
            &LoadContext {
                content: Some(options),
            },
        ));
        assert!(source.reads_uncovered());
    }

    #[test]
    fn a_retired_binding_reads_nothing_ahead() {
        let (source, options, _folder) = opened();
        source.bind(None, Presentation::default());
        read(&Arc::downgrade(&source), options);
        assert_eq!(politics_content::content_loads(&source), 0);
    }
}
