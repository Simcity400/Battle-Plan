#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeCollisionRefusal {
    BoundsProviderRequired,
    IncompleteTriangle { index_offset: usize },
    VertexIndexOutOfRange { index_offset: usize, vertex_index: u16 },
}

/// Operands after the native object-space transform and direction normalization.
/// Candidate identity, publication custody and bounds fallback belong to the caller.
pub fn first_collision_triangle_hit(
    local_origin: [f32; 3],
    normalized_local_direction: [f32; 3],
    scaled_positions: &[[f32; 3]],
    indices: &[u16],
) -> Result<Option<usize>, NativeCollisionRefusal> {
    if indices.is_empty() {
        return Err(NativeCollisionRefusal::BoundsProviderRequired);
    }
    for (triangle_ordinal, triangle) in indices.chunks(3).enumerate() {
        let index_offset = triangle_ordinal * 3;
        if triangle.len() != 3 {
            return Err(NativeCollisionRefusal::IncompleteTriangle { index_offset });
        }
        let mut vertices = [[0.0; 3]; 3];
        for (corner, &vertex_index) in triangle.iter().enumerate() {
            vertices[corner] = *scaled_positions.get(usize::from(vertex_index)).ok_or(
                NativeCollisionRefusal::VertexIndexOutOfRange {
                    index_offset: index_offset + corner,
                    vertex_index,
                },
            )?;
        }
        if triangle_hit(local_origin, normalized_local_direction, vertices) {
            return Ok(Some(triangle_ordinal));
        }
    }
    Ok(None)
}

