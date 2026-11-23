use crate::special_projects::Catalog;

#[test]
fn project_integer_constants_merge_normalize_and_keep_output() {
    content_reader_script::test_game::enter();
    let root =
        std::env::temp_dir().join(format!("content-reader-project-output-{}", std::process::id()));
    let folder = root.join("common/special_projects/projects");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("a.txt"), "mod_project={prototype_time=mod_time.short complexity=mod_complexity.small project_output={country_effects={add_stability=0.1} enable_subunits={mod_unit limit={always=yes}} equipment_bonus={mod_equipment={soft_attack=0.2}}}}").unwrap();
    std::fs::write(folder.join("b.txt"), "mod_project={prototype_time=mod_time.fixed complexity={min=12} project_output={country_effects={add_war_support=0.2}}} mod_invalid={complexity={min=15 max=2}} mod_literal={prototype_time=\"021\" prototype_time=\"\" complexity={min=\"0x1\" max=04}}").unwrap();
    let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let catalog = Catalog::read(&layers).unwrap();
    assert!(catalog.warnings.is_empty(), "{:?}", catalog.warnings);
    let constants = content_reader_politics::script_constants::parse(&[("constants", b"mod_time={schema={any_key=yes data=int} short=30} mod_time={schema={any_key=yes data=fixed_point} fixed=50.0} mod_complexity={schema={any_key=yes data={{key=min data=int}{key=max data=int}}} small={min=5 max=20}}")]);
    let project = &catalog.projects["mod_project"];
    assert_eq!(project.prototype_time(Some(&constants)).unwrap(), 30);
    assert_eq!(project.complexity(Some(&constants), 9).unwrap(), (12, 20));
    assert_eq!(
        project
            .output
            .country_effects
            .iter()
            .map(|item| item.key_str().unwrap())
            .collect::<Vec<_>>(),
        ["add_stability", "add_war_support"]
    );
    assert_eq!(project.output.enable_subunits.len(), 1);
    assert_eq!(
        project.output.equipment_bonus[0].key_str().unwrap(),
        "mod_equipment"
    );
    assert_eq!(
        catalog.projects["mod_invalid"].complexity(None, 9).unwrap(),
        (9, 9)
    );
    assert_eq!(
        catalog.projects["mod_literal"]
            .prototype_time(None)
            .unwrap(),
        17
    );
    assert_eq!(
        catalog.projects["mod_literal"].complexity(None, 9).unwrap(),
        (1, 4)
    );
    assert!(
        project
            .prototype_time(None)
            .unwrap_err()
            .contains("constant catalogue")
    );
    std::fs::remove_dir_all(root).unwrap();
}
