//! Authored SAnimationMapData records for the generic CSprite route.
//!
//! a729: 14222c300 constructs and appends 0x90-byte records; 14222c690
//! assigns registered tokens 490..498, 502..504. Resource scheduling owns
//! texture loading; these records preserve authored operands for that owner.
//! Retained draw clocks belong to the sprite lifetime.

use crate::container_sprite_inputs::AnimationAdmissionRule;
use crate::paradox_text::Value;
use crate::{GameUiError, Result};
use portable_content_manifest::UsedContentKey;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AdmittedContainerAnimations {
    pub source_identity: UsedContentKey,
    /// Native vector order, after records whose type remains zero are discarded.
    pub records: Vec<ContainerAnimationRecord>,
}

#[derive(Clone, Debug)]
pub(crate) struct ContainerAnimationRecord {
    /// The authored ordinal, retained for diagnostics; draw ordinals index records.
    pub authored_ordinal: usize,
    pub mask_file: String,
    pub texture_file: String,
    pub rotation_radians: f32,
    pub looping: bool,
    pub duration: f32,
    pub delay: f32,
    pub reciprocal_texture_scale: [f32; 2],
    pub rotation_offset: [f32; 2],
    /// Native values: scrolling 1, clockwise 2, counterclockwise 3, pulsing 4.
    pub native_kind: i32,
    /// Native values: add 0, multiply 1, overlay 2, normal 3.
    pub blend_mode: i32,
    pub show_mask: u32,
    pub trigger_mask: u32,
}

impl PartialEq for ContainerAnimationRecord {
    fn eq(&self, other: &Self) -> bool {
        self.authored_ordinal == other.authored_ordinal && self.mask_file == other.mask_file
            && self.texture_file == other.texture_file && self.looping == other.looping
            && self.native_kind == other.native_kind && self.blend_mode == other.blend_mode
            && self.show_mask == other.show_mask && self.trigger_mask == other.trigger_mask
            && self.rotation_radians.to_bits() == other.rotation_radians.to_bits()
            && self.duration.to_bits() == other.duration.to_bits()
            && self.delay.to_bits() == other.delay.to_bits()
            && self.reciprocal_texture_scale.map(f32::to_bits) == other.reciprocal_texture_scale.map(f32::to_bits)
            && self.rotation_offset.map(f32::to_bits) == other.rotation_offset.map(f32::to_bits)
    }
}
impl Eq for ContainerAnimationRecord {}

/// Source parser context. A viewer's locale/control is not a source witness.
#[derive(Clone, Copy, Debug)]
pub(crate) struct NativeAnimationParseContext {
    pub mxcsr: u32,
    pub decimal_point: u8,
}

impl ContainerAnimationRecord {
    fn fresh(authored_ordinal: usize) -> Self {
        Self {
            authored_ordinal, mask_file: String::new(), texture_file: String::new(),
            rotation_radians: 0.0, looping: true, duration: 1.0, delay: 0.0,
            reciprocal_texture_scale: [1.0; 2], rotation_offset: [0.0; 2],
            native_kind: 0, blend_mode: 0, show_mask: u32::MAX, trigger_mask: 0,
        }
    }

    pub(crate) fn rule(&self) -> AnimationAdmissionRule {
        AnimationAdmissionRule {
            trigger_mask: self.trigger_mask, show_mask: self.show_mask,
            looping: self.looping, duration: self.duration,
        }
    }

}

