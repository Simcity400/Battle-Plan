
use crate::gui::{GuiElement, LayoutSet};
use crate::{ErrorKind, GameUiError, Result};
use portable_content_manifest::authored_ui::engine_recursive_path_cmp;
use serde::Deserialize;
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
use std::fs;
use std::path::{Path, PathBuf};

const FROZEN_CENSUS_JSON: &str =
    include_str!("../../../re_evidence/rendering_laws/XUI01/file_census.json");
pub const FROZEN_FILE_COUNT: usize = 454;
pub const FROZEN_WINDOW_COUNT: usize = 9_155;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedGuiDefinitionFile {
    pub rel_path: String,
    pub bytes: Vec<u8>,
}

impl ResolvedGuiDefinitionFile {
    pub fn read(resolver: &crate::assets::AssetResolver, file: &crate::assets::ResolvedFile) -> Result<Self> {
        Ok(Self::new(&file.rel_path, resolver.read_file(file)?))
    }

    pub fn new(rel_path: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            rel_path: normalize_rel(&rel_path.into()),
            bytes: bytes.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConstructSchema {
    Window,
    ContainerWindow,
    Sprite,
    InstantText,
    ButtonGeometry,
    CollectionGeometry,
    ScrollGeometry,
    LayoutProperty,
    BrowserGeometry,
    CheckboxGeometry,
    DropdownGeometry,
    EditBoxGeometry,
    OverlapCollectionGeometry,
    TextBoxGeometry,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnsupportedConstruct {
    Browser,
    Checkbox,
    Dropdown,
    EditBox,
    OverlapCollection,
    StatefulTextBox,
    Unknown(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedWindowDefinition {
    pub window_id: String,
    pub source_file: String,
    pub ordinal: usize,
    pub element_type: String,
    pub name: String,
    pub hierarchy_path: String,
    pub schemas: BTreeSet<ConstructSchema>,
    pub downstream_runtime_dependencies: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RejectedWindowDefinition {
    pub window_id: String,
    pub source_file: String,
    pub ordinal: usize,
    pub element_type: String,
    pub name: String,
    pub hierarchy_path: String,
    pub unsupported: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EffectiveGuiRootDefinition {
    pub source_file: String,
    pub window_id: String,
    pub root: GuiElement,
    pub admitted: bool,
    pub unsupported: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileAdmission {
    pub source_file: String,
    pub definition_file_admitted: bool,
    pub admitted_windows: usize,
    pub rejected_windows: usize,
    pub parse_recoveries: usize,
    pub decode_losses: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GuiDefinitionCatalog {
    pub layouts: LayoutSet,
    pub files: Vec<FileAdmission>,
    pub admitted_windows: Vec<AdmittedWindowDefinition>,
    pub rejected_windows: Vec<RejectedWindowDefinition>,
    pub effective_roots: Vec<EffectiveGuiRootDefinition>,
}

impl GuiDefinitionCatalog {
    pub fn recovery_parsed_files(&self) -> Vec<String> {
        self.files
            .iter()
            .filter(|row| row.parse_recoveries != 0)
            .map(|row| format!("{} ({} recoveries)", row.source_file, row.parse_recoveries))
            .collect()
    }

    pub fn decode_lossy_files(&self) -> Vec<String> {
        self.files
            .iter()
            .filter(|row| row.decode_losses.is_some_and(|count| count != 0))
            .map(|row| {
                format!(
                    "{} ({} destructive decode events)",
                    row.source_file,
                    row.decode_losses.unwrap_or_default()
                )
            })
            .collect()
    }

    pub fn admitted_file_count(&self) -> usize {
        self.files
            .iter()
            .filter(|row| row.definition_file_admitted)
            .count()
    }

    pub fn rejected_file_count(&self) -> usize {
        self.files
            .iter()
            .filter(|row| !row.definition_file_admitted)
            .count()
    }

    pub fn total_window_count(&self) -> usize {
        self.admitted_windows.len() + self.rejected_windows.len()
    }
}

#[derive(Deserialize)]
struct FrozenCensus {
    schema: String,
    authority: FrozenAuthority,
    summary: FrozenSummary,
    files: Vec<FrozenFile>,
}

#[derive(Deserialize)]
struct FrozenAuthority {
    program: String,
    image_base: String,
    sha256: String,
}

#[derive(Deserialize)]
struct FrozenSummary {
    active_files: usize,
}

#[derive(Clone, Deserialize)]
struct FrozenFile {
    source_file: String,
    source_layer: String,
    source_layer_kind: String,
    bytes: usize,
    sha256: String,
    window_count: usize,
}

pub fn frozen_playset_files(game_root: &Path) -> Result<Vec<ResolvedGuiDefinitionFile>> {
    let resolver = map_viewer::GameFileResolver::vanilla(game_root)
        .map_err(|error| GameUiError::missing(error.to_string()))?;
    frozen_census()?.files.iter().map(|row| {
        let bytes = resolver.read_required_bytes(&row.source_file, "frozen GUI evidence")
            .map_err(|error| GameUiError::missing(error.to_string()))?;
        Ok(ResolvedGuiDefinitionFile::new(&row.source_file, bytes))
    }).collect()
}


pub fn load_frozen_definition_catalog(
    resolved_files: Vec<ResolvedGuiDefinitionFile>,
) -> Result<GuiDefinitionCatalog> {
    let census = frozen_census()?;
    let expected = census
        .files
        .iter()
        .map(|row| (row.source_file.clone(), row.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut actual = BTreeMap::new();
    let mut folded = BTreeSet::new();
    for row in resolved_files {
        let rel = normalize_rel(&row.rel_path);
        if !folded.insert(rel.to_ascii_lowercase()) {
            return invalid(format!("duplicate/case-colliding effective GUI path {rel}"));
        }
        if actual.insert(rel.clone(), row.bytes).is_some() {
            return invalid(format!("duplicate effective GUI path {rel}"));
        }
    }

    let missing = expected
        .keys()
        .filter(|key| !actual.contains_key(*key))
        .cloned()
        .collect::<Vec<_>>();
    let unexpected = actual
        .keys()
        .filter(|key| !expected.contains_key(*key))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() || !unexpected.is_empty() {
        return invalid(format!(
            "XUI01 effective GUI namespace drift: missing={missing:?} unexpected={unexpected:?}"
        ));
    }

    let mut layouts = LayoutSet::default();
    for (rel, expected_row) in &expected {
        let bytes = actual
            .get(rel)
            .expect("missing rows were rejected before parse");
        if bytes.len() != expected_row.bytes {
            return invalid(format!(
                "XUI01 GUI byte-count drift for {rel}: expected {}, got {}",
                expected_row.bytes,
                bytes.len()
            ));
        }
        let digest = Sha256::hex(&bytes);
        if digest != expected_row.sha256 {
            return invalid(format!(
                "XUI01 GUI SHA-256 drift for {rel}: expected {}, got {digest}",
                expected_row.sha256
            ));
        }
        layouts.parse_content(bytes, rel).map_err(|err| {
            GameUiError::new(
                ErrorKind::Parse,
                format!("parse frozen XUI01 GUI {rel}: {err}"),
            )
        })?;
        let parsed_windows = layouts
            .documents
            .last()
            .map(|doc| count_windows(&doc.roots))
            .unwrap_or(0);
        if parsed_windows != expected_row.window_count {
            return invalid(format!(
                "XUI01 window-count drift for {rel}: expected {}, got {parsed_windows}",
                expected_row.window_count
            ));
        }
    }
    if layouts.stats.file_count != FROZEN_FILE_COUNT
        || layouts.stats.window_count != FROZEN_WINDOW_COUNT
    {
        return invalid(format!(
            "XUI01 aggregate parse drift: expected {FROZEN_FILE_COUNT} files/{FROZEN_WINDOW_COUNT} windows, got {} files/{} windows",
            layouts.stats.file_count, layouts.stats.window_count
        ));
    }

    let catalog = catalog_from_layouts(layouts, false);
    if catalog.total_window_count() != FROZEN_WINDOW_COUNT {
        return invalid(format!(
            "XUI01 admission traversal lost definitions: expected {FROZEN_WINDOW_COUNT}, got {}",
            catalog.total_window_count()
        ));
    }
    if catalog.admitted_windows.is_empty() {
        return invalid("XUI01 schema admission produced zero definitions");
    }
    Ok(catalog)
}

pub fn load_effective_definition_catalog(
    resolved_files: Vec<ResolvedGuiDefinitionFile>,
) -> Result<GuiDefinitionCatalog> {
    let mut exact_paths = BTreeSet::new();
    let mut ordered = resolved_files;
    ordered.sort_by(|a, b| engine_recursive_path_cmp(&a.rel_path, &b.rel_path));
    let mut layouts = LayoutSet::default();
    for row in ordered {
        let rel = normalize_rel(&row.rel_path);
        if !exact_paths.insert(rel.clone()) {
            return invalid(format!("duplicate effective GUI path {rel}"));
        }
        layouts.parse_content(&row.bytes, &rel).map_err(|error| {
            GameUiError::new(
                ErrorKind::Parse,
                format!("parse effective GUI {rel}: {error}"),
            )
        })?;
    }
    Ok(catalog_from_layouts(layouts, true))
}

fn catalog_from_layouts(
    layouts: LayoutSet,
    collapse_case_insensitive_root_overrides: bool,
) -> GuiDefinitionCatalog {
    let root_winners = collapse_case_insensitive_root_overrides.then(|| {
        let mut winners = BTreeMap::<String, (usize, usize)>::new();
        for (document_index, document) in layouts.documents.iter().enumerate() {
            for (root_index, root) in document.roots.iter().enumerate() {
                winners.insert(root.name.to_ascii_lowercase(), (document_index, root_index));
            }
        }
        winners
    });
    let mut files = Vec::with_capacity(layouts.documents.len());
    let mut admitted_windows = Vec::new();
    let mut rejected_windows = Vec::new();
    let mut effective_roots = Vec::new();
    for (document_index, document) in layouts.documents.iter().enumerate() {
        let mut ordinal = 0usize;
        let before_admitted = admitted_windows.len();
        let before_rejected = rejected_windows.len();
        for (root_index, root) in document.roots.iter().enumerate() {
            if root_winners.as_ref().is_some_and(|winners| {
                winners.get(&root.name.to_ascii_lowercase()).copied()
                    != Some((document_index, root_index))
            }) {
                ordinal += count_windows(std::slice::from_ref(root));
                continue;
            }
            admit_windows(
                root,
                "",
                &document.source_file,
                &mut ordinal,
                &mut admitted_windows,
                &mut rejected_windows,
                &mut effective_roots,
            );
        }
        files.push(FileAdmission {
            source_file: document.source_file.clone(),
            definition_file_admitted: true,
            admitted_windows: admitted_windows.len() - before_admitted,
            rejected_windows: rejected_windows.len() - before_rejected,
            parse_recoveries: document.recoveries.len(),
            decode_losses: document.decode_losses.as_ref().map(|report| report.events),
        });
    }
    GuiDefinitionCatalog {
        layouts,
        files,
        admitted_windows,
        rejected_windows,
        effective_roots,
    }
}

pub fn classify_construct(
    element_type: &str,
) -> std::result::Result<ConstructSchema, UnsupportedConstruct> {
    match element_type.to_ascii_lowercase().as_str() {
        "windowtype" => Ok(ConstructSchema::Window),
        "containerwindowtype" => Ok(ConstructSchema::ContainerWindow),
        "icontype" | "background" => Ok(ConstructSchema::Sprite),
        "instanttextboxtype" => Ok(ConstructSchema::InstantText),
        "buttontype" | "guibuttontype" | "decreasebutton" | "increasebutton" | "expandbutton" => {
            Ok(ConstructSchema::ButtonGeometry)
        }
        "smoothlistboxtype" | "listboxtype" | "gridboxtype" | "slotsize" | "max_slots" => {
            Ok(ConstructSchema::CollectionGeometry)
        }
        "extendedscrollbartype" | "scrollbartype" | "slider" | "track" | "drag_scroll" => {
            Ok(ConstructSchema::ScrollGeometry)
        }
        "background_margin"
        | "bordersize"
        | "bound_tooltip"
        | "closed_position"
        | "cursor"
        | "expandedwindow"
        | "open_position"
        | "pdx_tooltip_anchor"
        | "peek_check_size"
        | "peek_check_top_left"
        | "peek_position"
        | "positiontype"
        | "tilesize" => Ok(ConstructSchema::LayoutProperty),
        "browsertype" => Ok(ConstructSchema::BrowserGeometry),
        "checkboxtype" => Ok(ConstructSchema::CheckboxGeometry),
        "dropdownboxtype" => Ok(ConstructSchema::DropdownGeometry),
        "editboxtype" => Ok(ConstructSchema::EditBoxGeometry),
        "overlappingelementsboxtype" => Ok(ConstructSchema::OverlapCollectionGeometry),
        "textboxtype" => Ok(ConstructSchema::TextBoxGeometry),
        other => Err(UnsupportedConstruct::Unknown(other.to_string())),
    }
}

fn frozen_census() -> Result<FrozenCensus> {
    let census: FrozenCensus = serde_json::from_str(FROZEN_CENSUS_JSON).map_err(|err| {
        GameUiError::new(
            ErrorKind::Parse,
            format!("parse embedded XUI01 frozen census: {err}"),
        )
    })?;
    if census.schema != "xui01-file-census-v1"
        || census.authority.program != "hoi4.exe 1.19.2.0.a729"
        || census.authority.image_base != "0x140000000"
        || census.authority.sha256
            != "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
        || census.summary.active_files != FROZEN_FILE_COUNT
        || census.files.len() != FROZEN_FILE_COUNT
        || census
            .files
            .iter()
            .map(|row| row.window_count)
            .sum::<usize>()
            != FROZEN_WINDOW_COUNT
    {
        return invalid("embedded XUI01 census authority/count contract drifted");
    }
    Ok(census)
}

fn admit_windows(
    element: &GuiElement,
    parent_path: &str,
    source_file: &str,
    ordinal: &mut usize,
    admitted: &mut Vec<AdmittedWindowDefinition>,
    rejected: &mut Vec<RejectedWindowDefinition>,
    effective_roots: &mut Vec<EffectiveGuiRootDefinition>,
) {
    let hierarchy_path = if parent_path.is_empty() {
        element.name.clone()
    } else {
        format!("{parent_path}/{}", element.name)
    };
    if is_window(&element.element_type) {
        *ordinal += 1;
        let window_id = format!("{source_file}#{:05}", *ordinal);
        let mut schemas = BTreeSet::new();
        let mut unsupported = BTreeSet::new();
        let mut downstream_runtime_dependencies = BTreeSet::new();
        classify_scope(
            element,
            true,
            &mut schemas,
            &mut unsupported,
            &mut downstream_runtime_dependencies,
        );
        if parent_path.is_empty() {
            effective_roots.push(EffectiveGuiRootDefinition {
                source_file: source_file.to_string(),
                window_id: window_id.clone(),
                root: element.clone(),
                admitted: unsupported.is_empty(),
                unsupported: unsupported.clone(),
            });
        }
        if unsupported.is_empty() {
            admitted.push(AdmittedWindowDefinition {
                window_id,
                source_file: source_file.to_string(),
                ordinal: *ordinal,
                element_type: element.element_type.clone(),
                name: element.name.clone(),
                hierarchy_path: hierarchy_path.clone(),
                schemas,
                downstream_runtime_dependencies,
            });
        } else {
            rejected.push(RejectedWindowDefinition {
                window_id,
                source_file: source_file.to_string(),
                ordinal: *ordinal,
                element_type: element.element_type.clone(),
                name: element.name.clone(),
                hierarchy_path: hierarchy_path.clone(),
                unsupported,
            });
        }
    }
    for child in &element.children {
        admit_windows(
            child,
            &hierarchy_path,
            source_file,
            ordinal,
            admitted,
            rejected,
            effective_roots,
        );
    }
}

fn classify_scope(
    element: &GuiElement,
    root: bool,
    schemas: &mut BTreeSet<ConstructSchema>,
    unsupported: &mut BTreeSet<String>,
    downstream_runtime_dependencies: &mut BTreeSet<String>,
) {
    if !root && is_window(&element.element_type) {
        return;
    }
    match classify_construct(&element.element_type) {
        Ok(schema) => {
            schemas.insert(schema);
        }
        Err(reason) => {
            unsupported.insert(format!("{}:{reason:?}", element.element_type));
        }
    }
    downstream_runtime_dependencies.extend(
        runtime_dependency_owners(&element.element_type)
            .iter()
            .map(|owner| (*owner).to_string()),
    );
    for child in &element.children {
        classify_scope(
            child,
            false,
            schemas,
            unsupported,
            downstream_runtime_dependencies,
        );
    }
}

pub fn runtime_dependency_owners(element_type: &str) -> &'static [&'static str] {
    match element_type.to_ascii_lowercase().as_str() {
        "browsertype" => &["XUI26"],
        "checkboxtype" => &["XUI04"],
        "dropdownboxtype" => &["XUI02", "XUI04"],
        "editboxtype" => &["XUI06", "XUI25"],
        "overlappingelementsboxtype" => &["XUI03"],
        "textboxtype" => &["XUI06"],
        _ => &[],
    }
}

fn count_windows(elements: &[GuiElement]) -> usize {
    elements
        .iter()
        .map(|element| {
            usize::from(is_window(&element.element_type)) + count_windows(&element.children)
        })
        .sum()
}

fn is_window(kind: &str) -> bool {
    matches!(
        kind.to_ascii_lowercase().as_str(),
        "windowtype" | "containerwindowtype"
    )
}

fn normalize_rel(value: &str) -> String {
    value.replace('\\', "/").trim_start_matches('/').to_string()
}

fn rel_to_path(value: &str) -> PathBuf {
    value.split('/').collect()
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(GameUiError::new(ErrorKind::Other, message))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construct_table_admits_definition_schemas_and_rejects_unknowns() {
        assert_eq!(
            classify_construct("background"),
            Ok(ConstructSchema::Sprite)
        );
        assert_eq!(
            classify_construct("borderSize"),
            Ok(ConstructSchema::LayoutProperty)
        );
        assert_eq!(
            classify_construct("checkboxType"),
            Ok(ConstructSchema::CheckboxGeometry)
        );
        assert_eq!(
            runtime_dependency_owners("dropDownBoxType"),
            &["XUI02", "XUI04"]
        );
        assert_eq!(
            classify_construct("futureWidget"),
            Err(UnsupportedConstruct::Unknown("futurewidget".to_string()))
        );
    }

    #[test]
    fn exact_a729_recursive_order_is_componentwise_and_case_sensitive() {
        let mut paths = [
            "interface/a.gui",
            "interface/a/nested.gui",
            "interface/Z.gui",
        ];
        paths.sort_by(|left, right| engine_recursive_path_cmp(left, right));
        assert_eq!(
            paths,
            [
                "interface/Z.gui",
                "interface/a/nested.gui",
                "interface/a.gui",
            ]
        );
    }

    #[test]
    fn effective_catalog_keeps_only_case_insensitive_last_parsed_root_winner() {
        let dir = std::env::temp_dir().join(format!(
            "game_ui_defadm_root_override_{}_{}",
            std::process::id(),
            line!()
        ));
        fs::create_dir_all(&dir).unwrap();
        let overridden_only = dir.join("A.gui");
        let uppercase = dir.join("Z.gui");
        let lowercase = dir.join("a.gui");
        fs::write(
            &overridden_only,
            br#"guiTypes = { windowType = { name = "SHAREDROOT" } }"#,
        )
        .unwrap();
        fs::write(
            &uppercase,
            br#"guiTypes = {
                windowType = { name = "SharedRoot" }
                windowType = { name = "IndependentRoot" }
            }"#,
        )
        .unwrap();
        fs::write(
            &lowercase,
            br#"guiTypes = { windowType = { name = "sharedroot" } }"#,
        )
        .unwrap();

        let catalog = load_effective_definition_catalog(vec![
            ResolvedGuiDefinitionFile::new("interface/a.gui", std::fs::read(&lowercase).unwrap()),
            ResolvedGuiDefinitionFile::new("interface/Z.gui", std::fs::read(&uppercase).unwrap()),
            ResolvedGuiDefinitionFile::new("interface/A.gui", std::fs::read(&overridden_only).unwrap()),
        ])
        .expect("case-insensitive GUI_TYPE override must resolve");

        assert_eq!(catalog.layouts.stats.window_count, 4);
        assert_eq!(catalog.layouts.documents.len(), 3);
        assert_eq!(catalog.total_window_count(), 2);
        assert_eq!(catalog.effective_roots.len(), 2);
        assert_eq!(
            catalog
                .admitted_windows
                .iter()
                .find(|window| window.name == "IndependentRoot")
                .unwrap()
                .window_id,
            "interface/Z.gui#00002"
        );
        assert_eq!(catalog.effective_roots[1].source_file, "interface/a.gui");
        assert_eq!(catalog.effective_roots[1].root.name, "sharedroot");
        assert_eq!(catalog.files[0].source_file, "interface/A.gui");
        assert_eq!(catalog.files[0].admitted_windows, 0);
        assert_eq!(catalog.files[1].source_file, "interface/Z.gui");
        assert_eq!(catalog.files[1].admitted_windows, 1);
        assert_eq!(catalog.files[2].source_file, "interface/a.gui");
        assert_eq!(catalog.files[2].admitted_windows, 1);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn recovery_parsed_files_are_reported_and_still_admitted() {
        let dir = std::env::temp_dir().join(format!(
            "game_ui_defadm_recovery_{}_{}",
            std::process::id(),
            line!()
        ));
        fs::create_dir_all(&dir).unwrap();
        let clean = dir.join("clean.gui");
        let damaged = dir.join("damaged.gui");
        fs::write(
            &clean,
            b"guiTypes = { windowType = { name = \"clean_window\" } }",
        )
        .unwrap();
        fs::write(
            &damaged,
            b"guiTypes = { windowType = { name = \"damaged_window\" }",
        )
        .unwrap();
        let catalog = load_effective_definition_catalog(vec![
            ResolvedGuiDefinitionFile::new("interface/clean.gui", std::fs::read(&clean).unwrap()),
            ResolvedGuiDefinitionFile::new("interface/damaged.gui", std::fs::read(&damaged).unwrap()),
        ])
        .expect("a recovery-parsed .gui must still load");

        assert_eq!(catalog.rejected_file_count(), 0);
        assert_eq!(catalog.admitted_file_count(), catalog.files.len());
        let reported = catalog.recovery_parsed_files();
        assert_eq!(reported.len(), 1, "{reported:?}");
        assert!(
            reported[0].starts_with("interface/damaged.gui"),
            "{reported:?}"
        );
        assert!(catalog
            .admitted_windows
            .iter()
            .any(|row| row.name == "damaged_window"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn embedded_census_is_exact_and_has_integrated_dlc() {
        let census = frozen_census().expect("frozen census");
        assert_eq!(census.files.len(), FROZEN_FILE_COUNT);
        assert_eq!(
            census
                .files
                .iter()
                .map(|row| row.window_count)
                .sum::<usize>(),
            FROZEN_WINDOW_COUNT
        );
        assert!(census.files.iter().any(|row| {
            row.source_file == "interface/mtg_music.gui"
                && row.source_layer_kind == "integrated_dlc"
        }));
    }
}
