//! A25 exact runtime-text carrier admission.
//!
//! The engine has already selected scripted localisation, grammar and numeric
//! formatting before these rows are recorded.  This module validates the
//! recorded final bytes and exposes them without re-evaluating Clausewitz
//! expressions.  Static localisation remains outside this sparse carrier.

use crate::world::{Leaf, WorldFrame};
use portable_render_stream::{
    AuthenticatedUiPass, HudCommandExecution, HudPaintCommand, UiPassOperation,
};
use serde::Deserialize;
use serde_json::Value;
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const RUNTIME_TEXT_SCHEMA: &str = "hoi4-runtime-text-v1";
pub const A729_TEXT_SETTER_RVA: u32 = 0x022B_59E0;
pub const A729_FORMATTER_RVA: u32 = 0x0223_2AB0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeTextCatalog {
    pub contexts_complete: bool,
    pub contexts: BTreeMap<String, RuntimeTextRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeTextRecord {
    pub context_id: String,
    pub frame_hour: i64,
    pub pov_country_or_global: String,
    pub view_id: String,
    pub element_id: String,
    pub source: RuntimeTextSource,
    /// Exact post-setter bytes selected by the engine text-layout object.
    pub final_text_bytes: Vec<u8>,
    /// UTF-8 interpretation mechanically verified against `final_text_bytes`.
    pub final_text_utf8: String,
    pub final_text_sha256: String,
    pub variables: BTreeMap<String, String>,
    pub data_functions: BTreeMap<String, String>,
    pub dynamic_colors: BTreeMap<String, char>,
    pub layout: RuntimeTextLayout,
    pub producer: RuntimeTextProducer,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RuntimeTextSource {
    LocalisationKey(String),
    Literal(String),
    /// Exact raw `text = ...` token retained from `CTextBoxType+0x138` by
    /// the authenticated A25 registration/setter join.
    AuthoredToken(String),
    /// No authored token was present; the proven final-setter caller RVA is
    /// the only exact source identity available.
    SetterCallerRva(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeTextLayout {
    /// The final string is reproducible with the admitted authored rectangle,
    /// alignment and effective BMFont.  No duplicate glyph stream is stored.
    AuthoredReproducible,
    /// The engine selected a result which cannot be reconstructed from static
    /// assets (fallback font, truncation, or inline icons).  Runs are relative
    /// to the owning element's top-left.
    CapturedRuntime(RuntimeTextCapturedLayout),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeTextCapturedLayout {
    /// Owner-top-left-relative `[x, y, width, height]`. `None` means there is
    /// no runtime-layout clip; the owning HUD destination is not an implicit
    /// substitute clip.
    pub clip: Option<[i32; 4]>,
    pub glyphs: Vec<RuntimeGlyphRun>,
    pub inline_icons: Vec<RuntimeInlineIconRun>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeGlyphRun {
    pub font: String,
    pub glyph_id: u32,
    pub x: i32,
    pub y: i32,
    pub rgba: [u8; 4],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeInlineIconRun {
    pub sprite: String,
    pub frame: u32,
    pub x: i32,
    pub y: i32,
    pub baseline: i32,
    pub advance: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeTextProducer {
    pub setter_entry_rva: u32,
    /// Exact setter caller when it was observed live. A pre-hook baseline row
    /// with an authored registration token has no lawful historical caller.
    pub setter_caller_rva: Option<u32>,
    pub formatter_entry_rva: Option<u32>,
    pub identity_source: RuntimeTextIdentitySource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeTextIdentitySource {
    AuthoredWidgetRegistration,
    ProvenSetterCaller,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeTextAdmissionError {
    pub path: String,
    pub message: String,
}

impl RuntimeTextAdmissionError {
    fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for RuntimeTextAdmissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

impl std::error::Error for RuntimeTextAdmissionError {}

impl WorldFrame {
    /// Admit the sparse A25 carrier from the untouched snapshot domain.
    ///
    /// Missing, unobtained and measured-empty are deliberately distinct:
    /// no key -> `Absent`; `state=unobtained` -> `Unobtained`; and a measured
    /// envelope with `contexts=[]` -> `Measured` with an empty catalog.
    pub fn runtime_text(&self) -> Result<Leaf<RuntimeTextCatalog>, RuntimeTextAdmissionError> {
        admit_runtime_text(self.domain("runtime_text"))
    }
}

pub fn admit_runtime_text(
    value: Option<&Value>,
) -> Result<Leaf<RuntimeTextCatalog>, RuntimeTextAdmissionError> {
    let Some(value) = value else {
        return Ok(Leaf::Absent);
    };
    let envelope: WireEnvelope = WireEnvelope::deserialize(value).map_err(|error| {
        RuntimeTextAdmissionError::new("runtime_text", format!("invalid envelope: {error}"))
    })?;
    if envelope.schema != RUNTIME_TEXT_SCHEMA {
        return Err(RuntimeTextAdmissionError::new(
            "runtime_text.schema",
            format!("expected {RUNTIME_TEXT_SCHEMA}, got {}", envelope.schema),
        ));
    }
    match envelope.state {
        WireState::Unobtained => {
            if envelope.contexts_complete.is_some() || envelope.contexts.is_some() {
                return Err(RuntimeTextAdmissionError::new(
                    "runtime_text",
                    "unobtained carrier must not contain contexts",
                ));
            }
            Ok(Leaf::Unobtained)
        }
        WireState::Measured => {
            let complete = envelope.contexts_complete.ok_or_else(|| {
                RuntimeTextAdmissionError::new(
                    "runtime_text.contexts_complete",
                    "required for measured carrier",
                )
            })?;
            let rows = envelope.contexts.ok_or_else(|| {
                RuntimeTextAdmissionError::new(
                    "runtime_text.contexts",
                    "required for measured carrier",
                )
            })?;
            let mut contexts = BTreeMap::new();
            let mut semantic_keys = BTreeSet::new();
            for (index, row) in rows.into_iter().enumerate() {
                let path = format!("runtime_text.contexts[{index}]");
                let admitted = admit_record(row, &path)?;
                let semantic = (
                    admitted.frame_hour,
                    admitted.pov_country_or_global.clone(),
                    admitted.view_id.clone(),
                    admitted.element_id.clone(),
                    admitted.source.clone(),
                );
                if !semantic_keys.insert(semantic) {
                    return Err(RuntimeTextAdmissionError::new(
                        path,
                        "duplicate semantic runtime-text row",
                    ));
                }
                if contexts
                    .insert(admitted.context_id.clone(), admitted)
                    .is_some()
                {
                    return Err(RuntimeTextAdmissionError::new(path, "duplicate context_id"));
                }
            }
            Ok(Leaf::Measured(RuntimeTextCatalog {
                contexts_complete: complete,
                contexts,
            }))
        }
    }
}

/// Bind every typed selected-Present text command to the exact A25 row that
/// produced it. The command carries only recorded dynamic identity tokens;
/// this join never consults a vanilla view/widget catalog.
pub(crate) fn validate_ui_pass_runtime_text_bindings(
    pass: &AuthenticatedUiPass,
    runtime_text: &Leaf<RuntimeTextCatalog>,
    frame_hour: i64,
) -> Result<(), String> {
    let mut bindings = Vec::new();
    for operation in &pass.ui_pass_operations {
        let UiPassOperation::HudCommand { command } = operation else {
            continue;
        };
        let HudCommandExecution::InterpretedPaint {
            paint:
                HudPaintCommand::Text {
                    runtime_text_context_id,
                    runtime_text_frame_hour,
                    runtime_text_pov_country_or_global,
                    runtime_text_view_id,
                    runtime_text_element_id,
                    ..
                },
        } = &command.execution
        else {
            continue;
        };
        bindings.push(RuntimeTextCommandBinding {
            ordinal: command.ordinal,
            context_id: runtime_text_context_id,
            frame_hour: *runtime_text_frame_hour,
            pov_country_or_global: runtime_text_pov_country_or_global,
            view_id: runtime_text_view_id,
            element_id: runtime_text_element_id,
        });
    }
    validate_runtime_text_command_bindings(&bindings, runtime_text, frame_hour)
}

#[derive(Clone, Copy)]
struct RuntimeTextCommandBinding<'a> {
    ordinal: u32,
    context_id: &'a str,
    frame_hour: i64,
    pov_country_or_global: &'a str,
    view_id: &'a str,
    element_id: &'a str,
}

fn validate_runtime_text_command_bindings(
    bindings: &[RuntimeTextCommandBinding<'_>],
    runtime_text: &Leaf<RuntimeTextCatalog>,
    frame_hour: i64,
) -> Result<(), String> {
    if bindings.is_empty() {
        return Ok(());
    }
    let Leaf::Measured(catalog) = runtime_text else {
        return Err("typed UI text commands require measured A25 runtime_text".into());
    };
    if !catalog.contexts_complete {
        return Err("typed UI text commands require complete measured A25 runtime_text".into());
    }
    for binding in bindings {
        let record = catalog.contexts.get(binding.context_id).ok_or_else(|| {
            format!(
                "typed UI text command {} references missing A25 context {}",
                binding.ordinal, binding.context_id
            )
        })?;
        validate_runtime_text_record_binding(
            binding.ordinal,
            frame_hour,
            binding.frame_hour,
            binding.pov_country_or_global,
            binding.view_id,
            binding.element_id,
            record,
        )?;
    }
    Ok(())
}

fn validate_runtime_text_record_binding(
    ordinal: u32,
    frame_hour: i64,
    bound_hour: i64,
    bound_pov: &str,
    bound_view: &str,
    bound_element: &str,
    record: &RuntimeTextRecord,
) -> Result<(), String> {
    if bound_hour != frame_hour || record.frame_hour != frame_hour {
        return Err(format!(
            "typed UI text command {ordinal} A25 frame-hour identity mismatch"
        ));
    }
    if bound_pov != record.pov_country_or_global {
        return Err(format!(
            "typed UI text command {ordinal} A25 POV identity mismatch"
        ));
    }
    if bound_view != record.view_id {
        return Err(format!(
            "typed UI text command {ordinal} A25 view identity mismatch"
        ));
    }
    if bound_element != record.element_id {
        return Err(format!(
            "typed UI text command {ordinal} A25 element identity mismatch"
        ));
    }
    Ok(())
}

fn admit_record(
    row: WireRecord,
    path: &str,
) -> Result<RuntimeTextRecord, RuntimeTextAdmissionError> {
    require_nonempty(&row.context_id, path, "context_id")?;
    require_nonempty(&row.pov_country_or_global, path, "pov_country_or_global")?;
    require_nonempty(&row.view_id, path, "view_id")?;
    require_nonempty(&row.element_id, path, "element_id")?;
    let source = match row.source {
        WireSource::LocalisationKey { identity } => {
            require_nonempty(&identity, path, "source.identity")?;
            RuntimeTextSource::LocalisationKey(identity)
        }
        WireSource::Literal { identity } => RuntimeTextSource::Literal(identity),
        WireSource::AuthoredToken { identity } => {
            require_nonempty(&identity, path, "source.identity")?;
            RuntimeTextSource::AuthoredToken(identity)
        }
        WireSource::SetterCallerRva { rva } => {
            if rva == 0 {
                return Err(RuntimeTextAdmissionError::new(
                    format!("{path}.source.rva"),
                    "zero caller RVA is not an identity",
                ));
            }
            RuntimeTextSource::SetterCallerRva(rva)
        }
    };
    let bytes = decode_hex(&row.final_text.bytes_hex).map_err(|message| {
        RuntimeTextAdmissionError::new(format!("{path}.final_text.bytes_hex"), message)
    })?;
    let decoded = std::str::from_utf8(&bytes).map_err(|error| {
        RuntimeTextAdmissionError::new(
            format!("{path}.final_text.bytes_hex"),
            format!("not UTF-8: {error}"),
        )
    })?;
    if decoded != row.final_text.utf8 {
        return Err(RuntimeTextAdmissionError::new(
            format!("{path}.final_text.utf8"),
            "does not exactly match bytes_hex",
        ));
    }
    let digest = Sha256::hex(&bytes);
    if !eq_lower_hex_64(&row.final_text.sha256, &digest) {
        return Err(RuntimeTextAdmissionError::new(
            format!("{path}.final_text.sha256"),
            "does not match exact final bytes",
        ));
    }
    let dynamic_colors = row
        .dynamic_colors
        .into_iter()
        .map(|(key, value)| {
            let mut chars = value.chars();
            match (chars.next(), chars.next()) {
                (Some(code), None) => Ok((key, code)),
                _ => Err(RuntimeTextAdmissionError::new(
                    format!("{path}.dynamic_colors.{key}"),
                    "colour result must be exactly one Unicode scalar",
                )),
            }
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let layout = match row.layout {
        WireLayout::AuthoredReproducible {} => {
            if requires_captured_runtime_layout(decoded) {
                return Err(RuntimeTextAdmissionError::new(
                    format!("{path}.layout"),
                    "engine post-layout text contains inline/control data requiring captured runtime layout",
                ));
            }
            RuntimeTextLayout::AuthoredReproducible
        }
        WireLayout::CapturedRuntime {
            clip,
            glyphs,
            inline_icons,
        } => {
            if glyphs.is_empty() && inline_icons.is_empty() && !bytes.is_empty() {
                return Err(RuntimeTextAdmissionError::new(
                    format!("{path}.layout"),
                    "non-empty text has an empty captured runtime layout",
                ));
            }
            if let Some([x, y, width, height]) = clip {
                if width <= 0 || height <= 0 {
                    return Err(RuntimeTextAdmissionError::new(
                        format!("{path}.layout.clip"),
                        "captured clip width and height must be positive",
                    ));
                }
                if x.checked_add(width).is_none() || y.checked_add(height).is_none() {
                    return Err(RuntimeTextAdmissionError::new(
                        format!("{path}.layout.clip"),
                        "captured clip extent overflows i32",
                    ));
                }
            }
            for (index, glyph) in glyphs.iter().enumerate() {
                require_nonempty(
                    &glyph.font,
                    &format!("{path}.layout.glyphs[{index}]"),
                    "font",
                )?;
            }
            for (index, icon) in inline_icons.iter().enumerate() {
                let icon_path = format!("{path}.layout.inline_icons[{index}]");
                require_nonempty(&icon.sprite, &icon_path, "sprite")?;
                if icon.advance <= 0 {
                    return Err(RuntimeTextAdmissionError::new(
                        format!("{icon_path}.advance"),
                        "inline-icon advance must be positive",
                    ));
                }
                if icon.x.checked_add(icon.advance).is_none()
                    || icon.y.checked_add(icon.baseline).is_none()
                {
                    return Err(RuntimeTextAdmissionError::new(
                        icon_path,
                        "inline-icon coordinate/extent overflows i32",
                    ));
                }
            }
            RuntimeTextLayout::CapturedRuntime(RuntimeTextCapturedLayout {
                clip,
                glyphs: glyphs
                    .into_iter()
                    .map(|glyph| RuntimeGlyphRun {
                        font: glyph.font,
                        glyph_id: glyph.glyph_id,
                        x: glyph.x,
                        y: glyph.y,
                        rgba: glyph.rgba,
                    })
                    .collect(),
                inline_icons: inline_icons
                    .into_iter()
                    .map(|icon| RuntimeInlineIconRun {
                        sprite: icon.sprite,
                        frame: icon.frame,
                        x: icon.x,
                        y: icon.y,
                        baseline: icon.baseline,
                        advance: icon.advance,
                    })
                    .collect(),
            })
        }
    };
    if row.producer.setter_entry_rva != A729_TEXT_SETTER_RVA {
        return Err(RuntimeTextAdmissionError::new(
            format!("{path}.producer.setter_entry_rva"),
            format!("expected exact a729 setter RVA 0x{A729_TEXT_SETTER_RVA:X}"),
        ));
    }
    if row.producer.setter_caller_rva == Some(0) {
        return Err(RuntimeTextAdmissionError::new(
            format!("{path}.producer.setter_caller_rva"),
            "zero caller RVA is not an identity",
        ));
    }
    if let Some(formatter) = row.producer.formatter_entry_rva {
        if formatter != A729_FORMATTER_RVA {
            return Err(RuntimeTextAdmissionError::new(
                format!("{path}.producer.formatter_entry_rva"),
                format!("expected exact a729 formatter RVA 0x{A729_FORMATTER_RVA:X}"),
            ));
        }
    }
    let identity_source = match row.producer.identity_source {
        WireIdentitySource::AuthoredWidgetRegistration => {
            RuntimeTextIdentitySource::AuthoredWidgetRegistration
        }
        WireIdentitySource::ProvenSetterCaller => RuntimeTextIdentitySource::ProvenSetterCaller,
    };
    match (&source, identity_source, row.producer.setter_caller_rva) {
        (
            RuntimeTextSource::SetterCallerRva(source_rva),
            RuntimeTextIdentitySource::ProvenSetterCaller,
            Some(producer_rva),
        ) if *source_rva == producer_rva => {}
        (RuntimeTextSource::SetterCallerRva(_), _, _) => {
            return Err(RuntimeTextAdmissionError::new(
                format!("{path}.producer"),
                "setter-caller source requires the same proven producer caller RVA",
            ));
        }
        (
            RuntimeTextSource::AuthoredToken(_),
            RuntimeTextIdentitySource::AuthoredWidgetRegistration,
            _,
        ) => {}
        (RuntimeTextSource::AuthoredToken(_), _, _) => {
            return Err(RuntimeTextAdmissionError::new(
                format!("{path}.producer.identity_source"),
                "authored token requires authored-widget registration identity",
            ));
        }
        (_, RuntimeTextIdentitySource::ProvenSetterCaller, None) => {
            return Err(RuntimeTextAdmissionError::new(
                format!("{path}.producer.setter_caller_rva"),
                "proven setter-caller identity requires an observed caller RVA",
            ));
        }
        _ => {}
    }
    Ok(RuntimeTextRecord {
        context_id: row.context_id,
        frame_hour: row.frame_hour,
        pov_country_or_global: row.pov_country_or_global,
        view_id: row.view_id,
        element_id: row.element_id,
        source,
        final_text_bytes: bytes,
        final_text_utf8: row.final_text.utf8,
        final_text_sha256: digest,
        variables: row.variables,
        data_functions: row.data_functions,
        dynamic_colors,
        layout,
        producer: RuntimeTextProducer {
            setter_entry_rva: row.producer.setter_entry_rva,
            setter_caller_rva: row.producer.setter_caller_rva,
            formatter_entry_rva: row.producer.formatter_entry_rva,
            identity_source,
        },
    })
}

/// U+0011 is the authenticated engine colour lead and newlines are already
/// selected by the engine layout object. Other C0 controls and pound-sign icon
/// syntax require captured glyph/icon runs; replay must not silently strip them.
fn requires_captured_runtime_layout(text: &str) -> bool {
    text.chars()
        .any(|ch| ch == '\u{00a3}' || (ch.is_control() && !matches!(ch, '\n' | '\r' | '\u{0011}')))
}

fn require_nonempty(value: &str, path: &str, field: &str) -> Result<(), RuntimeTextAdmissionError> {
    if value.trim().is_empty() {
        Err(RuntimeTextAdmissionError::new(
            format!("{path}.{field}"),
            "must not be empty",
        ))
    } else {
        Ok(())
    }
}

fn decode_hex(value: &str) -> Result<Vec<u8>, &'static str> {
    if value.len() % 2 != 0 {
        return Err("hex length must be even");
    }
    let mut out = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().chunks_exact(2) {
        let hi = hex_nibble(pair[0]).ok_or("invalid hex digit")?;
        let lo = hex_nibble(pair[1]).ok_or("invalid hex digit")?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn eq_lower_hex_64(value: &str, expected: &str) -> bool {
    value.len() == 64
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        && value.eq_ignore_ascii_case(expected)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireEnvelope {
    schema: String,
    state: WireState,
    contexts_complete: Option<bool>,
    contexts: Option<Vec<WireRecord>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum WireState {
    Unobtained,
    Measured,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRecord {
    context_id: String,
    frame_hour: i64,
    pov_country_or_global: String,
    view_id: String,
    element_id: String,
    source: WireSource,
    final_text: WireFinalText,
    #[serde(default)]
    variables: BTreeMap<String, String>,
    #[serde(default)]
    data_functions: BTreeMap<String, String>,
    #[serde(default)]
    dynamic_colors: BTreeMap<String, String>,
    layout: WireLayout,
    producer: WireProducer,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum WireSource {
    LocalisationKey { identity: String },
    Literal { identity: String },
    AuthoredToken { identity: String },
    SetterCallerRva { rva: u32 },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireFinalText {
    bytes_hex: String,
    utf8: String,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum WireLayout {
    AuthoredReproducible {},
    CapturedRuntime {
        clip: Option<[i32; 4]>,
        #[serde(default)]
        glyphs: Vec<WireGlyph>,
        #[serde(default)]
        inline_icons: Vec<WireIcon>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireGlyph {
    font: String,
    glyph_id: u32,
    x: i32,
    y: i32,
    rgba: [u8; 4],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireIcon {
    sprite: String,
    frame: u32,
    x: i32,
    y: i32,
    baseline: i32,
    advance: i32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireProducer {
    setter_entry_rva: u32,
    setter_caller_rva: Option<u32>,
    formatter_entry_rva: Option<u32>,
    identity_source: WireIdentitySource,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum WireIdentitySource {
    AuthoredWidgetRegistration,
    ProvenSetterCaller,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn final_text(text: &str) -> Value {
        let bytes = text.as_bytes();
        json!({
            "bytes_hex": bytes.iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
            "utf8": text,
            "sha256": Sha256::hex(bytes),
        })
    }

    fn row(text: &str) -> Value {
        json!({
            "context_id": "rt:1",
            "frame_hour": 40,
            "pov_country_or_global": "FRA",
            "view_id": "topbar",
            "element_id": "stability",
            "source": {"kind": "localisation_key", "identity": "STABILITY"},
            "final_text": final_text(text),
            "variables": {"VAL|%0": "52%"},
            "data_functions": {},
            "dynamic_colors": {},
            "layout": {"mode": "authored_reproducible"},
            "producer": {
                "setter_entry_rva": A729_TEXT_SETTER_RVA,
                "setter_caller_rva": 0x0188_A0DEu32,
                "formatter_entry_rva": A729_FORMATTER_RVA,
                "identity_source": "proven_setter_caller"
            }
        })
    }

    fn captured_row(text: &str) -> Value {
        let mut value = row(text);
        value["layout"] = json!({
            "mode": "captured_runtime",
            "clip": [-4, 2, 80, 24],
            "glyphs": [{
                "font": "hoi4_16",
                "glyph_id": 17,
                "x": -2,
                "y": 3,
                "rgba": [248, 249, 250, 255]
            }],
            "inline_icons": [{
                "sprite": "GFX_inline_command_power",
                "frame": 0,
                "x": 12,
                "y": 1,
                "baseline": 14,
                "advance": 18
            }]
        });
        value
    }

    #[test]
    fn distinguishes_absent_unobtained_and_measured_empty() {
        assert_eq!(admit_runtime_text(None).unwrap(), Leaf::Absent);
        let unobtained = json!({"schema": RUNTIME_TEXT_SCHEMA, "state": "unobtained"});
        assert_eq!(
            admit_runtime_text(Some(&unobtained)).unwrap(),
            Leaf::Unobtained
        );
        let empty = json!({
            "schema": RUNTIME_TEXT_SCHEMA,
            "state": "measured",
            "contexts_complete": true,
            "contexts": []
        });
        let Leaf::Measured(catalog) = admit_runtime_text(Some(&empty)).unwrap() else {
            panic!("expected measured catalog")
        };
        assert!(catalog.contexts_complete);
        assert!(catalog.contexts.is_empty());
    }

    #[test]
    fn admits_exact_a729_setter_bytes() {
        let value = json!({
            "schema": RUNTIME_TEXT_SCHEMA,
            "state": "measured",
            "contexts_complete": true,
            "contexts": [row("\u{11}R17.62M\u{11}!")]
        });
        let Leaf::Measured(catalog) = admit_runtime_text(Some(&value)).unwrap() else {
            panic!("expected measured catalog")
        };
        let admitted = &catalog.contexts["rt:1"];
        assert_eq!(admitted.final_text_bytes[0], 0x11);
        assert_eq!(admitted.producer.setter_entry_rva, A729_TEXT_SETTER_RVA);
    }

    #[test]
    fn authored_layout_rejects_inline_controls_and_pound_icon_tokens() {
        for text in ["cost \u{13}abc", "cost £political_power"] {
            let value = json!({
                "schema": RUNTIME_TEXT_SCHEMA,
                "state": "measured",
                "contexts_complete": true,
                "contexts": [row(text)]
            });
            assert!(admit_runtime_text(Some(&value)).is_err());
        }

        let value = json!({
            "schema": RUNTIME_TEXT_SCHEMA,
            "state": "measured",
            "contexts_complete": true,
            "contexts": [captured_row("cost £political_power")]
        });
        assert!(admit_runtime_text(Some(&value)).is_ok());
    }

    #[test]
    fn exact_ui_text_identity_matches_a25_and_rejects_each_dynamic_axis() {
        let value = json!({
            "schema": RUNTIME_TEXT_SCHEMA,
            "state": "measured",
            "contexts_complete": true,
            "contexts": [row("52%")]
        });
        let Leaf::Measured(catalog) = admit_runtime_text(Some(&value)).unwrap() else {
            panic!("expected measured catalog")
        };
        let record = &catalog.contexts["rt:1"];
        let binding = RuntimeTextCommandBinding {
            ordinal: 7,
            context_id: "rt:1",
            frame_hour: 40,
            pov_country_or_global: "FRA",
            view_id: "topbar",
            element_id: "stability",
        };
        validate_runtime_text_command_bindings(&[binding], &Leaf::Measured(catalog.clone()), 40)
            .unwrap();
        assert!(validate_runtime_text_command_bindings(&[binding], &Leaf::Absent, 40).is_err());
        assert!(validate_runtime_text_command_bindings(&[binding], &Leaf::Unobtained, 40).is_err());
        let mut incomplete = catalog.clone();
        incomplete.contexts_complete = false;
        assert!(validate_runtime_text_command_bindings(
            &[binding],
            &Leaf::Measured(incomplete),
            40,
        )
        .is_err());
        let mut missing = binding;
        missing.context_id = "rt:missing";
        assert!(validate_runtime_text_command_bindings(
            &[missing],
            &Leaf::Measured(catalog.clone()),
            40,
        )
        .is_err());
        validate_runtime_text_record_binding(7, 40, 40, "FRA", "topbar", "stability", record)
            .unwrap();

        for (case, result) in [
            (
                "outer_hour",
                validate_runtime_text_record_binding(
                    7,
                    41,
                    40,
                    "FRA",
                    "topbar",
                    "stability",
                    record,
                ),
            ),
            (
                "bound_hour",
                validate_runtime_text_record_binding(
                    7,
                    40,
                    41,
                    "FRA",
                    "topbar",
                    "stability",
                    record,
                ),
            ),
            (
                "pov",
                validate_runtime_text_record_binding(
                    7,
                    40,
                    40,
                    "GER",
                    "topbar",
                    "stability",
                    record,
                ),
            ),
            (
                "view",
                validate_runtime_text_record_binding(
                    7,
                    40,
                    40,
                    "FRA",
                    "modded_topbar",
                    "stability",
                    record,
                ),
            ),
            (
                "element",
                validate_runtime_text_record_binding(
                    7,
                    40,
                    40,
                    "FRA",
                    "topbar",
                    "modded_stability",
                    record,
                ),
            ),
        ] {
            assert!(
                result.is_err(),
                "mismatched A25 identity axis admitted: {case}"
            );
        }
    }

    #[test]
    fn admits_checked_owner_relative_captured_layout_without_implicit_clip() {
        let value = json!({
            "schema": RUNTIME_TEXT_SCHEMA,
            "state": "measured",
            "contexts_complete": true,
            "contexts": [captured_row("A!")]
        });
        let Leaf::Measured(catalog) = admit_runtime_text(Some(&value)).unwrap() else {
            panic!("expected measured catalog")
        };
        let RuntimeTextLayout::CapturedRuntime(layout) = &catalog.contexts["rt:1"].layout else {
            panic!("expected captured runtime layout")
        };
        assert_eq!(layout.clip, Some([-4, 2, 80, 24]));
        assert_eq!(layout.glyphs[0].x, -2);
        assert_eq!(layout.inline_icons[0].advance, 18);

        let mut no_clip = captured_row("A!");
        no_clip["layout"]["clip"] = Value::Null;
        let value = json!({
            "schema": RUNTIME_TEXT_SCHEMA,
            "state": "measured",
            "contexts_complete": true,
            "contexts": [no_clip]
        });
        let Leaf::Measured(catalog) = admit_runtime_text(Some(&value)).unwrap() else {
            panic!("expected measured catalog")
        };
        let RuntimeTextLayout::CapturedRuntime(layout) = &catalog.contexts["rt:1"].layout else {
            unreachable!()
        };
        assert_eq!(layout.clip, None);
    }

    #[test]
    fn rejects_negative_zero_and_overflowing_captured_clip_geometry() {
        for clip in [
            json!([0, 0, -1, 10]),
            json!([0, 0, 10, 0]),
            json!([i32::MAX, 0, 1, 10]),
            json!([0, i32::MAX, 10, 1]),
        ] {
            let mut invalid = captured_row("A!");
            invalid["layout"]["clip"] = clip;
            let value = json!({
                "schema": RUNTIME_TEXT_SCHEMA,
                "state": "measured",
                "contexts_complete": true,
                "contexts": [invalid]
            });
            assert!(admit_runtime_text(Some(&value)).is_err());
        }
    }

    #[test]
    fn rejects_invalid_captured_runs_and_layout_kind_mismatch() {
        let mut blank_font = captured_row("A!");
        blank_font["layout"]["glyphs"][0]["font"] = json!(" ");
        let mut blank_sprite = captured_row("A!");
        blank_sprite["layout"]["inline_icons"][0]["sprite"] = json!("");
        let mut zero_advance = captured_row("A!");
        zero_advance["layout"]["inline_icons"][0]["advance"] = json!(0);
        let mut x_overflow = captured_row("A!");
        x_overflow["layout"]["inline_icons"][0]["x"] = json!(i32::MAX);
        let mut y_overflow = captured_row("A!");
        y_overflow["layout"]["inline_icons"][0]["y"] = json!(i32::MAX);

        for (case, invalid) in [
            ("blank_font", blank_font),
            ("blank_sprite", blank_sprite),
            ("zero_advance", zero_advance),
            ("x_overflow", x_overflow),
            ("y_overflow", y_overflow),
        ] {
            let value = json!({
                "schema": RUNTIME_TEXT_SCHEMA,
                "state": "measured",
                "contexts_complete": true,
                "contexts": [invalid]
            });
            assert!(
                admit_runtime_text(Some(&value)).is_err(),
                "invalid captured run was admitted: {case}"
            );
        }

        let mut authored_with_captured_fields = row("A!");
        authored_with_captured_fields["layout"] = json!({
            "mode": "authored_reproducible",
            "clip": [0, 0, 10, 10]
        });
        let mut captured_with_authored_fields = captured_row("A!");
        captured_with_authored_fields["layout"]["max_width"] = json!(120);
        for (case, invalid) in [
            (
                "authored_with_captured_fields",
                authored_with_captured_fields,
            ),
            (
                "captured_with_authored_fields",
                captured_with_authored_fields,
            ),
        ] {
            let value = json!({
                "schema": RUNTIME_TEXT_SCHEMA,
                "state": "measured",
                "contexts_complete": true,
                "contexts": [invalid]
            });
            assert!(
                admit_runtime_text(Some(&value)).is_err(),
                "layout-kind mismatch was admitted: {case}"
            );
        }
    }

    #[test]
    fn admits_exact_a25_source_identities_and_rejects_zero_caller() {
        let mut authored = row("52%");
        authored["source"] = json!({"kind": "authored_token", "identity": "STABILITY"});
        authored["producer"]["identity_source"] = json!("authored_widget_registration");
        authored["producer"]["setter_caller_rva"] = Value::Null;
        let value = json!({
            "schema": RUNTIME_TEXT_SCHEMA,
            "state": "measured",
            "contexts_complete": true,
            "contexts": [authored]
        });
        let Leaf::Measured(catalog) = admit_runtime_text(Some(&value)).unwrap() else {
            panic!("expected measured catalog")
        };
        assert_eq!(
            catalog.contexts["rt:1"].source,
            RuntimeTextSource::AuthoredToken("STABILITY".into())
        );
        assert_eq!(catalog.contexts["rt:1"].producer.setter_caller_rva, None);

        let mut caller = row("53%");
        caller["source"] = json!({"kind": "setter_caller_rva", "rva": 0x0188_A0DEu32});
        let value = json!({
            "schema": RUNTIME_TEXT_SCHEMA,
            "state": "measured",
            "contexts_complete": true,
            "contexts": [caller]
        });
        let Leaf::Measured(catalog) = admit_runtime_text(Some(&value)).unwrap() else {
            panic!("expected measured catalog")
        };
        assert_eq!(
            catalog.contexts["rt:1"].source,
            RuntimeTextSource::SetterCallerRva(0x0188_A0DE)
        );

        let mut zero = row("54%");
        zero["source"] = json!({"kind": "setter_caller_rva", "rva": 0});
        let invalid = json!({
            "schema": RUNTIME_TEXT_SCHEMA,
            "state": "measured",
            "contexts_complete": true,
            "contexts": [zero]
        });
        assert!(admit_runtime_text(Some(&invalid)).is_err());

        let mut missing = row("55%");
        missing["source"] = json!({"kind": "setter_caller_rva", "rva": 0x0188_A0DEu32});
        missing["producer"]["setter_caller_rva"] = Value::Null;
        let invalid = json!({
            "schema": RUNTIME_TEXT_SCHEMA,
            "state": "measured",
            "contexts_complete": true,
            "contexts": [missing]
        });
        assert!(admit_runtime_text(Some(&invalid)).is_err());
    }

    #[test]
    fn rejects_digest_identity_and_duplicate_semantic_rows() {
        let mut bad = row("52%");
        bad["final_text"]["sha256"] = json!("00".repeat(32));
        let value = json!({
            "schema": RUNTIME_TEXT_SCHEMA,
            "state": "measured",
            "contexts_complete": true,
            "contexts": [bad]
        });
        assert!(admit_runtime_text(Some(&value)).is_err());

        let one = row("52%");
        let mut two = row("53%");
        two["context_id"] = json!("rt:2");
        let duplicate = json!({
            "schema": RUNTIME_TEXT_SCHEMA,
            "state": "measured",
            "contexts_complete": true,
            "contexts": [one, two]
        });
        assert!(admit_runtime_text(Some(&duplicate)).is_err());
    }
}
