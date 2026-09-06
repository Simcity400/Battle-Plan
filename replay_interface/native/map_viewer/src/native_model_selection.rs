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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeModelLocalRay {
    pub origin: [f32; 3],
    pub direction: [f32; 3],
}

pub fn native_model_local_ray(
    matrix: [f32; 16],
    world_origin: [f32; 3],
    world_direction: [f32; 3],
) -> NativeModelLocalRay {
    let lengths = std::array::from_fn::<_, 3, _>(|axis| {
        let base = axis * 4;
        ((matrix[base] * matrix[base] + matrix[base + 1] * matrix[base + 1])
            + matrix[base + 2] * matrix[base + 2]).sqrt()
    });
    let mut rotation = [
        matrix[0], matrix[4], matrix[8],
        matrix[1], matrix[5], matrix[9],
        matrix[2], matrix[6], matrix[10],
    ];
    let mut reciprocal = [0.0; 3];
    for axis in 0..3 {
        if lengths[axis] != 0.0 {
            reciprocal[axis] = 1.0 / lengths[axis];
            for component in &mut rotation[axis * 3..axis * 3 + 3] {
                *component = reciprocal[axis] * *component;
            }
        }
    }
    rotation.swap(1, 3);
    rotation.swap(2, 6);
    rotation.swap(5, 7);
    let offset = std::array::from_fn::<_, 3, _>(|axis|
        f32::from_bits(matrix[12 + axis].to_bits() ^ 0x80000000) + world_origin[axis]);
    let origin = std::array::from_fn(|axis|
        ((rotation[3 + axis] * offset[1] + rotation[axis] * offset[0])
            + rotation[6 + axis] * offset[2]) * reciprocal[axis]);
    let direction = std::array::from_fn(|axis|
        ((rotation[axis] * world_direction[0] + rotation[3 + axis] * world_direction[1])
            + rotation[6 + axis] * world_direction[2]) * reciprocal[axis]);
    NativeModelLocalRay {
        origin,
        direction: crate::particle_runtime::native_normalize(direction, true),
    }
}

fn sse_min(first: f32, second: f32) -> f32 {
    if first < second { first } else { second }
}

fn sse_max(first: f32, second: f32) -> f32 {
    if first > second { first } else { second }
}

/// The caller supplies the witnessed static or animated local bounds for this object.
pub fn native_world_bounds(matrix: [f32; 16], local_bounds: [[f32; 3]; 2]) -> [[f32; 3]; 2] {
    let squared = std::array::from_fn::<_, 3, _>(|axis| {
        let base = axis * 4;
        (matrix[base] * matrix[base] + matrix[base + 1] * matrix[base + 1])
            + matrix[base + 2] * matrix[base + 2]
    });
    let scale = sse_max(squared[0], sse_max(squared[1], squared[2])).sqrt();
    local_bounds.map(|bound| std::array::from_fn(|axis| scale * bound[axis] + matrix[12 + axis]))
}

pub fn world_bounds_intersection(
    world_origin: [f32; 3],
    world_direction: [f32; 3],
    world_bounds: [[f32; 3]; 2],
) -> Option<f32> {
    let reciprocal = world_direction.map(|component| 1.0 / component);
    let interval = |axis: usize| {
        let near = usize::from(0.0 > reciprocal[axis]);
        ((world_bounds[near][axis] - world_origin[axis]) * reciprocal[axis],
            (world_bounds[1 - near][axis] - world_origin[axis]) * reciprocal[axis])
    };
    let (x_near, x_far) = interval(0);
    let (y_near, y_far) = interval(1);
    if x_near > y_far || y_near > x_far { return None; }
    let xy_far = sse_min(y_far, x_far);
    let xy_near = sse_max(y_near, x_near);
    let (z_near, z_far) = interval(2);
    if xy_near > z_far || z_near > xy_far { return None; }
    let near = sse_max(z_near, xy_near);
    let far = sse_min(z_far, xy_far);
    // JBE rejects unordered comparisons here, unlike the triangle branch's final JBE hit.
    if !(f32::MAX > near) || !(far > 0.0) { return None; }
    Some(near)
}

