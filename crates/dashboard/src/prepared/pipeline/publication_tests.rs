use super::*;
use crate::prepared::driver::publication::{Family, Hours, Made, Publisher};
use crate::products::regional_weather;
use dashboard_base::{
    prepared::{prefix, published, roots::Binding},
    test_fixtures::{CAMPAIGN, file_source, records, world_frame},
};
use dashboard_keyed::{native, query::Query};
use dashboard_series::Cell;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};

struct Fixture {
    source: Source,
    root: PathBuf,
    store: Arc<store::Store>,
    key: store::SourceKey,
}

impl Fixture {
    fn new() -> Self {
        install_gates();
        let replay = replay::campaign::Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![world_frame(0, false)])],
        )
        .unwrap();
        let (source, root) = file_source(replay);
        let (store, key) = crate::prepared::files(&source, None).unwrap();
        Self {
            source,
            root,
            store,
            key,
        }
    }

    fn publish(&self, families: &[Family], observer: &[u8]) {
        self.publish_with(families, observer, |_| {});
    }

    fn publish_with(
        &self,
        families: &[Family],
        observer: &[u8],
        pages: impl Fn(&mut store::Writer),
    ) {
        let (publisher, _) = Publisher::open(
            &self.store,
            self.key.clone(),
            Binding::new(b"publication query", b"no content"),
            plan::native(),
            families,
            &|_, _, _| true,
        )
        .unwrap();
        for family in families {
            let mut stage = publisher.stage(family.id).unwrap();
            pages(stage.pages().unwrap());
            let series = stage.series(1, 0).unwrap();
            let text = series
                .blob(&mut &observer[..], observer.len() as u64)
                .unwrap();
            series
                .catalogue(native::OBSERVER, Cell::Text(text))
                .unwrap();
            stage
                .processed(
                    Hours {
                        branch: 1,
                        run: 0,
                        first: 0,
                        last: 0,
                        first_hour: 0,
                        last_hour: 0,
                        ends_segment: true,
                        ends_branch: true,
                        proof: [0; 32],
                    },
                    Made::Prepared,
                    None,
                    None,
                )
                .unwrap();
            stage.submit().unwrap();
        }
    }

    fn query<T>(&self, read: impl FnOnce(&Query<'_>) -> T) -> T {
        let cancelled = AtomicBool::new(false);
        let params = json!({"branchId":"1","show":"temperature"});
        let prefix =
            prefix::read_or_empty(&self.source, None, "regional-weather", 1, None, &cancelled)
                .unwrap();
        read(&Query {
            source: &self.source,
            params: &params,
            content: None,
            store: &self.store,
            key: &self.key,
            branch: 1,
            prefix: &prefix,
            cancelled: &cancelled,
        })
    }

    fn weather(&self) -> Value {
        self.query(|query| {
            let mut reply = regional_weather::query(query).unwrap();
            query.prefix.mark(&mut reply).unwrap();
            reply
        })
    }
}

#[test]
fn standalone_readers_check_the_owner_before_decoding_catalogues_or_replies() {
    use dashboard_base::prepared::series;
    let current = Family::planned(plan::native(), &["casualties"]).unwrap()[0];
    for (stamp, version, accepted) in [
        (current.stamp ^ 1, current.version, false),
        (current.stamp, current.version - 1, false),
        (current.stamp, current.version, true),
    ] {
        let fixture = Fixture::new();
        fixture.publish_with(
            &[Family {
                stamp,
                version,
                ..current
            }],
            b"unused",
            |pages| {
                series::write_catalogue(pages, "casualty", 1, &BTreeMap::new(), &"old catalogue")
                    .unwrap();
                pages
                    .write_at("casualty-pairs-1", 0, b"invalid compression".as_slice())
                    .unwrap();
            },
        );
        let read = series::read_catalogue::<u64>(&fixture.store, &fixture.key, "casualty", 1);
        let shared = series::read_shared_catalogue::<u64>(
            &fixture.store,
            &fixture.key,
            "casualty",
            1,
        );
        let reply = series::read_branch_reply::<Value>(
            &fixture.store,
            &fixture.key,
            "casualty-pairs-1",
            0,
            1,
        );
        if accepted {
            assert!(read.is_err());
            assert!(shared.is_err());
            assert!(reply.is_err());
            assert_eq!(
                series::read_catalogue::<String>(&fixture.store, &fixture.key, "casualty", 1)
                    .unwrap()
                    .unwrap()
                    .1,
                "old catalogue",
            );
        } else {
            assert!(read.unwrap().is_none());
            assert!(shared.unwrap().is_none());
            assert!(reply.unwrap().is_none());
        }
    }
}

