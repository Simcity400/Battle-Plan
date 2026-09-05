//! A focus localises under its `text = <key>` when it declares one, never
//! under its id.

use game_ui::static_research::StaticResearchUiCatalog;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

const CONTENT_DIGEST: &str =
    "3131313131313131313131313131313131313131313131313131313131313131";

fn canonical(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            let mut ordered = Map::new();
            for key in keys {
                ordered.insert(key.clone(), canonical(&object[key]));
            }
            Value::Object(ordered)
        }
        other => other.clone(),
    }
}

fn packaged(name: &str, path: &str, mut semantic: Value) -> replay_core::PackagedStaticArtifact {
    let mut covered = semantic.clone();
    covered
        .as_object_mut()
        .expect("artifact root is an object")
        .remove("artifact_sha256");
    let embedded = snapshot::sha256::Sha256::hex(
        &serde_json::to_vec(&canonical(&covered)).expect("canonical artifact serialises"),
    );
    semantic["artifact_sha256"] = json!(embedded);
    let bytes = serde_json::to_vec(&semantic).expect("artifact serialises");
    let container = snapshot::sha256::Sha256::hex(&bytes);
    replay_core::PackagedStaticArtifact::unauthenticated_for_consumer_tests(
        name,
        path,
        &container,
        CONTENT_DIGEST,
        bytes,
    )
}

fn style() -> Value {
    json!({
        "name": "default_style",
        "default": true,
        "unavailable": "GFX_focus_unavailable",
        "available": "GFX_focus_can_start",
        "current": "GFX_focus_current",
        "completed": "GFX_focus_completed",
        "source_file": "common/national_focus/uk.txt",
        "source_layer": "vanilla",
    })
}

fn focus_row(id: &str, title_loc_key: &str) -> Value {
    json!({
        "id": id,
        "kind": "focus",
        "title_loc_key": title_loc_key,
        "description_loc_key": format!("{title_loc_key}_desc"),
        "mutually_exclusive": [],
        "scripted_position_offsets": 0,
        "allow_branch_blocks": 0,
        "background_style_name": "default_style",
        "background_style": style(),
        "source_file": "common/national_focus/uk.txt",
        "source_layer": "vanilla",
    })
}

fn continuous_focus_row(id: &str) -> Value {
    json!({
        "id": id,
        "kind": "continuous_focus",
        "palette_id": "generic_focus",
        "title_loc_key": id,
        "description_loc_key": format!("{id}_desc"),
        "title_loc_key_from_text_override": false,
        "icon": format!("GFX_goal_{id}"),
        "has_available": true,
        "has_enable": false,
        "modifier_blocks": 1,
        "source_file": "common/continuous_focus/generic.txt",
        "source_layer": "vanilla",
    })
}

/// `overrides` is what the artifact DECLARES; callers pass a wrong roster to
/// exercise the admission cross-check.
fn focus_defs_artifact(rows: &[(&str, &str)], overrides: &[&str]) -> Value {
    focus_defs_artifact_with_continuous(rows, overrides, &["continuous_non_factory_construct"])
}

fn focus_defs_artifact_with_continuous(
    rows: &[(&str, &str)],
    overrides: &[&str],
    continuous_ids: &[&str],
) -> Value {
    let mut focuses = Map::new();
    for (id, title) in rows {
        focuses.insert((*id).to_string(), focus_row(id, title));
    }
    let mut continuous = Map::new();
    for id in continuous_ids {
        continuous.insert((*id).to_string(), continuous_focus_row(id));
    }
    json!({
        "schema": "focus_defs_static.static_artifact.v1",
        "artifact_kind": "deterministic_static_replay_enrichment",
        "artifact_sha256": "",
        "meta": {
            "tree_count": 1,
            "focus_count": rows.len(),
            "shared_focus_count": 0,
            "joint_focus_count": 0,
            "continuous_focus_count": continuous_ids.len(),
            "continuous_focus_palette_count": 1,
            "focus_kind_counts": {
                "focus": rows.len(),
                "shared_focus": 0,
                "joint_focus": 0,
                "continuous_focus": continuous_ids.len(),
            },
            "focus_background_style_count": 1,
            "focus_text_loc_key_overrides": overrides,
            "default_focus_background_styles": ["default_style"],
            "unresolved_focus_background_styles": [],
            "id_collisions_last_wins": [],
            "files_parsed": 2,
            "national_files_parsed": 1,
            "continuous_files_parsed": 1,
        },
        "trees": {
            "uk_tree": {
                "id": "uk_tree",
                "focus_count": rows.len(),
                "focuses": rows.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
                "shared_focus_refs": [],
                "source_file": "common/national_focus/uk.txt",
                "source_layer": "vanilla",
            }
        },
        "focuses": focuses,
        "continuous_focus_palettes": {
            "generic_focus": {
                "id": "generic_focus",
                "default": true,
                "reset_on_civilwar": false,
                "focus_count": continuous_ids.len(),
                "focuses": continuous_ids,
                "source_file": "common/continuous_focus/generic.txt",
                "source_layer": "vanilla",
            }
        },
        "continuous_focuses": continuous,
        "focus_background_styles": {"default_style": style()},
        "source_files": [
            {
                "relpath": "common/national_focus/uk.txt",
                "bytes": 4096,
                "sha256": "a".repeat(64),
            },
            {
                "relpath": "common/continuous_focus/generic.txt",
                "bytes": 2048,
                "sha256": "b".repeat(64),
            },
        ],
    })
}

