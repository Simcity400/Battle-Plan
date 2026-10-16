//! Localisation fixture read through the actual content-stack reader.
use installed_content::localisation::Localisation;

/// Localisation as the game content loads it: `yml` is one file of a
/// folder layer, read by `installed_content::localisation::read`.
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
    let layers = installed_content::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let localisation = installed_content::localisation::read(&layers, "english").unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    localisation
}
