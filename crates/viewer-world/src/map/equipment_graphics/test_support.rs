//! Existing synthetic worlds for equipment graphic-pool tests.
use super::*;
use std::sync::{Arc, OnceLock};

pub fn with_world<T>(
    frame: &replay::frame::DecodedFrame,
    dlcs: &[&str],
    run: impl FnOnce(Limits<'_>) -> T,
) -> T {
    static IDEOLOGIES: OnceLock<installed_content::ideologies::Ideologies> = OnceLock::new();
    let ideologies = IDEOLOGIES.get_or_init(|| {
        let root = std::env::temp_dir().join(format!(
            "equipment-graphics-ideologies-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let file = root.join("common/ideologies/synthetic.txt");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "ideologies = { government = {} }").unwrap();
        let layers = installed_content::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
        let (ideologies, warnings) = installed_content::ideologies::read(&layers).unwrap();
        std::fs::remove_dir_all(root).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        ideologies
    });
    let dlcs = dlcs.iter().map(|s| (*s).to_owned()).collect();
    let aliases = Arc::default();
    let names = installed_content::trigger_keys::TriggerNames {
        counts: Some(Default::default()),
        ..Default::default()
    };
    let world = dashboard_text::triggers::World::read(
        frame,
        dashboard_text::triggers::ContentFacts {
            ideologies: Ok(ideologies),
            ..dashboard_text::triggers::ContentFacts::content_only(&dlcs, &names, &aliases)
        },
    );
    let evaluate = |items: &[Item], scopes| world.evaluate(items, scopes);
    run(Limits::new(&LimitValues::default(), &evaluate))
}

pub fn empty_frame() -> replay::frame::DecodedFrame {
    use replay::{
        frame::{DecodedFrame, TableView},
        rows::{TableId, TableRows},
    };
    DecodedFrame {
        branch: 1,
        hour: 1,
        metrics: Default::default(),
        tables: std::array::from_fn(|i| {
            TableView::Complete(TableRows::empty(TableId::ALL[i])).into()
        }),
    }
}
