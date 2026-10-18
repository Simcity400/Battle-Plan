//! Prepared query timings and store work on a recording prepared elsewhere.
use super::*;
use std::{collections::BTreeMap, path::PathBuf, time::Instant};

/// Runs every prepared request the readiness widget table issues, twice, and
/// prints each one's time, reply size and store work. The prepared store must be
/// complete for this recording and content: point LOCALAPPDATA at a copy of one
/// a readiness run prepared (it is only read here).
#[test]
#[ignore = "requires BATTLE_PLAN_GAME_DIR, BATTLE_PLAN_USER_DIR, BATTLE_PLAN_QUERY_PROFILE_RECORDING and a prepared LOCALAPPDATA"]
fn readiness_prepared_queries_time_and_store_work() {
    let required =
        |name: &str| PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("Set {name}")));
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let recording = std::fs::canonicalize(required("BATTLE_PLAN_QUERY_PROFILE_RECORDING")).unwrap();
    let only = std::env::var("BATTLE_PLAN_QUERY_PROFILE_ONLY").or_else(|_| std::env::var("HOI4_QUERY_PROFILE_ONLY")).ok();
    let cancelled = AtomicBool::new(false);
    let revision = crate::protocol::replay_revision(&recording).unwrap();
    let source = Source::open(
        crate::SourceKey {
            source_id: "query-profile".into(),
            revision: revision.clone(),
        },
        recording,
        &cancelled,
    )
    .unwrap();
    let config = crate::config::Config {
        game_dir: required("BATTLE_PLAN_GAME_DIR"),
        user_dir: required("BATTLE_PLAN_USER_DIR"),
        ..Default::default()
    };
    let content = ContentOptions {
        options: config.resolve_options().unwrap(),
        allow_unverified: false,
    };
    let restored = super::super::pipeline::restore(&source, Some(&content), &cancelled).unwrap();
    let (store, key) = super::super::files(&source, Some(&content)).unwrap();
    let saved = store.saved_key().unwrap();
    eprintln!(
        "restored {}, reusable {}, saved key {}",
        restored.is_some(),
        store.is_reusable(&key).unwrap(),
        saved.is_some()
    );
    if let Some(saved) = saved.filter(|saved| *saved != key) {
        // Name the first input whose stamp differs from the prepared store's.
        let (saved, key) = (
            serde_json::to_value(&saved).unwrap(),
            serde_json::to_value(&key).unwrap(),
        );
        eprintln!("recording {} / {}", saved["recording"], key["recording"]);
        let (saved, key) = (
            saved["content"].as_array().unwrap(),
            key["content"].as_array().unwrap(),
        );
        eprintln!("content inputs {} / {}", saved.len(), key.len());
        if let Some(index) = (0..saved.len().min(key.len())).find(|&i| saved[i] != key[i]) {
            eprintln!("input {index}: {} / {}", saved[index], key[index]);
        }
        panic!("LOCALAPPDATA must hold this recording's prepared store");
    }
    let metadata = source.metadata();
    let branch = &metadata.branches[0];
    let tag = metadata
        .countries
        .iter()
        .find(|country| Some(&country.id) == metadata.player_country_id.as_ref())
        .or_else(|| metadata.countries.first())
        .unwrap()
        .tag
        .clone();
    let bindings = crate::readiness::Bindings {
        branch: branch.id.clone(),
        tag,
        first: branch.first_hour,
        last: branch.last_hour,
        width: 800,
    };
    let widgets = crate::readiness::widgets(crate::readiness::WIDGETS).unwrap();
    let mut total = [0f64; 2];
    for widget in &widgets {
        let mut replies = BTreeMap::new();
        for (index, template) in widget.requests.iter().chain(&widget.auxiliary).enumerate() {
            if template.route != crate::readiness::Route::Prepared {
                continue;
            }
            let label = format!(
                "{} {} {}",
                widget.kind,
                template.product,
                template
                    .params
                    .get("family")
                    .and_then(|family| family.get("kind"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
            );
            if only
                .as_ref()
                .is_some_and(|only| !label.contains(only.as_str()))
            {
                continue;
            }
            let crate::readiness::Instantiated::Ready(params) =
                crate::readiness::instantiate(&template.params, &bindings, &replies).unwrap()
            else {
                continue;
            };
            let request = ProductRequest {
                source_id: "query-profile".into(),
                revision: revision.clone(),
                product: template.product.clone(),
                params,
            };
            for (round, total) in total.iter_mut().enumerate() {
                let before = store::counters::now();
                let started = Instant::now();
                let reply = query(&source, &request, Some(&content), &cancelled);
                let spent = started.elapsed().as_secs_f64() * 1000.0;
                let work = store::counters::now() - before;
                *total += spent;
                let outcome = match &reply {
                    Ok(text) => format!("{} bytes", text.len()),
                    Err(error) => format!("error {error}"),
                };
                eprintln!(
                    "query {round} {label}: {spent:.1} ms, {outcome}, stats {} manifests {} pack opens {}",
                    work.stats, work.manifest_reads, work.pack_opens
                );
                if let Ok(text) = &reply
                    && round == 0
                {
                    let mut value: Value = serde_json::from_str(text).unwrap();
                    replies.insert(index, value.get_mut("data").map(Value::take).unwrap());
                }
            }
        }
    }
    eprintln!("total: first {:.0} ms, again {:.0} ms", total[0], total[1]);
}
