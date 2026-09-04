use game_ui::assets::AssetResolver;
use game_ui::mod_ui_discovery::{
    discover_admitted_runtime_mod_ui,
    discover_runtime_mod_ui_from_userdir_for_evidence as discover_runtime_mod_ui,
};
use snapshot::sha256::Sha256;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn temp_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "hoi4_xui24_{label}_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn write(root: &Path, rel: &str, text: &str) -> PathBuf {
    let path = rel
        .split('/')
        .fold(root.to_path_buf(), |path, part| path.join(part));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, text).unwrap();
    path
}

fn fixture(
    label: &str,
    added_gui: &str,
    scripted: &str,
    replace_interface: bool,
) -> (PathBuf, PathBuf) {
    let root = temp_root(label);
    let game = root.join("game");
    let user = root.join("user");
    let mod_root = user.join("content/mod_a");
    write(
        &game,
        "interface/base.gui",
        "guiTypes={ containerWindowType={ name=vanilla_base } }",
    );
    write(
        &game,
        "interface/base.gfx",
        "spriteTypes={ spriteType={ name=GFX_base texturefile=\"gfx/base.tga\" } }",
    );
    write(
        &game,
        "localisation/english/base_l_english.yml",
        "l_english:\n BASE:0 \"Base\"\n",
    );
    write(
        &mod_root,
        "interface/base.gui",
        "guiTypes={ containerWindowType={ name=mod_base } }",
    );
    write(&mod_root, "interface/added.gui", added_gui);
    write(
        &mod_root,
        "interface/added.gfx",
        "spriteTypes={ spriteType={ name=GFX_added texturefile=\"gfx/added.tga\" } }",
    );
    write(
        &mod_root,
        "localisation/english/mod_l_english.yml",
        "l_english:\n MOD_PANEL:0 \"Mod Panel\"\n",
    );
    write(
        &mod_root,
        "common/scripted_localisation/mod_text.txt",
        scripted,
    );
    let descriptor_text = if replace_interface {
        format!(
            "name=\"A\"\npath=\"{}\"\nreplace_path=\"interface\"\n",
            mod_root.display()
        )
    } else {
        format!("name=\"A\"\npath=\"{}\"\n", mod_root.display())
    };
    let descriptor = write(&user, "mod/a.mod", &descriptor_text);
    let receipt = serde_json::json!({
        "schema": "extractor.ui14_engine_vfs.v1",
        "executable_sha256": "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
        "layers": [{
            "id": "mod/a.mod",
            "descriptor_sha256": Sha256::hex(&fs::read(&descriptor).unwrap()),
            "engine_descriptor_ordinal": 15,
            "engine_mount_ordinal": 1
        }],
        "effective_winners": []
    });
    fs::write(
        user.join("ui14_engine_vfs.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
    let staging = serde_json::json!({
        "schema": "extractor.runtime_mod_staging.v1",
        "dlc_archives": [],
        "mods": [{
            "id": "mod/a.mod",
            "descriptor_relpath": "mod/a.mod",
            "descriptor_sha256": Sha256::hex(&fs::read(&descriptor).unwrap()),
            "content_path": mod_root.to_string_lossy()
        }],
        "ui14_engine_vfs_sha256": Sha256::hex(&serde_json::to_vec(&receipt).unwrap())
    });
    fs::write(
        user.join("static_mod_staging_manifest.json"),
        serde_json::to_vec_pretty(&staging).unwrap(),
    )
    .unwrap();
    (game, user)
}

#[test]
fn discovers_mod_added_panel_override_localisation_and_scripted_loc() {
    let (game, user) = fixture(
        "positive",
        "guiTypes={ containerWindowType={ name=mod_added instantTextBoxType={ name=title text=MOD_PANEL } } }",
        "defined_text={ name=MOD_DYNAMIC text={ localization_key=MOD_PANEL } }",
        false,
    );
    let result = discover_runtime_mod_ui(&game, &user).expect("discover admitted mod UI");
    assert!(result.production_admitted);
    assert_eq!(result.gui_files, 2);
    assert_eq!(result.mod_added_gui_files, 1);
    assert_eq!(result.overridden_gui_files, 1);
    assert!(result
        .panels
        .iter()
        .any(|row| row.name == "mod_added" && row.mod_added_file));
    assert!(result.panels.iter().any(|row| row.name == "mod_base"));
    assert_eq!(result.scripted_localisation.len(), 1);
    assert!(result.scripted_localisation[0].requires_captured_runtime_value);
    assert_eq!(
        result.scripted_localisation[0].candidate_keys,
        vec!["MOD_PANEL"]
    );
}

#[test]
fn replace_path_removes_only_the_exact_lower_parent_before_mod_readds() {
    let (game, user) = fixture(
        "replace",
        "guiTypes={ containerWindowType={ name=mod_added } }",
        "defined_text={ name=MOD_DYNAMIC text={ localization_key=MOD_PANEL } }",
        true,
    );
    write(
        &game,
        "interface/nested/base.gui",
        "guiTypes={ containerWindowType={ name=nested_base } }",
    );
    portable_test_fs_retry::remove_file(user.join("content/mod_a/interface/base.gui")).unwrap();
    let resolver = AssetResolver::from_runtime_userdir(&game, &user).unwrap();
    let files = resolver
        .list_files_recursive("interface", Some("gui"))
        .unwrap();
    assert_eq!(
        files
            .iter()
            .map(|row| row.rel_path.as_str())
            .collect::<Vec<_>>(),
        vec!["interface/added.gui", "interface/nested/base.gui"]
    );
    assert!(resolver
        .resolve_optional_file("interface/base.gui")
        .unwrap()
        .is_none());
    assert!(resolver
        .resolve_optional_file("interface/nested/base.gui")
        .unwrap()
        .is_some());
}

#[test]
fn recorded_engine_mount_ordinal_controls_two_mod_precedence() {
    let (game, user) = fixture(
        "two_mods",
        "guiTypes={ containerWindowType={ name=mod_a_added } }",
        "defined_text={ name=MOD_DYNAMIC text={ localization_key=MOD_PANEL } }",
        false,
    );
    let mod_z = user.join("content/mod_z");
    write(
        &mod_z,
        "interface/base.gui",
        "guiTypes={ containerWindowType={ name=mod_z_winner } }",
    );
    let descriptor_z = write(
        &user,
        "mod/z.mod",
        &format!("name=\"Z\"\npath=\"{}\"\n", mod_z.display()),
    );
    let descriptor_a = user.join("mod/a.mod");
    let receipt = serde_json::json!({
        "schema": "extractor.ui14_engine_vfs.v1",
        "executable_sha256": "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
        "layers": [
            {"id":"mod/z.mod","descriptor_sha256":Sha256::hex(&fs::read(&descriptor_z).unwrap()),"engine_descriptor_ordinal":15,"engine_mount_ordinal":15},
            {"id":"mod/a.mod","descriptor_sha256":Sha256::hex(&fs::read(&descriptor_a).unwrap()),"engine_descriptor_ordinal":0,"engine_mount_ordinal":0}
        ],
        "effective_winners": []
    });
    fs::write(
        user.join("ui14_engine_vfs.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
    let staging = serde_json::json!({
        "schema":"extractor.runtime_mod_staging.v1",
        "dlc_archives":[],
        "mods":[
            {"id":"mod/a.mod","descriptor_relpath":"mod/a.mod","descriptor_sha256":Sha256::hex(&fs::read(&descriptor_a).unwrap()),"content_path":user.join("content/mod_a").to_string_lossy()},
            {"id":"mod/z.mod","descriptor_relpath":"mod/z.mod","descriptor_sha256":Sha256::hex(&fs::read(&descriptor_z).unwrap()),"content_path":mod_z.to_string_lossy()}
        ],
        "ui14_engine_vfs_sha256":Sha256::hex(&serde_json::to_vec(&receipt).unwrap())
    });
    fs::write(
        user.join("static_mod_staging_manifest.json"),
        serde_json::to_vec_pretty(&staging).unwrap(),
    )
    .unwrap();
    let result = discover_runtime_mod_ui(&game, &user).unwrap();
    assert!(result.panels.iter().any(|row| row.name == "mod_z_winner"));
    assert!(!result.panels.iter().any(|row| row.name == "mod_base"));
}

#[test]
fn ui15_materialized_dlc_namespace_registers_and_mods_remain_higher_priority() {
    let (game, user) = fixture(
        "dlc",
        "guiTypes={ containerWindowType={ name=mod_added } }",
        "defined_text={ name=MOD_DYNAMIC text={ localization_key=MOD_PANEL } }",
        false,
    );
    let dlc_root = user.join("materialized/dlc003");
    write(
        &dlc_root,
        "interface/dlc.gui",
        "guiTypes={ containerWindowType={ name=dlc_panel } }",
    );
    write(
        &dlc_root,
        "interface/base.gui",
        "guiTypes={ containerWindowType={ name=dlc_base_loses_to_mod } }",
    );
    write(
        &dlc_root,
        "interface/dlc.gfx",
        "spriteTypes={ spriteType={ name=GFX_dlc texturefile=\"gfx/dlc.tga\" } }",
    );
    write(
        &dlc_root,
        "localisation/english/dlc_l_english.yml",
        "l_english:\n DLC_PANEL:0 \"DLC Panel\"\n",
    );
    write(
        &dlc_root,
        "common/scripted_localisation/dlc.txt",
        "defined_text={ name=DLC_DYNAMIC text={ localization_key=DLC_PANEL } }",
    );
    let descriptor = write(
        &user,
        "_ui15_dlc/dlc003.mod",
        &format!("name=\"DLC003\"\npath=\"{}\"\n", dlc_root.display()),
    );
    let mut staging: serde_json::Value =
        serde_json::from_slice(&fs::read(user.join("static_mod_staging_manifest.json")).unwrap())
            .unwrap();
    staging["dlc_archives"] = serde_json::json!([{
        "id":"_ui15_dlc/dlc003.mod","descriptor_relpath":"_ui15_dlc/dlc003.mod",
        "descriptor_sha256":Sha256::hex(&fs::read(&descriptor).unwrap()),"content_path":dlc_root.to_string_lossy(),"ordinal":3
    }]);
    fs::write(
        user.join("static_mod_staging_manifest.json"),
        serde_json::to_vec_pretty(&staging).unwrap(),
    )
    .unwrap();
    let result = discover_runtime_mod_ui(&game, &user).unwrap();
    assert!(result.panels.iter().any(|row| row.name == "dlc_panel"));
    assert!(result.panels.iter().any(|row| row.name == "mod_base"));
    assert!(!result
        .panels
        .iter()
        .any(|row| row.name == "dlc_base_loses_to_mod"));
    assert!(result
        .scripted_localisation
        .iter()
        .any(|row| row.name == "DLC_DYNAMIC"));
}

#[test]
fn ui15_missing_duplicate_and_wrong_namespace_ordinals_fail_closed() {
    let (game, user) = fixture(
        "bad_dlc",
        "guiTypes={ containerWindowType={ name=good } }",
        "defined_text={ name=MOD_DYNAMIC text={ localization_key=MOD_PANEL } }",
        false,
    );
    let dlc_root = user.join("materialized/dlc");
    fs::create_dir_all(&dlc_root).unwrap();
    let d1 = write(
        &user,
        "_ui15_dlc/a.mod",
        &format!("path=\"{}\"\n", dlc_root.display()),
    );
    let d2 = write(
        &user,
        "_ui15_dlc/b.mod",
        &format!("path=\"{}\"\n", dlc_root.display()),
    );
    let base: serde_json::Value =
        serde_json::from_slice(&fs::read(user.join("static_mod_staging_manifest.json")).unwrap())
            .unwrap();
    let row = |id: &str, descriptor: &Path| serde_json::json!({"id":id,"descriptor_relpath":id,"descriptor_sha256":Sha256::hex(&fs::read(descriptor).unwrap()),"content_path":dlc_root.to_string_lossy()});
    let mut missing = base.clone();
    missing["dlc_archives"] = serde_json::json!([row("_ui15_dlc/a.mod", &d1)]);
    fs::write(
        user.join("static_mod_staging_manifest.json"),
        serde_json::to_vec_pretty(&missing).unwrap(),
    )
    .unwrap();
    assert!(AssetResolver::from_runtime_userdir(&game, &user)
        .unwrap_err()
        .to_string()
        .contains("lacks staged ordinal"));
    let mut duplicate = base.clone();
    let mut r1 = row("_ui15_dlc/a.mod", &d1);
    r1["ordinal"] = serde_json::json!(7);
    let mut r2 = row("_ui15_dlc/b.mod", &d2);
    r2["ordinal"] = serde_json::json!(7);
    duplicate["dlc_archives"] = serde_json::json!([r1, r2]);
    fs::write(
        user.join("static_mod_staging_manifest.json"),
        serde_json::to_vec_pretty(&duplicate).unwrap(),
    )
    .unwrap();
    assert!(AssetResolver::from_runtime_userdir(&game, &user)
        .unwrap_err()
        .to_string()
        .contains("repeats an ordinal"));
    let bad = write(
        &user,
        "dlc_bad.mod",
        &format!("path=\"{}\"\n", dlc_root.display()),
    );
    let mut wrong = base;
    wrong["dlc_archives"] = serde_json::json!([{"id":"dlc_bad.mod","descriptor_relpath":"dlc_bad.mod","descriptor_sha256":Sha256::hex(&fs::read(&bad).unwrap()),"content_path":dlc_root.to_string_lossy(),"ordinal":1}]);
    fs::write(
        user.join("static_mod_staging_manifest.json"),
        serde_json::to_vec_pretty(&wrong).unwrap(),
    )
    .unwrap();
    assert!(AssetResolver::from_runtime_userdir(&game, &user)
        .unwrap_err()
        .to_string()
        .contains("namespace is invalid"));
}

#[test]
fn installed_random_list_shape_is_admitted_but_unknown_or_empty_shapes_fail_closed() {
    let (game, user) = fixture(
        "random_list",
        "guiTypes={ containerWindowType={ name=good } }",
        "defined_text={ name=FILM text={ random_list={ seed=film 10={ localization_key=FILM_A } 25={ localization_key=FILM_B } } } }",
        false,
    );
    let result = discover_runtime_mod_ui(&game, &user).unwrap();
    assert_eq!(
        result.scripted_localisation[0].candidate_keys,
        vec!["FILM_A", "FILM_B"]
    );

    let (game, user) = fixture(
        "random_unknown",
        "guiTypes={ containerWindowType={ name=good } }",
        "defined_text={ name=BAD text={ random_list={ future=thing 10={ localization_key=OK } } } }",
        false,
    );
    assert!(discover_runtime_mod_ui(&game, &user)
        .unwrap_err()
        .to_string()
        .contains("unknown scripted-localisation random_list construct"));

    let (game, user) = fixture(
        "empty_script",
        "guiTypes={ containerWindowType={ name=good } }",
        "defined_text={ name=EMPTY text={ trigger={ always=yes } } }",
        false,
    );
    assert!(discover_runtime_mod_ui(&game, &user)
        .unwrap_err()
        .to_string()
        .contains("no localization_key candidate"));
}

#[test]
fn unknown_widget_is_rejected_locally_while_unknown_script_construct_fails_closed() {
    let (game, user) = fixture(
        "unknown_widget",
        "guiTypes={ containerWindowType={ name=good } containerWindowType={ name=bad futureWidget={ name=nope } } }",
        "defined_text={ name=MOD_DYNAMIC text={ localization_key=MOD_PANEL } }",
        false,
    );
    let discovery = discover_runtime_mod_ui(&game, &user).unwrap();
    assert!(discovery.panels.iter().any(|panel| panel.name == "good"));
    assert!(!discovery.panels.iter().any(|panel| panel.name == "bad"));

    let (game, user) = fixture(
        "unknown_script",
        "guiTypes={ containerWindowType={ name=good } }",
        "future_defined_text={ name=BAD text={ localization_key=MOD_PANEL } }",
        false,
    );
    let error = discover_runtime_mod_ui(&game, &user).unwrap_err();
    assert!(error
        .to_string()
        .contains("unknown scripted-localisation top-level construct"));
}

#[test]
fn product_discovery_refuses_a_raw_install_resolver() {
    let root = std::env::temp_dir().join("xui24_raw_resolver_must_not_enter_product");
    let resolver = AssetResolver::new(vec![root]);
    let error = discover_admitted_runtime_mod_ui(&resolver, "l_english")
        .expect_err("raw roots must not become a product fallback");
    assert!(error
        .to_string()
        .contains("portable content-admission resolver"));
}
