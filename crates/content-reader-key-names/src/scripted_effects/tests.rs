use super::*;

#[test]
fn body_pass_uses_readers_and_appends_duplicate_templates() {
    content_reader_script::test_game::enter();
    let names = content_reader_script::key_names::KeyNames {
        scripted_effects: [b"one".to_vec()].into(),
        ..Default::default()
    };
    let mut effects = ScriptedEffects::default();
    effects.read_file(
        b"one={@amount=2 army_experience=@amount} ONE={random={chance=50 army_experience=3}}",
        &names,
    );
    let body = effects.bodies[b"one".as_slice()].as_ref().unwrap();
    assert_eq!(body.len(), 2);
    assert_eq!(body[0].value.scalar(), Some(b"2".as_slice()));
    assert!(body[1].keyword_is("random"));
    let generated =
        content_reader_script::effect_keys::read_body(b"meta_effect={text={army_experience=[X]} X=2}", &names)
            .unwrap();
    assert_eq!(
        generated[0].value.items().unwrap()[0].value.scalar(),
        Some(b"army_experience=[X]".as_slice())
    );
    let unknown =
        content_reader_script::effect_keys::read_body(b"unknown_key={army_experience=2}", &names).unwrap();
    assert!(unknown[0].keyword_is("unknown_key"));
    assert!(unknown[0].value.items().unwrap().is_empty());
}

#[test]
fn name_pass_skips_bodies_and_folds_tokens() {
    let names =
        file_names(b"@name = One One = { @hidden = Lost } Two = @hidden \"Three\" = {} One = {}");
    assert_eq!(
        names,
        [b"one".to_vec(), b"two".to_vec(), b"three".to_vec()].into()
    );
}

#[test]
fn names_resolve_recorded_stack_and_only_directory_files() -> io::Result<()> {
    content_reader_script::test_game::enter();
    let root = std::env::temp_dir().join(format!("effect-names-{}", std::process::id()));
    let vanilla = root.join("base");
    let modded = root.join("mod");
    for folder in [&vanilla, &modded] {
        std::fs::create_dir_all(folder.join("common/scripted_effects/sub"))?;
    }
    std::fs::write(
        vanilla.join("common/scripted_effects/base.txt"),
        b"old = {}",
    )?;
    std::fs::write(modded.join("common/scripted_effects/mod.txt"), b"New = {}")?;
    std::fs::write(
        modded.join("common/scripted_effects/sub/nested.txt"),
        b"nested = {}",
    )?;
    let layers =
        ResolvedLayers::from_folders(&[(&vanilla, &[]), (&modded, &["common/scripted_effects"])])?;
    let loaded = names(&layers)?;
    assert_eq!(loaded, [b"new".to_vec()].into());
    std::fs::write(
        modded.join("common/scripted_effects/mod.txt"),
        b"New = {army_experience=2}",
    )?;
    std::fs::write(
        modded.join("common/scripted_effects/sub/nested.txt"),
        b"New = {army_experience=3} nested = {army_experience=9}",
    )?;
    let layers =
        ResolvedLayers::from_folders(&[(&vanilla, &[]), (&modded, &["common/scripted_effects"])])?;
    let bodies = read(
        &layers,
        &content_reader_script::key_names::KeyNames {
            scripted_effects: loaded,
            ..Default::default()
        },
    )?;
    let body = bodies.bodies[b"new".as_slice()].as_ref().unwrap();
    assert_eq!(
        body.iter()
            .map(|item| item.value.scalar().unwrap())
            .collect::<Vec<_>>(),
        [b"2".as_slice(), b"3"]
    );
    assert!(!bodies.bodies.contains_key(b"nested".as_slice()));
    std::fs::remove_dir_all(root)?;
    Ok(())
}
