//! A source's lazy content warnings, as the product route serves them.

use crate::test_fixtures::{CAMPAIGN, records, source};
use replay::container::Replay;
use std::sync::{Arc, atomic::AtomicBool};

pub(crate) const DIAGNOSTIC_PRODUCTS: [&str; 2] = ["diagnostic-a", "diagnostic-b"];

#[derive(serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
struct DiagnosticParams {
    branch_id: String,
    #[serde(default)]
    selection: u32,
}

#[derive(serde::Serialize, ts_rs::TS)]
struct DiagnosticReply {
    value: String,
    warnings: Vec<String>,
}

impl super::ReplyWarnings for DiagnosticReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.warnings
    }
}

#[derive(Default)]
struct DiagnosticCache<const INDEX: usize> {
    value: crate::catalogue::CachedResult<Vec<u8>>,
    loads: std::sync::atomic::AtomicUsize,
}

struct DiagnosticProduct<const INDEX: usize>;

impl<const INDEX: usize> super::Product for DiagnosticProduct<INDEX> {
    const KEY: &'static str = DIAGNOSTIC_PRODUCTS[INDEX];
    const CONTENT_DEPENDENT: bool = true;
    const MACHINE_DEPENDENT: bool = true;
    type Params = DiagnosticParams;
    type Reply = DiagnosticReply;

    fn load(
        source: &crate::Source,
        params: &Self::Params,
        context: &super::LoadContext,
        _: &AtomicBool,
    ) -> std::io::Result<Self::Reply> {
        let cache = source.state::<DiagnosticCache<INDEX>>();
        let cache = cache.lock().unwrap();
        let bytes = cache.value.get_or_init(|| {
            cache
                .loads
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let content = source.content(&context.content.as_ref().unwrap().options)?;
            std::thread::scope(|scope| {
                installed_content_core::parallel::join(installed_content_core::parallel::spawn(scope, || {
                    content
                        .layers
                        .note_shader_warning(format!("synthetic shader {INDEX}"));
                    content
                        .layers
                        .read_uncovered(&format!("outside_policy/{INDEX}.bin"))
                }))
            })
        })?;
        let gate = source
            .state::<Option<Arc<std::sync::Barrier>>>()
            .lock()
            .unwrap()
            .clone();
        if let Some(gate) = gate {
            gate.wait();
        }
        Ok(DiagnosticReply {
            value: format!(
                "{}:{}:{}",
                String::from_utf8_lossy(&bytes),
                params.branch_id,
                params.selection
            ),
            warnings: Vec::new(),
        })
    }
}

pub(crate) struct DiagnosticFixture {
    folder: crate::catalogue::tests::Folder,
    pub context: super::LoadContext,
}

impl DiagnosticFixture {
    pub fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("content-diagnostics-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("outside_policy")).unwrap();
        for index in 0..2 {
            std::fs::write(
                root.join(format!("outside_policy/{index}.bin")),
                format!("value-{index}"),
            )
            .unwrap();
        }
        let context = super::LoadContext {
            content: Some(super::ContentOptions {
                options: installed_content_core::ResolveOptions {
                    game_dir: root.clone(),
                    user_dir: root.join("user"),
                    cache_dir: root.join("cache"),
                },
                allow_unverified: true,
                presentation: Default::default(),
            }),
        };
        Self {
            folder: crate::catalogue::tests::Folder(root),
            context,
        }
    }

    pub fn source(&self) -> crate::Source {
        let source = crate::Source::from_replay(
            crate::SourceKey {
                source_id: "diagnostic-fixture".into(),
                revision: "1".into(),
            },
            "synthetic.battleplan".into(),
            Arc::new(crate::test_fixtures::replay(vec![source(1, 0, Some(1))])),
            &AtomicBool::new(false),
        )
        .unwrap();
        let layers =
            installed_content_core::ResolvedLayers::from_folders(&[(&self.folder.0, &[])]).unwrap();
        source.set_test_content(
            self.context.content.as_ref().unwrap().options.clone(),
            Arc::new(installed_content_core::content::ResolvedContent {
                layers,
                stats: Default::default(),
                changed: Default::default(),
            }),
        );
        source
    }
}

