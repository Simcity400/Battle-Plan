
pub use dashboard_core::test_fixtures::*;
use replay_capture_spool::spool::Record;
use replay_frame::frame::DecodedFrame;

pub fn file_source(replay: replay::campaign::Replay) -> (crate::Source, std::path::PathBuf) {
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

pub fn declare_languages(root: &std::path::Path) {
    let list = root.join("localisation/languages.yml");
    if !list.exists() {
        std::fs::create_dir_all(list.parent().unwrap()).unwrap();
        std::fs::write(
            list,
            "\u{feff}l_english:\n l_english: \"English\"\nl_french:\n l_english: \"French\"\n",
        )
        .unwrap();
    }
}

fn recorded_tables() -> engine_executable::executable_tables::Tables {
    use engine_executable::executable_tables::Tables;
    static TABLES: std::sync::OnceLock<Tables> = std::sync::OnceLock::new();
    TABLES.get_or_init(|| game::executable().executable(b"test").build())
}

pub fn content_identity(
    root: &std::path::Path,
) -> (
    crate::products::ContentOptions,
    replay_data::identity::Identity,
) {
    let options = crate::products::ContentOptions {
        options: content_reader_layers::ResolveOptions {
            game_dir: root.to_owned(),
            user_dir: root.join("user"),
            cache_dir: root.join("cache"),
        },
        allow_unverified: true,
        presentation: Default::default(),
    };
    std::fs::create_dir_all(root.join("dlc")).unwrap();
    declare_languages(root);
    let gathered = content_reader_layers::gather(&content_reader_layers::GatherOptions {
        game_dir: options.options.game_dir.clone(),
        user_dir: options.options.user_dir.clone(),
        cache_dir: options.options.cache_dir.clone(),
        dlc_order: Some(Vec::new()),
    })
    .unwrap();
    let executable = recorded_tables().identity();
    let identity = gathered
        .identity(
            "test".into(),
            executable.sha256,
            executable.size,
            "test".into(),
            [1; 32],
        )
        .unwrap();
    (options, identity)
}

pub fn write_with_facts(
    path: &std::path::Path,
    options: &crate::products::ContentOptions,
    identity: &replay_data::identity::Identity,
    input: Vec<Record>,
) {
    let gathered = content_reader_layers::gather(&content_reader_layers::GatherOptions {
        game_dir: options.options.game_dir.clone(),
        user_dir: options.options.user_dir.clone(),
        cache_dir: options.options.cache_dir.clone(),
        dlc_order: Some(Vec::new()),
    })
    .unwrap();
    let (pack, _) = content_reader_layers::content::facts::write(
        &content_reader_layers::content::facts::Source {
            layers: &gathered.layers,
            withheld: Default::default(),
        },
        identity,
        Ok(engine_executable::executable_tables::recorded::encode(recorded_tables())),
        &options.options.cache_dir,
        &|| false,
        content_reader_key_names::equipment_facts::gather,
    )
    .unwrap();
    let replay = replay::campaign::Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[input]).unwrap();
    std::fs::write(path, replay.bytes()).unwrap();
    replay::campaign::writer::revise(
        path,
        replay.recording(),
        &replay::campaign::writer::Addition {
            facts: Some(&pack),
            ..Default::default()
        },
        true,
        3,
    )
    .unwrap();
}

pub fn with_facts(
    options: &crate::products::ContentOptions,
    identity: &replay_data::identity::Identity,
    input: Vec<Record>,
) -> replay::campaign::Replay {
    let path = std::env::temp_dir().join(format!(
        "hoi4-fixture-{}.battleplan",
        uuid::Uuid::new_v4()
    ));
    write_with_facts(&path, options, identity, input);
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    replay::campaign::Replay::open(bytes).unwrap()
}

pub fn source_with_content(
    frames: Vec<DecodedFrame>,
    root: &std::path::Path,
) -> (crate::Source, crate::products::LoadContext) {
    source_with_mods(frames, root, &[])
}

