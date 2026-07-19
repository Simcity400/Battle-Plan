//! Product adapter from the mod-aware game VFS to XUI01 definition admission.

use crate::Result;
use game_ui::definition_admission::{
    load_effective_definition_catalog, GuiDefinitionCatalog, ResolvedGuiDefinitionFile,
};
use map_viewer::GameFileResolver;

pub fn load_product_gui_definitions(
    resolver: Option<&GameFileResolver>,
) -> Result<Option<GuiDefinitionCatalog>> {
    let Some(resolver) = resolver else {
        return Ok(None);
    };
    let rows = resolver
        .resolve_dir_files_recursive("interface", Some("gui"))?
        .into_iter()
        .map(|(rel_path, path)| ResolvedGuiDefinitionFile::new(rel_path, path))
        .collect::<Vec<_>>();

    let catalog = load_effective_definition_catalog(rows)?;
    eprintln!(
        "replay_app: effective XUI01 GUI definitions admitted: {}/{} files, {}/{} windows; {} windows rejected for explicit unsupported constructs",
        catalog.admitted_file_count(),
        catalog.files.len(),
        catalog.admitted_windows.len(),
        catalog.total_window_count(),
        catalog.rejected_windows.len(),
    );
    Ok(Some(catalog))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn product_resolver_adapter_uses_effective_namespace_without_frozen_counts() {
        let Some(root) = std::env::var_os("BATTLE_PLAN_GAME_ROOT").or_else(|| std::env::var_os("HOI4_GAME_ROOT")).map(std::path::PathBuf::from) else {
            return;
        };
        let resolver = GameFileResolver::vanilla(root).expect("retained game resolver");
        let catalog = load_product_gui_definitions(Some(&resolver))
            .expect("product GUI definition load")
            .expect("game resolver yields a catalog");
        assert!(catalog.admitted_file_count() > 0);
        assert_eq!(catalog.rejected_file_count(), 0);
        assert!(!catalog.admitted_windows.is_empty());
        assert_eq!(catalog.rejected_windows.len(), 0);
    }
}