pub(crate) fn diagnostic_reply(
    source: &crate::Source,
    context: &super::LoadContext,
    index: usize,
    params: serde_json::Value,
) -> String {
    match index {
        0 => super::load_json::<DiagnosticProduct<0>>(
            source,
            "diagnostic-fixture",
            params,
            context,
            &AtomicBool::new(false),
        ),
        1 => super::load_json::<DiagnosticProduct<1>>(
            source,
            "diagnostic-fixture",
            params,
            context,
            &AtomicBool::new(false),
        ),
        _ => unreachable!(),
    }
    .unwrap()
}

pub(crate) fn diagnostic_serve(
    source: &crate::Source,
    context: &super::LoadContext,
    product: &str,
    params: &serde_json::Value,
) -> crate::prepared::served_check::Reply {
    let index = DIAGNOSTIC_PRODUCTS
        .iter()
        .position(|name| *name == product)
        .unwrap();
    let serde_json::Value::Object(mut reply) =
        serde_json::from_str(&diagnostic_reply(source, context, index, params.clone())).unwrap()
    else {
        panic!("object reply")
    };
    for key in ["sourceId", "revision", "product"] {
        reply.remove(key);
    }
    crate::prepared::served_check::Reply::Answered(serde_json::Value::Object(reply))
}

#[test]
fn products_report_only_their_content_diagnostics_on_cold_and_warm_shared_sources() {
    use super::Product;
    let fixture = DiagnosticFixture::new();
    let params = || serde_json::json!({"branchId":"1"});
    let alone: Vec<_> = (0..2)
        .map(|index| diagnostic_reply(&fixture.source(), &fixture.context, index, params()))
        .collect();
    for index in 0..2 {
        assert!(alone[index].contains(&format!("synthetic shader {index}")));
        assert!(alone[index].contains(&format!("outside_policy/{index}.bin")));
        assert!(!alone[index].contains(&format!("outside_policy/{}.bin", 1 - index)));
    }
    for warm in [false, true] {
        for order in [[0, 1], [1, 0]] {
            let source = fixture.source();
            if warm {
                DiagnosticProduct::<0>::load(
                    &source,
                    &DiagnosticParams {
                        branch_id: "1".into(),
                        selection: 0,
                    },
                    &fixture.context,
                    &AtomicBool::new(false),
                )
                .unwrap();
                DiagnosticProduct::<1>::load(
                    &source,
                    &DiagnosticParams {
                        branch_id: "1".into(),
                        selection: 0,
                    },
                    &fixture.context,
                    &AtomicBool::new(false),
                )
                .unwrap();
            }
            for _ in 0..2 {
                for index in order {
                    assert_eq!(
                        diagnostic_reply(&source, &fixture.context, index, params()),
                        alone[index]
                    );
                }
            }
            assert_eq!(
                source
                    .state::<DiagnosticCache<0>>()
                    .lock()
                    .unwrap()
                    .loads
                    .load(std::sync::atomic::Ordering::Relaxed),
                1
            );
            assert_eq!(
                source
                    .state::<DiagnosticCache<1>>()
                    .lock()
                    .unwrap()
                    .loads
                    .load(std::sync::atomic::Ordering::Relaxed),
                1
            );
            assert!(
                !source.metadata().unwrap().warnings.iter().any(|line| line
                    .contains("outside_policy")
                    || line.contains("synthetic shader"))
            );
        }
    }
    let source = fixture.source();
    *source
        .state::<Option<Arc<std::sync::Barrier>>>()
        .lock()
        .unwrap() = Some(Arc::new(std::sync::Barrier::new(2)));
    std::thread::scope(|scope| {
        let a = scope.spawn(|| diagnostic_reply(&source, &fixture.context, 0, params()));
        let b = scope.spawn(|| diagnostic_reply(&source, &fixture.context, 1, params()));
        assert_eq!(a.join().unwrap(), alone[0]);
        assert_eq!(b.join().unwrap(), alone[1]);
    });
}