/// Geometry operands have no candidate, object-identity or publication authority on their own.
pub fn model_geometry_hit(
    matrix: [f32; 16],
    world_origin: [f32; 3],
    world_direction: [f32; 3],
    scaled_positions: &[[f32; 3]],
    indices: &[u16],
    witnessed_local_bounds: Option<[[f32; 3]; 2]>,
) -> Result<bool, NativeCollisionRefusal> {
    if indices.is_empty() {
        let bounds = witnessed_local_bounds.ok_or(NativeCollisionRefusal::BoundsProviderRequired)?;
        return Ok(world_bounds_intersection(world_origin, world_direction,
            native_world_bounds(matrix, bounds)).is_some());
    }
    let ray = native_model_local_ray(matrix, world_origin, world_direction);
    first_collision_triangle_hit(ray.origin, ray.direction, scaled_positions, indices)
        .map(|hit| hit.is_some())
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


#[cfg(all(test, target_arch = "x86_64"))]
mod geometry_tests {
    use super::*;

    std::arch::global_asm!(include_str!("native_model_geometry_oracle.asm"));

    extern "win64" {
        fn arroword_oracle_1424e6f10(vector: *const [f32; 3], output: *mut [f32; 3]) -> *mut [f32; 3];
        fn arroword_oracle_142261a90(matrix: *const [f32; 16], negative_translation: *mut [f32; 3],
            rotation: *mut [f32; 9], reciprocal: *mut [f32; 3], lengths: *mut [f32; 3]);
        fn arroword_oracle_142261df0(object: *const u8, bounds: *mut [[f32; 3]; 2], force_static: u8) -> u8;
        fn arroword_oracle_1422627e0(object: *const u8, origin: *const [f32; 3], direction: *const [f32; 3]) -> u8;
        fn arroword_oracle_1424e6270(origin: *const [f32; 3], direction: *const [f32; 3],
            lower: f32, upper: f32, min: *const [f32; 3], max: *const [f32; 3], near: *mut f32) -> u8;
    }

    const IDENTITY: [f32; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

    fn matrices() -> Vec<[f32; 16]> {
        let mut scaled = IDENTITY;
        scaled[0] = -2.0; scaled[5] = 3.0; scaled[10] = 4.0;
        scaled[12] = 2.0; scaled[13] = -3.0; scaled[14] = 1.0;
        let mut rotated = IDENTITY;
        rotated[0] = 0.0; rotated[1] = 1.0; rotated[4] = -1.0; rotated[5] = 0.0;
        let mut shear = scaled;
        shear[1] = 0.5; shear[6] = -0.25; shear[8] = 1.0;
        let mut tiny = IDENTITY;
        tiny[0] = f32::from_bits(1); tiny[5] = -0.0;
        vec![IDENTITY, scaled, rotated, shear, tiny, [0.0; 16]]
    }

    // These fixtures expose only the object/mesh fields read by the pinned native routines.
    // The animation pointer stays null and the collision entry requires a complete valid mesh.
    fn with_object<T>(matrix: [f32; 16], local_bounds: [[f32; 3]; 2], positions: &[[f32; 3]],
        indices: &[u16], callback: impl FnOnce(*const u8) -> T) -> T {
        let mut mesh = [0u8; 0x130];
        for (destination, value) in mesh[0xa0..0xb8].chunks_exact_mut(4).zip(local_bounds.into_iter().flatten()) {
            destination.copy_from_slice(&value.to_ne_bytes());
        }
        mesh[0x108..0x110].copy_from_slice(&(positions.as_ptr() as usize).to_ne_bytes());
        mesh[0x120..0x128].copy_from_slice(&(indices.as_ptr() as usize).to_ne_bytes());
        mesh[0x12c..0x130].copy_from_slice(&i32::try_from(indices.len()).unwrap().to_ne_bytes());
        let mut object = [0u8; 0x78];
        for (destination, value) in object[0x10..0x50].chunks_exact_mut(4).zip(matrix) {
            destination.copy_from_slice(&value.to_ne_bytes());
        }
        object[0x58..0x60].copy_from_slice(&(mesh.as_ptr() as usize).to_ne_bytes());
        callback(object.as_ptr())
    }

    fn native_bounds(matrix: [f32; 16], bounds: [[f32; 3]; 2]) -> [[f32; 3]; 2] {
        with_object(matrix, bounds, &[], &[], |object| {
            let mut out = [[0.0; 3]; 2];
            assert_ne!(unsafe { arroword_oracle_142261df0(object, &mut out, 0) }, 0);
            out
        })
    }

    fn native_slab(origin: [f32; 3], direction: [f32; 3], bounds: [[f32; 3]; 2]) -> Option<f32> {
        let mut near = 0.0;
        let hit = unsafe { arroword_oracle_1424e6270(&origin, &direction, 0.0, f32::MAX,
            &bounds[0], &bounds[1], &mut near) };
        (hit != 0).then_some(near)
    }

    fn same_float(actual: f32, native: f32) {
        if native.is_nan() { assert!(actual.is_nan()); }
        else { assert_eq!(actual.to_bits(), native.to_bits()); }
    }

    fn same_hit(actual: Option<f32>, native: Option<f32>) {
        match (actual, native) {
            (Some(actual), Some(native)) => same_float(actual, native),
            (None, None) => {},
            _ => panic!("intersection mismatch: actual={actual:?} native={native:?}"),
        }
    }

    #[test]
    fn native_geometry_oracle_preserves_every_instruction_and_only_relocates_addresses() {
        use std::collections::BTreeSet;
        use portable_engine_evidence::native_model_ray::{EXPORTS, GEOMETRY_EXPORTS};
        let functions = ["142261a90", "1424e8690", "1424e6f10", "1424e85d0", "1424e6270", "142261df0", "1422627e0"];
        let mut rows = Vec::new();
        let mut addresses = BTreeSet::new();
        for function in functions {
            let name = format!("func_{function}.instructions.txt");
            let source = EXPORTS.iter().chain(GEOMETRY_EXPORTS).find(|r| r.0 == name).unwrap().1;
            let mut instructions = Vec::new();
            for line in source.lines() {
                let mut fields = line.split_whitespace();
                let Some(address) = fields.next() else { continue };
                if u64::from_str_radix(address, 16).is_err() { continue; }
                fields.next().unwrap();
                addresses.insert(address.to_owned());
                instructions.push((address.to_owned(), fields.collect::<Vec<_>>().join(" ")));
            }
            rows.push((function, instructions));
        }
        let constants = [("1427115d0", "80000000"), ("14270e270", "3f800000"),
            ("142b29ea0", "b8d1b717"), ("142934a78", "38d1b717")];
        let unavailable = ["14254853c", "14239ad00", "1422c25e0"];
        let mut expected = vec![".text".to_owned()];
        for (function, instructions) in rows {
            expected.extend([".p2align 4".to_owned(), format!(".globl arroword_oracle_{function}"),
                format!("arroword_oracle_{function}:")]);
            for (address, mut operation) in instructions {
                for (data, _) in constants {
                    operation = operation.replace(&format!("[0x{data}]"), &format!("[RIP + .Larroword_data_{data}]"));
                }
                let opcode = operation.split_whitespace().next().unwrap();
                if opcode == "CALL" || opcode.starts_with('J') {
                    let target = operation.split_whitespace().nth(1).unwrap().strip_prefix("0x").unwrap();
                    let symbol = if addresses.contains(target) { format!(".Larroword_{target}") }
                        else { assert!(unavailable.contains(&target)); format!(".Larroword_unavailable_{target}") };
                    operation = format!("{opcode} {symbol}");
                }
                expected.extend([format!(".Larroword_{address}:"), operation]);
            }
        }
        for target in unavailable { expected.extend([format!(".Larroword_unavailable_{target}:"), "UD2".to_owned()]); }
        for (address, bits) in constants { expected.extend([".p2align 2".to_owned(),
            format!(".Larroword_data_{address}:"), format!(".long 0x{bits}")]); }
        assert_eq!(include_str!("native_model_geometry_oracle.asm").lines().collect::<Vec<_>>(), expected);
    }

    #[test]
    fn shared_normalizer_matches_native_division_zero_and_unordered_arms() {
        for vector in [[3.0, -4.0, 0.0], [0.0, -0.0, 0.0], [f32::from_bits(1), -0.0, 0.0],
            [f32::MAX, 1.0, 0.0], [f32::INFINITY, 1.0, 0.0], [f32::from_bits(0x7fc00001), 1.0, -0.0]] {
            let mut native = [0.0; 3];
            unsafe { arroword_oracle_1424e6f10(&vector, &mut native); }
            let actual = crate::particle_runtime::native_normalize(vector, true);
            for axis in 0..3 { same_float(actual[axis], native[axis]); }
        }
    }

    #[test]
    fn local_ray_composition_matches_full_native_collision_function_across_matrices() {
        let positions = [[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [0.0, 1.0, 0.0]];
        let indices = [0u16, 1, 2];
        let mut hits = 0;
        let mut misses = 0;
        for matrix in matrices() {
            with_object(matrix, [[0.0; 3]; 2], &positions, &indices, |object| {
                for x in [-4.0, -1.0, 0.0, 0.25, 2.0] {
                    for y in [-3.0, -0.5, 0.0, 1.0] {
                        for direction in [[0.0, 0.0, -1.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]] {
                            let origin = [x, y, 2.0];
                            let native = unsafe { arroword_oracle_1422627e0(object, &origin, &direction) } != 0;
                            assert_eq!(model_geometry_hit(matrix, origin, direction, &positions, &indices, None), Ok(native),
                                "matrix={matrix:?} origin={origin:?} direction={direction:?}");
                            if native { hits += 1; } else { misses += 1; }
                        }
                    }
                }
            });
        }
        assert!(hits > 0 && misses > 0);
    }

    #[test]
    fn native_transform_parts_preserve_zero_axis_reciprocals_and_sign_bits() {
        let mut matrix = IDENTITY;
        matrix[0] = 0.0; matrix[5] = 2.0; matrix[10] = -4.0;
        matrix[12] = 0.0; matrix[13] = -0.0; matrix[14] = 3.0;
        let mut translation = [0.0; 3]; let mut rotation = [0.0; 9];
        let mut reciprocal = [0.0; 3]; let mut lengths = [0.0; 3];
        unsafe { arroword_oracle_142261a90(&matrix, &mut translation, &mut rotation, &mut reciprocal, &mut lengths); }
        assert_eq!(translation.map(f32::to_bits), [0x80000000, 0, (-3.0f32).to_bits()]);
        assert_eq!(reciprocal, [0.0, 0.5, 0.25]);
        assert_eq!(lengths, [0.0, 2.0, 4.0]);
        let ray = native_model_local_ray(matrix, [0.0, 2.0, 7.0], [0.0, 0.0, -1.0]);
        assert_eq!(ray.origin, [0.0, 1.0, -1.0]);
        assert_eq!(ray.direction, [0.0, 0.0, 1.0]);
    }

    #[test]
    fn world_bounds_match_native_static_getter_including_maxss_operand_order() {
        let local = [[-1.0, -2.0, -3.0], [4.0, 5.0, 6.0]];
        let mut inputs = matrices();
        for axis in [0, 4, 8] {
            let mut matrix = IDENTITY; matrix[axis] = f32::from_bits(0x7fc00001); inputs.push(matrix);
        }
        for matrix in inputs {
            let actual = native_world_bounds(matrix, local);
            let native = native_bounds(matrix, local);
            for bound in 0..2 { for axis in 0..3 { same_float(actual[bound][axis], native[bound][axis]); } }
        }
    }

    #[test]
    fn slab_matches_native_parallel_signed_zero_nan_and_strict_endpoint_branches() {
        let bounds = [[-1.0; 3], [1.0; 3]];
        for origin in [[0.0; 3], [-1.0, 0.0, 0.0], [1.0; 3], [2.0, 0.0, 0.0],
            [f32::from_bits(0x7fc00001), 0.0, 0.0]] {
            for direction in [[1.0, 0.0, 0.0], [-1.0, -0.0, 0.0], [0.0; 3], [-0.0; 3],
                [0.0, 1.0, 0.0], [0.0, 0.0, -1.0], [f32::INFINITY, 0.0, 1.0],
                [f32::from_bits(0xffc00001), 1.0, 1.0]] {
                same_hit(world_bounds_intersection(origin, direction, bounds), native_slab(origin, direction, bounds));
            }
        }
        for bounds in [[[0.0; 3]; 2], [[-1.0, -1.0, f32::MAX], [1.0, 1.0, f32::MAX]],
            [[-1.0, -1.0, -2.0], [1.0, 1.0, -1.0]]] {
            let native = native_slab([0.0; 3], [0.0, 0.0, 1.0], bounds);
            assert!(native.is_none());
            same_hit(world_bounds_intersection([0.0; 3], [0.0, 0.0, 1.0], bounds), native);
        }
    }

    #[test]
    fn witnessed_bounds_dispatch_composes_native_getter_and_slab_without_static_substitution() {
        let local = [[-1.0; 3], [1.0; 3]];
        for matrix in matrices() {
            for origin in [[0.0, 0.0, 5.0], [10.0, 10.0, 10.0]] {
                let direction = [0.0, 0.0, -1.0];
                let native = native_slab(origin, direction, native_bounds(matrix, local)).is_some();
                assert_eq!(model_geometry_hit(matrix, origin, direction, &[], &[], Some(local)), Ok(native));
                assert_eq!(model_geometry_hit(matrix, origin, direction, &[], &[], None),
                    Err(NativeCollisionRefusal::BoundsProviderRequired));
            }
        }
    }
}
