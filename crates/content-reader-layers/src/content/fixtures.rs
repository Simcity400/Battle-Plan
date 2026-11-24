use super::*;
use crate::test_support::{Record, zip_records};

pub fn raw_zip(members: &[(&[u8], u8, &[u8])]) -> Vec<u8> {
    let records: Vec<Record> = members
        .iter()
        .map(|(name, host, data)| Record {
            name,
            host: *host,
            flags: 0,
            deflate: false,
            data,
        })
        .collect();
    zip_records(&[], &records)
}
pub fn archive_layer(id: &str, path: &Path, replace_paths: &[&[u8]]) -> ResolvedLayer {
    let mut layer = ResolvedLayer {
        content_digest: [0; 32],
        inventory_witnesses: Default::default(),
        inventory_paths: Default::default(),
        coverage: CoveragePolicy::default(),
        role: LayerRole::Mod,
        facts: None,
        id: id.into(),
        source: LayerSource::Archive(path.to_path_buf()),
        descriptor: None,
        replace_paths: replace_paths
            .iter()
            .map(|path| replace_path(path).unwrap())
            .collect(),
        files: Default::default(),
        lookup: Default::default(),
        hashes: Default::default(),
        archive_stamp: None,
        descriptor_hash: None,
        overlay: None,
        display_name: None,
        art_unmounted: None,
    };
    layer.inventory().unwrap();
    layer
        .hash(&mut HashCache::default(), &mut HashStats::default())
        .unwrap();
    layer
}
pub fn folder_of(id: &str, root: &Path, files: &[(&str, &[u8])]) -> ResolvedLayer {
    for (path, data) in files {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, data).unwrap();
    }
    let mut layer = folder_layer(id, root, Vec::new());
    layer.inventory().unwrap();
    layer
        .hash(&mut HashCache::default(), &mut HashStats::default())
        .unwrap();
    layer
}
pub fn overlaid_stack(
    root: &Path,
    base: &[(&str, &[u8])],
    art: &[(&str, &[u8])],
    above: &[(&str, &[u8])],
) -> ResolvedLayers {
    let _ = fs::remove_dir_all(root);
    let layer = |id: &str, files: &[(&str, &[u8])]| {
        fs::create_dir_all(root.join(id)).unwrap();
        folder_of(id, &root.join(id), files)
    };
    let mut overlay = layer("art", art);
    overlay.overlay = Some(Overlay {
        name: "Art".into(),
        verified: false,
    });
    ResolvedLayers::from_test_layers(vec![layer("base", base), overlay, layer("above", above)])
        .unwrap()
}

pub fn overlay_folders(
    base: &Path,
    visual: &Path,
    name: &str,
    defines_visible: bool,
) -> ResolvedLayers {
    let mut layers = ResolvedLayers::from_folders(&[(base, &[]), (visual, &[])]).unwrap();
    layers.layers[1].overlay = Some(Overlay {
        name: name.into(),
        verified: false,
    });
    layers.layers[1].role = LayerRole::Visual;
    layers.defines_visible = defines_visible;
    layers.finish().unwrap();
    layers
}

pub fn folder_layer(id: &str, root: &Path, replace_paths: Vec<String>) -> ResolvedLayer {
    ResolvedLayer {
        content_digest: [0; 32],
        inventory_witnesses: Default::default(),
        inventory_paths: Default::default(),
        coverage: CoveragePolicy::default(),
        role: LayerRole::Mod,
        facts: None,
        id: id.into(),
        source: LayerSource::Folder(fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf())),
        descriptor: None,
        replace_paths: replace_paths.into_iter().map(String::into_bytes).collect(),
        files: Default::default(),
        lookup: Default::default(),
        hashes: Default::default(),
        archive_stamp: None,
        descriptor_hash: None,
        overlay: None,
        display_name: None,
        art_unmounted: None,
    }
}