#[test]
fn unrelated_lazy_reads_do_not_reach_metadata_catalogue_or_other_products() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::products::{ContentOptions, LoadContext};
    use replay::spool::Record;
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
            "common/defines/a.lua",
            "NDefines.NGame.MAX_SCRIPTED_LOC_RECURSION = 32",
        ),
    ]);
    std::fs::create_dir(folder.0.join("dlc")).unwrap();
    let options = ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir: folder.0.clone(),
            user_dir: folder.0.join("user"),
            cache_dir: folder.0.join("cache"),
        },
        allow_unverified: true,
        presentation: Default::default(),
    };
    // At recording time only policy-2 paths exist, so this is its exact hash.
    let gathered = installed_content_core::gather(&installed_content_core::GatherOptions {
        game_dir: options.options.game_dir.clone(),
        user_dir: options.options.user_dir.clone(),
        cache_dir: options.options.cache_dir.clone(),
        dlc_order: Some(Vec::new()),
    })
    .unwrap();
    let mut identity = gathered
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
    identity.coverage_policy = 2;
    for (path, text) in [
        (
            "events/a.txt",
            "add_namespace=s country_event={id=s.1 title=TITLE}",
        ),
        ("gfx/texticons/late.dds", "synthetic icon bytes"),
    ] {
        let path = folder.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let mut input = records(1, vec![source(1, 100, Some(1))]);
    let Record::SessionStart(session) = &mut input[0] else {
        unreachable!()
    };
    session.identity = identity;
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "policy-two".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[input]).unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap();
    let catalogue = crate::catalogue::catalogue(&source, &options).unwrap();
    assert!(
        source
            .metadata().unwrap()
            .warnings
            .iter()
            .all(|note| !note.starts_with("Content: "))
    );
    let layers = &catalogue.content.layers;
    // The event reader loads headers/body lazily, after the catalogue was built.
    let events = installed_content_events::events::LazyEvents::new(Arc::new(layers.clone()));
    assert!(events.get(b"s.1").unwrap().refused.is_none());
    let expected = layers.read_warnings().warning().unwrap();
    assert!(expected.contains("Content: 1 file(s)"));
    assert!(
        expected.contains("coverage policy 2; these bytes are not hash-verified: events/a.txt")
    );
    assert!(!catalogue.refusal_lines().contains(&expected));
    assert!(!source.metadata().unwrap().warnings.contains(&expected));
    let context = LoadContext {
        content: Some(options),
    };
    let reply = || {
        let json = (crate::products::oracle_route("country-fuel").unwrap().load)(
            &source,
            "lease",
            serde_json::json!({"branchId":"1", "tag":"GER"}),
            &context,
            &AtomicBool::new(false),
        )
        .unwrap();
        serde_json::from_str::<serde_json::Value>(&json).unwrap()
    };
    assert!(
        !reply()["data"]["warnings"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!(expected))
    );
    layers.read("gfx/texticons/late.dds").unwrap();
    let updated = layers.read_warnings().warning().unwrap();
    assert!(updated.contains("Content: 2 file(s)"));
    assert!(!catalogue.refusal_lines().contains(&updated));
    assert!(!source.metadata().unwrap().warnings.contains(&updated));
    let cached = reply();
    let notes: Vec<_> = cached["data"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|note| {
            note.as_str()
                .is_some_and(|note| note.starts_with("Content: "))
        })
        .collect();
    assert!(notes.is_empty());
    assert!(!folder.0.join("cache").join("history").exists());
}