impl AdmittedContainerAnimations {
    /// The context argument is an authenticated source witness, not the host's
    /// default mode. Definitions without authored float fields need no witness.
    pub(crate) fn parse(raw: &Value, source_identity: UsedContentKey, context: Option<NativeAnimationParseContext>) -> Result<Self> {
        source_identity.validate().map_err(GameUiError::parse)?;
        let mut records = Vec::new();
        for (ordinal, value) in raw.values_ci("animation").enumerate() {
            let fields = value.as_object().ok_or_else(|| refusal(ordinal, "animation", "expected block"))?;
            let mut record = ContainerAnimationRecord::fresh(ordinal);
            for field in fields {
                let Some(key) = field.key.as_deref() else {
                    return Err(refusal(ordinal, "animation", "anonymous field"));
                };
                let key = key.to_ascii_lowercase();
                let value = &field.value;
                if matches!(key.as_str(), "animationrotation" | "animationtime" | "animationdelay"
                    | "animationtexturescale" | "animationrotationoffset")
                {
                    require_float_control(context, ordinal, &key)?;
                }
                match key.as_str() {
                    "animationmaskfile" => record.mask_file = scalar(value, ordinal, &key)?.to_owned(),
                    "animationtexturefile" => record.texture_file = scalar(value, ordinal, &key)?.to_owned(),
                    "animationrotation" => {
                        let mut degrees = 0.0;
                        assign_float(value, &mut degrees, ordinal, &key)?;
                        record.rotation_radians = degrees * f32::from_bits(0x3c8efa35);
                    }
                    "animationlooping" => match scalar(value, ordinal, &key)? {
                        "yes" => record.looping = true,
                        "no" => record.looping = false,
                        _ => {} // 1424b06e0 leaves the destination unchanged.
                    },
                    "animationtime" => assign_float(value, &mut record.duration, ordinal, &key)?,
                    "animationdelay" => assign_float(value, &mut record.delay, ordinal, &key)?,
                    "animationtype" => match scalar(value, ordinal, &key)?.to_ascii_lowercase().as_str() {
                        "scrolling" => record.native_kind = 1,
                        "rotating" | "rotating_cw" => record.native_kind = 2,
                        "rotating_ccw" => record.native_kind = 3,
                        "pulsing" => record.native_kind = 4,
                        _ => {}
                    },
                    "animationblendmode" => match scalar(value, ordinal, &key)?.to_ascii_lowercase().as_str() {
                        "add" => record.blend_mode = 0,
                        "multiply" => record.blend_mode = 1,
                        "overlay" => record.blend_mode = 2,
                        "normal" => record.blend_mode = 3,
                        _ => {}
                    },
                    "animationtexturescale" => {
                        let mut scale = [0.0; 2];
                        assign_pair(value, &mut scale, ordinal, &key)?;
                        record.reciprocal_texture_scale = scale.map(|component| {
                            if component != 0.0 { 1.0 / component } else { 1.0 }
                        });
                    }
                    "animationrotationoffset" => assign_pair(value, &mut record.rotation_offset, ordinal, &key)?,
                    "animationframes" => {
                        if let Some(mask) = frame_mask(value, ordinal, &key)? { record.show_mask = mask; }
                    }
                    "animationtriggerframes" => {
                        if let Some(mask) = frame_mask(value, ordinal, &key)? { record.trigger_mask |= mask; }
                    }
                    _ => {}
                }
            }
            if record.native_kind != 0 { records.push(record); }
        }
        if records.len() > i32::MAX as usize { return Err(GameUiError::parse("container animation count exceeds native signed range")); }
        Ok(Self { source_identity, records })
    }
}

fn refusal(ordinal: usize, key: &str, reason: &str) -> GameUiError {
    GameUiError::parse(format!("container animation {ordinal} {key}: {reason}"))
}

fn require_float_control(context: Option<NativeAnimationParseContext>, ordinal: usize, key: &str) -> Result<()> {
    if !context.is_some_and(|context| context.mxcsr & 0xffc0 == 0x1f80 && context.decimal_point == b'.') {
        return Err(refusal(ordinal, key, "native floating-point control unavailable"));
    }
    #[cfg(target_arch = "x86_64")]
    {
        let mut control = 0_u32;
        // Read only; never change the renderer thread's environment.
        unsafe {
            std::arch::asm!("stmxcsr [{out}]", out = in(reg) &mut control,
                options(nostack, preserves_flags));
        }
        if control & 0xffc0 == 0x1f80 { return Ok(()); }
    }
    Err(refusal(ordinal, key, "viewer floating-point control unsupported"))
}

fn scalar<'a>(value: &'a Value, ordinal: usize, key: &str) -> Result<&'a str> {
    let text = value.as_scalar().ok_or_else(|| refusal(ordinal, key, "expected scalar"))?;
    if text.starts_with('@') { return Err(refusal(ordinal, key, "unresolved authored variable")); }
    if text.contains('\0') || !text.is_ascii() {
        return Err(refusal(ordinal, key, "native token encoding unavailable"));
    }
    Ok(text)
}