pub fn source_with_mods(
    frames: Vec<DecodedFrame>,
    root: &std::path::Path,
    mods: &[&str],
) -> (crate::Source, crate::products::LoadContext) {
    std::fs::create_dir_all(root.join("user")).unwrap();
    let launcher = serde_json::json!({"enabled_mods": mods, "disabled_dlcs": []}).to_string();
    for (path, text) in [
        ("hoi4.exe", "test"),
        (
            "user/settings.txt",
            "language=l_english force_pow2_textures=no",
        ),
        ("user/dlc_load.json", &launcher),
    ] {
        std::fs::write(root.join(path), text).unwrap();
    }
    let (options, identity) = content_identity(root);
    let mut input = records(1, frames);
    let Record::SessionStart(session) = &mut input[0] else {
        unreachable!()
    };
    session.identity = identity.clone();
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "content-fixture".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        std::sync::Arc::new(with_facts(&options, &identity, input).into_timelines(&Default::default()).unwrap()),
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

pub fn recorded_mod_defines(
    base: &str,
    changes: &str,
    files: &[(&str, &str)],
) -> (visual_mods::Machine, crate::Source, crate::products::LoadContext) {
    let machine = visual_mods::Machine::new(&uuid::Uuid::new_v4().to_string());
    machine.write("common/defines/base.lua", base.as_bytes());
    machine.write_mod("probe", "common/defines/z_probe.lua", changes.as_bytes());
    for &(path, text) in files {
        machine.write_mod("probe", path, text.as_bytes());
    }
    let (source, load) = source_with_mods(vec![source(1, 100, Some(1))], &machine.root, &["mod/probe.mod"]);
    (machine, source, load)
}

pub fn materialized_frame(panel: &mut dyn dashboard_follow::panel::Panel, replay: &replay::campaign::Replay, branch: u64, hour: i32) -> dashboard_follow::panel::PanelData {
    use dashboard_follow::live::GroupState;
    let replay::campaign::SeekResult::Frame(frame) = replay.seek_selected(branch, hour, Some(panel.tables())).unwrap() else { panic!("missing fixture frame at {hour}") };
    let mut data = panel.data(&frame)?;
    if let Some(live) = panel.live() {
        let states = live.values(&frame).into_iter().map(GroupState::Values).collect::<Vec<_>>();
        data["live"] = live.json(&states);
    }
    Ok(data)
}

pub struct Played {
    pub hour: i32,
    pub rows: u64,
    pub visited: u64,
    pub product_calls: u64,
    pub pending: bool,
    pub value: std::sync::Arc<dashboard_follow::panel::PanelData>,
}

thread_local! {
    static PRODUCT_CALLS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

pub(crate) fn product_called() {
    PRODUCT_CALLS.set(PRODUCT_CALLS.get() + 1);
}

pub fn played(replay: &replay::campaign::Replay, exact: &dashboard_follow::panel::AtHour, first: i32, to: i32) -> Vec<Played> {
    use playhead::pages::{Pages, Span, WorldPages};
    use std::sync::{Arc, atomic::AtomicBool};
    let pages: Arc<dyn Pages> = Arc::new(WorldPages::over(replay_recording::columnar_frames::Recording::open(replay.bytes().to_vec()).unwrap(), Span::Whole).unwrap());
    let seed = (exact.advance)(first, true, &AtomicBool::new(false), None);
    assert!(seed.rows > 0, "the seed reads populated rows");
    let (send, played) = std::sync::mpsc::channel();
    let advance = exact.advance.clone();
    let hook: playhead::Hook = Box::new(move |step, lanes| {
        PRODUCT_CALLS.set(0);
        let (read, visited) = crate::products::at_hour::counted(|| advance(step.last(), step.reset(), &AtomicBool::new(false), Some(dashboard_follow::panel::ProductStep { step, lanes, held: None, wake: None })));
        let _ = send.send(Played { hour: step.last(), rows: read.rows, visited, product_calls: PRODUCT_CALLS.get(), pending: read.pending, value: read.value });
    });
    let settings = playhead::Settings { step_threads: 1, fetch_threads: 1, ahead: 1, ring: 3, speed: 1.0, rate: 1.0, ..playhead::Settings::default() };
    let playhead = playhead::Playhead::new(Arc::clone(&pages), playhead::Timeline::whole(&*pages).unwrap(), settings, Some(hook)).unwrap();
    let reader = playhead.declare(seed.lanes, None).unwrap();
    let mut consumer = playhead.consumer(reader);
    playhead.seek(first);
    let (mut head, mut out) = (first, Vec::new());
    loop {
        let mut next = None;
        playhead
            .drive_until(&mut consumer, head, || match played.try_recv() {
                Ok(item) => {
                    next = Some(item);
                    std::ops::ControlFlow::Break(Ok(()))
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => std::ops::ControlFlow::Continue(()),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => std::ops::ControlFlow::Break(Err("step hook channel disconnected".into())),
            })
            .unwrap();
        let read: Played = next.unwrap();
        let hour = read.hour;
        out.push(read);
        if hour >= to { break; }
        if hour >= head { head = (head + 1).min(to); }
    }
    drop(consumer);
    playhead.finish();
    out
}

pub mod snow_content;
pub mod visual_mods;
