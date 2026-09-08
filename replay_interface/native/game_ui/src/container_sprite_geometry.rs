//! Native a729 sprite UV construction (14222cb90) and frame Offset
//! (14222a990 / 14222d5a0). Geometry ownership and pixel sampling stay in the
//! compositor. Inputs describe the reached CSprite state, not authored guesses.

use crate::gfx::SpriteDefinition;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpriteGeometryRejection {
    UnsupportedSpriteKind,
    InvalidFrameCount,
    FrameCountDisagreesWithDefinition,
    InvalidAuthoredScalar,
    InvalidTextureDimensions,
    NonfiniteTextureScale,
    NonfiniteCoordinate,
    InvalidLogicalDimensions,
}

/// 1401fabf0 token 11029 parses settings+250, marks +251 present, then passes
/// that byte to 1423f0790 at 1401fb08e. This proves an assignment, not the
/// global's initial value or its lifetime. Callers must retain a named missing
/// normalization input until the consumed setting is established.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpriteTextureNormalization {
    ActualDimensions,
    NextPowerOfTwo,
}

impl SpriteTextureNormalization {
    pub(crate) fn from_native_force_pow2(force_pow2: bool) -> Self {
        if force_pow2 {
            Self::NextPowerOfTwo
        } else {
            Self::ActualDimensions
        }
    }

    fn extent(self, dimension: u32) -> f32 {
        match self {
            Self::ActualDimensions => dimension as f32,
            // Positive signed dimensions cannot overflow the native u32
            // shift/OR/INC sequence; the largest rounded result is 0x80000000.
            Self::NextPowerOfTwo => dimension.next_power_of_two() as f32,
        }
    }
}

/// Resource-affecting options stored at texture-manager entry +3c. A request
/// for an existing path does not replace them (1423f17e0). The owner must bind
/// these to the consumed entry or witness its first ordinary registration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OrdinarySpriteTextureLoadOptions {
    pub(crate) dds_mips_to_skip: u32,
    pub(crate) pad_to_power_of_two: bool,
    pub(crate) srgb: bool,
}

