//! XUI01 production admission for the frozen effective GUI definition set.
//!
//! This layer admits file-derived definitions only. Runtime visibility,
//! population, interaction and pixels remain owned by later XUI gates.

use crate::gui::{GuiElement, LayoutSet};
use crate::{ErrorKind, GameUiError, Result};
use serde::Deserialize;
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const FROZEN_CENSUS_JSON: &str =
    include_str!("../../../re_evidence/rendering_laws/XUI01/file_census.json");
pub const FROZEN_FILE_COUNT: usize = 454;
pub const FROZEN_WINDOW_COUNT: usize = 9_155;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedGuiDefinitionFile {
    pub rel_path: String,
    pub path: PathBuf,
}

impl ResolvedGuiDefinitionFile {
    pub fn new(rel_path: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            rel_path: normalize_rel(&rel_path.into()),
            path: path.into(),
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileAdmission {
    pub source_file: String,
    pub definition_file_admitted: bool,
    pub admitted_windows: usize,
    pub rejected_windows: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GuiDefinitionCatalog {
    pub layouts: LayoutSet,
    pub files: Vec<FileAdmission>,
    pub admitted_windows: Vec<AdmittedWindowDefinition>,
    pub rejected_windows: Vec<RejectedWindowDefinition>,
}

impl GuiDefinitionCatalog {
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

/// Returns the exact frozen inputs by their retained playset locations.
/// This is useful for independent corpus verification. Production callers
/// with active mods should instead enumerate through their effective VFS and
/// pass those resolved rows to [`load_frozen_definition_catalog`].
pub fn frozen_playset_files(game_root: &Path) -> Result<Vec<ResolvedGuiDefinitionFile>> {
    let census = frozen_census()?;
    census
        .files
        .iter()
        .map(|row| {
            let path = match row.source_layer_kind.as_str() {
                "vanilla" => game_root.join(rel_to_path(&row.source_file)),
                "integrated_dlc" => game_root
                    .join("integrated_dlc")
                    .join(&row.source_layer)
                    .join(rel_to_path(&row.source_file)),
                kind => {
                    return Err(GameUiError::new(
                        ErrorKind::Other,
                        format!("XUI01 frozen census has unsupported layer kind {kind:?}"),
                    ))
                }
            };
            Ok(ResolvedGuiDefinitionFile::new(&row.source_file, path))
        })
        .collect()
}

/// Loads the frozen XUI01 definition namespace and admits every window whose
/// own semantic scope uses only explicit schemas. Any file/count/hash drift is
/// a fatal namespace rejection; genuinely unsupported widgets reject only the
/// owning window definition.
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
        if actual.insert(rel.clone(), row.path).is_some() {
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
        let path = actual
            .get(rel)
            .expect("missing rows were rejected before parse");
        let bytes = fs::read(path).map_err(|err| {
            GameUiError::new(
                ErrorKind::Io,
                format!("read XUI01 GUI {} at {}: {err}", rel, path.display()),
            )
        })?;
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
        layouts.parse_file(path, rel).map_err(|err| {
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

    let catalog = catalog_from_layouts(layouts);
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
    let mut folded = BTreeSet::new();
    let mut ordered = resolved_files;
    ordered.sort_by(|a, b| {
        a.rel_path
            .to_ascii_lowercase()
            .cmp(&b.rel_path.to_ascii_lowercase())
    });
    let mut layouts = LayoutSet::default();
    for row in ordered {
        let rel = normalize_rel(&row.rel_path);
        if !folded.insert(rel.to_ascii_lowercase()) {
            return invalid(format!("duplicate/case-colliding effective GUI path {rel}"));
        }
        layouts.parse_file(&row.path, &rel).map_err(|error| {
            GameUiError::new(
                ErrorKind::Parse,
                format!("parse effective GUI {rel}: {error}"),
            )
        })?;
    }
    if layouts.documents.is_empty() {
        return invalid("effective GUI namespace is empty");
    }
    Ok(catalog_from_layouts(layouts))
}

fn catalog_from_layouts(layouts: LayoutSet) -> GuiDefinitionCatalog {
    let mut files = Vec::with_capacity(layouts.documents.len());
    let mut admitted_windows = Vec::new();
    let mut rejected_windows = Vec::new();
    for document in &layouts.documents {
        let mut ordinal = 0usize;
        let before_admitted = admitted_windows.len();
        let before_rejected = rejected_windows.len();
        for root in &document.roots {
            admit_windows(
                root,
                "",
                &document.source_file,
                &mut ordinal,
                &mut admitted_windows,
                &mut rejected_windows,
            );
        }
        files.push(FileAdmission {
            source_file: document.source_file.clone(),
            definition_file_admitted: true,
            admitted_windows: admitted_windows.len() - before_admitted,
            rejected_windows: rejected_windows.len() - before_rejected,
        });
    }
    GuiDefinitionCatalog {
        layouts,
        files,
        admitted_windows,
        rejected_windows,
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
