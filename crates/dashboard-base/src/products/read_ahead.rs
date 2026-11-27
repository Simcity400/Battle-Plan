use dashboard_effects::ScriptedEffects;
use job_priority::SpawnCarried as _;
use super::{ContentOptions, politics_content};
use crate::{
    catalogue::{self, Catalogue},
    source::Source,
};
use std::sync::{Arc, PoisonError, Weak, atomic::Ordering};

#[derive(Default)]
struct ReadAhead(bool);

impl ReadAhead {
    fn start(&mut self, spawn: impl FnOnce() -> std::io::Result<()>) {
        if !self.0 {
            self.0 = spawn().is_ok();
        }
    }
}

pub fn warm_in_background(source: &Arc<Source>, options: ContentOptions) {
    let state = source.state::<ReadAhead>();
    state.lock().unwrap_or_else(PoisonError::into_inner).start(|| {
        let source = Arc::downgrade(source);
        std::thread::Builder::new()
            .name("page content read-ahead".into())
            .spawn_carried(move || job_priority::confined(|| read(&source, options)))
            .map(drop)
    });
}

fn read(source: &Weak<Source>, mut options: ContentOptions) {
    let retired = |options: &ContentOptions| options.presentation.cancel().load(Ordering::Acquire);
    if retired(&options) {
        return;
    }
    if let Some(source) = source.upgrade() {
        source.resolve_bound(&mut options);
    }
    let steps: [fn(&Source, &ContentOptions, Arc<Catalogue>); 4] = [
        |_, _, catalogue| warm_guis(&catalogue),
        |source, _, catalogue| drop(politics_content::warm(source, catalogue)),
        |_, _, catalogue| {
            if let Ok(content) = catalogue.scripted_content() {
                drop(content.effects());
            }
        },
        |_, _, catalogue| catalogue.read_readers(),
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

pub(crate) fn warm_guis(catalogue: &Catalogue) {
    if catalogue.content.layers.presentation().shows_visual_mods() {
        content_reader_layers::content::strict(|| drop(catalogue.guis()));
    } else {
        drop(catalogue.guis());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{presentation::Presentation, source::Work};

    fn opened() -> (Arc<Source>, ContentOptions, catalogue::tests::Folder) {
        opened_with(&[])
    }

    fn opened_with(
        files: &[(&str, &str)],
    ) -> (Arc<Source>, ContentOptions, catalogue::tests::Folder) {
        let mut all = vec![
            (
                "common/defines/work.lua",
                "NDefines={NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}},\
                 NGame={MAX_SCRIPTED_LOC_RECURSION=23}}",
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
    fn a_failed_launch_leaves_read_ahead_available_for_retry() {
        let mut state = ReadAhead::default();
        state.start(|| Err(std::io::Error::other("cannot spawn")));
        let mut launches = 0;
        for _ in 0..2 {
            state.start(|| { launches += 1; Ok(()) });
        }
        assert_eq!(launches, 1);
    }

    #[test]
    fn an_opened_sources_page_content_is_read_ahead_for_its_products() {
        let (source, options, _folder) = opened();
        read(&Arc::downgrade(&source), options.clone());
        let catalogue = catalogue::bound_catalogue(&source, &options)
            .unwrap()
            .unwrap();
        let warmed = politics_content::held(&source, &catalogue).expect("read ahead");
        let read = politics_content::politics_content(&source, &options).unwrap();
        assert!(Arc::ptr_eq(&warmed, &read));
        assert_eq!(politics_content::content_loads(&source), 1);
        assert!(catalogue.guis_loaded());
        assert!(catalogue.scripted_content().unwrap().effects_read());
        assert!(catalogue.readers_read());
    }

}
