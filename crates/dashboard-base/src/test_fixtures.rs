//! Test sources and content options over the recording builders in
//! [`dashboard_core::test_fixtures`].

pub use dashboard_core::test_fixtures::*;
use replay::spool::Record;
use replay_frame::frame::DecodedFrame;

/// A real read-only recording binding for native query tests, with scratch outside Git.
pub fn file_source(replay: replay::container::Replay) -> (crate::Source, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("replay-query-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("campaign.battleplan");
    std::fs::write(&path, replay.bytes()).unwrap();
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: root.to_string_lossy().into_owned(),
            revision: crate::protocol::replay_revision(&path).unwrap(),
        },
        path,
        std::sync::Arc::new(replay),
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    (source, root)
}

/// Gather a synthetic content root whose executable contains `test`.
pub fn content_identity(
    root: &std::path::Path,
) -> (
    crate::products::ContentOptions,
    replay_data::identity::Identity,
) {
    let options = crate::products::ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir: root.to_owned(),
            user_dir: root.join("user"),
            cache_dir: root.join("cache"),
        },
        allow_unverified: true,
        presentation: Default::default(),
    };
    std::fs::create_dir_all(root.join("dlc")).unwrap();
    let gathered = installed_content_core::gather(&installed_content_core::GatherOptions {
        game_dir: options.options.game_dir.clone(),
        user_dir: options.options.user_dir.clone(),
        cache_dir: options.options.cache_dir.clone(),
        dlc_order: Some(Vec::new()),
    })
    .unwrap();
    let identity = gathered
        .identity(
            "test".into(),
            [
                0x9f, 0x86, 0xd0, 0x81, 0x88, 0x4c, 0x7d, 0x65, 0x9a, 0x2f, 0xea, 0xa0, 0xc5, 0x5a,
                0xd0, 0x15, 0xa3, 0xbf, 0x4f, 0x1b, 0x2b, 0x0b, 0x82, 0x2c, 0xd1, 0x5d, 0x6c, 0x15,
                0xb0, 0xf0, 0x0a, 0x08,
            ],
            4,
            "test".into(),
            [1; 32],
        )
        .unwrap();
    (options, identity)
}

/// Open frames against an English synthetic content root.
pub fn source_with_content(
    frames: Vec<DecodedFrame>,
    root: &std::path::Path,
) -> (crate::Source, crate::products::LoadContext) {
    std::fs::create_dir_all(root.join("user")).unwrap();
    for (path, text) in [
        ("hoi4.exe", "test"),
        (
            "user/settings.txt",
            "language=l_english force_pow2_textures=no",
        ),
        (
            "user/dlc_load.json",
            r#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
        ),
    ] {
        std::fs::write(root.join(path), text).unwrap();
    }
    let (options, identity) = content_identity(root);
    let mut input = records(1, frames);
    let Record::SessionStart(session) = &mut input[0] else {
        unreachable!()
    };
    session.identity = identity;
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "content-fixture".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        std::sync::Arc::new(
            replay::container::Replay::from_records(CAMPAIGN.into(), &[input]).unwrap(),
        ),
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    (
        source,
        crate::products::LoadContext {
            content: Some(options),
        },
    )
}

pub mod visual_mods;