fn localisation_artifact(keys: &[(&str, &str)]) -> replay_core::PackagedStaticArtifact {
    let mut rows = Map::new();
    for (key, name) in keys {
        rows.insert(
            (*key).to_string(),
            json!({
                "name": name,
                "raw": name,
                "has_interpolation": false,
                "source_file": "localisation/english/focus_l_english.yml",
                "source_layer": "vanilla",
            }),
        );
    }
    packaged(
        replay_core::LOCALISATION_JOIN_ARTIFACT,
        "static/localisation_join/localisation_catalog.json",
        json!({
            "schema": "static_localisation_join_r3.static_artifact.v1",
            "artifact_kind": "deterministic_static_replay_enrichment",
            "artifact_sha256": "",
            "request": "R3",
            "keys": Value::Object(rows),
            "meta": {"key_count": keys.len()},
        }),
    )
}

fn empty_catalog_artifacts() -> (
    replay_core::PackagedStaticArtifact,
    replay_core::PackagedStaticArtifact,
) {
    let idea_technology = packaged(
        replay_core::IDEA_TECH_LOC_XREF_ARTIFACT,
        "static/idea_tech_loc_xref_static/idea_tech_loc_xref_catalog.json",
        json!({
            "schema": "idea_tech_loc_xref_static.static_artifact.v2",
            "artifact_kind": "deterministic_static_replay_enrichment",
            "artifact_sha256": "",
            "tokens": {"ideas": Map::new(), "technologies": Map::new()},
            "omitted": {"ideas": [], "technologies": []},
            "meta": {
                "idea_count": 0,
                "idea_mapped": 0,
                "idea_omitted_no_loc_key": 0,
                "tech_count": 0,
                "tech_mapped": 0,
                "tech_omitted_no_loc_key": 0,
                "tech_start_year_mapped": 0,
                "tech_start_year_absent": 0,
                "tech_start_year_unresolved_count": 0,
            },
        }),
    );
    let technology_sharing = packaged(
        replay_core::TECHNOLOGY_SHARING_ARTIFACT,
        "static/technology_sharing_static/technology_sharing_static.json",
        json!({
            "schema": "technology_sharing_static.static_artifact.v1",
            "artifact_kind": "deterministic_static_replay_enrichment",
            "artifact_sha256": "",
            "definitions": [],
            "definitions_by_id": Map::new(),
            "join_keys": {"compatibility_status": "no_runtime_join_proven"},
            "meta": {
                "definition_count": 0,
                "runtime_token_proof": "engine_runtime_intern_ordinal_not_string_hash",
                "runtime_hash_proof": "fnv1a_only_indexes_intern_table_not_runtime_token",
            },
        }),
    );
    (idea_technology, technology_sharing)
}

fn admit(
    focus_defs: Value,
    localisation_keys: &[(&str, &str)],
) -> Result<StaticResearchUiCatalog, String> {
    let definitions = replay_core::admit_static_focus_definitions(&packaged(
        replay_core::FOCUS_DEFS_STATIC_ARTIFACT,
        "static/focus_defs_static/focus_defs_static.json",
        focus_defs,
    ))
    .map_err(|error| error.to_string())?;
    let localisation = localisation_artifact(localisation_keys);
    let (idea_technology, technology_sharing) = empty_catalog_artifacts();
    game_ui::static_research::admit_static_research_catalogs(
        Some(&localisation),
        Some(&idea_technology),
        Some(&technology_sharing),
        &definitions,
    )
    .map_err(|error| error.to_string())
}

