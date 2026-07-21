//! A25/XUI06 exact a729 runtime-localised-text producer and d76 carrier.
//!
//! Two authenticated engine hooks form the identity join:
//! `CTernary<CTextBox*>::insert` binds a runtime textbox to its owning authored
//! fixed window, and the final text setter supplies the evaluated string.  The
//! post-setter `CInstantTextObject` supplies Clausewitz's exact wrap/truncation-
//! selected string. Static localisation is intentionally not duplicated.

use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub mod owner_glyph_capture;

pub const DOMAIN_ID: u16 = 76;
pub const DOMAIN_NAME: &str = "runtime_text_inputs_live";
pub const SCHEMA_VERSION: u32 = 3;
pub const AUTHORITY_EXACT_A729_RUNTIME_TEXT_HOOKS: u32 = 1;
pub const MAGIC: u32 = u32::from_le_bytes(*b"RTI1");
pub const HEADER_LEN: usize = 144;
pub const MAX_ROWS: usize = 262_144;
pub const MAX_STRING_BYTES: usize = 65_536;
pub const MAX_TEXT_BYTES: usize = 1 << 20;

pub const TEXT_SETTER_RVA: u32 = 0x022B_59E0;
pub const TEXTBOX_REGISTRATION_RVA: u32 = 0x0224_5570;
pub const TEXTBOX_DESTRUCTOR_RVA: u32 = 0x0228_EC70;
/// Generic GUI sprite/image setter shared by A25 observers.
pub const SPRITE_SETTER_RVA: u32 = 0x0229_A780;
pub const FORMATTER_RVA: u32 = 0x0223_2AB0;

/// SHA-256 of the Ghidra function-evidence manifest containing the final setter.
pub const SETTER_MANIFEST_SHA256: [u8; 32] =
    hex32(b"1a8870c102be3241d6ce48294dc53dacdb6696aec8f035a054e03ca979a2ef5d");
/// SHA-256 of the manifest proving the fixed-window/textbox registration join.
pub const REGISTRATION_MANIFEST_SHA256: [u8; 32] =
    hex32(b"6d83eea3192175ab51211198b195dd6f9c7546e8097b408863d7d700dd578123");
/// SHA-256 of the manifest proving the post-setter `CInstantTextObject+0x188`
pub const POST_LAYOUT_MANIFEST_SHA256: [u8; 32] =
    hex32(b"28eab18b7243ed6b611e28ad8b8ce5710837c36642b26f8ee04a697476507a82");