impl OrdinarySpriteTextureLoadOptions {
    /// 14222b2a0 / 14222b5f0 request skip=0, pad=0, sRGB=0. Base and animation
    /// requests differ in alpha-mask bookkeeping, which does not mutate texels.
    /// This constructor is not authority over an earlier cached registration.
    pub(crate) fn from_first_ordinary_registration() -> Self {
        Self { dds_mips_to_skip: 0, pad_to_power_of_two: false, srgb: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OrdinaryTextureSourceRejection {
    MissingEffectiveRegistration,
    MissingForcePow2Setting,
    InvalidDds,
    NativeTextureFileTooLarge,
    UnsupportedTextureShape,
    UnsupportedMipSkip,
    UnsupportedMipTopology,
    UnsupportedFormat,
    UnsupportedSrgb,
    UnsupportedPadding,
    CompressedExtentNeedsRounding,
    TruncatedMip,
}

/// A borrowed, unchanged DDS mip 0 proved to be the only uploaded mip in the
/// immutable DX11 file-resource path. The caller authenticates these encoded
/// bytes with its existing resolver lease and decodes these same bytes. This
/// descriptor establishes resource topology; sampler/effect binding remains
/// with the shader owner. It does not create a texture-manager lifetime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OrdinarySingleMipTextureSource<'a> {
    encoded: &'a [u8],
    metadata: crate::dds::DdsInfo,
    normalization: SpriteTextureNormalization,
}

impl<'a> OrdinarySingleMipTextureSource<'a> {
    pub(crate) fn admit(
        authenticated_encoded: &'a [u8],
        effective_registration: Option<&OrdinarySpriteTextureLoadOptions>,
        explicit_force_pow2: Option<bool>,
    ) -> Result<Self, OrdinaryTextureSourceRejection> {
        use OrdinaryTextureSourceRejection as Reject;
        let options = effective_registration.ok_or(Reject::MissingEffectiveRegistration)?;
        let force_pow2 = explicit_force_pow2.ok_or(Reject::MissingForcePow2Setting)?;
        // 1423f2d50 refuses files larger than 16 MiB before parsing/upload.
        if authenticated_encoded.len() > 0x100_0000 {
            return Err(Reject::NativeTextureFileTooLarge);
        }
        let metadata = crate::dds::info_bytes(authenticated_encoded)
            .map_err(|_| Reject::InvalidDds)?;
        if metadata.is_cubemap || metadata.is_volume {
            return Err(Reject::UnsupportedTextureShape);
        }
        if options.dds_mips_to_skip != 0 {
            return Err(Reject::UnsupportedMipSkip);
        }
        if metadata.mip_count != 1 {
            return Err(Reject::UnsupportedMipTopology);
        }
        if options.srgb {
            return Err(Reject::UnsupportedSrgb);
        }
        let width = metadata.width;
        let height = metadata.height;
        // 1423f8fa0 is a no-op for an already power-of-two extent, even when
        // requested by the entry or the explicit global setting. Otherwise it
        // zero-pads rows and changes dimensions; unchanged mip 0 is insufficient.
        if (options.pad_to_power_of_two || force_pow2)
            && (!width.is_power_of_two() || !height.is_power_of_two())
        {
            return Err(Reject::UnsupportedPadding);
        }
        let payload_bytes = match metadata.format.as_str() {
            "A8R8G8B8" => width.checked_mul(height).and_then(|n| n.checked_mul(4)),
            "R8G8B8" => width.checked_mul(height).and_then(|n| n.checked_mul(3)),
            "DXT1" | "DXT3" | "DXT5" => {
                // 142410030 rounds compressed extents up to whole blocks before
                // storing resource dimensions. The shared decoder crops partial
                // blocks to the source extent, so that different branch refuses.
                if width % 4 != 0 || height % 4 != 0 {
                    return Err(Reject::CompressedExtentNeedsRounding);
                }
                let block_bytes = if metadata.format == "DXT1" { 8 } else { 16 };
                (width / 4).checked_mul(height / 4).and_then(|n| n.checked_mul(block_bytes))
            }
            // Native format 13 retains X8's fourth byte, while the existing
            // decoder forces alpha=255. Do not silently equate those inputs.
            _ => return Err(Reject::UnsupportedFormat),
        }.and_then(|n| n.checked_add(128)).ok_or(Reject::TruncatedMip)?;
        if authenticated_encoded.len() < payload_bytes {
            return Err(Reject::TruncatedMip);
        }
        // 142410030 loads one record even for DDS header mip-count zero, then
        // sets count=1. 1423fb4f0 uploads that count; 1423ffc90 exposes mip 0/count1.
        Ok(Self {
            encoded: authenticated_encoded,
            metadata,
            normalization: SpriteTextureNormalization::from_native_force_pow2(force_pow2),
        })
    }

    pub(crate) fn encoded(&self) -> &'a [u8] {
        self.encoded
    }

    pub(crate) fn dimensions(&self) -> [u32; 2] {
        // Existing DdsInfo validation bounds dimensions below the i32 limit.
        [self.metadata.width as u32, self.metadata.height as u32]
    }

    pub(crate) fn normalization(&self) -> SpriteTextureNormalization {
        self.normalization
    }
}

/// The instance operands copied to CSpriteType immediately before its draw.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ContainerSpriteGeometryState {
    /// CSprite+190/+194 -> type+ac/+b0 through 14222dc10.
    pub(crate) crop_right_bottom: [u32; 2],
    /// CSprite+198/+19c -> type+b8/+b4 through 14222dee0.
    pub(crate) crop_left_top: [u32; 2],
    /// CSprite+174 -> type+f8 through 14222dc50.
    pub(crate) texture_v_scale: f32,
    /// CSprite+178 -> type+110 through 14222dc20.
    pub(crate) flip_u: bool,
}

