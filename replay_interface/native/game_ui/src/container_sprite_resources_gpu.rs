//! The admitted ordinary DDS resource's DX11 upload. The retained allocation
//! supplies bytes and first-load options; the caller supplies reached slots.
//! These mechanical upload checks do not corroborate native cache/lifetime/order;
//! see extractor/native/runs/arrow_native_ui_20260908/CANDIDATE_RESOURCE_LIFECYCLES.json.

use super::RegisteredOrdinaryTexture;
use crate::container_sprite_geometry::{
    OrdinarySingleMipTextureSource, OrdinaryTextureSourceRejection,
};
use portable_render_stream::{
    BufferSource, ShaderResourceViewDesc, ShaderStage, TextureBinding, TextureResourceDesc,
    TextureResourceDimension, TextureSubresourcePayload,
};
use snapshot::sha256::Sha256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OrdinaryTextureGpuRejection {
    InvalidBinding,
    ContentIdentityMismatch,
    Source(OrdinaryTextureSourceRejection),
    UnsupportedFormat,
    LayoutOverflow,
}

impl RegisteredOrdinaryTexture {
    /// Build the resource supplied by 1423fd6d0 -> 1423ffc90 -> 1423fb4f0.
    /// This does not register/load another native resource or change its cache
    /// entry. The same registered content and retained bytes travel together.
    pub(crate) fn native_texture_binding(
        &self,
        stage: ShaderStage,
        slot: u8,
        semantic: &str,
    ) -> Result<TextureBinding, OrdinaryTextureGpuRejection> {
        use OrdinaryTextureGpuRejection as Reject;
        if !matches!(stage, ShaderStage::Vertex | ShaderStage::Pixel)
            || slot >= 128
            || semantic.trim().is_empty()
        {
            return Err(Reject::InvalidBinding);
        }
        if self.content.validate().is_err()
            || self.encoded.len() as u64 != self.content.payload_bytes
            || Sha256::hex(&self.encoded) != self.content.payload_sha256
        {
            return Err(Reject::ContentIdentityMismatch);
        }
        // Reuse the owning DDS admission and parser, including original
        // registration options, rather than deriving topology from RgbaImage.
        let source = OrdinarySingleMipTextureSource::admit(
            &self.encoded,
            Some(&self.effective_options),
            Some(self.force_pow2_at_load),
        )
        .map_err(Reject::Source)?;
        let metadata = crate::dds::info_bytes(source.encoded())
            .map_err(|_| Reject::Source(OrdinaryTextureSourceRejection::InvalidDds))?;
        let [width, height] = source.dimensions();
        let (format, row_pitch, rows, source_row_pitch) = match metadata.format.as_str() {
            // 142410030: native13. 142411320: DXGI87. No swizzle or alpha repair.
            "A8R8G8B8" => {
                let pitch = width.checked_mul(4).ok_or(Reject::LayoutOverflow)?;
                (87, pitch, height, pitch)
            }
            // native11 -> DXGI88. 1423fc160/1423f8e90 expand B,G,R to B,G,R,0;
            // DXGI B8G8R8X8 supplies opaque sampled alpha, not an uploaded 255.
            "R8G8B8" => (
                88,
                width.checked_mul(4).ok_or(Reject::LayoutOverflow)?,
                height,
                width.checked_mul(3).ok_or(Reject::LayoutOverflow)?,
            ),
            // Native1/3/5 retain compressed blocks. Admission already refuses
            // non-block-aligned extents; 1423fc160 computes block-row pitches.
            "DXT1" | "DXT3" | "DXT5" => {
                let (format, block_bytes) = match metadata.format.as_str() {
                    "DXT1" => (71, 8),
                    "DXT3" => (74, 16),
                    _ => (77, 16),
                };
                let pitch = (width / 4)
                    .checked_mul(block_bytes)
                    .ok_or(Reject::LayoutOverflow)?;
                (format, pitch, height / 4, pitch)
            }
            _ => return Err(Reject::UnsupportedFormat),
        };
        let source_bytes = source_row_pitch
            .checked_mul(rows)
            .ok_or(Reject::LayoutOverflow)? as usize;
        let payload = source
            .encoded()
            .get(
                128..128usize
                    .checked_add(source_bytes)
                    .ok_or(Reject::LayoutOverflow)?,
            )
            .ok_or(Reject::Source(OrdinaryTextureSourceRejection::TruncatedMip))?;
        let slice_pitch = row_pitch.checked_mul(rows).ok_or(Reject::LayoutOverflow)?;
        let bytes = if format == 88 {
            let mut expanded = Vec::with_capacity(slice_pitch as usize);
            for bgr in payload.chunks_exact(3) {
                expanded.extend_from_slice(&[bgr[0], bgr[1], bgr[2], 0]);
            }
            expanded
        } else {
            payload.to_vec()
        };
        let payload_sha256 = Sha256::hex(&bytes);
        Ok(TextureBinding {
            stage,
            slot,
            semantic_token: semantic.into(),
            content: self.content.clone(),
            resource: TextureResourceDesc {
                dimension: TextureResourceDimension::Texture2d,
                width,
                height,
                depth: 1,
                mip_levels: 1,
                array_size: 1,
                dxgi_format_u32: format,
                sample_count: 1,
                sample_quality: 0,
                // Fresh file descriptor 1409da100 zeros +14/+16. The reached
                // 1423ffc90 call supplies shader-resource=1, RT=0, depth=0,
                // samples=0, selecting immutable/SRV-only/no CPU or misc flags.
                usage_u32: 1,
                bind_flags_u32: 8,
                cpu_access_flags_u32: 0,
                misc_flags_u32: 0,
                ordered_subresources: vec![TextureSubresourcePayload {
                    subresource_index: 0,
                    mip_level: 0,
                    array_slice: 0,
                    row_pitch,
                    slice_pitch,
                    source: BufferSource::Inline {
                        bytes,
                        sha256: payload_sha256,
                    },
                }],
            },
            // 1423ffc90 sets Texture2D, MostDetailedMip=0, actual loaded count=1.
            view: ShaderResourceViewDesc::Texture2d {
                dxgi_format_u32: format,
                most_detailed_mip: 0,
                mip_levels: 1,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container_sprite_geometry::OrdinarySpriteTextureLoadOptions;
    use crate::container_sprite_resources::{
        OrdinarySpriteTextureRegistry, OrdinaryTextureRequest, OrdinaryTextureRole,
    };
    use portable_render_stream::UsedContentKey;
    use std::sync::Arc;

    fn dds(width: u32, height: u32, kind: &str, payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0; 128];
        bytes[..4].copy_from_slice(b"DDS ");
        let mut put = |offset, value: u32| {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        };
        for (offset, value) in [(4, 124), (12, height), (16, width), (28, 1), (76, 32)] {
            put(offset, value);
        }
        match kind {
            "A8R8G8B8" | "R8G8B8" => {
                put(80, if kind == "A8R8G8B8" { 0x41 } else { 0x40 });
                put(88, if kind == "A8R8G8B8" { 32 } else { 24 });
                put(92, 0xff0000);
                put(96, 0xff00);
                put(100, 0xff);
                put(104, if kind == "A8R8G8B8" { 0xff000000 } else { 0 });
            }
            _ => {
                put(80, 4);
                bytes[84..88].copy_from_slice(kind.as_bytes());
            }
        }
        bytes.extend_from_slice(payload);
        bytes
    }

    fn content(bytes: &[u8]) -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: "a".repeat(64),
            stored_game_relative_path: "gfx/native.dds".into(),
            normalized_game_relative_path: "gfx/native.dds".into(),
            winning_layer_ordinal: 0,
            layer_kind: replay_core::ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: Sha256::hex(bytes),
            payload_sha256: Sha256::hex(bytes),
            payload_bytes: bytes.len() as u64,
        }
    }

    fn registered(bytes: Vec<u8>) -> Arc<RegisteredOrdinaryTexture> {
        let content = content(&bytes);
        // Execute the owning post-lease registration, identity and source
        // admission path. This fixture does not claim filesystem admission.
        OrdinarySpriteTextureRegistry::new_viewer_session()
            .register_admitted_bytes(
                OrdinaryTextureRequest {
                    role: OrdinaryTextureRole::Base,
                    native_path: "gfx/native.dds",
                    content: &content,
                },
                bytes.into(),
                OrdinarySpriteTextureLoadOptions::from_first_ordinary_registration(),
                Some(false),
            )
            .unwrap()
    }

    fn uploaded(binding: &TextureBinding) -> &[u8] {
        let BufferSource::Inline { bytes, sha256 } =
            &binding.resource.ordered_subresources[0].source
        else {
            panic!("the retained allocation supplies immutable payload bytes")
        };
        assert_eq!(&Sha256::hex(bytes), sha256);
        bytes
    }

    #[test]
    fn argb_upload_keeps_encoded_bgra_alpha_and_source_identity() {
        let payload = [3, 5, 7, 11, 13, 17, 19, 0];
        let mut bytes = dds(2, 1, "A8R8G8B8", &payload);
        bytes.extend_from_slice(&[91, 92]); // Source identity covers unused trailing bytes too.
        let texture = registered(bytes.clone());
        assert_eq!(&texture.image().pixels[..4], &[7, 5, 3, 11]);
        let binding = texture
            .native_texture_binding(ShaderStage::Pixel, 3, "mask")
            .unwrap();
        assert_eq!(binding.content, content(&bytes));
        assert_eq!(
            (binding.stage, binding.slot, binding.semantic_token.as_str()),
            (ShaderStage::Pixel, 3, "mask")
        );
        assert_eq!(uploaded(&binding), payload);
        assert_eq!((binding.resource.width, binding.resource.height), (2, 1));
        assert_eq!(binding.resource.dxgi_format_u32, 87);
        assert_eq!(
            (
                binding.resource.usage_u32,
                binding.resource.bind_flags_u32,
                binding.resource.cpu_access_flags_u32,
                binding.resource.misc_flags_u32
            ),
            (1, 8, 0, 0)
        );
        assert_eq!(
            (
                binding.resource.mip_levels,
                binding.resource.array_size,
                binding.resource.depth,
                binding.resource.sample_count,
                binding.resource.sample_quality
            ),
            (1, 1, 1, 1, 0)
        );
        let sub = &binding.resource.ordered_subresources[0];
        assert_eq!(
            (
                sub.subresource_index,
                sub.mip_level,
                sub.array_slice,
                sub.row_pitch,
                sub.slice_pitch
            ),
            (0, 0, 0, 8, 8)
        );
        assert_eq!(
            binding.view,
            ShaderResourceViewDesc::Texture2d {
                dxgi_format_u32: 87,
                most_detailed_mip: 0,
                mip_levels: 1,
            }
        );
    }

    #[test]
    fn bgr24_upload_expands_zero_x_without_decoded_rgba_or_opaque_byte_repair() {
        let payload = [1, 3, 5, 7, 9, 11, 13, 15, 17, 19, 21, 23];
        let texture = registered(dds(2, 2, "R8G8B8", &payload));
        assert_eq!(&texture.image().pixels[..4], &[5, 3, 1, 255]);
        let binding = texture
            .native_texture_binding(ShaderStage::Pixel, 0, "base")
            .unwrap();
        assert_eq!(binding.resource.dxgi_format_u32, 88);
        assert_eq!(
            uploaded(&binding),
            &[1, 3, 5, 0, 7, 9, 11, 0, 13, 15, 17, 0, 19, 21, 23, 0]
        );
        assert_eq!(
            (
                binding.resource.ordered_subresources[0].row_pitch,
                binding.resource.ordered_subresources[0].slice_pitch
            ),
            (8, 16)
        );
    }

    #[test]
    fn all_three_compressed_formats_keep_native_blocks_and_block_row_pitches() {
        for (kind, format, bytes_per_block) in [("DXT1", 71, 8), ("DXT3", 74, 16), ("DXT5", 77, 16)]
        {
            let payload = (0..4 * bytes_per_block)
                .map(|n| (n * 7 + 3) as u8)
                .collect::<Vec<_>>();
            let texture = registered(dds(8, 8, kind, &payload));
            let binding = texture
                .native_texture_binding(ShaderStage::Pixel, 1, "animation")
                .unwrap();
            assert_eq!(binding.resource.dxgi_format_u32, format);
            assert_eq!(uploaded(&binding), payload);
            assert_eq!(
                (
                    binding.resource.ordered_subresources[0].row_pitch,
                    binding.resource.ordered_subresources[0].slice_pitch
                ),
                (2 * bytes_per_block, 4 * bytes_per_block)
            );
        }
    }

    #[test]
    fn gpu_payload_never_uses_mutated_decoded_pixels_or_forges_retained_content() {
        let texture = registered(dds(1, 1, "A8R8G8B8", &[1, 2, 3, 4]));
        let mut changed = (*texture).clone();
        Arc::make_mut(&mut changed.image).pixels.fill(99);
        assert_eq!(
            uploaded(
                &changed
                    .native_texture_binding(ShaderStage::Pixel, 0, "base")
                    .unwrap()
            ),
            &[1, 2, 3, 4]
        );
        Arc::make_mut(&mut changed.encoded)[128] ^= 1;
        assert_eq!(
            changed
                .native_texture_binding(ShaderStage::Pixel, 0, "base")
                .unwrap_err(),
            OrdinaryTextureGpuRejection::ContentIdentityMismatch
        );
    }

    #[test]
    fn upload_rechecks_retained_load_options_and_binding_bounds() {
        let texture = registered(dds(1, 1, "A8R8G8B8", &[1, 2, 3, 4]));
        let mut changed = (*texture).clone();
        changed.effective_options.srgb = true;
        assert_eq!(
            changed
                .native_texture_binding(ShaderStage::Pixel, 0, "base")
                .unwrap_err(),
            OrdinaryTextureGpuRejection::Source(OrdinaryTextureSourceRejection::UnsupportedSrgb)
        );
        for (stage, slot, name) in [
            (ShaderStage::Pixel, 128, "base"),
            (ShaderStage::Pixel, 0, " "),
            (ShaderStage::Compute, 0, "base"),
        ] {
            assert_eq!(
                texture
                    .native_texture_binding(stage, slot, name)
                    .unwrap_err(),
                OrdinaryTextureGpuRejection::InvalidBinding
            );
        }
    }
}