fn triangle_hit(origin: [f32; 3], direction: [f32; 3], [a, b, c]: [[f32; 3]; 3]) -> bool {
    let edge1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let edge2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let cross = [
        edge2[2] * direction[1] - edge2[1] * direction[2],
        edge2[0] * direction[2] - edge2[2] * direction[0],
        edge2[1] * direction[0] - edge2[0] * direction[1],
    ];
    let determinant = (edge1[1] * cross[1] + edge1[0] * cross[0]) + edge1[2] * cross[2];
    if determinant > f32::from_bits(0xb8d1b717)
        && f32::from_bits(0x38d1b717) > determinant
    {
        return false;
    }
    let inverse_determinant = 1.0 / determinant;
    let offset = [origin[0] - a[0], origin[1] - a[1], origin[2] - a[2]];
    let u = ((offset[1] * cross[1] + offset[0] * cross[0]) + offset[2] * cross[2])
        * inverse_determinant;
    if 0.0 > u || u > 1.0 {
        return false;
    }
    let v = (((edge1[2] * offset[1] - edge1[1] * offset[2]) * direction[0]
        + (edge1[0] * offset[2] - edge1[2] * offset[0]) * direction[1])
        + (edge1[1] * offset[0] - edge1[0] * offset[1]) * direction[2])
        * inverse_determinant;
    if 0.0 > v {
        return false;
    }
    // The native branch tests no forward distance and accepts an unordered final sum.
    !(v + u > 1.0)
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::*;

    const TRIANGLE: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];

    fn native_triangle(origin: [f32; 3], direction: [f32; 3], vertices: [[f32; 3]; 3]) -> bool {
        let indices = [0u16, 1, 2];
        let constants = [f32::from_bits(0xb8d1b717), f32::from_bits(0x38d1b717), 1.0];
        let result: u32;
        // Execute the verified native triangle instructions, with only operand bindings and
        // branch labels relocated. No call, stack access or store occurs in this block.
        unsafe {
            std::arch::asm!(
                include_str!("native_model_selection_oracle.asm"),
                in("rdx") vertices.as_ptr(),
                inlateout("r9") indices.as_ptr() => _,
                in("rdi") origin.as_ptr(),
                in("rsi") direction.as_ptr(),
                in("r11") constants.as_ptr(),
                lateout("eax") result,
                lateout("xmm6") _, lateout("xmm7") _, lateout("xmm8") _,
                lateout("xmm9") _, lateout("xmm10") _, lateout("xmm11") _,
                lateout("xmm12") _, lateout("xmm13") _, lateout("xmm14") _,
                lateout("xmm15") _,
                clobber_abi("C"),
                options(nostack, readonly),
            );
        }
        result != 0
    }

    fn agrees(origin: [f32; 3], direction: [f32; 3], vertices: [[f32; 3]; 3]) -> bool {
        let native = native_triangle(origin, direction, vertices);
        assert_eq!(triangle_hit(origin, direction, vertices), native,
            "origin={:?}, direction={:?}, vertices={:?}",
            origin.map(f32::to_bits), direction.map(f32::to_bits), vertices.map(|v| v.map(f32::to_bits)));
        native
    }

    #[test]
    fn executing_oracle_is_the_pinned_native_instruction_block_with_only_binding_relocations() {
        let original = portable_engine_evidence::native_model_ray::EXPORTS
            .iter().find(|row| row.0.ends_with("instructions.txt")).unwrap().1;
        let mut instructions: Vec<String> = [
            "MOVSS XMM5, dword ptr [RSI]", "MOVSS XMM4, dword ptr [RSI + 0x4]",
            "MOVSS XMM15, dword ptr [RSI + 0x8]", "MOVSS XMM11, dword ptr [R11 + 0x8]",
            "XORPS XMM14, XMM14", "ADD R9, 0x4",
        ].into_iter().map(str::to_owned).collect();
        let mut native_count = 0;
        for line in original.lines() {
            let mut columns = line.split_whitespace();
            let Some(address) = columns.next().and_then(|s| u64::from_str_radix(s, 16).ok()) else { continue };
            if !(0x142262aa6..=0x142262c53).contains(&address) { continue; }
            columns.next().unwrap();
            if address == 0x142262b91 { instructions.push("2:".to_owned()); }
            let mut instruction = columns.collect::<Vec<_>>().join(" ");
            for (from, to) in [
                ("[0x142b29ea0]", "[R11]"), ("[0x142934a78]", "[R11 + 0x4]"),
                ("[RSP + 0x30]", "[RDI + 0x4]"), ("[RBP + 0x60]", "[RDI + 0x8]"),
                ("[RBP + 0x78]", "[RDI]"), ("[RSP + 0x38]", "[RSI]"),
                ("[RSP + 0x3c]", "[RSI + 0x4]"), ("0x142262b91", "2f"),
                ("0x142262c5f", "3f"), ("0x142262c59", "3f"), ("0x142262cdd", "4f"),
            ] { instruction = instruction.replace(from, to); }
            instructions.push(instruction);
            native_count += 1;
        }
        assert!(native_count > 80);
        instructions.extend(["3:", "XOR EAX, EAX", "JMP 5f", "4:", "MOV EAX, 1", "5:"]
            .into_iter().map(str::to_owned));
        assert_eq!(include_str!("native_model_selection_oracle.asm").lines().collect::<Vec<_>>(), instructions);
    }

    #[test]
    fn native_triangle_matches_raw_instruction_outputs_across_finite_operands() {
        let mut state = 0x27e0_aa6u32;
        let mut value = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            f32::from_bits(state & 0xff7fffff)
        };
        for _ in 0..256 {
            let vertices = std::array::from_fn(|_| std::array::from_fn(|_| value()));
            let origin = std::array::from_fn(|_| value());
            for direction in [[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0],
                [0.0, -1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, -1.0]] {
                agrees(origin, direction, vertices);
            }
        }
    }

    #[test]
    fn native_triangle_accepts_both_windings_and_behind_origin_without_a_distance_test() {
        for vertices in [TRIANGLE, [TRIANGLE[0], TRIANGLE[2], TRIANGLE[1]]] {
            for z in [-1.0, 0.0, 1.0] {
                for dz in [-1.0, 1.0] {
                    assert!(agrees([0.25, 0.25, z], [0.0, 0.0, dz], vertices));
                    assert!(!agrees([1.0, 1.0, z], [0.0, 0.0, dz], vertices));
                }
            }
        }
    }

    #[test]
    fn native_triangle_keeps_barycentric_edges_and_signed_zero() {
        for zero in [0.0, -0.0] {
            for direction_zero in [0.0, -0.0] {
                for xy in [[zero, zero], [1.0, zero], [zero, 1.0], [0.5, 0.5]] {
                    assert!(agrees([xy[0], xy[1], 1.0], [direction_zero, direction_zero, -1.0], TRIANGLE));
                }
            }
        }
        assert!(!agrees([-0.0001, 0.0, 1.0], [0.0, 0.0, -1.0], TRIANGLE));
        assert!(!agrees([0.5, f32::from_bits(0x3f000002), 1.0], [0.0, 0.0, -1.0], TRIANGLE));
    }

    #[test]
    fn native_triangle_epsilon_cutoff_is_open_at_both_exact_bit_boundaries() {
        for bits in [0x38d1b716, 0x38d1b717, 0x38d1b718] {
            let width = f32::from_bits(bits);
            for signed_width in [width, -width] {
                let vertices = [TRIANGLE[0], [signed_width, 0.0, 0.0], TRIANGLE[2]];
                assert_eq!(agrees([0.0, 0.0, 1.0], [0.0, 0.0, -1.0], vertices), bits >= 0x38d1b717);
            }
        }
    }

    #[test]
    fn native_comiss_preserves_unordered_acceptance_and_infinite_operand_branches() {
        for nan in [f32::from_bits(0x7fc00001), f32::from_bits(0xffc00001), f32::from_bits(0x7f800001)] {
            assert!(agrees([nan, 0.25, 1.0], [0.0, 0.0, -1.0], TRIANGLE));
            assert!(agrees([0.25, 0.25, 1.0], [nan, 0.0, -1.0], TRIANGLE));
        }
        for infinity in [f32::INFINITY, f32::NEG_INFINITY] {
            agrees([infinity, 0.25, 1.0], [0.0, 0.0, -1.0], TRIANGLE);
            agrees([0.25, 0.25, infinity], [0.0, 0.0, -1.0], TRIANGLE);
        }
    }

    #[test]
    fn native_dot_addition_grouping_survives_cancellation() {
        let n = f32::from_bits(0x3f3504f3);
        let vertices = [[0.0; 3], [1.0; 3], [0.0, -1.0, 16777216.0]];
        assert!(agrees([0.0; 3], [n, n, 0.0], vertices));
        let large = 16777216.0 * n;
        assert_ne!(((-large + large) + n).to_bits(), (large + (-large + n)).to_bits());
    }

    #[test]
    fn triangle_traversal_uses_native_first_hit_order_not_nearest_distance() {
        let positions = [TRIANGLE[0], TRIANGLE[1], TRIANGLE[2],
            [0.0, 0.0, 9.0], [1.0, 0.0, 9.0], [0.0, 1.0, 9.0]];
        let origin = [0.25, 0.25, 10.0];
        let direction = [0.0, 0.0, -1.0];
        for indices in [[0, 1, 2, 3, 4, 5], [3, 4, 5, 0, 1, 2]] {
            let native = indices.chunks_exact(3).position(|tri| native_triangle(origin, direction,
                [positions[tri[0] as usize], positions[tri[1] as usize], positions[tri[2] as usize]]));
            assert_eq!(first_collision_triangle_hit(origin, direction, &positions, &indices), Ok(native));
            assert_eq!(native, Some(0));
        }
    }

    #[test]
    fn malformed_accesses_are_named_and_bounds_fallback_is_not_an_empty_miss() {
        let origin = [0.25, 0.25, 1.0];
        let direction = [0.0, 0.0, -1.0];
        assert_eq!(first_collision_triangle_hit(origin, direction, &TRIANGLE, &[]),
            Err(NativeCollisionRefusal::BoundsProviderRequired));
        assert_eq!(first_collision_triangle_hit(origin, direction, &TRIANGLE, &[0, 1]),
            Err(NativeCollisionRefusal::IncompleteTriangle { index_offset: 0 }));
        assert_eq!(first_collision_triangle_hit(origin, direction, &TRIANGLE, &[0, 1, u16::MAX]),
            Err(NativeCollisionRefusal::VertexIndexOutOfRange { index_offset: 2, vertex_index: u16::MAX }));
        assert!(native_triangle(origin, direction, TRIANGLE));
        assert_eq!(first_collision_triangle_hit(origin, direction, &TRIANGLE, &[0, 1, 2, u16::MAX]), Ok(Some(0)));
        assert_eq!(first_collision_triangle_hit([2.0, 2.0, 1.0], direction, &TRIANGLE, &[0, 1, 2, 0]),
            Err(NativeCollisionRefusal::IncompleteTriangle { index_offset: 3 }));
        assert_eq!(first_collision_triangle_hit([2.0, 2.0, 1.0], direction, &TRIANGLE, &[0, 1, 2, 0, 1, 9]),
            Err(NativeCollisionRefusal::VertexIndexOutOfRange { index_offset: 5, vertex_index: 9 }));
    }
}