#[test]
fn a_focus_label_resolves_through_the_declared_title_key_not_the_focus_id() {
    let catalog = admit(
        focus_defs_artifact(
            &[
                ("uk_industrial_focus", "industrial_effort"),
                ("shadow_scheme_focus", "shadow_scheme_focus"),
            ],
            &["uk_industrial_focus"],
        ),
        &[
            ("industrial_effort", "Industrial Effort"),
            ("shadow_scheme_focus", "Shadow Scheme"),
        ],
    )
    .expect("the packaged catalogs are admitted");

    assert_eq!(
        catalog
            .focus_label("uk_industrial_focus")
            .expect("the overridden focus resolves through its declared title key")
            .display_name,
        "Industrial Effort"
    );
    assert_eq!(
        catalog
            .focus_label("shadow_scheme_focus")
            .expect("a focus without an override still resolves under its id")
            .display_name,
        "Shadow Scheme"
    );
}

#[test]
fn a_focus_id_the_packaged_definitions_do_not_carry_is_refused_by_name() {
    let catalog = admit(
        focus_defs_artifact(&[("shadow_scheme_focus", "shadow_scheme_focus")], &[]),
        &[("shadow_scheme_focus", "Shadow Scheme")],
    )
    .expect("the packaged catalogs are admitted");

    let error = catalog
        .focus_label("uk_industrial_focus")
        .expect_err("a runtime focus with no packaged definition has no localisation key");
    assert!(
        error.message.contains("focus_defs_static has no focus")
            && error.message.contains("uk_industrial_focus"),
        "{}",
        error.message
    );
}

#[test]
fn a_declared_title_key_the_packaged_localisation_lacks_displays_the_key_and_is_named() {
    let catalog = admit(
        focus_defs_artifact(
            &[("uk_industrial_focus", "industrial_effort")],
            &["uk_industrial_focus"],
        ),
        &[("shadow_scheme_focus", "Shadow Scheme")],
    )
    .expect("the packaged catalogs are admitted");

    let label = catalog
        .focus_label("uk_industrial_focus")
        .expect("a missed title key is displayed as a729 displays it");
    assert_eq!(label.display_name, "industrial_effort");
    assert_eq!(
        label.localisation_miss,
        Some(replay_core::LocalisationMiss::recorder_packaged(
            "industrial_effort",
            replay_core::LOCALISATION_JOIN_ARTIFACT
        ))
    );

    let labels = catalog
        .bind_focus(&replay_core::FocusView {
            focus_tree: replay_core::Leaf::Measured("uk_tree".to_string()),
            current: replay_core::Leaf::Absent,
            current_set: replay_core::Leaf::Measured(vec!["uk_industrial_focus".to_string()]),
            current_continuous: replay_core::Leaf::Absent,
            current_continuous_set: replay_core::Leaf::Absent,
            completed_count: replay_core::Leaf::Measured(0),
            progress: replay_core::Leaf::Absent,
            completed: replay_core::Leaf::Absent,
            available: replay_core::Leaf::Absent,
            visible_order: replay_core::Leaf::Unobtained,
            paused: replay_core::Leaf::Absent,
        })
        .expect("a frame naming a focus with a missed title still binds");
    let replay_core::Leaf::Measured(current) = &labels.current else {
        panic!("the current set was measured");
    };
    assert_eq!(current[0].display_name, "industrial_effort");
    assert!(current[0].localisation_miss.is_some());
}

#[test]
fn a_focus_defs_artifact_whose_override_roster_hides_a_rewritten_key_is_refused() {
    let error = admit(
        focus_defs_artifact(&[("uk_industrial_focus", "industrial_effort")], &[]),
        &[("industrial_effort", "Industrial Effort")],
    )
    .expect_err("an artifact that under-declares its overrides must be refused");
    assert!(
        error.contains("focus_text_loc_key_overrides"),
        "the refusal names the roster: {error}"
    );

    let error = admit(
        focus_defs_artifact(
            &[("shadow_scheme_focus", "shadow_scheme_focus")],
            &["shadow_scheme_focus"],
        ),
        &[("shadow_scheme_focus", "Shadow Scheme")],
    )
    .expect_err("an artifact that over-declares its overrides must be refused");
    assert!(
        error.contains("focus_text_loc_key_overrides"),
        "the refusal names the roster: {error}"
    );
}