#[test]
fn standalone_job_pages_accept_only_the_current_job_version() {
    use dashboard_base::prepared::series;
    let current = Family::planned(plan::native(), &["land-owners"]).unwrap()[0];
    for version in [current.version - 1, current.version] {
        let fixture = Fixture::new();
        fixture.publish_with(&[Family { version, ..current }], b"unused", |pages| {
            series::write_reply(pages, "land-owners-1", 0, &"current job").unwrap();
        });
        assert_eq!(
            series::read_branch_reply::<String>(
                &fixture.store,
                &fixture.key,
                "land-owners-1",
                0,
                1
            )
            .unwrap(),
            (version == current.version).then(|| "current job".to_owned()),
        );
    }
}

#[test]
fn stale_casualty_catalogues_leave_pairs_and_breakdown_pending() {
    use dashboard_base::prepared::series;
    let current = Family::planned(plan::native(), &["casualties"]).unwrap()[0];
    for window in [Value::Null, json!(0)] {
        let fixture = Fixture::new();
        let requests = [
            (
                "casualty-pairs",
                json!({"branchId":"1","tags":[],"between":false,"window":window}),
            ),
            (
                "casualty-breakdown",
                json!({"branchId":"1","tag":"GER","window":window}),
            ),
        ];
        let query = |product: &str, params: &Value| {
            let request = crate::transport::ProductRequest {
                source_id: fixture.source.key().source_id.clone(),
                revision: fixture.source.key().revision.clone(),
                product: product.into(),
                params: params.clone(),
            };
            let reply = crate::prepared::query::query(
                &fixture.source,
                &request,
                None,
                &AtomicBool::new(false),
            )
            .unwrap();
            serde_json::from_str::<Value>(&reply).unwrap()
        };
        let cold: Vec<_> = requests
            .iter()
            .map(|(product, params)| query(product, params))
            .collect();
        fixture.publish_with(
            &[Family {
                version: current.version - 1,
                ..current
            }],
            b"unused",
            |pages| {
                series::write_catalogue(pages, "casualty", 1, &BTreeMap::new(), &"old catalogue")
                    .unwrap();
                pages
                    .write_at("casualty-pairs-1", 0, b"invalid compression".as_slice())
                    .unwrap();
                pages
                    .write_at("casualty-coverage-1", 0, b"invalid series".as_slice())
                    .unwrap();
            },
        );
        for ((product, params), cold) in requests.iter().zip(cold) {
            let reply = query(product, params);
            assert_eq!(reply["data"], cold["data"], "{product}");
            assert_eq!(reply["data"]["coverage"]["complete"], false, "{reply}");
            assert_eq!(
                reply["data"]["pending"],
                json!([{"firstHour":0,"lastHour":0}])
            );
            assert_eq!(reply["data"]["observed"], json!([]));
            assert!(
                reply["gate"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("casualties"))
            );
        }
        let context = crate::products::LoadContext { content: None };
        assert!(
            crate::products::war_casualties::breakdown::prepared::query(
                &fixture.source,
                &crate::products::war_casualties::breakdown::CasualtyBreakdownParams {
                    branch_id: "1".into(),
                    tag: "GER".into(),
                    window: None,
                },
                &context,
                &fixture.store,
                &fixture.key,
                &|_, _, _| panic!("a stale catalogue must not reach the wars reader"),
                &AtomicBool::new(false),
            )
            .unwrap()
            .is_none()
        );
        assert!(
            crate::products::war_casualties::prepared::query(
                "series-state",
                json!({"branchId":"1","tags":[],"family":{},"reads":[{"tag":"GER","hour":0}]}),
                &fixture.store,
                &fixture.key,
                &AtomicBool::new(false),
            )
            .unwrap()
            .is_null()
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.store.clear();
        let _ = std::fs::remove_dir_all(self.store.directory());
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn weather_version_three_is_unprepared_without_deserializing_its_observer() {
    let fixture = Fixture::new();
    let cold = fixture.weather();
    let current = registry::family_named("regional-weather").unwrap();
    assert_eq!(current.version, 4);
    let old = Box::leak(Box::new(registry::Descriptor {
        version: 3,
        ..*current
    }));
    let old_plan = plan::test_native(&[old], &crate::prepared::driver::native::names()).unwrap();
    let family = Family::planned(old_plan, &["regional-weather"]).unwrap()[0];
    let observer = br#"{"temperature":{"status":"available","content_warnings":[],"refused":{}}}"#;
    let error = serde_json::from_slice::<regional_weather::block::Observer>(observer)
        .err()
        .unwrap();
    assert!(error.to_string().contains("missing field `empty_regions`"));
    fixture.publish(&[family], observer);

    let reply = fixture.weather();
    assert_eq!(reply, cold);
    assert_eq!(reply["coverage"]["complete"], false);
    assert_eq!(reply["coverage"]["lastHour"], -1);
    assert_eq!(reply["pending"], json!([{"firstHour":0,"lastHour":0}]));
    for field in ["known", "reads", "series"] {
        assert_eq!(reply["window"][field], json!([]));
    }
    assert_ne!(reply["temperature"], "available");
    let request = crate::transport::ProductRequest {
        source_id: fixture.source.key().source_id.clone(),
        revision: fixture.source.key().revision.clone(),
        product: "regional-weather".into(),
        params: json!({"branchId":"1","show":"temperature"}),
    };
    let response =
        crate::prepared::query::query(&fixture.source, &request, None, &AtomicBool::new(false))
            .unwrap();
    let response: Value = serde_json::from_str(&response).unwrap();
    assert_eq!(response["data"], cold);
    assert_eq!(response["gate"], json!(["regional-weather"]));
}

#[test]
fn native_open_checks_both_stamp_and_frontier_version_before_deserialization() {
    let current = Family::planned(plan::native(), &["regional-weather"]).unwrap()[0];
    for (stamp, version) in [
        (current.stamp ^ 1, current.version),
        (current.stamp, current.version - 1),
    ] {
        let fixture = Fixture::new();
        fixture.publish(
            &[Family {
                stamp,
                version,
                ..current
            }],
            b"invalid observer JSON",
        );
        fixture.query(|query| {
            assert!(
                query
                    .open_native::<regional_weather::block::Observer>()
                    .unwrap()
                    .is_none()
            );
        });
    }
    let fixture = Fixture::new();
    fixture.publish(&[current], b"invalid observer JSON");
    fixture.query(|query| {
        assert!(
            query
                .open_native::<regional_weather::block::Observer>()
                .is_err()
        );
    });
    let fixture = Fixture::new();
    let observer = serde_json::to_vec(&regional_weather::block::Observer::default()).unwrap();
    fixture.publish(&[current], &observer);
    fixture.query(|query| {
        assert!(
            query
                .open_native::<regional_weather::block::Observer>()
                .unwrap()
                .is_some()
        );
    });
}

#[test]
fn every_family_and_job_rejects_a_stale_native_publication() {
    let fixture = Fixture::new();
    let names: Vec<_> = registry::FAMILIES
        .iter()
        .map(|family| family.id.name())
        .chain(registry::JOBS.iter().map(|job| job.name))
        .collect();
    let mut families = Family::planned(plan::native(), &names).unwrap();
    fixture.publish(&families, b"unused");
    let current = current_gates(&fixture.source, None).unwrap().unwrap();
    for &name in &names {
        let root = current
            .gate(1, name)
            .unwrap()
            .publication()
            .unwrap()
            .unwrap();
        assert!(accepts_publication(name, root), "{name}");
        let mut old = root.clone();
        old.frontier.version = old.frontier.version.saturating_sub(1);
        assert!(!accepts_publication(name, &old), "{name}");
    }
    for family in &mut families {
        family.stamp ^= 1;
    }
    fixture.publish(&families, b"invalid observer JSON");
    for name in names {
        assert!(
            published::open(&fixture.source, None, name, native::SCHEMA, 1)
                .unwrap()
                .is_none(),
            "{name}"
        );
        assert!(
            published::open_through(&fixture.source, None, name, native::SCHEMA, 1, Some(0))
                .unwrap()
                .is_none(),
            "{name}"
        );
    }
}

#[test]
fn rootless_refusal_pages_are_still_accepted() {
    let fixture = Fixture::new();
    let page = CoveragePage {
        coverage: Coverage {
            schema: SCHEMA,
            first_hour: 0,
            last_hour: -1,
            complete: true,
        },
        refused: BTreeMap::from([(
            "casualties".to_owned(),
            Refusal {
                hour: 0,
                error: "Unsupported synthetic casualties".into(),
                from: None,
            },
        )]),
    };
    let mut writer = fixture.store.replace(fixture.key.clone()).unwrap();
    writer
        .write_at(
            &registry::coverage_section("casualties", 1),
            0,
            serde_json::to_vec(&page).unwrap().as_slice(),
        )
        .unwrap();
    writer.commit().unwrap();
    let published = current_gates(&fixture.source, None).unwrap().unwrap();
    assert!(own_root(&published, "casualties", 1).unwrap().is_none());
    assert_eq!(
        own_published(&published, "casualties", 1).unwrap().unwrap(),
        &page
    );
    assert!(
        refusal_from(&published, 1, "casualty-pairs")
            .unwrap()
            .is_some()
    );
}