const fn hex32(input: &[u8; 64]) -> [u8; 32] {
    const fn nibble(byte: u8) -> u8 {
        match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => 0xff,
        }
    }
    let mut out = [0u8; 32];
    let mut index = 0;
    while index < 32 {
        out[index] = (nibble(input[index * 2]) << 4) | nibble(input[index * 2 + 1]);
        index += 1;
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RuntimeTextSourceIdentity {
    /// Exact raw `text = ...` token retained at `CTextBoxType+0x138`.
    AuthoredToken(String),
    /// No authored token was present; the exact setter caller is the source.
    SetterCallerRva(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeTextLayout {
    /// The engine-selected post-layout bytes can be laid out by the admitted
    /// authored rectangle/font law. Inline controls are forbidden here.
    AuthoredReproducible,
    /// Existing RenderDoc/capture evidence supplied the irreducible final runs.
    CapturedRuntime {
        clip: Option<[i32; 4]>,
        glyphs: Vec<GlyphRun>,
        inline_icons: Vec<InlineIconRun>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlyphRun {
    /// Exact position in the mixed ordinary-glyph/inline-icon decision stream.
    pub run_ordinal: u32,
    pub font: String,
    pub glyph_id: u32,
    pub x: i32,
    pub y: i32,
    pub rgba: [u8; 4],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineIconRun {
    /// Exact position in the mixed ordinary-glyph/inline-icon decision stream.
    pub run_ordinal: u32,
    pub sprite: String,
    pub frame: u32,
    pub x: i32,
    pub y: i32,
    pub baseline: i32,
    pub advance: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeTextInput {
    pub context_id: String,
    pub pov_country_or_global: String,
    pub view_id: String,
    pub element_id: String,
    pub source: RuntimeTextSourceIdentity,
    pub final_text_bytes: Vec<u8>,
    pub setter_caller_rva: u32,
    pub formatter_observed: bool,
    pub layout: RuntimeTextLayout,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeTextFrame {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub frame_hour: u64,
    pub completeness_record_identity: String,
    pub expected_context_count: u32,
    pub rows: Vec<RuntimeTextInput>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CarrierError {
    UnauthenticatedSource,
    Source(String),
    Invalid(String),
    Truncated,
    TrailingBytes,
}

pub trait ExactRuntimeTextSource {
    fn is_exact_a729_runtime_text_source(&self) -> bool;
    fn completeness_record_identity(&self, frame_hour: u64) -> Result<String, String>;
    fn expected_context_count(&self, frame_hour: u64) -> Result<u32, String>;
    fn collect_frame(
        &self,
        frame_hour: u64,
        pov_country_or_global: &str,
    ) -> Result<Vec<RuntimeTextInput>, String>;
}

pub fn collect_and_encode<S: ExactRuntimeTextSource>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
    pov_country_or_global: &str,
) -> Result<Vec<u8>, CarrierError> {
    if !source.is_exact_a729_runtime_text_source() {
        return Err(CarrierError::UnauthenticatedSource);
    }
    encode_payload(&RuntimeTextFrame {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        completeness_record_identity: source
            .completeness_record_identity(frame_hour)
            .map_err(CarrierError::Source)?,
        expected_context_count: source
            .expected_context_count(frame_hour)
            .map_err(CarrierError::Source)?,
        rows: source
            .collect_frame(frame_hour, pov_country_or_global)
            .map_err(CarrierError::Source)?,
    })
}

/// Build the only structurally valid d76 payload for a scheduled producer
/// failure. The compact record carrying it must also set a failure status bit;
/// recorder admission will project null rather than measured-empty.
pub fn encode_source_unavailable(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, CarrierError> {
    if status_flags == 0 {
        return invalid("source-unavailable payload requires nonzero status flags");
    }
    encode_payload(&RuntimeTextFrame {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        completeness_record_identity: format!("a25:{frame_hour}:source_unavailable"),
        expected_context_count: 0,
        rows: Vec::new(),
    })
}

pub fn validate_frame(frame: &RuntimeTextFrame) -> Result<(), CarrierError> {
    nonempty(&frame.completeness_record_identity, "completeness identity")?;
    if frame.rows.len() > MAX_ROWS || frame.expected_context_count as usize != frame.rows.len() {
        return invalid("runtime-text completeness/count mismatch");
    }
    let mut contexts = BTreeSet::new();
    let mut semantics = BTreeSet::new();
    for row in &frame.rows {
        for (value, label) in [
            (&row.context_id, "context id"),
            (&row.pov_country_or_global, "pov/global"),
            (&row.view_id, "view id"),
            (&row.element_id, "element id"),
        ] {
            nonempty(value, label)?;
        }
        if !contexts.insert(row.context_id.as_str()) {
            return invalid("duplicate runtime-text context id");
        }
        if !semantics.insert((
            row.pov_country_or_global.as_str(),
            row.view_id.as_str(),
            row.element_id.as_str(),
            &row.source,
        )) {
            return invalid("duplicate runtime-text semantic identity");
        }
        if row.final_text_bytes.len() > MAX_TEXT_BYTES
            || std::str::from_utf8(&row.final_text_bytes).is_err()
        {
            return invalid("final text is not bounded UTF-8");
        }
        match &row.source {
            RuntimeTextSourceIdentity::AuthoredToken(token) => {
                nonempty(token, "authored source token")?;
            }
            RuntimeTextSourceIdentity::SetterCallerRva(rva) if *rva == 0 => {
                return invalid("source caller RVA is zero")
            }
            RuntimeTextSourceIdentity::SetterCallerRva(rva) if *rva != row.setter_caller_rva => {
                return invalid("source caller RVA disagrees with producer caller RVA")
            }
            RuntimeTextSourceIdentity::SetterCallerRva(_) => {}
        }
        match &row.layout {
            RuntimeTextLayout::AuthoredReproducible => {
                if requires_captured_layout(&row.final_text_bytes) {
                    return invalid(
                        "engine post-layout text contains inline/control data requiring captured layout",
                    );
                }
            }
            RuntimeTextLayout::CapturedRuntime {
                clip,
                glyphs,
                inline_icons,
            } => {
                if glyphs.len() > MAX_ROWS || inline_icons.len() > MAX_ROWS {
                    return invalid("captured layout run count exceeds bound");
                }
                validate_captured_clip(*clip)?;
                if !row.final_text_bytes.is_empty() && glyphs.is_empty() && inline_icons.is_empty()
                {
                    return invalid("captured layout has no runs for non-empty text");
                }
                for glyph in glyphs {
                    nonempty(&glyph.font, "glyph font")?;
                }
                for icon in inline_icons {
                    nonempty(&icon.sprite, "inline icon sprite")?;
                    if icon.advance <= 0
                        || icon.x.checked_add(icon.advance).is_none()
                        || icon.y.checked_add(icon.baseline).is_none()
                    {
                        return invalid("inline icon coordinate/extent is invalid");
                    }
                }
                validate_mixed_run_ordinals(glyphs, inline_icons)?;
            }
        }
    }
    Ok(())
}

fn validate_captured_clip(clip: Option<[i32; 4]>) -> Result<(), CarrierError> {
    if let Some([x, y, width, height]) = clip {
        if width <= 0
            || height <= 0
            || x.checked_add(width).is_none()
            || y.checked_add(height).is_none()
        {
            return invalid("captured layout clip coordinate/extent is invalid");
        }
    }
    Ok(())
}

pub fn encode_payload(frame: &RuntimeTextFrame) -> Result<Vec<u8>, CarrierError> {
    validate_frame(frame)?;
    let mut out = Vec::new();
    u32v(&mut out, MAGIC);
    u32v(&mut out, SCHEMA_VERSION);
    u32v(&mut out, AUTHORITY_EXACT_A729_RUNTIME_TEXT_HOOKS);
    u32v(&mut out, frame.status_flags);
    u64v(&mut out, frame.elapsed_wall_ms);
    u64v(&mut out, frame.frame_hour);
    out.extend_from_slice(&SETTER_MANIFEST_SHA256);
    out.extend_from_slice(&REGISTRATION_MANIFEST_SHA256);
    out.extend_from_slice(&POST_LAYOUT_MANIFEST_SHA256);
    u32v(&mut out, frame.expected_context_count);
    u32v(&mut out, frame.rows.len() as u32);
    u32v(&mut out, HEADER_LEN as u32);
    string(&mut out, &frame.completeness_record_identity)?;
    for row in &frame.rows {
        string(&mut out, &row.context_id)?;
        string(&mut out, &row.pov_country_or_global)?;
        string(&mut out, &row.view_id)?;
        string(&mut out, &row.element_id)?;
        match &row.source {
            RuntimeTextSourceIdentity::AuthoredToken(token) => {
                u32v(&mut out, 0);
                string(&mut out, token)?;
            }
            RuntimeTextSourceIdentity::SetterCallerRva(rva) => {
                u32v(&mut out, 1);
                u32v(&mut out, *rva);
            }
        }
        bytes(&mut out, &row.final_text_bytes, MAX_TEXT_BYTES)?;
        out.extend_from_slice(&Sha256::digest(&row.final_text_bytes));
        u32v(&mut out, row.setter_caller_rva);
        u32v(&mut out, u32::from(row.formatter_observed));
        encode_layout(&mut out, &row.layout)?;
    }
    Ok(out)
}

pub fn decode_payload(payload: &[u8]) -> Result<RuntimeTextFrame, CarrierError> {
    let mut r = Reader::new(payload);
    if r.u32()? != MAGIC || r.u32()? != SCHEMA_VERSION {
        return invalid("magic/schema mismatch");
    }
    if r.u32()? != AUTHORITY_EXACT_A729_RUNTIME_TEXT_HOOKS {
        return Err(CarrierError::UnauthenticatedSource);
    }
    let status_flags = r.u32()?;
    let elapsed_wall_ms = r.u64()?;
    let frame_hour = r.u64()?;
    if r.array32()? != SETTER_MANIFEST_SHA256
        || r.array32()? != REGISTRATION_MANIFEST_SHA256
        || r.array32()? != POST_LAYOUT_MANIFEST_SHA256
    {
        return invalid("Ghidra producer manifest hash mismatch");
    }
    let expected_context_count = r.u32()?;
    let count = r.u32()? as usize;
    if r.u32()? as usize != HEADER_LEN || count > MAX_ROWS {
        return invalid("header/count invalid");
    }
    let completeness_record_identity = r.string()?;
    let mut rows = Vec::with_capacity(count);
    for _ in 0..count {
        let context_id = r.string()?;
        let pov_country_or_global = r.string()?;
        let view_id = r.string()?;
        let element_id = r.string()?;
        let source = match r.u32()? {
            0 => RuntimeTextSourceIdentity::AuthoredToken(r.string()?),
            1 => RuntimeTextSourceIdentity::SetterCallerRva(r.u32()?),
            _ => return invalid("source identity code invalid"),
        };
        let final_text_bytes = r.bytes(MAX_TEXT_BYTES)?;
        let expected_digest: [u8; 32] = Sha256::digest(&final_text_bytes).into();
        if r.array32()? != expected_digest {
            return invalid("final text SHA-256 mismatch");
        }
        let setter_caller_rva = r.u32()?;
        let formatter_observed = r.boolean()?;
        let layout = decode_layout(&mut r)?;
        rows.push(RuntimeTextInput {
            context_id,
            pov_country_or_global,
            view_id,
            element_id,
            source,
            final_text_bytes,
            setter_caller_rva,
            formatter_observed,
            layout,
        });
    }
    if !r.done() {
        return Err(CarrierError::TrailingBytes);
    }
    let frame = RuntimeTextFrame {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        completeness_record_identity,
        expected_context_count,
        rows,
    };
    validate_frame(&frame)?;
    Ok(frame)
}

/// Project one already-decoded d76 frame into the exact production replay
/// `runtime_text` envelope. This keeps the shared recorder fold thin and makes
/// every source/layout mapping independently testable before scheduling d76.
pub fn project_replay_runtime_text(
    frame: &RuntimeTextFrame,
) -> Result<serde_json::Value, CarrierError> {
    validate_frame(frame)?;
    let frame_hour = i64::try_from(frame.frame_hour)
        .map_err(|_| CarrierError::Invalid("frame hour exceeds replay i64 range".into()))?;
    let contexts = frame
        .rows
        .iter()
        .map(|row| {
            let source = match &row.source {
                RuntimeTextSourceIdentity::AuthoredToken(identity) => serde_json::json!({
                    "kind": "authored_token",
                    "identity": identity,
                }),
                RuntimeTextSourceIdentity::SetterCallerRva(rva) => serde_json::json!({
                    "kind": "setter_caller_rva",
                    "rva": rva,
                }),
            };
            let identity_source = match &row.source {
                RuntimeTextSourceIdentity::AuthoredToken(_) => "authored_widget_registration",
                RuntimeTextSourceIdentity::SetterCallerRva(_) => "proven_setter_caller",
            };
            let layout = match &row.layout {
                RuntimeTextLayout::AuthoredReproducible => serde_json::json!({
                    "mode": "authored_reproducible",
                }),
                RuntimeTextLayout::CapturedRuntime {
                    clip,
                    glyphs,
                    inline_icons,
                } => serde_json::json!({
                    "mode": "captured_runtime",
                    "clip": clip,
                    "glyphs": glyphs.iter().map(|glyph| serde_json::json!({
                        "run_ordinal": glyph.run_ordinal,
                        "font": glyph.font,
                        "glyph_id": glyph.glyph_id,
                        "x": glyph.x,
                        "y": glyph.y,
                        "rgba": glyph.rgba,
                    })).collect::<Vec<_>>(),
                    "inline_icons": inline_icons.iter().map(|icon| serde_json::json!({
                        "run_ordinal": icon.run_ordinal,
                        "sprite": icon.sprite,
                        "frame": icon.frame,
                        "x": icon.x,
                        "y": icon.y,
                        "baseline": icon.baseline,
                        "advance": icon.advance,
                    })).collect::<Vec<_>>(),
                }),
            };
            let bytes_hex = row
                .final_text_bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            let utf8 = std::str::from_utf8(&row.final_text_bytes)
                .map_err(|_| CarrierError::Invalid("final text is not UTF-8".into()))?;
            let final_text_sha256 = format!("{:x}", Sha256::digest(&row.final_text_bytes));
            Ok(serde_json::json!({
                "context_id": row.context_id,
                "frame_hour": frame_hour,
                "pov_country_or_global": row.pov_country_or_global,
                "view_id": row.view_id,
                "element_id": row.element_id,
                "source": source,
                "final_text": {
                    "bytes_hex": bytes_hex,
                    "utf8": utf8,
                    "sha256": final_text_sha256,
                },
                "variables": {},
                "data_functions": {},
                "dynamic_colors": {},
                "layout": layout,
                "producer": {
                    "setter_entry_rva": TEXT_SETTER_RVA,
                    "setter_caller_rva": (row.setter_caller_rva != 0).then_some(row.setter_caller_rva),
                    "formatter_entry_rva": row.formatter_observed.then_some(FORMATTER_RVA),
                    "identity_source": identity_source,
                },
            }))
        })
        .collect::<Result<Vec<_>, CarrierError>>()?;
    Ok(serde_json::json!({
        "schema": "hoi4-runtime-text-v1",
        "state": "measured",
        "contexts_complete": true,
        "contexts": contexts,
    }))
}

fn encode_layout(out: &mut Vec<u8>, layout: &RuntimeTextLayout) -> Result<(), CarrierError> {
    match layout {
        RuntimeTextLayout::AuthoredReproducible => u32v(out, 0),
        RuntimeTextLayout::CapturedRuntime {
            clip,
            glyphs,
            inline_icons,
        } => {
            u32v(out, 1);
            match clip {
                None => u32v(out, 0),
                Some(clip) => {
                    u32v(out, 1);
                    for value in clip {
                        i32v(out, *value);
                    }
                }
            }
            u32v(out, glyphs.len() as u32);
            u32v(out, inline_icons.len() as u32);
            for glyph in glyphs {
                u32v(out, glyph.run_ordinal);
                string(out, &glyph.font)?;
                u32v(out, glyph.glyph_id);
                i32v(out, glyph.x);
                i32v(out, glyph.y);
                out.extend_from_slice(&glyph.rgba);
            }
            for icon in inline_icons {
                u32v(out, icon.run_ordinal);
                string(out, &icon.sprite)?;
                u32v(out, icon.frame);
                i32v(out, icon.x);
                i32v(out, icon.y);
                i32v(out, icon.baseline);
                i32v(out, icon.advance);
            }
        }
    }
    Ok(())
}

fn decode_layout(r: &mut Reader<'_>) -> Result<RuntimeTextLayout, CarrierError> {
    match r.u32()? {
        0 => Ok(RuntimeTextLayout::AuthoredReproducible),
        1 => {
            let clip = match r.u32()? {
                0 => None,
                1 => Some([r.i32()?, r.i32()?, r.i32()?, r.i32()?]),
                _ => return invalid("clip presence code invalid"),
            };
            let glyph_count = r.u32()? as usize;
            let icon_count = r.u32()? as usize;
            if glyph_count > MAX_ROWS || icon_count > MAX_ROWS {
                return invalid("layout run count exceeds bound");
            }
            let mut glyphs = Vec::with_capacity(glyph_count);
            for _ in 0..glyph_count {
                glyphs.push(GlyphRun {
                    run_ordinal: r.u32()?,
                    font: r.string()?,
                    glyph_id: r.u32()?,
                    x: r.i32()?,
                    y: r.i32()?,
                    rgba: r.array4()?,
                });
            }
            let mut inline_icons = Vec::with_capacity(icon_count);
            for _ in 0..icon_count {
                inline_icons.push(InlineIconRun {
                    run_ordinal: r.u32()?,
                    sprite: r.string()?,
                    frame: r.u32()?,
                    x: r.i32()?,
                    y: r.i32()?,
                    baseline: r.i32()?,
                    advance: r.i32()?,
                });
            }
            Ok(RuntimeTextLayout::CapturedRuntime {
                clip,
                glyphs,
                inline_icons,
            })
        }
        _ => invalid("layout mode invalid"),
    }
}

fn validate_mixed_run_ordinals(
    glyphs: &[GlyphRun],
    inline_icons: &[InlineIconRun],
) -> Result<(), CarrierError> {
    let mut ordinals = glyphs
        .iter()
        .map(|run| run.run_ordinal)
        .chain(inline_icons.iter().map(|run| run.run_ordinal))
        .collect::<Vec<_>>();
    ordinals.sort_unstable();
    for (expected, actual) in ordinals.into_iter().enumerate() {
        if actual as usize != expected {
            return invalid("captured mixed glyph/icon run order is duplicate or incomplete");
        }
    }
    Ok(())
}

fn nonempty(value: &str, label: &str) -> Result<(), CarrierError> {
    if value.is_empty() || value.len() > MAX_STRING_BYTES {
        invalid(format!("{label} is empty or exceeds bound"))
    } else {
        Ok(())
    }
}

/// Engine control U+0011 is the authenticated colour lead and explicit line
/// breaks are reproduced from the post-layout string. Every other C0 control,
/// plus authored pound-sign icon syntax, needs owner-scoped captured runs.
fn requires_captured_layout(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return true;
    };
    text.chars()
        .any(|ch| ch == '\u{00a3}' || (ch.is_control() && !matches!(ch, '\n' | '\r' | '\u{0011}')))
}
fn invalid<T>(message: impl Into<String>) -> Result<T, CarrierError> {
    Err(CarrierError::Invalid(message.into()))
}
fn u32v(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn i32v(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn u64v(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn string(out: &mut Vec<u8>, value: &str) -> Result<(), CarrierError> {
    bytes(out, value.as_bytes(), MAX_STRING_BYTES)
}
fn bytes(out: &mut Vec<u8>, value: &[u8], max: usize) -> Result<(), CarrierError> {
    if value.len() > max {
        return invalid("byte string exceeds bound");
    }
    u32v(out, value.len() as u32);
    out.extend_from_slice(value);
    Ok(())
}

struct Reader<'a> {
    data: &'a [u8],
    cursor: usize,
}
impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, cursor: 0 }
    }
    fn take(&mut self, len: usize) -> Result<&'a [u8], CarrierError> {
        let end = self
            .cursor
            .checked_add(len)
            .ok_or(CarrierError::Truncated)?;
        let out = self
            .data
            .get(self.cursor..end)
            .ok_or(CarrierError::Truncated)?;
        self.cursor = end;
        Ok(out)
    }
    fn u32(&mut self) -> Result<u32, CarrierError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, CarrierError> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, CarrierError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn boolean(&mut self) -> Result<bool, CarrierError> {
        match self.u32()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => invalid("boolean invalid"),
        }
    }
    fn array4(&mut self) -> Result<[u8; 4], CarrierError> {
        Ok(self.take(4)?.try_into().unwrap())
    }
    fn array32(&mut self) -> Result<[u8; 32], CarrierError> {
        Ok(self.take(32)?.try_into().unwrap())
    }
    fn bytes(&mut self, max: usize) -> Result<Vec<u8>, CarrierError> {
        let n = self.u32()? as usize;
        if n > max {
            return invalid("byte length exceeds bound");
        };
        Ok(self.take(n)?.to_vec())
    }
    fn string(&mut self) -> Result<String, CarrierError> {
        String::from_utf8(self.bytes(MAX_STRING_BYTES)?)
            .map_err(|_| CarrierError::Invalid("string is not UTF-8".into()))
    }
    fn done(&self) -> bool {
        self.cursor == self.data.len()
    }
}

#[cfg(windows)]
pub mod a729_hook;
#[cfg(windows)]
pub mod owner_glyph_probe;

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> RuntimeTextInput {
        RuntimeTextInput {
            context_id: "politics_view::support_value::0".into(),
            pov_country_or_global: "FRA".into(),
            view_id: "politics_view".into(),
            element_id: "support_value".into(),
            source: RuntimeTextSourceIdentity::AuthoredToken("PARTY_POPULARITY|%0".into()),
            final_text_bytes: b"\x11G52%\x11!".to_vec(),
            setter_caller_rva: 0x0188_A0DE,
            formatter_observed: true,
            layout: RuntimeTextLayout::AuthoredReproducible,
        }
    }

    #[test]
    fn exact_carrier_roundtrips_and_hash_gates() {
        let frame = RuntimeTextFrame {
            elapsed_wall_ms: 7,
            status_flags: 0,
            frame_hour: 42,
            completeness_record_identity: "a25:42:complete".into(),
            expected_context_count: 1,
            rows: vec![row()],
        };
        let encoded = encode_payload(&frame).unwrap();
        assert_eq!(decode_payload(&encoded).unwrap(), frame);
        let mut bad = encoded;
        bad[40] ^= 1;
        assert!(decode_payload(&bad).is_err());

        let mut bad_layout_authority = encode_payload(&frame).unwrap();
        bad_layout_authority[96] ^= 1;
        assert!(decode_payload(&bad_layout_authority).is_err());
    }

    #[test]
    fn completeness_duplicates_and_bad_utf8_fail_closed() {
        let mut frame = RuntimeTextFrame {
            elapsed_wall_ms: 0,
            status_flags: 0,
            frame_hour: 1,
            completeness_record_identity: "complete".into(),
            expected_context_count: 2,
            rows: vec![row(), row()],
        };
        assert!(validate_frame(&frame).is_err());
        frame.expected_context_count = 1;
        frame.rows.truncate(1);
        frame.rows[0].final_text_bytes = vec![0xff];
        assert!(validate_frame(&frame).is_err());
        frame.rows[0].final_text_bytes = Vec::new();
        frame.rows[0].source = RuntimeTextSourceIdentity::AuthoredToken(String::new());
        assert!(validate_frame(&frame).is_err());
        frame.rows[0].source = RuntimeTextSourceIdentity::SetterCallerRva(1);
        frame.rows[0].setter_caller_rva = 2;
        assert!(validate_frame(&frame).is_err());
    }

    #[test]
    fn authored_layout_rejects_engine_inline_controls_and_pound_icons() {
        let mut frame = RuntimeTextFrame {
            elapsed_wall_ms: 0,
            status_flags: 0,
            frame_hour: 1,
            completeness_record_identity: "complete".into(),
            expected_context_count: 1,
            rows: vec![row()],
        };
        frame.rows[0].final_text_bytes = b"cost \x13abc".to_vec();
        assert!(validate_frame(&frame).is_err());

        frame.rows[0].final_text_bytes = "cost £political_power".as_bytes().to_vec();
        assert!(validate_frame(&frame).is_err());

        frame.rows[0].layout = RuntimeTextLayout::CapturedRuntime {
            clip: None,
            glyphs: Vec::new(),
            inline_icons: vec![InlineIconRun {
                run_ordinal: 0,
                sprite: "GFX_pol_power".into(),
                frame: 0,
                x: 0,
                y: 0,
                baseline: 12,
                advance: 12,
            }],
        };
        validate_frame(&frame).unwrap();

        if let RuntimeTextLayout::CapturedRuntime { inline_icons, .. } =
            &mut frame.rows[0].layout
        {
            inline_icons[0].run_ordinal = 1;
        }
        assert!(validate_frame(&frame).is_err());
        if let RuntimeTextLayout::CapturedRuntime {
            clip, inline_icons, ..
        } = &mut frame.rows[0].layout
        {
            inline_icons[0].run_ordinal = 0;
            *clip = Some([0, 0, 0, 12]);
        }
        assert!(validate_frame(&frame).is_err());
        if let RuntimeTextLayout::CapturedRuntime {
            clip, inline_icons, ..
        } = &mut frame.rows[0].layout
        {
            *clip = None;
            inline_icons[0].x = i32::MAX;
        }
        assert!(validate_frame(&frame).is_err());
    }

    #[test]
    fn exact_replay_projection_preserves_source_bytes_and_layout() {
        let mut input = row();
        input.layout = RuntimeTextLayout::CapturedRuntime {
            clip: Some([1, 2, 30, 40]),
            glyphs: vec![GlyphRun {
                run_ordinal: 0,
                font: "hoi_16mbs".into(),
                glyph_id: 52,
                x: 3,
                y: 4,
                rgba: [5, 6, 7, 8],
            }],
            inline_icons: vec![InlineIconRun {
                run_ordinal: 1,
                sprite: "GFX_pol_power".into(),
                frame: 1,
                x: 9,
                y: 10,
                baseline: 11,
                advance: 12,
            }],
        };
        let frame = RuntimeTextFrame {
            elapsed_wall_ms: 7,
            status_flags: 0,
            frame_hour: 42,
            completeness_record_identity: "a25:42:complete".into(),
            expected_context_count: 1,
            rows: vec![input],
        };
        let projected = project_replay_runtime_text(&frame).unwrap();
        assert_eq!(projected["schema"], "hoi4-runtime-text-v1");
        assert_eq!(projected["contexts_complete"], true);
        assert_eq!(projected["contexts"][0]["source"]["kind"], "authored_token");
        assert_eq!(
            projected["contexts"][0]["final_text"]["bytes_hex"],
            "11473532251121"
        );
        assert_eq!(
            projected["contexts"][0]["layout"]["glyphs"][0]["glyph_id"],
            52
        );
        assert_eq!(
            projected["contexts"][0]["layout"]["glyphs"][0]["run_ordinal"],
            0
        );
        assert_eq!(
            projected["contexts"][0]["layout"]["inline_icons"][0]["advance"],
            12
        );
        assert_eq!(
            projected["contexts"][0]["layout"]["inline_icons"][0]["run_ordinal"],
            1
        );

        let mut caller_frame = frame;
        caller_frame.rows[0].source =
            RuntimeTextSourceIdentity::SetterCallerRva(caller_frame.rows[0].setter_caller_rva);
        let caller = project_replay_runtime_text(&caller_frame).unwrap();
        assert_eq!(caller["contexts"][0]["source"]["kind"], "setter_caller_rva");
        assert_eq!(
            caller["contexts"][0]["producer"]["identity_source"],
            "proven_setter_caller"
        );
    }

    #[test]
    fn authored_baseline_can_lawfully_omit_unobserved_historical_caller() {
        let mut input = row();
        input.setter_caller_rva = 0;
        let frame = RuntimeTextFrame {
            elapsed_wall_ms: 7,
            status_flags: 0,
            frame_hour: 42,
            completeness_record_identity: "a25:42:complete".into(),
            expected_context_count: 1,
            rows: vec![input],
        };
        validate_frame(&frame).unwrap();
        let projected = project_replay_runtime_text(&frame).unwrap();
        assert!(projected["contexts"][0]["producer"]["setter_caller_rva"].is_null());

        let mut invalid = frame;
        invalid.rows[0].source = RuntimeTextSourceIdentity::SetterCallerRva(1);
        assert!(validate_frame(&invalid).is_err());
    }

    struct Source(bool);
    impl ExactRuntimeTextSource for Source {
        fn is_exact_a729_runtime_text_source(&self) -> bool {
            self.0
        }
        fn completeness_record_identity(&self, hour: u64) -> Result<String, String> {
            Ok(format!("a25:{hour}:complete"))
        }
        fn expected_context_count(&self, _: u64) -> Result<u32, String> {
            Ok(1)
        }
        fn collect_frame(&self, _: u64, pov: &str) -> Result<Vec<RuntimeTextInput>, String> {
            let mut value = row();
            value.pov_country_or_global = pov.into();
            Ok(vec![value])
        }
    }

    #[test]
    fn source_must_authenticate() {
        assert_eq!(
            collect_and_encode(&Source(false), 1, 0, 0, "FRA"),
            Err(CarrierError::UnauthenticatedSource)
        );
        assert!(collect_and_encode(&Source(true), 1, 0, 0, "FRA").is_ok());
    }

    #[test]
    fn scheduled_source_failure_has_one_valid_non_measured_empty_carrier() {
        assert!(encode_source_unavailable(42, 7, 0).is_err());
        let encoded = encode_source_unavailable(42, 7, 1).unwrap();
        let decoded = decode_payload(&encoded).unwrap();
        assert_eq!(decoded.status_flags, 1);
        assert_eq!(decoded.expected_context_count, 0);
        assert!(decoded.rows.is_empty());
        assert_eq!(
            decoded.completeness_record_identity,
            "a25:42:source_unavailable"
        );
    }
}