/// Admits finite decimal operands with an unambiguous native %f conversion.
/// Unsupported CRT spellings are refused at the owning animation, not rewritten.
fn assign_float(value: &Value, destination: &mut f32, ordinal: usize, key: &str) -> Result<()> {
    let text = scalar(value, ordinal, key)?;
    if text.is_empty() { return Ok(()); }
    if text.len() > 128 || !text.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.' | b'e' | b'E')) {
        return Err(refusal(ordinal, key, "native float spelling unavailable"));
    }
    let parsed = text.parse::<f32>().map_err(|_| refusal(ordinal, key, "native float spelling unavailable"))?;
    if !parsed.is_finite() { return Err(refusal(ordinal, key, "native nonfinite conversion unavailable")); }
    *destination = parsed;
    Ok(())
}

fn assign_pair(value: &Value, destination: &mut [f32; 2], ordinal: usize, key: &str) -> Result<()> {
    let entries = value.as_object().ok_or_else(|| refusal(ordinal, key, "expected vector block"))?;
    let mut axis = 0;
    for entry in entries {
        if axis == 2 { return Err(refusal(ordinal, key, "native vector overflow")); }
        if let Some(name) = entry.key.as_deref() {
            // 140acebe0 advances only after the expected next axis is parsed.
            if !name.eq_ignore_ascii_case(["x", "y"][axis]) {
                return Err(refusal(ordinal, key, "native vector axis order unavailable"));
            }
        }
        assign_float(&entry.value, &mut destination[axis], ordinal, key)?;
        axis += 1;
    }
    Ok(())
}