// Presentation rollback compares retained native bits, including signed zero
// and NaN payloads; numeric f32 equality does not preserve that identity.
impl PartialEq for ContainerSpriteGeometryState {
    fn eq(&self, other: &Self) -> bool {
        self.crop_right_bottom == other.crop_right_bottom
            && self.crop_left_top == other.crop_left_top
            && self.texture_v_scale.to_bits() == other.texture_v_scale.to_bits()
            && self.flip_u == other.flip_u
    }
}

impl Eq for ContainerSpriteGeometryState {}

impl ContainerSpriteGeometryState {
    /// 1422e9210 establishes these exact instance values. This constructor is
    /// for a witnessed new CSprite lifetime; subsequent setter values belong
    /// to that lifetime and must not be reset by the draw caller.
    pub(crate) fn from_csprite_constructor() -> Self {
        Self {
            crop_right_bottom: [0, 0],
            crop_left_top: [0, 0],
            texture_v_scale: 1.0,
            flip_u: false,
        }
    }
}

/// Staged writes of 1422e8ee0. The owner applies this packet to the reached
/// sprite or C2dPositionObject lifetime before refreshing its parent transform.
/// Float bits make candidate comparison and rollback exact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NativeSpritePositionWrite {
    /// Object +128/+12c: the actual integer setter argument converted to f32.
    pub(crate) requested_position_bits: [u32; 2],
    /// Local matrix +80/+84. The setter also resets the matrix to identity
    /// and sets its translation Z to -1; world composition belongs to the owner.
    pub(crate) local_translation_bits: [u32; 2],
}

impl NativeSpritePositionWrite {
    /// Borrow the actual setter argument, object +130/+134, and the result of
    /// virtual +158. CSprite stores physical graphics dimensions; container
    /// C2dPositionObjects store logical dimensions and return size [0, 0].
    pub(crate) fn stage_set_position(
        applied_position: &[i32; 2],
        stored_extent: &[u32; 2],
        native_integer_size: &[i32; 2],
    ) -> Self {
        let [x, y] = applied_position.map(|value| value as f32);
        let local_x = x + (stored_extent[0] as f32 * -1.0) * 0.5;
        let local_y = (stored_extent[1] as f32 * 0.5 - native_integer_size[1] as f32) - y;
        Self {
            requested_position_bits: [x.to_bits(), y.to_bits()],
            local_translation_bits: [local_x.to_bits(), local_y.to_bits()],
        }
    }

    /// Fresh static CIcon, upper_left orientation, both center flags clear.
    /// 142358910 reads its position through 140a3d660's fixed integer pair;
    /// 1422e6b90 converts the retained f32 pair back to i32 before 1422e66f0
    /// dispatches the sprite setter. This is not the container CPosition reader.
    /// The caller supplies that successfully read pair (constructor absence is
    /// [0, 0]) and rejects other orientation/centering or 3D position branches.
    pub(crate) fn stage_static_icon(
        fixed_position: &[i32; 2],
        stored_physical_extent: &[u32; 2],
        native_integer_size: &[i32; 2],
    ) -> Self {
        let applied = fixed_position.map(|value| native_position_cvttss2si(value as f32));
        Self::stage_set_position(&applied, stored_physical_extent, native_integer_size)
    }

    /// 14229f050 uses this for both its own evaluated CPosition and its content
    /// node's evaluated left/top margins. It adds CVTT(f64(logical)/2) before
    /// calling 1422e8ee0; that setter subtracts f32(logical)/2. Odd dimensions
    /// therefore leave half-pixel offsets. Do not cancel these two operations.
    pub(crate) fn stage_container_node(
        evaluated_position: &[i32; 2],
        logical_extent: &[i32; 2],
    ) -> Result<Self, SpriteGeometryRejection> {
        if logical_extent.iter().any(|value| *value <= 0) {
            return Err(SpriteGeometryRejection::InvalidLogicalDimensions);
        }
        // Positive i32 / 2 is exactly the witnessed f64 multiply then CVTT.
        let applied = [
            evaluated_position[0].wrapping_add(logical_extent[0] / 2),
            evaluated_position[1].wrapping_add(logical_extent[1] / 2),
        ];
        Ok(Self::stage_set_position(
            &applied,
            &logical_extent.map(|value| value as u32),
            &[0, 0],
        ))
    }

