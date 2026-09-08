//! Reached ordinary static sprite CPU upload; rasterization stays on D3D.
//! Inputs are the actual setter/traversal products; a layout Rect is not one.
//! Ordered CPU instructions are pinned by
//! portable_engine_evidence::ordinary_sprite_root_projection; shader observations
//! and their generating script live in extractor/native/runs/arrow_native_ui_20260908/.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpriteProjectionRefusal {
    NonfiniteOperand,
    AlternateDrawMatrixBranch,
    RotatedSpriteType,
    NonfiniteUploadMatrix,
    NonNormalShaderArithmetic,
    InvalidNativeViewport,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NativeSpriteUploadMatrix {
    bits: [[u32; 4]; 4],
}

impl NativeSpriteUploadMatrix {
    pub(crate) fn ordinary_for_graphics(
        world: &[[f32; 4]; 4],
        graphics: &crate::container_sprite_clip::NativeClipGraphics,
        sprite_type_rotation: f32,
    ) -> Result<Self, SpriteProjectionRefusal> {
        let projection = graphics_projection(graphics.logical_size, graphics.physical_size)?;
        Self::ordinary(world, &projection, sprite_type_rotation)
    }

    /// Borrows the reached object's world matrix (not a position reconstructed
    /// from a draw rectangle) and the graphics object's current +190 matrix.
    /// The native CPU loop is 14222d7b0..14222d7e8. Ordinary Rust operations
    /// preserve its separate products and additions; do not use mul_add or
    /// substitute a vertex->world->projection sequence. The caller retains
    /// responsibility for the admitted floating-point execution policy.
    pub(crate) fn ordinary(
        world: &[[f32; 4]; 4],
        graphics_projection: &[[f32; 4]; 4],
        sprite_type_rotation: f32,
    ) -> Result<Self, SpriteProjectionRefusal> {
        if !sprite_type_rotation.is_finite()
            || world
                .iter()
                .flatten()
                .chain(graphics_projection.iter().flatten())
                .any(|v| !v.is_finite())
        {
            return Err(SpriteProjectionRefusal::NonfiniteOperand);
        }
        // UCOMISS/JP/JNZ at 14222d74d and 14222d76b. Both signs of zero
        // take the ordinary type-rotation arm; NaN does not.
        if world[3][3] != 1.0 {
            return Err(SpriteProjectionRefusal::AlternateDrawMatrixBranch);
        }
        if sprite_type_rotation != 0.0 {
            return Err(SpriteProjectionRefusal::RotatedSpriteType);
        }
        let result: [[f32; 4]; 4] = std::array::from_fn(|row| {
            std::array::from_fn(|column| {
                let x = world[row][0] * graphics_projection[0][column];
                let y = world[row][1] * graphics_projection[1][column];
                let xy = y + x;
                let z = world[row][2] * graphics_projection[2][column];
                let xyz = xy + z;
                let w = world[row][3] * graphics_projection[3][column];
                xyz + w
            })
        });
        if result.iter().flatten().any(|v| !v.is_finite()) {
            return Err(SpriteProjectionRefusal::NonfiniteUploadMatrix);
        }
        Ok(Self {
            bits: result.map(|row| row.map(f32::to_bits)),
        })
    }

    pub(crate) fn rows(self) -> [[f32; 4]; 4] {
        self.bits.map(|row| row.map(f32::from_bits))
    }
}