fn frame_mask(value: &Value, ordinal: usize, key: &str) -> Result<Option<u32>> {
    let entries = value.as_object().ok_or_else(|| refusal(ordinal, key, "expected frame list"))?;
    if entries.is_empty() { return Ok(None); }
    let mut mask = 0;
    for entry in entries {
        if entry.key.is_some() { return Err(refusal(ordinal, key, "expected anonymous frame number")); }
        let mut frame = 0;
        map_viewer::authored_fixed::assign_native_i32(scalar(&entry.value, ordinal, key)?, &mut frame);
        mask |= 1_u32 << (frame as u32 & 31);
    }
    Ok(Some(mask))
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::{ContentLayerKind, paradox_text::parse_str};

    fn content(path: &str) -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: "a".repeat(64),
            stored_game_relative_path: path.into(), normalized_game_relative_path: path.into(),
            winning_layer_ordinal: 0, layer_kind: ContentLayerKind::Base, layer_id: "base".into(),
            source_sha256: "b".repeat(64), payload_sha256: "b".repeat(64), payload_bytes: 1,
        }
    }

    fn parse(text: &str) -> AdmittedContainerAnimations {
        AdmittedContainerAnimations::parse(&parse_str(text).root, content("interface/test.gfx"), Some(context(0x1f80))).unwrap()
    }

    fn context(mxcsr: u32) -> NativeAnimationParseContext {
        NativeAnimationParseContext { mxcsr, decimal_point: b'.' }
    }

    #[test]
    fn constructor_defaults_and_type_zero_discard_preserve_native_vector_order() {
        let parsed = parse("animation = { animationtime = 9 } animation = { animationtype = pulsing } animation = { animationtype = unknown } animation = { animationtype = scrolling }");
        assert_eq!(parsed.records.len(), 2);
        let first = &parsed.records[0];
        assert_eq!(first.authored_ordinal, 1);
        assert_eq!(first.native_kind, 4);
        assert_eq!(first.rule(), AnimationAdmissionRule { trigger_mask: 0, show_mask: u32::MAX, looping: true, duration: 1.0 });
        assert_eq!(first.delay.to_bits(), 0);
        assert_eq!(first.rotation_radians.to_bits(), 0);
        assert_eq!(first.rotation_offset, [0.0; 2]);
        assert_eq!(first.reciprocal_texture_scale, [1.0; 2]);
        assert_eq!(first.blend_mode, 0);
        assert!(first.mask_file.is_empty() && first.texture_file.is_empty());
        assert_eq!(parsed.records[1].authored_ordinal, 3);
    }

    #[test]
    fn duplicate_show_masks_replace_but_trigger_masks_accumulate_with_native_shift() {
        let parsed = parse("animation = { animationtype = pulsing animationframes = { 1 2 } animationframes = { -1 33 } animationframes = {} animationtriggerframes = { 1 } animationtriggerframes = { 2 34 } }");
        assert_eq!(parsed.records[0].show_mask, 0x8000_0002);
        assert_eq!(parsed.records[0].trigger_mask, 6);
    }

    #[test]
    fn native_integer_assignment_accepts_prefix_and_auto_radix() {
        let parsed = parse("animation = { animationtype = pulsing animationframes = { 010 0x21 2suffix bad } }");
        assert_eq!(parsed.records[0].show_mask, (1 << 8) | (1 << 1) | (1 << 2) | 1);
    }

    #[test]
    fn duplicate_scalar_assignments_and_unknown_enums_preserve_prior_values() {
        let parsed = parse("animation = { animationtype = ROTATING_CCW animationtype = unknown animationblendmode = normal animationblendmode = unknown animationlooping = no animationlooping = YES animationtime = 4 animationtime = 2 animationdelay = -0 }");
        let record = &parsed.records[0];
        assert_eq!(record.native_kind, 3);
        assert_eq!(record.blend_mode, 3);
        assert!(!record.looping);
        assert_eq!(record.duration, 2.0);
        assert_eq!(record.delay.to_bits(), 0x8000_0000);
    }

    #[test]
    fn rotation_uses_native_degree_constant_and_fresh_zero_temporary() {
        let parsed = parse("animation = { animationtype = rotating animationrotation = 90 } animation = { animationtype = pulsing animationrotation = 90 animationrotation = \"\" }");
        assert_eq!(parsed.records[0].rotation_radians.to_bits(), (90.0_f32 * f32::from_bits(0x3c8efa35)).to_bits());
        assert_eq!(parsed.records[1].rotation_radians.to_bits(), 0);
    }

    #[test]
    fn scale_restarts_local_zero_while_rotation_offset_retains_missing_components() {
        let parsed = parse("animation = { animationtype = scrolling animationtexturescale = { x = 4 y = 8 } animationtexturescale = { 2 } animationrotationoffset = { x = 3 y = 5 } animationrotationoffset = { 7 } }");
        assert_eq!(parsed.records[0].reciprocal_texture_scale, [0.5, 1.0]);
        assert_eq!(parsed.records[0].rotation_offset, [7.0, 5.0]);
    }

    #[test]
    fn both_signed_zero_scale_components_produce_positive_one() {
        let parsed = parse("animation = { animationtype = scrolling animationtexturescale = { -0 0 } }");
        assert_eq!(parsed.records[0].reciprocal_texture_scale.map(f32::to_bits), [0x3f80_0000; 2]);
    }

    #[test]
    fn unsupported_float_and_unresolved_variables_name_the_animation() {
        for operand in ["@duration", "nan", "0x1p2", "1suffix"] {
            let raw = parse_str(&format!("animation = {{ animationtype = pulsing animationtime = {operand} }}")).root;
            let error = AdmittedContainerAnimations::parse(&raw, content("interface/test.gfx"), Some(context(0x1f80))).unwrap_err();
            assert!(error.to_string().contains("container animation 0 animationtime"));
        }
    }

    #[test]
    fn float_assignment_requires_source_control_independently_of_viewer_defaults() {
        let raw = parse_str("animation = { animationtype = pulsing animationtime = 2 }").root;
        for control in [None, Some(context(0x3f80)), Some(context(0x9f80)), Some(context(0x1fc0))] {
            assert!(AdmittedContainerAnimations::parse(&raw, content("interface/test.gfx"), control)
                .unwrap_err().to_string().contains("native floating-point control unavailable"));
        }
        let defaults = parse_str("animation = { animationtype = pulsing }").root;
        assert!(AdmittedContainerAnimations::parse(&defaults, content("interface/test.gfx"), None).is_ok());
    }
}