    pub(crate) fn local_translation(self) -> [f32; 2] {
        self.local_translation_bits.map(f32::from_bits)
    }
}

fn native_position_cvttss2si(value: f32) -> i32 {
    if value >= -2_147_483_648.0 && value < 2_147_483_648.0 {
        value as i32
    } else {
        i32::MIN
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FrameDirection {
    Horizontal,
    Vertical,
}

/// Four original vertex UVs and the separately consumed frame-step constants.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ContainerSpriteUvGeometry {
    /// Native strip order: first-X/first-Y, first-X/second-Y,
    /// second-X/first-Y, second-X/second-Y. V runs in the opposite direction.
    pub(crate) vertex_uvs: [[f32; 2]; 4],
    pub(crate) frame_step: [f32; 2],
    frame_direction: FrameDirection,
    frame_count: i32,
    frame_dimensions: [f32; 2],
}

impl ContainerSpriteUvGeometry {
    /// Consumes the existing parsed definition plus its preserved raw fields.
    /// Only generic spriteType uses the witnessed four-vertex builder; bordered
    /// and specialized sprite types have other geometry owners.
    pub(crate) fn from_definition(
        sprite: &SpriteDefinition,
        authenticated_texture_dimensions: [u32; 2],
        instance: ContainerSpriteGeometryState,
        normalization: SpriteTextureNormalization,
    ) -> Result<Self, SpriteGeometryRejection> {
        if !sprite.kind.eq_ignore_ascii_case("spriteType") {
            return Err(SpriteGeometryRejection::UnsupportedSpriteKind);
        }
        let frame_count = i32::try_from(sprite.no_of_frames)
            .ok()
            .filter(|count| *count > 0)
            .ok_or(SpriteGeometryRejection::InvalidFrameCount)?;
        // Existing SpriteDefinition is the parsed frame-count owner. Reconcile
        // its value against native assignments instead of duplicating its
        // fallback/max rules or silently accepting contradictory raw fields.
        let mut assigned_frames = frame_count;
        for value in sprite.raw.values_ci("noOfFrames") {
            let scalar = authored_scalar(value)?;
            map_viewer::authored_fixed::assign_native_i32(scalar, &mut assigned_frames);
        }
        if assigned_frames != frame_count {
            return Err(SpriteGeometryRejection::FrameCountDisagreesWithDefinition);
        }
        let mut direction = FrameDirection::Horizontal; // 14222a020 +10c=0.
        for value in sprite.raw.values_ci("framedirectiony") {
            // 14222c300 token571 initializes a fresh local bool to false for
            // each field, then 1424abca0 parses it. Unrecognized scalar tokens
            // leave that local false, so repeated fields still overwrite +10c.
            direction = if authored_scalar(value)? == "yes" {
                FrameDirection::Vertical
            } else {
                FrameDirection::Horizontal
            };
        }
        Self::build(
            authenticated_texture_dimensions,
            frame_count,
            direction,
            instance,
            normalization,
        )
    }

    fn build(
        dimensions: [u32; 2],
        frame_count: i32,
        direction: FrameDirection,
        instance: ContainerSpriteGeometryState,
        normalization: SpriteTextureNormalization,
    ) -> Result<Self, SpriteGeometryRejection> {
        if dimensions
            .iter()
            .any(|extent| *extent == 0 || *extent > i32::MAX as u32)
        {
            return Err(SpriteGeometryRejection::InvalidTextureDimensions);
        }
        if !instance.texture_v_scale.is_finite() {
            return Err(SpriteGeometryRejection::NonfiniteTextureScale);
        }
        let [width, height] = dimensions;
        // +94 is one in 14222a020. 14222c300 writes noOfFrames only at +90.
        // Vertical layout swaps the two axes before the native reciprocals.
        let [columns, rows] = match direction {
            FrameDirection::Horizontal => [frame_count, 1],
            FrameDirection::Vertical => [1, frame_count],
        };
        let inverse_columns = 1.0_f32 / columns as f32;
        let inverse_rows = 1.0_f32 / rows as f32;
        let inverse_width = 1.0_f32 / normalization.extent(width);
        let inverse_height = 1.0_f32 / normalization.extent(height);
        let mut u0 = instance.crop_left_top[0] as f32 * inverse_width;
        let v0 = instance.crop_left_top[1] as f32 * inverse_height;
        let u_far = width.wrapping_sub(instance.crop_right_bottom[0]) as f32 * inverse_width;
        let v_far = height.wrapping_sub(instance.crop_right_bottom[1]) as f32 * inverse_height;
        let du = (u_far - u0) * inverse_columns;
        let dv = ((v_far - v0) * instance.texture_v_scale) * inverse_rows;
        // Frame steps are written before the U-flip branch; do not negate them.
        let frame_step = match direction {
            FrameDirection::Horizontal => [du, 0.0],
            FrameDirection::Vertical => [0.0, dv],
        };
        let mut u1 = u0 + du;
        let v1 = v0 + dv;
        if instance.flip_u {
            u0 = 1.0 - u0;
            u1 = 1.0 - u1;
        }
        let vertex_uvs = [[u0, v1], [u0, v0], [u1, v1], [u1, v0]];
        if vertex_uvs
            .iter()
            .flatten()
            .chain(frame_step.iter())
            .any(|value| !value.is_finite())
        {
            return Err(SpriteGeometryRejection::NonfiniteCoordinate);
        }
        Ok(Self {
            vertex_uvs,
            frame_step,
            frame_direction: direction,
            frame_count,
            frame_dimensions: [width as f32 * inverse_columns, height as f32 * inverse_rows],
        })
    }

    /// 14222a990 followed by 14222d5a0's signed-int conversion and multiply.
    /// Selection/admission of the native zero-based index belongs to the caller.
    pub(crate) fn frame_offset(
        self,
        native_zero_based_index: i32,
    ) -> Result<[f32; 2], SpriteGeometryRejection> {
        let indices = match self.frame_direction {
            FrameDirection::Horizontal => [native_zero_based_index, 0],
            FrameDirection::Vertical => [0, native_zero_based_index],
        };
        let offset = [
            indices[0] as f32 * self.frame_step[0],
            indices[1] as f32 * self.frame_step[1],
        ];
        if offset.iter().any(|value| !value.is_finite()) {
            return Err(SpriteGeometryRejection::NonfiniteCoordinate);
        }
        Ok(offset)
    }

    pub(crate) fn frame_count(self) -> i32 {
        self.frame_count
    }

    /// 14222b4b0 then 1422e9530, for witnessed instance scale +124 == 1.
    /// The integer getter must not replace the fractional vertex dimensions.
    pub(crate) fn native_size_at_unit_scale(self) -> [i32; 2] {
        self.frame_dimensions.map(|extent| {
            native_position_cvttss2si(native_position_cvttss2si(extent) as f32 * 1.0)
        })
    }

    /// 14222cb90 for zero crops, type +f0 == 1 and geometric stretch +f4 == 1.
    /// The caller must witness those reached operands; this method neither
    /// resets nor infers them. Texture V scale and U flip only affect UVs.
    pub(crate) fn uncropped_local_quad_at_unit_scale(self) -> [[f32; 3]; 4] {
        let [width, height] = self.frame_dimensions;
        [
            [0.0, 0.0, 1.0],
            [0.0, height, 1.0],
            [width, 0.0, 1.0],
            [width, height, 1.0],
        ]
    }
}

fn authored_scalar(value: &crate::paradox_text::Value) -> Result<&str, SpriteGeometryRejection> {
    let scalar = value
        .as_scalar()
        .ok_or(SpriteGeometryRejection::InvalidAuthoredScalar)?;
    if !scalar.is_ascii() || scalar.contains('\0') || scalar.starts_with('@') {
        return Err(SpriteGeometryRejection::InvalidAuthoredScalar);
    }
    Ok(scalar)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::SpriteRegistry;

    fn definition(fields: &str) -> SpriteDefinition {
        let mut registry = SpriteRegistry::new();
        registry.parse_content(format!("spriteTypes = {{ spriteType = {{ name = test texturefile = test.dds {fields} }} }}").as_bytes(), "test.gfx").unwrap();
        registry.get("test").unwrap().clone()
    }

    fn single_mip_dds(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![0; 128 + (width * height * 4) as usize];
        bytes[..4].copy_from_slice(b"DDS ");
        for (offset, value) in [
            (4, 124), (8, 0x100f), (12, height), (16, width), (28, 1),
            (76, 32), (80, 0x41), (88, 32), (92, 0x00ff_0000),
            (96, 0x0000_ff00), (100, 0x0000_00ff), (104, 0xff00_0000), (108, 0x1000),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn ordinary_single_mip_requires_registration_and_explicit_setting() {
        use OrdinaryTextureSourceRejection as Reject;
        let bytes = single_mip_dds(3, 2);
        let options = OrdinarySpriteTextureLoadOptions::from_first_ordinary_registration();
        assert_eq!(OrdinarySingleMipTextureSource::admit(&bytes, None, Some(false)).unwrap_err(), Reject::MissingEffectiveRegistration);
        assert_eq!(OrdinarySingleMipTextureSource::admit(&bytes, Some(&options), None).unwrap_err(), Reject::MissingForcePow2Setting);
        let source = OrdinarySingleMipTextureSource::admit(&bytes, Some(&options), Some(false)).unwrap();
        assert_eq!(source.dimensions(), [3, 2]);
        assert_eq!(source.normalization(), SpriteTextureNormalization::ActualDimensions);
        assert!(std::ptr::eq(source.encoded().as_ptr(), bytes.as_ptr()));
        let skipped = OrdinarySpriteTextureLoadOptions { dds_mips_to_skip: 1, ..options };
        assert_eq!(OrdinarySingleMipTextureSource::admit(&bytes, Some(&skipped), Some(false)).unwrap_err(), Reject::UnsupportedMipSkip);
    }

    #[test]
    fn ordinary_single_mip_distinguishes_noop_padding_and_changed_resource() {
        use OrdinaryTextureSourceRejection as Reject;
        let options = OrdinarySpriteTextureLoadOptions::from_first_ordinary_registration();
        let bytes = single_mip_dds(4, 4);
        let source = OrdinarySingleMipTextureSource::admit(&bytes, Some(&options), Some(true)).unwrap();
        assert_eq!(source.dimensions(), [4, 4]);
        assert_eq!(source.normalization(), SpriteTextureNormalization::NextPowerOfTwo);
        let npot = single_mip_dds(3, 2);
        assert_eq!(OrdinarySingleMipTextureSource::admit(&npot, Some(&options), Some(true)).unwrap_err(), Reject::UnsupportedPadding);
        let padded = OrdinarySpriteTextureLoadOptions { pad_to_power_of_two: true, ..options };
        assert_eq!(OrdinarySingleMipTextureSource::admit(&npot, Some(&padded), Some(false)).unwrap_err(), Reject::UnsupportedPadding);
    }

    #[test]
    fn ordinary_single_mip_rejects_extra_mips_truncation_and_alpha_mismatch() {
        use OrdinaryTextureSourceRejection as Reject;
        let options = OrdinarySpriteTextureLoadOptions::from_first_ordinary_registration();
        let mut bytes = single_mip_dds(4, 4);
        bytes[28..32].copy_from_slice(&0_u32.to_le_bytes());
        assert!(OrdinarySingleMipTextureSource::admit(&bytes, Some(&options), Some(false)).is_ok());
        bytes[28..32].copy_from_slice(&2_u32.to_le_bytes());
        assert_eq!(OrdinarySingleMipTextureSource::admit(&bytes, Some(&options), Some(false)).unwrap_err(), Reject::UnsupportedMipTopology);
        bytes[28..32].copy_from_slice(&1_u32.to_le_bytes());
        bytes.pop();
        assert_eq!(OrdinarySingleMipTextureSource::admit(&bytes, Some(&options), Some(false)).unwrap_err(), Reject::TruncatedMip);
        bytes.push(0);
        bytes[104..108].copy_from_slice(&0_u32.to_le_bytes());
        assert_eq!(OrdinarySingleMipTextureSource::admit(&bytes, Some(&options), Some(false)).unwrap_err(), Reject::UnsupportedFormat);
    }

    #[test]
    fn retained_geometry_equality_preserves_float_bits() {
        let mut first = ContainerSpriteGeometryState::from_csprite_constructor();
        first.texture_v_scale = f32::from_bits(0x7fc0_0042);
        let mut second = first;
        assert_eq!(first, second);
        second.texture_v_scale = f32::from_bits(0x7fc0_0043);
        assert_ne!(first, second);
        first.texture_v_scale = 0.0;
        second.texture_v_scale = -0.0;
        assert_ne!(first, second);
    }

    #[test]
    fn static_icon_preserves_its_integer_float_integer_round_trip() {
        let icon = NativeSpritePositionWrite::stage_static_icon(
            &[16_777_217, i32::MAX], &[100, 100], &[4, 4],
        );
        assert_eq!(icon.requested_position_bits.map(f32::from_bits),
            [16_777_216.0, -2_147_483_648.0]);
        // CButtonDrag passes its fixed i32 pair directly to the same setter.
        let button = NativeSpritePositionWrite::stage_set_position(
            &[16_777_217, i32::MAX], &[100, 100], &[4, 4],
        );
        assert_eq!(button.requested_position_bits[1], 2_147_483_648.0_f32.to_bits());
        assert_ne!(icon, button);
    }

    #[test]
    fn container_node_preserves_odd_logical_half_pixels_and_wrapping_add() {
        let own = NativeSpritePositionWrite::stage_container_node(&[7, 9], &[101, 99]).unwrap();
        assert_eq!(own.local_translation(), [6.5, -8.5]);
        let content = NativeSpritePositionWrite::stage_container_node(&[0, 0], &[101, 99]).unwrap();
        assert_eq!(content.local_translation(), [-0.5, 0.5]);
        let wrapped = NativeSpritePositionWrite::stage_container_node(&[i32::MAX, 0], &[2, 2]).unwrap();
        assert_eq!(wrapped.requested_position_bits[0], (-2_147_483_648.0_f32).to_bits());
        assert_eq!(NativeSpritePositionWrite::stage_container_node(&[0, 0], &[0, 2]),
            Err(SpriteGeometryRejection::InvalidLogicalDimensions));
    }

    #[test]
    fn fractional_frame_height_survives_the_integer_position_getter() {
        let geometry = ContainerSpriteUvGeometry::from_definition(
            &definition("noOfFrames = 3 framedirectiony = yes"),
            [12, 13], ContainerSpriteGeometryState::from_csprite_constructor(),
            SpriteTextureNormalization::ActualDimensions,
        ).unwrap();
        assert_eq!(geometry.native_size_at_unit_scale(), [12, 4]);
        let quad = geometry.uncropped_local_quad_at_unit_scale();
        assert_eq!(quad[1][1].to_bits(), (13.0_f32 * (1.0_f32 / 3.0)).to_bits());
        assert!(quad[1][1] > 4.0);
        let write = NativeSpritePositionWrite::stage_static_icon(
            &[10, 20], &[100, 100], &geometry.native_size_at_unit_scale(),
        );
        assert_eq!(write.local_translation(), [-40.0, 26.0]);
        assert!(write.local_translation()[1] + quad[1][1] > 30.0);
    }

    #[test]
    fn horizontal_atlas_uses_native_uv_order_and_separate_frame_offset() {
        let geometry = ContainerSpriteUvGeometry::from_definition(
            &definition("noOfFrames = 4"),
            [64, 32],
            ContainerSpriteGeometryState::from_csprite_constructor(),
            SpriteTextureNormalization::ActualDimensions,
        )
        .unwrap();
        assert_eq!(
            geometry.vertex_uvs,
            [[0.0, 1.0], [0.0, 0.0], [0.25, 1.0], [0.25, 0.0]]
        );
        assert_eq!(geometry.frame_offset(2).unwrap(), [0.5, 0.0]);
        assert_eq!(geometry.frame_count(), 4);
    }

    #[test]
    fn vertical_layout_swaps_frame_axes_and_offsets_only_v() {
        let geometry = ContainerSpriteUvGeometry::from_definition(
            &definition("noOfFrames = 4 framedirectiony = yes"),
            [32, 64],
            ContainerSpriteGeometryState::from_csprite_constructor(),
            SpriteTextureNormalization::ActualDimensions,
        )
        .unwrap();
        assert_eq!(
            geometry.vertex_uvs,
            [[0.0, 0.25], [0.0, 0.0], [1.0, 0.25], [1.0, 0.0]]
        );
        assert_eq!(geometry.frame_offset(3).unwrap(), [0.0, 0.75]);
    }

    #[test]
    fn cropping_v_scale_and_u_flip_preserve_the_unflipped_frame_step() {
        let geometry = ContainerSpriteUvGeometry::from_definition(
            &definition("noOfFrames = 2"),
            [16, 8],
            ContainerSpriteGeometryState {
                crop_right_bottom: [4, 2],
                crop_left_top: [2, 1],
                texture_v_scale: 0.5,
                flip_u: true,
            },
            SpriteTextureNormalization::ActualDimensions,
        )
        .unwrap();
        // u0=.125, uFar=.75 => du=.3125, then flip both endpoints.
        // v0=.125, vFar=.75 => dv=.3125.
        assert_eq!(
            geometry.vertex_uvs,
            [
                [0.875, 0.4375],
                [0.875, 0.125],
                [0.5625, 0.4375],
                [0.5625, 0.125]
            ]
        );
        assert_eq!(geometry.frame_offset(1).unwrap(), [0.3125, 0.0]);
    }

    #[test]
    fn normalization_policy_is_consumed_for_non_power_of_two_textures() {
        let sprite = definition("noOfFrames = 3");
        let state = ContainerSpriteGeometryState::from_csprite_constructor();
        let actual = ContainerSpriteUvGeometry::from_definition(
            &sprite,
            [24, 12],
            state,
            SpriteTextureNormalization::from_native_force_pow2(false),
        )
        .unwrap();
        let pow2 = ContainerSpriteUvGeometry::from_definition(
            &sprite,
            [24, 12],
            state,
            SpriteTextureNormalization::from_native_force_pow2(true),
        )
        .unwrap();
        assert_eq!(
            pow2.vertex_uvs,
            [[0.0, 0.75], [0.0, 0.0], [0.25, 0.75], [0.25, 0.0]]
        );
        assert_eq!(actual.vertex_uvs[0], [0.0, 1.0]);
        assert_ne!(actual.frame_step, pow2.frame_step);
    }

    #[test]
    fn repeated_native_direction_fields_reset_the_local_bool_each_time() {
        let geometry = ContainerSpriteUvGeometry::from_definition(
            &definition("noOfFrames = 2 framedirectiony = yes framedirectiony = UNKNOWN"),
            [16, 8],
            ContainerSpriteGeometryState::from_csprite_constructor(),
            SpriteTextureNormalization::ActualDimensions,
        )
        .unwrap();
        assert_eq!(geometry.frame_step, [0.5, 0.0]);
    }

    #[test]
    fn raw_frame_assignments_cannot_silently_disagree_with_existing_definition() {
        let result = ContainerSpriteUvGeometry::from_definition(
            &definition("noOfFrames = 2 noOfFrames = 3"),
            [16, 8],
            ContainerSpriteGeometryState::from_csprite_constructor(),
            SpriteTextureNormalization::ActualDimensions,
        );
        assert_eq!(
            result,
            Err(SpriteGeometryRejection::FrameCountDisagreesWithDefinition)
        );
    }
}
