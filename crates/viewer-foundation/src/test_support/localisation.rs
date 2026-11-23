use content_reader_localisation::localisation::Localisation;

pub fn loaded(yml: &str) -> Localisation {
    let root = std::env::temp_dir().join(format!(
        "hoi4_viewer_label_names_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let file = root.join("localisation/victory_points_l_english.yml");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, format!("\u{feff}{yml}")).unwrap();
    let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let localisation = content_reader_localisation::localisation::read(&layers, "english").unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    localisation
}