#[test]
fn a_description_key_that_is_not_the_title_key_suffixed_is_refused() {
    let mut artifact = focus_defs_artifact(
        &[("uk_industrial_focus", "industrial_effort")],
        &["uk_industrial_focus"],
    );
    artifact["focuses"]["uk_industrial_focus"]["description_loc_key"] =
        json!("uk_industrial_focus_desc");
    let error = admit(artifact, &[("industrial_effort", "Industrial Effort")])
        .expect_err("the description key follows the title key, not the id");
    assert!(
        error.contains("malformed identity"),
        "the refusal names the row: {error}"
    );
}

#[test]
fn the_join_walks_every_focus_leaf_of_a_frame() {
    let catalog = admit(
        focus_defs_artifact(
            &[("uk_industrial_focus", "industrial_effort")],
            &["uk_industrial_focus"],
        ),
        &[("industrial_effort", "Industrial Effort")],
    )
    .expect("the packaged catalogs are admitted");

    let labels = catalog
        .bind_focus(&replay_core::FocusView {
            focus_tree: replay_core::Leaf::Measured("uk_tree".to_string()),
            current: replay_core::Leaf::Absent,
            current_set: replay_core::Leaf::Measured(vec!["uk_industrial_focus".to_string()]),
            current_continuous: replay_core::Leaf::Absent,
            current_continuous_set: replay_core::Leaf::Absent,
            completed_count: replay_core::Leaf::Measured(1),
            progress: replay_core::Leaf::Absent,
            completed: replay_core::Leaf::Measured(vec!["uk_industrial_focus".to_string()]),
            available: replay_core::Leaf::Absent,
            visible_order: replay_core::Leaf::Unobtained,
            paused: replay_core::Leaf::Absent,
        })
        .expect("a frame naming only overridden focuses still binds");
    let names = |leaf: &replay_core::Leaf<Vec<game_ui::static_research::CatalogLabel>>| {
        match leaf {
            replay_core::Leaf::Measured(rows) => {
                rows.iter().map(|row| row.display_name.clone()).collect()
            }
            _ => Vec::<String>::new(),
        }
    };
    assert_eq!(names(&labels.current), vec!["Industrial Effort".to_string()]);
    assert_eq!(
        names(&labels.completed),
        vec!["Industrial Effort".to_string()]
    );
}

