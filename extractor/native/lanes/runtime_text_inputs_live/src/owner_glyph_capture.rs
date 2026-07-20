//! Fail-closed admission for the A25 owner-scoped glyph capture.
//!
//! The live detour is intentionally a separate concern from this contract.
//! Raw process pointers may be used inside the detour while an invocation is
//! active, but they cannot be represented here.  This module is therefore the
//! narrow boundary between a live observation and the production RTI1 row.

use crate::{GlyphRun, InlineIconRun, RuntimeTextInput, RuntimeTextLayout, RuntimeTextSourceIdentity};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const OWNER_RENDER_RVA: u32 = 0x022D_8350;
pub const BITMAP_FONT_DRAW_RVA: u32 = 0x0228_B2F0;
pub const GLYPH_PREPARATION_RVA: u32 = 0x0228_5960;
pub const GROUPED_SUBMIT_RVA: u32 = 0x0228_60E0;
pub const GLYPH_VERTEX_STRIDE: usize = 0x1c;
pub const GLYPH_VERTEX_COUNT: usize = 6;
pub const GLYPH_VERTEX_BYTES: usize = GLYPH_VERTEX_STRIDE * GLYPH_VERTEX_COUNT;
pub const A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const OWNER_GLYPH_RE_MANIFEST_SHA256: &str =
    "bf69324665fd7a9381a906a59985787eb9913c1449e42606b103ea576c4a5286";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CaptureHealth {
    pub gap_count: u64,
    pub drop_count: u64,
    pub reentrancy_count: u64,
    pub parse_failure_count: u64,
    pub hook_failure_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerGlyphCaptureAuthority {
    pub executable_sha256: String,
    pub re_manifest_sha256: String,
    pub owner_render_rva: u32,
    pub bitmap_font_draw_rva: u32,
    pub glyph_preparation_rva: u32,
    pub grouped_submit_rva: u32,
}

impl OwnerGlyphCaptureAuthority {
    pub fn exact_a729() -> Self {
        Self {
            executable_sha256: A729_EXECUTABLE_SHA256.into(),
            re_manifest_sha256: OWNER_GLYPH_RE_MANIFEST_SHA256.into(),
            owner_render_rva: OWNER_RENDER_RVA,
            bitmap_font_draw_rva: BITMAP_FONT_DRAW_RVA,
            glyph_preparation_rva: GLYPH_PREPARATION_RVA,
            grouped_submit_rva: GROUPED_SUBMIT_RVA,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontAssetReceipt {
    /// Stable identity written into the replay font binding.
    pub identity: String,
    pub descriptor_sha256: String,
    /// Exact ordered pages used by the effective runtime font.
    pub atlas_sha256: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineIconAssetReceipt {
    /// Stable identity written into the replay inline-icon binding.
    pub identity: String,
    pub definition_sha256: String,
    /// Exact ordered texture slots used by the sprite definition.
    pub texture_sha256: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GlyphDescriptorIdentity {
    UnicodeScalar(u32),
    /// Hash of the exact engine glyph descriptor bytes when the decision does
    /// not have a one-to-one Unicode scalar identity.
    DescriptorSha256(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerScopedGlyphObservation {
    pub run_ordinal: u32,
    pub font_identity: String,
    pub descriptor: GlyphDescriptorIdentity,
    pub glyph_id: u32,
    pub owner_relative_x: i32,
    pub owner_relative_y: i32,
    pub rgba: [u8; 4],
    /// Six exact 0x1c-byte vertices copied before the grouped submit discards
    /// owner scope. The replay does not serialize these bytes; admission keeps
    /// them as the mechanical descriptor-to-draw receipt.
    pub generated_vertices: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerScopedInlineIconObservation {
    pub run_ordinal: u32,
    pub token: String,
    pub sprite_identity: String,
    pub frame: u32,
    pub owner_relative_x: i32,
    pub owner_relative_y: i32,
    pub baseline: i32,
    pub advance: i32,
    /// Hash of the corresponding owner-scoped icon draw receipt.
    pub draw_receipt_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerScopedGlyphCapture {
    pub authority: OwnerGlyphCaptureAuthority,
    pub process_identity: String,
    pub hook_epoch: u64,
    pub frame_hour: u64,
    pub render_invocation_ordinal: u64,
    pub context_id: String,
    pub view_id: String,
    pub element_id: String,
    pub source: RuntimeTextSourceIdentity,
    pub selected_final_text_sha256: String,
    pub clip: Option<[i32; 4]>,
    pub glyph_decision_count: u32,
    pub inline_icon_decision_count: u32,
    pub glyphs: Vec<OwnerScopedGlyphObservation>,
    pub inline_icons: Vec<OwnerScopedInlineIconObservation>,
    pub fonts: Vec<FontAssetReceipt>,
    pub icon_assets: Vec<InlineIconAssetReceipt>,
    pub health: CaptureHealth,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedOwnerGlyphCapture {
    pub layout: RuntimeTextLayout,
    /// Exact assets that the selected-Present command must bind by authenticated
    /// content index. The RTI1 row stores only their stable identities.
    pub fonts: Vec<FontAssetReceipt>,
    pub icon_assets: Vec<InlineIconAssetReceipt>,
}

/// Pointer-free owner/text identity retained by the hook-thread automation
/// until the render selector chooses one Present. It contains no process
/// address and cannot be used to rediscover a row after its setter changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerGlyphRowIdentity {
    pub context_id: String,
    pub view_id: String,
    pub element_id: String,
    pub source: RuntimeTextSourceIdentity,
    pub selected_final_text_bytes: Vec<u8>,
    pub selected_final_text_sha256: String,
}

/// Stateful invocation-order gate. One instance owns one uninterrupted raw
/// hook stream; starting a new capture requires a fresh hook epoch and gate.
#[derive(Default)]
pub struct OwnerGlyphCaptureAdmission {
    next_ordinal_by_epoch: BTreeMap<u64, u64>,
}

impl OwnerGlyphCaptureAdmission {
    pub fn admit(
        &mut self,
        capture: OwnerScopedGlyphCapture,
        row: &RuntimeTextInput,
        frame_hour: u64,
    ) -> Result<AdmittedOwnerGlyphCapture, String> {
        self.admit_against_identity(
            capture,
            &OwnerGlyphRowIdentity {
                context_id: row.context_id.clone(),
                view_id: row.view_id.clone(),
                element_id: row.element_id.clone(),
                source: row.source.clone(),
                selected_final_text_bytes: row.final_text_bytes.clone(),
                selected_final_text_sha256: hex_sha256(&row.final_text_bytes),
            },
            frame_hour,
        )
    }

    pub fn admit_against_identity(
        &mut self,
        capture: OwnerScopedGlyphCapture,
        row: &OwnerGlyphRowIdentity,
        frame_hour: u64,
    ) -> Result<AdmittedOwnerGlyphCapture, String> {
        validate_authority(&capture.authority)?;
        if capture.process_identity.trim().is_empty() {
            return Err("owner-glyph capture has empty process identity".into());
        }
        if capture.hook_epoch == 0 {
            return Err("owner-glyph capture has zero hook epoch".into());
        }
        if capture.frame_hour != frame_hour {
            return Err("owner-glyph capture frame-hour identity mismatch".into());
        }
        if capture.context_id != row.context_id
            || capture.view_id != row.view_id
            || capture.element_id != row.element_id
            || capture.source != row.source
        {
            return Err("owner-glyph capture owner identity mismatch".into());
        }
        require_sha256(&row.selected_final_text_sha256, "selected final text")?;
        if hex_sha256(&row.selected_final_text_bytes) != row.selected_final_text_sha256
            || capture.selected_final_text_sha256 != row.selected_final_text_sha256
        {
            return Err("owner-glyph capture selected-text digest mismatch".into());
        }
        if capture.health != CaptureHealth::default() {
            return Err("owner-glyph capture contains loss or hook failures".into());
        }
        validate_clip(capture.clip)?;

        let next = self
            .next_ordinal_by_epoch
            .entry(capture.hook_epoch)
            .or_insert(0);
        if capture.render_invocation_ordinal != *next {
            return Err(format!(
                "owner-glyph render invocation gap: expected {}, got {}",
                *next, capture.render_invocation_ordinal
            ));
        }

        if capture.glyph_decision_count as usize != capture.glyphs.len()
            || capture.inline_icon_decision_count as usize != capture.inline_icons.len()
        {
            return Err("owner-glyph decision/emission count mismatch".into());
        }
        let selected_icon_tokens = selected_inline_icon_tokens(&row.selected_final_text_bytes)?;
        if selected_icon_tokens.len() != capture.inline_icons.len()
            || selected_icon_tokens
                .iter()
                .zip(&capture.inline_icons)
                .any(|(selected, observed)| selected != &observed.token)
        {
            return Err("inline-icon observations do not match selected control-0x13 tokens".into());
        }
        if !row.selected_final_text_bytes.is_empty()
            && capture.glyphs.is_empty()
            && capture.inline_icons.is_empty()
        {
            return Err("non-empty selected text has no captured glyph/icon decisions".into());
        }

        let font_assets = validate_font_assets(&capture.fonts)?;
        let icon_assets = validate_icon_assets(&capture.icon_assets)?;
        validate_run_ordinals(&capture.glyphs, &capture.inline_icons)?;

        let mut glyphs = Vec::with_capacity(capture.glyphs.len());
        for glyph in &capture.glyphs {
            validate_glyph_vertices(glyph.run_ordinal, &glyph.generated_vertices)?;
            match &glyph.descriptor {
                GlyphDescriptorIdentity::UnicodeScalar(value)
                    if char::from_u32(*value).is_none() =>
                {
                    return Err(format!(
                        "glyph run {} has invalid Unicode scalar",
                        glyph.run_ordinal
                    ));
                }
                GlyphDescriptorIdentity::DescriptorSha256(value) => {
                    require_sha256(value, "glyph descriptor")?;
                }
                GlyphDescriptorIdentity::UnicodeScalar(_) => {}
            }
            if !font_assets.contains(glyph.font_identity.as_str()) {
                return Err(format!(
                    "glyph run {} references unauthenticated font {}",
                    glyph.run_ordinal, glyph.font_identity
                ));
            }
            glyphs.push(GlyphRun {
                font: glyph.font_identity.clone(),
                glyph_id: glyph.glyph_id,
                x: glyph.owner_relative_x,
                y: glyph.owner_relative_y,
                rgba: glyph.rgba,
            });
        }

        let mut inline_icons = Vec::with_capacity(capture.inline_icons.len());
        for icon in &capture.inline_icons {
            if icon.token.is_empty() || icon.advance <= 0 {
                return Err(format!("inline-icon run {} is incomplete", icon.run_ordinal));
            }
            require_sha256(&icon.draw_receipt_sha256, "inline-icon draw receipt")?;
            if !icon_assets.contains(icon.sprite_identity.as_str()) {
                return Err(format!(
                    "inline-icon run {} references unauthenticated sprite {}",
                    icon.run_ordinal, icon.sprite_identity
                ));
            }
            icon.owner_relative_x
                .checked_add(icon.advance)
                .ok_or_else(|| "inline-icon x/advance overflow".to_string())?;
            icon.owner_relative_y
                .checked_add(icon.baseline)
                .ok_or_else(|| "inline-icon y/baseline overflow".to_string())?;
            inline_icons.push(InlineIconRun {
                sprite: icon.sprite_identity.clone(),
                frame: icon.frame,
                x: icon.owner_relative_x,
                y: icon.owner_relative_y,
                baseline: icon.baseline,
                advance: icon.advance,
            });
        }

        *next = next
            .checked_add(1)
            .ok_or_else(|| "owner-glyph invocation ordinal overflow".to_string())?;
        Ok(AdmittedOwnerGlyphCapture {
            layout: RuntimeTextLayout::CapturedRuntime {
                clip: capture.clip,
                glyphs,
                inline_icons,
            },
            fonts: capture.fonts,
            icon_assets: capture.icon_assets,
        })
    }
}

fn validate_authority(authority: &OwnerGlyphCaptureAuthority) -> Result<(), String> {
    if authority != &OwnerGlyphCaptureAuthority::exact_a729() {
        return Err("owner-glyph capture authority mismatch".into());
    }
    Ok(())
}

fn validate_clip(clip: Option<[i32; 4]>) -> Result<(), String> {
    if let Some([x, y, width, height]) = clip {
        if width <= 0 || height <= 0 {
            return Err("owner-glyph capture clip has non-positive extent".into());
        }
        x.checked_add(width)
            .ok_or_else(|| "owner-glyph capture clip x overflow".to_string())?;
        y.checked_add(height)
            .ok_or_else(|| "owner-glyph capture clip y overflow".to_string())?;
    }
    Ok(())
}

fn validate_font_assets(receipts: &[FontAssetReceipt]) -> Result<BTreeSet<&str>, String> {
    let mut identities = BTreeSet::new();
    for receipt in receipts {
        if receipt.identity.trim().is_empty() || !identities.insert(receipt.identity.as_str()) {
            return Err("font asset identities must be non-empty and unique".into());
        }
        require_sha256(&receipt.descriptor_sha256, "font descriptor")?;
        if receipt.atlas_sha256.is_empty() {
            return Err(format!("font {} has no authenticated atlas", receipt.identity));
        }
        for digest in &receipt.atlas_sha256 {
            require_sha256(digest, "font atlas")?;
        }
    }
    Ok(identities)
}

fn validate_icon_assets(receipts: &[InlineIconAssetReceipt]) -> Result<BTreeSet<&str>, String> {
    let mut identities = BTreeSet::new();
    for receipt in receipts {
        if receipt.identity.trim().is_empty() || !identities.insert(receipt.identity.as_str()) {
            return Err("inline-icon asset identities must be non-empty and unique".into());
        }
        require_sha256(&receipt.definition_sha256, "inline-icon definition")?;
        if receipt.texture_sha256.is_empty() {
            return Err(format!(
                "inline-icon {} has no authenticated texture",
                receipt.identity
            ));
        }
        for digest in &receipt.texture_sha256 {
            require_sha256(digest, "inline-icon texture")?;
        }
    }
    Ok(identities)
}

fn validate_run_ordinals(
    glyphs: &[OwnerScopedGlyphObservation],
    icons: &[OwnerScopedInlineIconObservation],
) -> Result<(), String> {
    let mut ordinals = glyphs
        .iter()
        .map(|run| run.run_ordinal)
        .chain(icons.iter().map(|run| run.run_ordinal))
        .collect::<Vec<_>>();
    ordinals.sort_unstable();
    for (expected, actual) in ordinals.into_iter().enumerate() {
        if actual as usize != expected {
            return Err("owner-glyph run order is duplicate or incomplete".into());
        }
    }
    Ok(())
}

fn validate_glyph_vertices(run_ordinal: u32, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() != GLYPH_VERTEX_BYTES {
        return Err(format!(
            "glyph run {run_ordinal} has {} vertex bytes, expected {}",
            bytes.len(),
            GLYPH_VERTEX_BYTES
        ));
    }
    let mut corners = BTreeSet::new();
    for vertex in bytes.chunks_exact(GLYPH_VERTEX_STRIDE) {
        let bits = |offset: usize| {
            u32::from_le_bytes(vertex[offset..offset + 4].try_into().unwrap())
        };
        for offset in [0, 4, 8, 12, 16] {
            if !f32::from_bits(bits(offset)).is_finite() {
                return Err(format!(
                    "glyph run {run_ordinal} contains non-finite vertex data"
                ));
            }
        }
        corners.insert((bits(0), bits(4), bits(12), bits(16)));
    }
    if corners.len() != 4 {
        return Err(format!(
            "glyph run {run_ordinal} is not an exact six-vertex quad receipt"
        ));
    }
    Ok(())
}

/// decimal digits plus `A-Z`, `_`, `a-z`, and `|`.
fn selected_inline_icon_tokens(bytes: &[u8]) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != 0x13 {
            index += 1;
            continue;
        }
        index += 1;
        let start = index;
        while index < bytes.len() && is_inline_icon_token_byte(bytes[index]) {
            index += 1;
        }
        if start == index || index - start > 0x80 {
            return Err("selected control-0x13 token is empty or exceeds engine bound".into());
        }
        tokens.push(
            String::from_utf8(bytes[start..index].to_vec())
                .map_err(|_| "selected control-0x13 token is not ASCII".to_string())?,
        );
    }
    Ok(tokens)
}

fn is_inline_icon_token_byte(byte: u8) -> bool {
    byte.is_ascii_digit()
        || byte.is_ascii_uppercase()
        || byte == b'_'
        || byte.is_ascii_lowercase()
        || byte == b'|'
}

fn require_sha256(value: &str, label: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} SHA-256 is not lowercase hex"));
    }
    Ok(())
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn row() -> RuntimeTextInput {
        RuntimeTextInput {
            context_id: "topbar::manpower::0".into(),
            pov_country_or_global: "GER".into(),
            view_id: "topbar".into(),
            element_id: "manpower".into(),
            source: RuntimeTextSourceIdentity::AuthoredToken("MANPOWER".into()),
            final_text_bytes: b"10".to_vec(),
            setter_caller_rva: 1,
            formatter_observed: true,
            layout: RuntimeTextLayout::AuthoredReproducible,
        }
    }

    fn capture(row: &RuntimeTextInput) -> OwnerScopedGlyphCapture {
        OwnerScopedGlyphCapture {
            authority: OwnerGlyphCaptureAuthority::exact_a729(),
            process_identity: "hoi4.exe:pid:creation-time".into(),
            hook_epoch: 1,
            frame_hour: 7,
            render_invocation_ordinal: 0,
            context_id: row.context_id.clone(),
            view_id: row.view_id.clone(),
            element_id: row.element_id.clone(),
            source: row.source.clone(),
            selected_final_text_sha256: hex_sha256(&row.final_text_bytes),
            clip: Some([0, 0, 32, 16]),
            glyph_decision_count: 1,
            inline_icon_decision_count: 0,
            glyphs: vec![OwnerScopedGlyphObservation {
                run_ordinal: 0,
                font_identity: "hoi_16b".into(),
                descriptor: GlyphDescriptorIdentity::UnicodeScalar('1' as u32),
                glyph_id: '1' as u32,
                owner_relative_x: 2,
                owner_relative_y: 3,
                rgba: [255; 4],
                generated_vertices: glyph_vertices(),
            }],
            inline_icons: Vec::new(),
            fonts: vec![FontAssetReceipt {
                identity: "hoi_16b".into(),
                descriptor_sha256: HASH.into(),
                atlas_sha256: vec![HASH.into()],
            }],
            icon_assets: Vec::new(),
            health: CaptureHealth::default(),
        }
    }

    fn glyph_vertices() -> Vec<u8> {
        let mut bytes = Vec::with_capacity(GLYPH_VERTEX_BYTES);
        for (x, y, u, v) in [
            (0.0f32, 0.0f32, 0.0f32, 0.0f32),
            (0.0, 1.0, 0.0, 1.0),
            (1.0, 0.0, 1.0, 0.0),
            (1.0, 1.0, 1.0, 1.0),
            (1.0, 0.0, 1.0, 0.0),
            (0.0, 1.0, 0.0, 1.0),
        ] {
            bytes.extend_from_slice(&x.to_bits().to_le_bytes());
            bytes.extend_from_slice(&y.to_bits().to_le_bytes());
            bytes.extend_from_slice(&0.0f32.to_bits().to_le_bytes());
            bytes.extend_from_slice(&u.to_bits().to_le_bytes());
            bytes.extend_from_slice(&v.to_bits().to_le_bytes());
            bytes.extend_from_slice(&0xffffffffu32.to_le_bytes());
            bytes.extend_from_slice(&0u32.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn admits_exact_pointer_free_owner_glyph_join() {
        let row = row();
        let admitted = OwnerGlyphCaptureAdmission::default()
            .admit(capture(&row), &row, 7)
            .unwrap();
        let RuntimeTextLayout::CapturedRuntime { glyphs, .. } = admitted.layout else {
            panic!("expected captured runtime layout");
        };
        assert_eq!(glyphs[0].font, "hoi_16b");
        assert_eq!(admitted.fonts[0].descriptor_sha256, HASH);
    }

    #[test]
    fn rejects_identity_digest_loss_and_invocation_gaps() {
        let row = row();
        let mut gate = OwnerGlyphCaptureAdmission::default();
        let mut wrong = capture(&row);
        wrong.selected_final_text_sha256 = HASH.into();
        assert!(gate.admit(wrong, &row, 7).is_err());

        let mut lost = capture(&row);
        lost.health.drop_count = 1;
        assert!(gate.admit(lost, &row, 7).is_err());

        let mut gap = capture(&row);
        gap.render_invocation_ordinal = 1;
        assert!(gate.admit(gap, &row, 7).is_err());
    }

    #[test]
    fn rejects_short_vertex_receipts_and_unauthenticated_assets() {
        let row = row();
        let mut short = capture(&row);
        short.glyphs[0].generated_vertices.pop();
        assert!(OwnerGlyphCaptureAdmission::default()
            .admit(short, &row, 7)
            .is_err());

        let mut unknown_font = capture(&row);
        unknown_font.fonts.clear();
        assert!(OwnerGlyphCaptureAdmission::default()
            .admit(unknown_font, &row, 7)
            .is_err());
    }

    #[test]
    fn admits_only_inline_icon_receipt_named_by_selected_control_token() {
        let mut row = row();
        row.final_text_bytes = b"A\x13army|fuel".to_vec();
        let mut capture = capture(&row);
        capture.glyph_decision_count = 0;
        capture.glyphs.clear();
        capture.fonts.clear();
        capture.inline_icon_decision_count = 1;
        capture.inline_icons = vec![OwnerScopedInlineIconObservation {
            run_ordinal: 0,
            token: "army|fuel".into(),
            sprite_identity: "GFX_army_fuel".into(),
            frame: 0,
            owner_relative_x: 4,
            owner_relative_y: 5,
            baseline: 8,
            advance: 12,
            draw_receipt_sha256: HASH.into(),
        }];
        capture.icon_assets = vec![InlineIconAssetReceipt {
            identity: "GFX_army_fuel".into(),
            definition_sha256: HASH.into(),
            texture_sha256: vec![HASH.into()],
        }];
        let admitted = OwnerGlyphCaptureAdmission::default()
            .admit(capture.clone(), &row, 7)
            .unwrap();
        let RuntimeTextLayout::CapturedRuntime { inline_icons, .. } = admitted.layout else {
            panic!("expected captured runtime layout");
        };
        assert_eq!(inline_icons[0].sprite, "GFX_army_fuel");

        capture.inline_icons[0].token = "wrong".into();
        assert!(OwnerGlyphCaptureAdmission::default()
            .admit(capture, &row, 7)
            .is_err());
    }
}
