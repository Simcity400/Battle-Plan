use content_reader_script::localization_binding::FormattedLocalization;

use crate::{faction_templates::read, test_support::Stack};

#[test]
fn faction_templates_resolve_mod_files_merge_names_and_localisation() {
    content_reader_script::test_game::enter();
    let stack = Stack::new("faction_templates");
    let (base, addon) = (stack.base(), stack.addon());
    stack.write(
        &base,
        "common/factions/templates/a.txt",
        "pact={name=shared_name} removed_by_file={name=old}\n",
    );
    stack.write(
        &addon,
        "common/factions/templates/a.txt",
        "pact={name=shared_name}\n",
    );
    stack.write(
        &base,
        "common/factions/templates/b.txt",
        "merged={name=kept_name}\n",
    );
    stack.write(
        &addon,
        "common/factions/templates/z.txt",
        "MERGED={icon=GFX_mod}\n",
    );
    stack.write(
        &addon,
        "common/factions/templates/sub/ignored.txt",
        "nested={name=ignored}\n",
    );
    stack.write(
        &addon,
        "common/factions/templates/ignored.TXT",
        "uppercase={name=ignored}\n",
    );
    stack.write(
        &base,
        "localisation/english/names_l_english.yml",
        "\u{feff}l_english:\n shared_name:0 \"Base pact\"\n",
    );
    stack.write(
        &addon,
        "localisation/english/names_l_english.yml",
        "\u{feff}l_english:\n shared_name:0 \"Mod pact\"\n",
    );
    let layers = stack.layers(&[]);
    let templates = read(&layers).unwrap();
    let localisation = crate::localisation::read(&layers, "english").unwrap();
    let FormattedLocalization::Key(key) = &templates.get("PACT").unwrap().unwrap().nodes[0].key
    else {
        panic!("expected the template's localisation key");
    };
    assert_eq!(localisation.localise(key).unwrap(), "Mod pact");
    assert_eq!(
        templates.get("merged").unwrap().unwrap().nodes[0].key,
        FormattedLocalization::Key(b"kept_name".to_vec())
    );
    for absent in ["removed_by_file", "nested", "uppercase"] {
        assert!(templates.get(absent).unwrap().is_none());
    }
    assert!(templates.warnings.is_empty(), "{:?}", templates.warnings);
}