fn graphics_projection(
    logical: [i32; 2],
    physical: [i32; 2],
) -> Result<[[f32; 4]; 4], SpriteProjectionRefusal> {
    if logical.into_iter().chain(physical).any(|v| v <= 0) {
        return Err(SpriteProjectionRefusal::InvalidNativeViewport);
    }
    // 1422206c0 +190 construction. Subtraction is native signed32 before
    // conversion. These positive operands cannot overflow that subtraction.
    // Preserve each operation; do not simplify into physical/logical ratios.
    let dx = (physical[0] - logical[0]) as f32;
    let hx = logical[0] as f32 * 0.5;
    let left = dx * -0.5 - hx;
    let right = hx - dx * 0.5;
    let dy = (physical[1] - logical[1]) as f32 * 0.5;
    let hy = logical[1] as f32 * 0.5;
    let bottom = dy - hy;
    let top = hy + dy;
    let result = [
        [2.0 / (right - left), 0.0, 0.0, 0.0],
        [0.0, 2.0 / (top - bottom), 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [
            (right + left) / (left - right),
            (top + bottom) / (bottom - top),
            -0.0,
            1.0,
        ],
    ];
    if result.iter().flatten().any(|v| !normal_or_zero(*v)) {
        return Err(SpriteProjectionRefusal::NonNormalShaderArithmetic);
    }
    Ok(result)
}

fn normal_or_zero(value: f32) -> bool {
    value == 0.0 || value.is_normal()
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::*;

    // Test-only transcription of the exact MULPS/ADDPS row at
    // 14222d7b0..14222d7e8, with original memory operands relocated to the
    // borrowed arrays. This is an independent native-instruction oracle for
    // the CPU upload arithmetic, not a GPU/viewport oracle.
    fn native_row(row: &[f32; 4], projection: &[[f32; 4]; 4]) -> [f32; 4] {
        let mut output = [0.0_f32; 4];
        unsafe {
            core::arch::asm!(
                "movups xmm3, [{projection}]",
                "movups xmm4, [{projection} + 16]",
                "movups xmm5, [{projection} + 32]",
                "movups xmm6, [{projection} + 48]",
                "movss xmm0, [{row}]",
                "shufps xmm0, xmm0, 0",
                "mulps xmm0, xmm3",
                "movss xmm2, [{row} + 4]",
                "shufps xmm2, xmm2, 0",
                "mulps xmm2, xmm4",
                "addps xmm2, xmm0",
                "movss xmm0, [{row} + 8]",
                "shufps xmm0, xmm0, 0",
                "mulps xmm0, xmm5",
                "addps xmm2, xmm0",
                "movss xmm1, [{row} + 12]",
                "shufps xmm1, xmm1, 0",
                "mulps xmm1, xmm6",
                "addps xmm2, xmm1",
                "movups [{output}], xmm2",
                row = in(reg) row.as_ptr(),
                projection = in(reg) projection.as_ptr(),
                output = in(reg) output.as_mut_ptr(),
                out("xmm0") _, out("xmm1") _, out("xmm2") _,
                out("xmm3") _, out("xmm4") _, out("xmm5") _, out("xmm6") _,
                options(nostack, preserves_flags),
            );
        }
        output
    }

    fn identity() -> [[f32; 4]; 4] {
        std::array::from_fn(|r| std::array::from_fn(|c| if r == c { 1.0 } else { 0.0 }))
    }

    #[test]
    fn upload_matrix_matches_exact_native_row_instructions() {
        for seed in 0..128_u32 {
            let mut world = std::array::from_fn(|r| {
                std::array::from_fn(|c| {
                    let n = (seed.wrapping_mul(37) + r as u32 * 17 + c as u32 * 13) % 127;
                    (n as f32 - 63.0) / 7.0
                })
            });
            world[3][3] = 1.0;
            let projection = std::array::from_fn(|r| {
                std::array::from_fn(|c| {
                    let n = (seed.wrapping_mul(23) + r as u32 * 11 + c as u32 * 19) % 113;
                    (n as f32 - 56.0) / 13.0
                })
            });
            let expected = world.map(|row| native_row(&row, &projection).map(f32::to_bits));
            assert_eq!(
                NativeSpriteUploadMatrix::ordinary(&world, &projection, 0.0)
                    .unwrap()
                    .bits,
                expected,
                "seed {seed}"
            );
        }
    }

    #[test]
    fn upload_preserves_ordered_cancellation_and_signed_zero_rotation() {
        let mut world = identity();
        world[0] = [16_777_216.0, 1.0, -16_777_216.0, 1.0];
        let projection = [[1.0; 4]; 4];
        let result = NativeSpriteUploadMatrix::ordinary(&world, &projection, -0.0).unwrap();
        assert_eq!(
            result.bits[0],
            native_row(&world[0], &projection).map(f32::to_bits)
        );
        assert_eq!(result.rows()[0], [1.0; 4]);
        assert_ne!(
            result.rows()[0][0],
            (world[0][0] + world[0][2]) + (world[0][1] + world[0][3])
        );
    }

    #[test]
    fn alternate_native_branches_and_nonfinite_uploads_are_named() {
        let mut world = identity();
        world[3][3] = 2.0;
        assert_eq!(
            NativeSpriteUploadMatrix::ordinary(&world, &identity(), 0.0),
            Err(SpriteProjectionRefusal::AlternateDrawMatrixBranch)
        );
        world[3][3] = 1.0;
        assert_eq!(
            NativeSpriteUploadMatrix::ordinary(&world, &identity(), 1.0),
            Err(SpriteProjectionRefusal::RotatedSpriteType)
        );
        assert_eq!(
            NativeSpriteUploadMatrix::ordinary(&world, &identity(), f32::NAN),
            Err(SpriteProjectionRefusal::NonfiniteOperand)
        );
        world[0][0] = f32::MAX;
        let mut projection = identity();
        projection[0][0] = 2.0;
        assert_eq!(
            NativeSpriteUploadMatrix::ordinary(&world, &projection, 0.0),
            Err(SpriteProjectionRefusal::NonfiniteUploadMatrix)
        );
    }

    #[test]
    fn projection_retains_distinct_logical_and_physical_operands() {
        // This exact uploaded matrix also participates in oracle-02 case3.
        let expected = [
            [0x3acccccd, 0, 0, 0],
            [0, 0x3b360b61, 0, 0],
            [0, 0, 0x3f800000, 0],
            [0x3f000000, 0xbf000000, 0x80000000, 0x3f800000],
        ];
        assert_eq!(
            graphics_projection([1280, 720], [1920, 1080])
                .unwrap()
                .map(|r| r.map(f32::to_bits)),
            expected
        );
        assert_ne!(
            graphics_projection([1280, 720], [1920, 1080]),
            graphics_projection([1920, 1080], [1920, 1080])
        );
        assert_eq!(
            graphics_projection([0, 720], [1920, 1080]),
            Err(SpriteProjectionRefusal::InvalidNativeViewport)
        );
    }
}