#[test]
fn the_bare_catalog_keeps_the_focus_id_when_no_text_is_declared() {
    let definitions = replay_core::admit_static_focus_definitions(&packaged(
        replay_core::FOCUS_DEFS_STATIC_ARTIFACT,
        "static/focus_defs_static/focus_defs_static.json",
        focus_defs_artifact(
            &[
                ("uk_industrial_focus", "industrial_effort"),
                ("shadow_scheme_focus", "shadow_scheme_focus"),
            ],
            &["uk_industrial_focus"],
        ),
    ))
    .expect("the packaged focus definitions are admitted");

    let keys = definitions
        .definitions()
        .map(|(id, definition)| (id.to_string(), definition.title_loc_key.clone()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        keys,
        BTreeMap::from([
            (
                "shadow_scheme_focus".to_string(),
                "shadow_scheme_focus".to_string()
            ),
            (
                "uk_industrial_focus".to_string(),
                "industrial_effort".to_string()
            ),
        ])
    );
}

#[test]
fn a_current_continuous_focus_resolves_through_the_packaged_continuous_namespace() {
    let catalog = admit(
        focus_defs_artifact(&[("uk_industrial_focus", "industrial_effort")], &["uk_industrial_focus"]),
        &[
            ("industrial_effort", "Industrial Effort"),
            (
                "continuous_non_factory_construct",
                "Construction Engineering",
            ),
        ],
    )
    .expect("the packaged catalogs are admitted");

    let labels = catalog
        .bind_focus(&replay_core::FocusView {
            focus_tree: replay_core::Leaf::Absent,
            current: replay_core::Leaf::Absent,
            current_set: replay_core::Leaf::Measured(vec![]),
            current_continuous: replay_core::Leaf::Absent,
            current_continuous_set: replay_core::Leaf::Measured(vec![
                "continuous_non_factory_construct".to_string(),
            ]),
            completed_count: replay_core::Leaf::Measured(0),
            progress: replay_core::Leaf::Absent,
            completed: replay_core::Leaf::Measured(vec![]),
            available: replay_core::Leaf::Absent,
            visible_order: replay_core::Leaf::Unobtained,
            paused: replay_core::Leaf::Absent,
        })
        .expect("a continuous focus binds through the continuous namespace");
    assert_eq!(
        match &labels.current_continuous {
            replay_core::Leaf::Measured(rows) => rows
                .iter()
                .map(|row| (row.id.clone(), row.display_name.clone()))
                .collect::<Vec<_>>(),
            other => panic!("{other:?}"),
        },
        vec![(
            "continuous_non_factory_construct".to_string(),
            "Construction Engineering".to_string()
        )]
    );
}

#[test]
fn a_continuous_focus_id_the_recording_did_not_package_is_refused_by_name_and_kind() {
    let catalog = admit(
        focus_defs_artifact_with_continuous(
            &[("uk_industrial_focus", "industrial_effort")],
            &["uk_industrial_focus"],
            &["continuous_repair"],
        ),
        &[
            ("industrial_effort", "Industrial Effort"),
            ("continuous_repair", "Reinforced Concrete"),
        ],
    )
    .expect("the packaged catalogs are admitted");

    let error = catalog
        .continuous_focus_label("continuous_non_factory_construct")
        .expect_err("an unpackaged continuous focus has no localisation key");
    assert!(
        error
            .message
            .contains("has no focus \"continuous_non_factory_construct\"")
            && error.message.contains("continuous focuses"),
        "{}",
        error.message
    );
}

#[test]
fn a_focus_id_named_in_the_wrong_namespace_is_refused_by_kind() {
    let catalog = admit(
        focus_defs_artifact(&[("uk_industrial_focus", "industrial_effort")], &["uk_industrial_focus"]),
        &[
            ("industrial_effort", "Industrial Effort"),
            (
                "continuous_non_factory_construct",
                "Construction Engineering",
            ),
        ],
    )
    .expect("the packaged catalogs are admitted");

    let error = catalog
        .continuous_focus_label("uk_industrial_focus")
        .expect_err("a tree focus is not a continuous focus");
    assert!(
        error
            .message
            .contains("is packaged as a tree/shared/joint focus"),
        "{}",
        error.message
    );
    let error = catalog
        .focus_label("continuous_non_factory_construct")
        .expect_err("a continuous focus is not a tree focus");
    assert!(
        error.message.contains("is packaged as a continuous focus"),
        "{}",
        error.message
    );
}

#[test]
fn a_catalog_that_packages_no_continuous_namespace_is_refused_at_admission() {
    let mut artifact =
        focus_defs_artifact(&[("uk_industrial_focus", "industrial_effort")], &["uk_industrial_focus"]);
    let object = artifact.as_object_mut().unwrap();
    object.remove("continuous_focuses");
    object.remove("continuous_focus_palettes");
    let error = replay_core::admit_static_focus_definitions(&packaged(
        replay_core::FOCUS_DEFS_STATIC_ARTIFACT,
        "static/focus_defs_static/focus_defs_static.json",
        artifact,
    ))
    .expect_err("a catalog predating the continuous namespace is not admitted");
    assert!(
        error
            .reason
            .contains("missing field `continuous_focuses`"),
        "{}",
        error.reason
    );
}

#[test]
fn a_continuous_focus_a_palette_does_not_own_is_refused() {
    let mut artifact = focus_defs_artifact_with_continuous(
        &[("uk_industrial_focus", "industrial_effort")],
        &["uk_industrial_focus"],
        &["continuous_repair"],
    );
    artifact["continuous_focuses"]["continuous_repair"]["palette_id"] =
        Value::from("other_palette");
    let error = replay_core::admit_static_focus_definitions(&packaged(
        replay_core::FOCUS_DEFS_STATIC_ARTIFACT,
        "static/focus_defs_static/focus_defs_static.json",
        artifact,
    ))
    .expect_err("a palette cannot claim a focus that names another palette");
    assert!(
        error.reason.contains("absent/foreign continuous focus"),
        "{}",
        error.reason
    );
}

#[test]
fn an_id_packaged_in_both_focus_namespaces_is_refused() {
    let artifact = focus_defs_artifact_with_continuous(
        &[("uk_industrial_focus", "industrial_effort")],
        &["uk_industrial_focus"],
        &["uk_industrial_focus"],
    );
    let error = replay_core::admit_static_focus_definitions(&packaged(
        replay_core::FOCUS_DEFS_STATIC_ARTIFACT,
        "static/focus_defs_static/focus_defs_static.json",
        artifact,
    ))
    .expect_err("one id cannot be both a tree node and a continuous focus");
    assert!(
        error
            .reason
            .contains("packaged in both the national and continuous namespaces"),
        "{}",
        error.reason
    );
}
