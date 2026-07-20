//! A38/XUI22 generic engine-materialisation carrier boundary.
//!
//! D51 variables and flags remain the first replay evaluator inputs.  These
//! rows carry only residual results that the generic evaluator cannot resolve.
//! No scripted-GUI id, panel, trigger, or property is hard-coded here.

use std::collections::{BTreeMap, BTreeSet};

pub const EXACT_A729_EXECUTABLE_SHA256: [u8; 32] = [
    0x87, 0x0e, 0xbf, 0x1f, 0x61, 0x52, 0x62, 0xfa, 0xcc, 0xa4, 0x8a, 0x3b, 0x94, 0x77, 0x75, 0xc4,
    0x1f, 0x0a, 0x53, 0x38, 0xf3, 0x06, 0x8c, 0x40, 0x95, 0x82, 0x09, 0xf5, 0xc4, 0x1c, 0x65, 0xad,
];
pub const EXACT20_A38_JOB_ID: &str =
    "20260720T020812Z-w3-a38-controller-owner-41ee5208";
pub const EXACT20_A38_STATIC_AUTHORITY_RECEIPT_SHA256: [u8; 32] = [
    0x7d, 0x0a, 0x36, 0xb1, 0x82, 0x2f, 0xaf, 0x85, 0xa9, 0x49, 0xd6, 0x28, 0xa7, 0x9b, 0xc4, 0xa0,
    0x9f, 0x95, 0xdc, 0x9c, 0x9a, 0xa3, 0xed, 0x4a, 0xec, 0xb7, 0x15, 0xb7, 0x2b, 0x0a, 0x90, 0x82,
];
pub const MAX_GUI_ROWS: usize = 65_536;
pub const MAX_RESULTS_PER_MAP: usize = 131_072;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineResult<T> {
    Value(T),
    /// Authenticated, explicit absence.  It is never coerced to false/empty.
    Unavailable(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptedGuiDefinitionIdentity {
    /// Canonical game-relative path from the authenticated effective-VFS
    /// winner index. Absolute install/mod paths are never portable identity.
    pub logical_path: String,
    pub winning_layer_ordinal: u32,
    /// `base`, `integrated_dlc`, `dlc`, or `mod`.
    pub layer_kind: String,
    pub layer_id: String,
    /// Container hash for archive-backed content, payload hash otherwise.
    pub source_sha256: [u8; 32],
    pub payload_sha256: [u8; 32],
    pub payload_bytes: u64,
    /// Static registry identity joined from the winning scripted-GUI
    /// definition. These are not inferred from live widget strings.
    pub context_type: String,
    pub window_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptedGuiRuntimeRow {
    /// Position in `CScriptedWindowManager::updaters`. The row vector must
    /// retain this exact engine order; sorting by country or GUI id is invalid.
    pub manager_order_index: u32,
    pub country_tag: String,
    pub gui_id: String,
    pub definition: ScriptedGuiDefinitionIdentity,
    pub visible: EngineResult<bool>,
    pub enabled: EngineResult<bool>,
    /// Authenticated unsupported-name inventory from the effective definition.
    pub expected_trigger_names: BTreeSet<String>,
    pub triggers: BTreeMap<String, EngineResult<bool>>,
    /// Key is `(authored element, authored property)`; value is the exact
    /// engine-materialised string, including sprite tokens and frame values.
    pub expected_property_keys: BTreeSet<(String, String)>,
    pub properties: BTreeMap<(String, String), EngineResult<String>>,
    pub expected_effect_names: BTreeSet<String>,
    pub effect_enabled: BTreeMap<String, EngineResult<bool>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptedGuiRuntimeFrame {
    pub hour_index: u64,
    pub producer_record_identity: String,
    pub executable_sha256: [u8; 32],
    pub ui14_engine_vfs_sha256: [u8; 32],
    /// Fresh pinned-shard A38 Ghidra receipt. Legacy discovery manifests are
    /// not admitted here.
    pub static_authority_receipt_sha256: [u8; 32],
    pub producer_manifest_sha256: [u8; 32],
    /// Complete effective `(country_tag, gui_id)` set for this sampled context.
    pub expected_gui_keys: BTreeSet<(String, String)>,
    pub rows: Vec<ScriptedGuiRuntimeRow>,
    /// Tombstones for keys present in the previous frame but absent now.
    pub tombstones: BTreeSet<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CarrierError {
    UnauthenticatedSource,
    Source(String),
    Invalid(String),
}

pub trait ExactScriptedGuiRuntimeSource {
    fn is_exact_a729_scripted_gui_source(&self) -> bool;
    fn executable_sha256(&self) -> [u8; 32];
    fn ui14_engine_vfs_sha256(&self) -> [u8; 32];
    fn static_authority_receipt_sha256(&self) -> [u8; 32];
    fn producer_manifest_sha256(&self) -> [u8; 32];
    fn collect_runtime_frame(&self, hour_index: u64) -> Result<ScriptedGuiRuntimeFrame, String>;
    /// Authenticates every definition against the ephemeral effective-winner
    /// index and the exact registry used by this producer. This is where the
    /// source rejects self-declared paths, hashes, keys, or coverage.
    fn authenticate_runtime_frame(&self, frame: &ScriptedGuiRuntimeFrame) -> Result<(), String>;
}

pub fn collect_and_validate_scripted_gui_runtime<S: ExactScriptedGuiRuntimeSource>(
    source: &S,
    hour_index: u64,
) -> Result<ScriptedGuiRuntimeFrame, CarrierError> {
    if !source.is_exact_a729_scripted_gui_source() {
        return Err(CarrierError::UnauthenticatedSource);
    }
    let mut frame = source
        .collect_runtime_frame(hour_index)
        .map_err(CarrierError::Source)?;
    if frame.hour_index != hour_index {
        return invalid("requested and returned hour differ");
    }
    if frame.producer_manifest_sha256 != source.producer_manifest_sha256() {
        return invalid("frame/source manifest mismatch");
    }
    if frame.executable_sha256 != source.executable_sha256()
        || frame.ui14_engine_vfs_sha256 != source.ui14_engine_vfs_sha256()
        || frame.static_authority_receipt_sha256 != source.static_authority_receipt_sha256()
    {
        return invalid("frame/source executable, VFS, or static-authority identity mismatch");
    }
    source
        .authenticate_runtime_frame(&frame)
        .map_err(CarrierError::Source)?;
    sanitize_runtime_diagnostics(&mut frame);
    validate_scripted_gui_runtime_frame(&frame)?;
    Ok(frame)
}

pub fn validate_scripted_gui_runtime_frame(
    frame: &ScriptedGuiRuntimeFrame,
) -> Result<(), CarrierError> {
    if frame.executable_sha256 != EXACT_A729_EXECUTABLE_SHA256 {
        return invalid("scripted-GUI frame is not bound to exact a729 executable");
    }
    for (hash, label) in [
        (frame.ui14_engine_vfs_sha256, "effective VFS hash"),
        (frame.producer_manifest_sha256, "producer manifest hash"),
    ] {
        if hash == [0; 32] {
            return invalid(&format!("{label} is zero"));
        }
    }
    if frame.static_authority_receipt_sha256 != EXACT20_A38_STATIC_AUTHORITY_RECEIPT_SHA256 {
        return invalid("scripted-GUI frame is not bound to the admitted exact20 A38 receipt");
    }
    validate_portable_identity(&frame.producer_record_identity, "producer record identity")?;
    if frame.rows.len() > MAX_GUI_ROWS {
        return invalid("scripted-GUI row count exceeds bound");
    }

    let mut actual = BTreeSet::new();
    for (position, row) in frame.rows.iter().enumerate() {
        if usize::try_from(row.manager_order_index).ok() != Some(position) {
            return invalid("scripted-GUI rows do not retain contiguous manager updater order");
        }
        validate_country_tag(&row.country_tag)?;
        validate_text(&row.gui_id, "gui id")?;
        validate_definition_identity(&row.definition)?;
        let key = (row.country_tag.clone(), row.gui_id.clone());
        if !actual.insert(key) {
            return invalid("duplicate country/gui runtime row");
        }
        validate_result(&row.visible, "visible")?;
        validate_result(&row.enabled, "enabled")?;
        validate_named_results(&row.triggers, "trigger")?;
        validate_named_results(&row.effect_enabled, "effect-enabled")?;
        if row.triggers.keys().cloned().collect::<BTreeSet<_>>() != row.expected_trigger_names {
            return invalid("trigger results do not exactly cover authenticated unsupported names");
        }
        if row.effect_enabled.keys().cloned().collect::<BTreeSet<_>>() != row.expected_effect_names
        {
            return invalid(
                "effect-enabled results do not exactly cover authenticated effect names",
            );
        }
        if row.properties.len() > MAX_RESULTS_PER_MAP {
            return invalid("property result map exceeds bound");
        }
        for ((element, property), value) in &row.properties {
            validate_text(element, "property element")?;
            validate_text(property, "property name")?;
            validate_result(value, "property result")?;
            if let EngineResult::Value(value) = value {
                validate_text_allow_empty(value, "resolved property value")?;
            }
        }
        if row.properties.keys().cloned().collect::<BTreeSet<_>>() != row.expected_property_keys {
            return invalid(
                "property results do not exactly cover authenticated dynamic properties",
            );
        }
    }

    if actual != frame.expected_gui_keys {
        return invalid("runtime rows do not exactly cover authenticated effective GUI key set");
    }
    if !frame.tombstones.is_disjoint(&frame.expected_gui_keys) {
        return invalid("tombstone overlaps current effective GUI set");
    }
    for (tag, gui_id) in frame
        .expected_gui_keys
        .iter()
        .chain(frame.tombstones.iter())
    {
        validate_country_tag(tag)?;
        validate_text(gui_id, "expected/tombstoned gui id")?;
    }
    Ok(())
}

fn validate_definition_identity(
    definition: &ScriptedGuiDefinitionIdentity,
) -> Result<(), CarrierError> {
    validate_game_relative_path(&definition.logical_path)?;
    if !definition
        .logical_path
        .strip_prefix("common/scripted_guis/")
        .and_then(|relative| relative.strip_suffix(".txt"))
        .is_some_and(|stem| !stem.is_empty() && !stem.ends_with('/'))
        || !matches!(
            definition.layer_kind.as_str(),
            "base" | "integrated_dlc" | "dlc" | "mod"
        )
        || definition.layer_id.trim().is_empty()
        || definition.source_sha256 == [0; 32]
        || definition.payload_sha256 == [0; 32]
        || definition.payload_bytes == 0
    {
        return invalid("scripted-GUI definition winner identity is incomplete");
    }
    validate_portable_identity(&definition.layer_id, "definition layer id")?;
    validate_portable_identity(&definition.context_type, "definition context type")?;
    validate_portable_identity(&definition.window_name, "definition window name")?;
    Ok(())
}

fn validate_game_relative_path(value: &str) -> Result<(), CarrierError> {
    validate_text(value, "definition logical path")?;
    if value.contains('\\')
        || value.starts_with('/')
        || value.contains(':')
        || value.split('/').any(|part| part.is_empty() || part == "." || part == "..")
    {
        return invalid("definition path is not canonical game-relative identity");
    }
    Ok(())
}

fn validate_named_results<T>(
    values: &BTreeMap<String, EngineResult<T>>,
    label: &str,
) -> Result<(), CarrierError> {
    if values.len() > MAX_RESULTS_PER_MAP {
        return invalid(&format!("{label} result map exceeds bound"));
    }
    for (name, result) in values {
        validate_text(name, label)?;
        validate_result(result, label)?;
    }
    Ok(())
}

fn validate_result<T>(result: &EngineResult<T>, label: &str) -> Result<(), CarrierError> {
    if let EngineResult::Unavailable(reason) = result {
        validate_portable_identity(reason, &format!("{label} unavailable reason"))?;
    }
    Ok(())
}

fn validate_country_tag(tag: &str) -> Result<(), CarrierError> {
    if tag.len() != 3
        || !tag
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    {
        return invalid("country tag is not a canonical three-byte tag");
    }
    Ok(())
}

fn validate_text(value: &str, label: &str) -> Result<(), CarrierError> {
    if value.is_empty() {
        return invalid(&format!("{label} is empty"));
    }
    validate_text_allow_empty(value, label)
}

fn validate_portable_identity(value: &str, label: &str) -> Result<(), CarrierError> {
    validate_text(value, label)?;
    if value.trim().is_empty() || leaks_host_detail(value) {
        return invalid(&format!("{label} contains pointer or host-path detail"));
    }
    Ok(())
}

fn sanitize_runtime_diagnostics(frame: &mut ScriptedGuiRuntimeFrame) {
    fn sanitize<T>(result: &mut EngineResult<T>) {
        if let EngineResult::Unavailable(reason) = result {
            *reason = portable_unavailable_diagnostic(reason);
        }
    }
    for row in &mut frame.rows {
        sanitize(&mut row.visible);
        sanitize(&mut row.enabled);
        for result in row.triggers.values_mut() {
            sanitize(result);
        }
        for result in row.properties.values_mut() {
            sanitize(result);
        }
        for result in row.effect_enabled.values_mut() {
            sanitize(result);
        }
    }
}

fn leaks_host_detail(value: &str) -> bool {
    let bytes = value.as_bytes();
    let rooted_unix_token = bytes.iter().enumerate().any(|(index, byte)| {
        *byte == b'/'
            && (index == 0
                || bytes[index - 1].is_ascii_whitespace()
                || matches!(bytes[index - 1], b'(' | b'=' | b'\'' | b'"'))
    });
    if rooted_unix_token
        || value.contains("\\\\")
        || bytes.windows(3).any(|part| {
            part[0].is_ascii_alphabetic()
                && part[1] == b':'
                && (part[2] == b'\\' || part[2] == b'/')
        })
    {
        return true;
    }
    value.to_ascii_lowercase().as_bytes().windows(8).any(|part| {
        part[0] == b'0'
            && part[1] == b'x'
            && part[2..].iter().all(|byte| byte.is_ascii_hexdigit())
    })
}

pub(crate) fn portable_unavailable_diagnostic(value: &str) -> String {
    if leaks_host_detail(value) {
        "producer_diagnostic_redacted".to_string()
    } else {
        value.to_string()
    }
}

fn validate_text_allow_empty(value: &str, label: &str) -> Result<(), CarrierError> {
    if value.len() > 65_536 || value.chars().any(|ch| ch == '\0') {
        return invalid(&format!("{label} is unbounded or contains NUL"));
    }
    Ok(())
}

fn invalid<T>(message: &str) -> Result<T, CarrierError> {
    Err(CarrierError::Invalid(message.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> ScriptedGuiRuntimeRow {
        ScriptedGuiRuntimeRow {
            manager_order_index: 0,
            country_tag: "USA".into(),
            gui_id: "usa_congress".into(),
            definition: ScriptedGuiDefinitionIdentity {
                logical_path: "common/scripted_guis/USA_congress.txt".into(),
                winning_layer_ordinal: 0,
                layer_kind: "base".into(),
                layer_id: "base".into(),
                source_sha256: [3; 32],
                payload_sha256: [3; 32],
                payload_bytes: 64,
                context_type: "country".into(),
                window_name: "usa_congress_window".into(),
            },
            visible: EngineResult::Value(true),
            enabled: EngineResult::Unavailable("unsupported country scope".into()),
            expected_trigger_names: BTreeSet::from(["senate_visible".into()]),
            triggers: BTreeMap::from([("senate_visible".into(), EngineResult::Value(true))]),
            expected_property_keys: BTreeSet::from([("senate_icon".into(), "image".into())]),
            properties: BTreeMap::from([(
                ("senate_icon".into(), "image".into()),
                EngineResult::Value("GFX_senate".into()),
            )]),
            expected_effect_names: BTreeSet::from(["invest".into()]),
            effect_enabled: BTreeMap::from([(
                "invest".into(),
                EngineResult::Unavailable("engine-only scripted trigger".into()),
            )]),
        }
    }

    fn frame() -> ScriptedGuiRuntimeFrame {
        ScriptedGuiRuntimeFrame {
            hour_index: 9,
            producer_record_identity: "a38:9:registry-7".into(),
            executable_sha256: EXACT_A729_EXECUTABLE_SHA256,
            ui14_engine_vfs_sha256: [1; 32],
            static_authority_receipt_sha256: EXACT20_A38_STATIC_AUTHORITY_RECEIPT_SHA256,
            producer_manifest_sha256: [2; 32],
            expected_gui_keys: BTreeSet::from([("USA".into(), "usa_congress".into())]),
            rows: vec![row()],
            tombstones: BTreeSet::new(),
        }
    }

    #[test]
    fn generic_complete_frame_accepts_concrete_and_explicit_unavailable_results() {
        validate_scripted_gui_runtime_frame(&frame()).unwrap();
    }

    #[test]
    fn missing_registry_rows_and_implicit_unavailability_fail_closed() {
        let mut value = frame();
        value.rows.clear();
        assert!(validate_scripted_gui_runtime_frame(&value).is_err());
        let mut value = frame();
        value.rows[0].enabled = EngineResult::Unavailable(String::new());
        assert!(validate_scripted_gui_runtime_frame(&value).is_err());
        let mut value = frame();
        value
            .tombstones
            .insert(("USA".into(), "usa_congress".into()));
        assert!(validate_scripted_gui_runtime_frame(&value).is_err());
        let mut value = frame();
        value.rows[0].triggers.clear();
        assert!(validate_scripted_gui_runtime_frame(&value).is_err());
    }

    #[test]
    fn canonical_definition_identity_and_portable_diagnostics_are_mandatory() {
        for bad_path in [
            "C:/game/common/scripted_guis/a.txt",
            "common\\scripted_guis\\a.txt",
            "common/scripted_guis/../a.txt",
            "common/scripted_guis/a.gui",
        ] {
            let mut value = frame();
            value.rows[0].definition.logical_path = bad_path.into();
            assert!(validate_scripted_gui_runtime_frame(&value).is_err(), "{bad_path}");
        }
        let mut value = frame();
        value.rows[0].definition.layer_kind = "local_folder".into();
        assert!(validate_scripted_gui_runtime_frame(&value).is_err());
        let mut value = frame();
        value.rows[0].definition.payload_bytes = 0;
        assert!(validate_scripted_gui_runtime_frame(&value).is_err());
        let mut value = frame();
        value.rows[0].definition.source_sha256 = [0; 32];
        assert!(validate_scripted_gui_runtime_frame(&value).is_err());

        let mut value = frame();
        value.rows[0].enabled =
            EngineResult::Unavailable("failed at C:\\Users\\Example\\capture.bin 0x141234567".into());
        assert!(validate_scripted_gui_runtime_frame(&value).is_err());
        sanitize_runtime_diagnostics(&mut value);
        assert_eq!(
            value.rows[0].enabled,
            EngineResult::Unavailable("producer_diagnostic_redacted".into())
        );
        validate_scripted_gui_runtime_frame(&value).unwrap();

        let mut mod_value = frame();
        mod_value.rows[0].definition.layer_kind = "mod".into();
        mod_value.rows[0].definition.layer_id = "workshop:3470184538".into();
        mod_value.rows[0].definition.logical_path =
            "common/scripted_guis/subfolder/panel.txt".into();
        validate_scripted_gui_runtime_frame(&mod_value).unwrap();
    }

    struct Source {
        authenticated: bool,
    }

    impl ExactScriptedGuiRuntimeSource for Source {
        fn is_exact_a729_scripted_gui_source(&self) -> bool {
            self.authenticated
        }
        fn producer_manifest_sha256(&self) -> [u8; 32] {
            [2; 32]
        }
        fn executable_sha256(&self) -> [u8; 32] {
            EXACT_A729_EXECUTABLE_SHA256
        }
        fn ui14_engine_vfs_sha256(&self) -> [u8; 32] {
            [1; 32]
        }
        fn static_authority_receipt_sha256(&self) -> [u8; 32] {
            EXACT20_A38_STATIC_AUTHORITY_RECEIPT_SHA256
        }
        fn authenticate_runtime_frame(
            &self,
            frame: &ScriptedGuiRuntimeFrame,
        ) -> Result<(), String> {
            if frame
                .rows
                .iter()
                .all(|row| row.definition.logical_path.starts_with("common/scripted_guis/"))
            {
                Ok(())
            } else {
                Err("definition absent from authenticated winner index".into())
            }
        }
        fn collect_runtime_frame(
            &self,
            _hour_index: u64,
        ) -> Result<ScriptedGuiRuntimeFrame, String> {
            Ok(frame())
        }
    }

    #[test]
    fn source_authentication_precedes_runtime_admission() {
        assert_eq!(
            collect_and_validate_scripted_gui_runtime(
                &Source {
                    authenticated: false
                },
                9
            ),
            Err(CarrierError::UnauthenticatedSource)
        );
        assert!(collect_and_validate_scripted_gui_runtime(
            &Source {
                authenticated: true
            },
            10
        )
        .is_err());
        assert!(collect_and_validate_scripted_gui_runtime(
            &Source {
                authenticated: true
            },
            9
        )
        .is_ok());
    }
}
