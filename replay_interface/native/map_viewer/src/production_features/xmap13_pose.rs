use crate::pdx_mesh::{PdxAnimData, PdxJoint};
use crate::Result;
use snapshot::sha256::Sha256;
use std::collections::BTreeMap;

/// The engine binds fifty `float4x4` bone matrices at VS b2, which is the whole
/// of the 3,200 bytes
/// (`extractor/native/runs/ok_unitpres/recovered_XMAP13_PRODUCTION_CONTRACT.md`).
pub(crate) const XMAP13_PALETTE_JOINTS: usize = 50;
pub(crate) const XMAP13_PALETTE_BYTES: usize = XMAP13_PALETTE_JOINTS * 16 * 4;

/// How far any lane of the f32 palette may sit from the same palette evaluated
/// in f64 from the same parsed inputs, as a fraction of the lane's own
/// magnitude where that exceeds one and absolutely below it.
///
/// This is the forward error of the pose actually being built - the clip's
/// own locals, through the actual hierarchy, in the production order - and
/// not a proxy for it. Every proxy tried before it was escaped: a per-matrix
/// residual (a two-joint chain at rest off by 0.25), a composed rest check (an
/// animated root's unanimated child off by 0.16), and a bind-pose check (a
/// clip one ULP from bind amplified ten times, and a bind local whose f64
/// rounding differed from the clip's by one ULP and let 4.9e-4 through). A
/// pose whose f32 palette leaves its f64 reference by more than this is
/// refused, whatever the clip does.
///
/// What the bound is a bound ON is the displacement of a skinned VERTEX, not
/// a lane: a vertex at bind position `v` (|v| at most the mesh's extent E) is
/// moved by the error matrix by at most |dLinear| x E + |dTranslation|. So the
/// linear lanes are held to this absolutely, and the translation lanes to this
/// times max(1, E), which keeps the induced displacement within a few times
/// this fraction of the mesh's own size. A lane's own magnitude buys nothing:
/// two lanes of 2^24 that cancel on a unit vertex can be wrong by 2 each and
/// move that vertex by 2, which a lane-relative rule admitted.
pub(crate) const XMAP13_PALETTE_F32_TOLERANCE: f32 = 1e-4;

/// The largest error `XMAP13_PALETTE_F32_TOLERANCE` admits on `lane` of a
/// skinning matrix for a mesh whose bind-pose vertices lie within
/// `vertex_extent` of the origin.
fn palette_lane_allowance(lane: usize, vertex_extent: f32) -> f64 {
    let tolerance = f64::from(XMAP13_PALETTE_F32_TOLERANCE);
    match lane {
        12 | 13 | 14 => tolerance * f64::from(vertex_extent).max(1.0),
        _ => tolerance,
    }
}

/// The fifty identity `float4x4` a skinned mesh at rest binds: every slot's
/// skinning matrix is the identity, written exactly.
pub(crate) fn xmap13_rest_palette_bytes() -> Vec<u8> {
    let mut out = vec![0u8; XMAP13_PALETTE_BYTES];
    for slot in 0..XMAP13_PALETTE_JOINTS {
        for (lane, value) in IDENTITY.iter().enumerate() {
            let at = (slot * 16 + lane) * 4;
            out[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    out
}

/// The game-relative path of a clip an animation table declares.
///
/// The table's `file` is a bare name and the engine opens it beside the table's
/// own file: across the whole install every declared clip is authored without a
/// directory and resolves in the declaring table's own directory of the
/// effective VFS
/// (`building_files.rs::the_shipped_entity_states_select_their_clips_through_the_shipped_registries`).
pub(crate) fn authored_clip_logical_path(table_logical_path: &str, file: &str) -> String {
    let table = table_logical_path.replace('\\', "/");
    let file = file.replace('\\', "/");
    match table.rsplit_once('/') {
        Some((dir, _)) if !dir.is_empty() => format!("{dir}/{file}"),
        _ => file,
    }
}

/// The seed for one unit's pose draw.
///
/// The canonical `semantic_source_id` and nothing else: it is stable across
/// hours, runs and machines, which is exactly what makes a replay reproducible.
/// Hashing rather than using the string's own bytes is what stops two units
/// whose ids share a prefix from drawing neighbouring phases.
pub(crate) fn xmap13_pose_seed(semantic_source_id: &str) -> u64 {
    let digest = Sha256::hex(semantic_source_id.as_bytes());
    let mut seed = 0u64;
    for byte in digest.as_bytes().iter().take(16) {
        seed = seed.wrapping_mul(31).wrapping_add(u64::from(*byte));
    }
    seed
}

/// A phase inside `clip_length_seconds`, drawn from `seed`.
///
/// Uniform over the clip's own length. The engine draws over an authored range
/// instead, but the authored `.asset` carries no such range - it carries the
/// clip name, an optional blend time and an optional speed - so the range is
/// engine runtime state and not something the installed content could be read for.
/// Under the ruling the viewer is CHOOSING a phase rather than reproducing one,
/// and the clip's own length is the interval it is choosing within.
pub(crate) fn xmap13_pose_phase(seed: u64, clip_length_seconds: f32) -> f32 {
    if !clip_length_seconds.is_finite() || clip_length_seconds <= 0.0 {
        return 0.0;
    }
    // 53 bits, which is every bit an f64 mantissa can hold, so the quotient is
    // exact and the product cannot land on the clip length itself.
    let unit = (seed >> 11) as f64 / (1u64 << 53) as f64;
    let phase = unit * f64::from(clip_length_seconds);
    let phase = phase as f32;
    if phase >= clip_length_seconds || !phase.is_finite() || phase < 0.0 {
        0.0
    } else {
        phase
    }
}

/// The clip's length in seconds.
pub(crate) fn xmap13_clip_length_seconds(anim: &PdxAnimData) -> f32 {
    if anim.fps <= 0.0 || !anim.fps.is_finite() || anim.sample_count <= 0 {
        return 0.0;
    }
    anim.sample_count as f32 / anim.fps
}

/// The fifty float4x4 skinning matrices for `skeleton` posed at `phase`.
///
/// A skinning matrix is not a bone's transform: it is the matrix that takes a
/// vertex authored in BIND pose into the posed bone's space, which is the
/// bone's current object transform times its INVERSE BIND. `PdxJoint`'s `tx` is
/// that inverse bind, so a palette that wrote object transforms straight out
/// would move every skinned vertex by the bind pose itself - and a mesh at rest
/// would be visibly deformed while every size, digest and `skinned` check still
/// passed. At rest this function returns fifty identities, which is the whole
/// statement.
///
/// Slots are the joints' own `index`, never their position in the vector: a
/// mesh's vertex bone indices address the palette by that index, so packing by
/// vector order would point every weighted vertex at another bone's matrix. The
/// vector order is not authored to match either, so it is sorted here.
///
/// The joints are matched to the CLIP by name, because the two files index
/// their own joints independently and a positional match would pose a bone with
/// another bone's track. A skeleton joint the clip does not animate keeps its
/// bind pose, which contributes an identity skinning matrix rather than a hole.
///
/// The packing ORDER inside each slot's sixteen floats is this viewer's, and
/// says so: the engine's buffer is fifty float4x4 at VS b2, which fixes the
/// size and the slot count, but nothing in this repository establishes its row
/// order, so a reader must not take the layout below as an engine claim.
pub(crate) fn xmap13_skin_palette(
    skeleton: &[PdxJoint],
    anim: Option<&PdxAnimData>,
    phase: f32,
    vertex_extent: f32,
) -> Result<Vec<u8>> {
    if !vertex_extent.is_finite() || vertex_extent < 0.0 {
        return Err(format!(
            "XMAP13 pose: the mesh's vertex extent {vertex_extent} is not a bound"
        )
        .into());
    }
    if skeleton.len() > XMAP13_PALETTE_JOINTS {
        return Err(format!(
            "XMAP13 pose: the mesh skeleton has {} joints and the engine's palette holds {}",
            skeleton.len(),
            XMAP13_PALETTE_JOINTS
        )
        .into());
    }
    // Slot by the joint's OWN index, and refuse anything that cannot address
    // the palette. A vertex weighted to a joint whose matrix is somewhere else
    // draws deformed while every check downstream passes.
    let mut by_slot: Vec<Option<&PdxJoint>> = vec![None; XMAP13_PALETTE_JOINTS];
    for joint in skeleton {
        let Ok(slot) = usize::try_from(joint.index) else {
            return Err(format!(
                "XMAP13 pose: joint {:?} has a negative index {}",
                joint.name, joint.index
            )
            .into());
        };
        if slot >= XMAP13_PALETTE_JOINTS {
            return Err(format!(
                "XMAP13 pose: joint {:?} indexes palette slot {slot} and the engine's palette \
                 holds {}",
                joint.name, XMAP13_PALETTE_JOINTS
            )
            .into());
        }
        if by_slot[slot].replace(joint).is_some() {
            return Err(format!(
                "XMAP13 pose: two skeleton joints claim palette slot {slot}"
            )
            .into());
        }
    }
    // The parent chain is resolved by index, and a parent must come before its
    // child so one ordered walk composes the whole tree. That is the skeleton
    // being a tree at all; a forward or absent parent is a skeleton this
    // function cannot pose rather than one it should pose partially.
    for (slot, joint) in by_slot.iter().enumerate() {
        let Some(joint) = joint else { continue };
        let Some(parent) = joint.parent else { continue };
        let Ok(parent) = usize::try_from(parent) else {
            return Err(format!(
                "XMAP13 pose: joint {:?} has a negative parent index {parent}",
                joint.name
            )
            .into());
        };
        if parent >= slot || by_slot[parent].is_none() {
            return Err(format!(
                "XMAP13 pose: joint {:?} at slot {slot} names parent slot {parent}, which is not \
                 an earlier joint of this skeleton",
                joint.name
            )
            .into());
        }
    }

    // `tx` is the INVERSE BIND. The bind object transform is its inverse, and a
    // joint's bind LOCAL is its parent's inverse bind times its own bind
    // object - so no second matrix inversion is needed for the chain.
    let mut inverse_bind = vec![IDENTITY; XMAP13_PALETTE_JOINTS];
    let mut bind_object = vec![IDENTITY; XMAP13_PALETTE_JOINTS];
    for (slot, joint) in by_slot.iter().enumerate() {
        let Some(joint) = joint else { continue };
        inverse_bind[slot] = affine_from_3x4(joint.transform_3x4);
        bind_object[slot] = affine_inverse(inverse_bind[slot]).ok_or_else(|| -> Box<dyn std::error::Error> {
            format!(
                "XMAP13 pose: joint {:?} has a singular inverse-bind transform",
                joint.name
            )
            .into()
        })?;
    }
    let sample = anim.map(|anim| sample_index(anim, phase));
    let channels = |joint: &PdxJoint| {
        anim.and_then(|anim| animated_channels(anim, &joint.name, sample.unwrap_or(0)))
    };
    let posed = compose_palette(&by_slot, &inverse_bind, &bind_object, |_, joint| {
        channels(joint).map(|(translation, quat, scale)| compose(translation, quat, scale))
    });
    // The same pose, evaluated in f64 from the same f32 inputs - the joints'
    // inverse binds and the clip's channels - through the same chain. Every
    // f32 lane has to land within XMAP13_PALETTE_F32_TOLERANCE of it, or the
    // pose is one f32 cannot build and the unit is refused rather than drawn
    // deformed. Rest is exact by construction and passes at zero.
    let mut inverse_bind_f64 = vec![IDENTITY_F64; XMAP13_PALETTE_JOINTS];
    let mut bind_object_f64 = vec![IDENTITY_F64; XMAP13_PALETTE_JOINTS];
    for (slot, joint) in by_slot.iter().enumerate() {
        let Some(joint) = joint else { continue };
        inverse_bind_f64[slot] = affine_from_3x4_f64(joint.transform_3x4);
        bind_object_f64[slot] = affine_inverse_f64(inverse_bind_f64[slot]).ok_or_else(
            || -> Box<dyn std::error::Error> {
                format!(
                    "XMAP13 pose: joint {:?} has an inverse-bind transform that is singular in f64",
                    joint.name
                )
                .into()
            },
        )?;
    }
    let reference = compose_palette_f64(&by_slot, &inverse_bind_f64, &bind_object_f64, |joint| {
        channels(joint).map(|(translation, quat, scale)| compose_f64(translation, quat, scale))
    });
    for (slot, joint) in by_slot.iter().enumerate() {
        let Some(joint) = joint else { continue };
        for lane in 0..16 {
            let actual = posed[slot][lane];
            let expected = reference[slot][lane];
            let off = (f64::from(actual) - expected).abs();
            if !actual.is_finite()
                || !expected.is_finite()
                || off > palette_lane_allowance(lane, vertex_extent)
            {
                return Err(format!(
                    "XMAP13 pose: joint {:?} at slot {slot} lane {lane} is {actual} in f32 against \
                     {expected} in f64, so this pose cannot be built in f32",
                    joint.name
                )
                .into());
            }
        }
    }

    let mut palette = vec![0u8; XMAP13_PALETTE_BYTES];
    for slot in 0..XMAP13_PALETTE_JOINTS {
        // A slot no joint claims is the identity, which leaves any vertex that
        // addressed it where it was authored.
        let matrix = match by_slot[slot] {
            Some(_) => posed[slot],
            None => IDENTITY,
        };
        for (lane, value) in matrix.iter().enumerate() {
            let at = (slot * 16 + lane) * 4;
            palette[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    Ok(palette)
}

/// Every claimed slot's skinning matrix, `object * inverse_bind`, with each
/// joint's local taken from `posed` where it answers.
///
/// A joint `posed` does NOT answer for sits at its bind pose relative to its
/// parent: `object = object_parent * inverse_bind_parent * bind_object`, so
/// `object * inverse_bind = object_parent * inverse_bind_parent` - the PARENT's
/// own skinning matrix, and the identity for a root. That is an algebraic
/// identity, so the matrix is COPIED and never reconstructed: nothing is
/// inverted on the way to an unanimated joint's palette, a skeleton at rest is
/// fifty exact identities, and an unanimated child of an animated parent is
/// exactly its parent. The bind object - the one inverse this module takes -
/// serves one thing, the object transform such a joint hands an ANIMATED
/// descendant, and enters no other lane. Reconstructing the rest palette
/// through the chain instead admitted, at a per-matrix residual of 4e-6, a
/// child whose rest translation lane was off by 0.25, and a composed-rest
/// check admitted, at 6e-5, a child off by 0.16 under an animated root; both
/// are pinned as EXACT now.
///
/// `by_slot` is parent-before-child by index, which `xmap13_skin_palette` has
/// already refused to proceed without, so one ordered walk composes the tree.
fn compose_palette(
    by_slot: &[Option<&PdxJoint>],
    inverse_bind: &[[f32; 16]],
    bind_object: &[[f32; 16]],
    posed: impl Fn(usize, &PdxJoint) -> Option<[f32; 16]>,
) -> Vec<[f32; 16]> {
    let mut object = vec![IDENTITY; XMAP13_PALETTE_JOINTS];
    let mut out = vec![IDENTITY; XMAP13_PALETTE_JOINTS];
    for (slot, joint) in by_slot.iter().enumerate() {
        let Some(joint) = joint else { continue };
        let parent = joint.parent.and_then(|parent| usize::try_from(parent).ok());
        match posed(slot, joint) {
            Some(local) => {
                object[slot] = match parent {
                    Some(parent) => multiply(object[parent], local),
                    None => local,
                };
                out[slot] = multiply(object[slot], inverse_bind[slot]);
            }
            None => {
                out[slot] = match parent {
                    Some(parent) => out[parent],
                    None => IDENTITY,
                };
                object[slot] = multiply(out[slot], bind_object[slot]);
            }
        }
    }
    out
}

/// The one skeleton a model's shapes share, or a refusal naming the difference.
///
/// The palette is built before the viewer's camera has chosen a LOD, so it has
/// to be the same palette whichever shape is drawn. In a PdxMesh every LOD of
/// one model carries the same skeleton, and this checks that rather than
/// assuming it: a model whose shapes disagree would have its LOD 1 drawn with
/// LOD 0's bones, which passes every size and digest check downstream and
/// deforms the mesh.
pub(crate) fn xmap13_model_skeleton<'a>(
    shapes: impl IntoIterator<Item = &'a [PdxJoint]>,
) -> Result<&'a [PdxJoint]> {
    let mut chosen: Option<&'a [PdxJoint]> = None;
    for skeleton in shapes {
        if skeleton.is_empty() {
            continue;
        }
        match chosen {
            None => chosen = Some(skeleton),
            Some(first) if same_skeleton(first, skeleton) => {}
            Some(_) => {
                return Err("XMAP13 pose: this model's shapes carry different skeletons, so no \
                            one palette can serve the LOD the camera picks"
                    .into())
            }
        }
    }
    Ok(chosen.unwrap_or_default())
}

fn same_skeleton(left: &[PdxJoint], right: &[PdxJoint]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.name == right.name
                && left.index == right.index
                && left.parent == right.parent
                && left.transform_3x4 == right.transform_3x4
        })
}

const IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// The sample the phase lands on, clamped into the clip.
fn sample_index(anim: &PdxAnimData, phase: f32) -> usize {
    if anim.sample_count <= 0 || anim.fps <= 0.0 || !phase.is_finite() || phase < 0.0 {
        return 0;
    }
    let index = (phase * anim.fps) as usize;
    index.min(anim.sample_count as usize - 1)
}

/// One joint's translation, rotation and scale at `sample`, from whichever
/// channels it carries.
///
/// A channel the joint does not animate falls back to its authored base value,
/// which is what the clip means by leaving the channel out. Returned as the
/// three parsed values rather than a matrix so that the production f32
/// composition and its f64 reference start from the same inputs.
fn animated_channels(
    anim: &PdxAnimData,
    joint_name: &str,
    sample: usize,
) -> Option<([f32; 3], [f32; 4], f32)> {
    let joint = anim.joints.iter().find(|row| row.name == joint_name)?;
    let ordinal = |flag: char| -> Option<usize> {
        if !joint.channels.contains(flag) {
            return None;
        }
        Some(
            anim.joints
                .iter()
                .take_while(|row| row.name != joint_name)
                .filter(|row| row.channels.contains(flag))
                .count(),
        )
    };
    let per_sample = |flag: char| -> usize {
        anim.joints
            .iter()
            .filter(|row| row.channels.contains(flag))
            .count()
    };
    let mut translation = joint.base_translation;
    if let Some(ordinal) = ordinal('t') {
        let at = (sample * per_sample('t') + ordinal) * 3;
        if let Some(values) = anim.samples_t.get(at..at + 3) {
            translation = [values[0], values[1], values[2]];
        }
    }
    let mut rotation = joint.base_rotation_quat;
    if let Some(ordinal) = ordinal('q') {
        let at = (sample * per_sample('q') + ordinal) * 4;
        if let Some(values) = anim.samples_q.get(at..at + 4) {
            rotation = [values[0], values[1], values[2], values[3]];
        }
    }
    let mut scale = joint.base_scale;
    if let Some(ordinal) = ordinal('s') {
        let at = sample * per_sample('s') + ordinal;
        if let Some(value) = anim.samples_s.get(at) {
            scale = *value;
        }
    }
    Some((translation, rotation, scale))
}

pub(crate) fn xmap13_node_transforms(
    skeleton: &[PdxJoint], anim: Option<&PdxAnimData>, phase: f32,
) -> Result<std::collections::BTreeMap<String, [f32; 16]>> {
    let palette = xmap13_skin_palette(skeleton, anim, phase, 0.0)?;
    skeleton.iter().map(|joint| {
        let start = joint.index as usize * 64;
        let skin = std::array::from_fn(|lane| f32::from_le_bytes(palette[start + lane * 4..start + lane * 4 + 4].try_into().unwrap()));
        let bind = affine_inverse(affine_from_3x4(joint.transform_3x4)).ok_or("light node inverse bind is singular")?;
        let object = multiply(skin, bind);
        Ok((joint.name.clone(), std::array::from_fn(|i| object[(i % 4) * 4 + i / 4])))
    }).collect()
}

/// A 4x4 from the authored 4x3 affine, which carries no projective row.
fn affine_from_3x4(rows: [f32; 12]) -> [f32; 16] {
    [
        rows[0], rows[1], rows[2], 0.0, rows[3], rows[4], rows[5], 0.0, rows[6], rows[7], rows[8],
        0.0, rows[9], rows[10], rows[11], 1.0,
    ]
}

/// Translation, rotation and uniform scale into one 4x4.
fn compose(translation: [f32; 3], quat: [f32; 4], scale: f32) -> [f32; 16] {
    let [x, y, z, w] = quat;
    let length = (x * x + y * y + z * z + w * w).sqrt();
    // An unnormalized quaternion would scale the basis a second time on top of
    // `scale`, so the authored value is normalized and a degenerate one falls
    // back to no rotation rather than to a matrix of NaNs.
    let (x, y, z, w) = if length.is_finite() && length > f32::EPSILON {
        (x / length, y / length, z / length, w / length)
    } else {
        (0.0, 0.0, 0.0, 1.0)
    };
    let scale = if scale.is_finite() { scale } else { 1.0 };
    [
        (1.0 - 2.0 * (y * y + z * z)) * scale,
        (2.0 * (x * y + z * w)) * scale,
        (2.0 * (x * z - y * w)) * scale,
        0.0,
        (2.0 * (x * y - z * w)) * scale,
        (1.0 - 2.0 * (x * x + z * z)) * scale,
        (2.0 * (y * z + x * w)) * scale,
        0.0,
        (2.0 * (x * z + y * w)) * scale,
        (2.0 * (y * z - x * w)) * scale,
        (1.0 - 2.0 * (x * x + y * y)) * scale,
        0.0,
        translation[0],
        translation[1],
        translation[2],
        1.0,
    ]
}

/// The inverse of an affine 4x4 whose last column is (0,0,0,1).
///
/// `None` where the linear part is singular, which is a skeleton that cannot be
/// posed rather than one to pose with infinities.
fn affine_inverse(m: [f32; 16]) -> Option<[f32; 16]> {
    let (a, b, c) = (m[0], m[4], m[8]);
    let (d, e, f) = (m[1], m[5], m[9]);
    let (g, h, i) = (m[2], m[6], m[10]);
    let determinant = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    // Only an actually-degenerate determinant is refused here. A MAGNITUDE
    // threshold is not a conditioning test and refuses matrices that invert
    // perfectly well: a uniform scale of 1e-7 has determinant 1e-21 and a
    // finite 1e7 inverse. What decides whether this inverse is usable is the
    // residual check at the end, which is scale-free.
    if !determinant.is_finite() || determinant == 0.0 {
        return None;
    }
    let inv = 1.0 / determinant;
    let linear = [
        (e * i - f * h) * inv,
        (c * h - b * i) * inv,
        (b * f - c * e) * inv,
        (f * g - d * i) * inv,
        (a * i - c * g) * inv,
        (c * d - a * f) * inv,
        (d * h - e * g) * inv,
        (b * g - a * h) * inv,
        (a * e - b * d) * inv,
    ];
    // `linear` is the adjugate in ROW-major order - `linear[row * 3 + column]`
    // is the inverse's (row, column) - while `out` is column-major, so the two
    // indices are not interchangeable. Writing `linear[column * 3 + row]` here
    // stored the TRANSPOSE, which is invisible whenever the linear part is
    // diagonal and deforms the mesh the moment it is not.
    let (tx, ty, tz) = (m[12], m[13], m[14]);
    let mut out = IDENTITY;
    for column in 0..3 {
        for row in 0..3 {
            out[column * 4 + row] = linear[row * 3 + column];
        }
    }
    out[12] = -(linear[0] * tx + linear[1] * ty + linear[2] * tz);
    out[13] = -(linear[3] * tx + linear[4] * ty + linear[5] * tz);
    out[14] = -(linear[6] * tx + linear[7] * ty + linear[8] * tz);
    if out.iter().any(|value| !value.is_finite()) {
        return None;
    }
    // The inverse has to BE one, in BOTH directions. This is the conditioning
    // test the determinant cannot be: a matrix whose inverse f32 cannot
    // represent usefully shows up as a residual whatever its scale, and a
    // well-conditioned matrix of any scale passes.
    //
    // Both, because a one-sided check validates the wrong product. The palette
    // multiplies `object * inverse_bind`, which is `inv * m`, while a check of
    // `m * inv` alone admits matrices whose other product is off by 32 - and
    // the pinned counterexample is exactly one of those.
    (residual_from_identity(multiply(m, out))? <= 1e-4
        && residual_from_identity(multiply(out, m))? <= 1e-4)
        .then_some(out)
}

/// How far a product is from the identity, or `None` if any lane is not finite.
///
/// The finiteness is checked lane by lane rather than folded through
/// `f32::max`, which RETURNS THE OTHER OPERAND when one is NaN and would carry
/// a NaN product through as a small residual.
fn residual_from_identity(product: [f32; 16]) -> Option<f32> {
    let mut worst = 0.0f32;
    for (actual, expected) in product.iter().zip(IDENTITY.iter()) {
        if !actual.is_finite() {
            return None;
        }
        let off = (actual - expected).abs();
        if off > worst {
            worst = off;
        }
    }
    Some(worst)
}

/// Column-major 4x4 multiply, matching the layout the packing writes.
fn multiply(a: [f32; 16], b: [f32; 16]) -> [f32; 16] {
    let mut out = [0.0f32; 16];
    for column in 0..4 {
        for row in 0..4 {
            let mut sum = 0.0f32;
            for k in 0..4 {
                sum += a[k * 4 + row] * b[column * 4 + k];
            }
            out[column * 4 + row] = sum;
        }
    }
    out
}

const IDENTITY_F64: [f64; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// `affine_from_3x4`, in f64.
fn affine_from_3x4_f64(rows: [f32; 12]) -> [f64; 16] {
    let r = rows.map(f64::from);
    [
        r[0], r[1], r[2], 0.0, r[3], r[4], r[5], 0.0, r[6], r[7], r[8], 0.0, r[9], r[10], r[11],
        1.0,
    ]
}

/// `compose`, in f64 from the same f32 channels.
fn compose_f64(translation: [f32; 3], quat: [f32; 4], scale: f32) -> [f64; 16] {
    let [x, y, z, w] = quat.map(f64::from);
    let length = (x * x + y * y + z * z + w * w).sqrt();
    let (x, y, z, w) = if length.is_finite() && length > f64::from(f32::EPSILON) {
        (x / length, y / length, z / length, w / length)
    } else {
        (0.0, 0.0, 0.0, 1.0)
    };
    let scale = if scale.is_finite() { f64::from(scale) } else { 1.0 };
    let t = translation.map(f64::from);
    [
        (1.0 - 2.0 * (y * y + z * z)) * scale,
        (2.0 * (x * y + z * w)) * scale,
        (2.0 * (x * z - y * w)) * scale,
        0.0,
        (2.0 * (x * y - z * w)) * scale,
        (1.0 - 2.0 * (x * x + z * z)) * scale,
        (2.0 * (y * z + x * w)) * scale,
        0.0,
        (2.0 * (x * z + y * w)) * scale,
        (2.0 * (y * z - x * w)) * scale,
        (1.0 - 2.0 * (x * x + y * y)) * scale,
        0.0,
        t[0],
        t[1],
        t[2],
        1.0,
    ]
}

/// `affine_inverse`, in f64 and without the f32 conditioning residual: the
/// reference is exact to f64, and a matrix singular even there has no pose.
fn affine_inverse_f64(m: [f64; 16]) -> Option<[f64; 16]> {
    let (a, b, c) = (m[0], m[4], m[8]);
    let (d, e, f) = (m[1], m[5], m[9]);
    let (g, h, i) = (m[2], m[6], m[10]);
    let determinant = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if !determinant.is_finite() || determinant == 0.0 {
        return None;
    }
    let inv = 1.0 / determinant;
    let linear = [
        (e * i - f * h) * inv,
        (c * h - b * i) * inv,
        (b * f - c * e) * inv,
        (f * g - d * i) * inv,
        (a * i - c * g) * inv,
        (c * d - a * f) * inv,
        (d * h - e * g) * inv,
        (b * g - a * h) * inv,
        (a * e - b * d) * inv,
    ];
    let (tx, ty, tz) = (m[12], m[13], m[14]);
    let mut out = IDENTITY_F64;
    for column in 0..3 {
        for row in 0..3 {
            out[column * 4 + row] = linear[row * 3 + column];
        }
    }
    out[12] = -(linear[0] * tx + linear[1] * ty + linear[2] * tz);
    out[13] = -(linear[3] * tx + linear[4] * ty + linear[5] * tz);
    out[14] = -(linear[6] * tx + linear[7] * ty + linear[8] * tz);
    out.iter().all(|value| value.is_finite()).then_some(out)
}

/// `multiply`, in f64.
fn multiply_f64(a: [f64; 16], b: [f64; 16]) -> [f64; 16] {
    let mut out = [0.0f64; 16];
    for column in 0..4 {
        for row in 0..4 {
            let mut sum = 0.0f64;
            for k in 0..4 {
                sum += a[k * 4 + row] * b[column * 4 + k];
            }
            out[column * 4 + row] = sum;
        }
    }
    out
}

/// `compose_palette`, in f64: the same copy rule and the same chain.
fn compose_palette_f64(
    by_slot: &[Option<&PdxJoint>],
    inverse_bind: &[[f64; 16]],
    bind_object: &[[f64; 16]],
    posed: impl Fn(&PdxJoint) -> Option<[f64; 16]>,
) -> Vec<[f64; 16]> {
    let mut object = vec![IDENTITY_F64; XMAP13_PALETTE_JOINTS];
    let mut out = vec![IDENTITY_F64; XMAP13_PALETTE_JOINTS];
    for (slot, joint) in by_slot.iter().enumerate() {
        let Some(joint) = joint else { continue };
        let parent = joint.parent.and_then(|parent| usize::try_from(parent).ok());
        match posed(joint) {
            Some(local) => {
                object[slot] = match parent {
                    Some(parent) => multiply_f64(object[parent], local),
                    None => local,
                };
                out[slot] = multiply_f64(object[slot], inverse_bind[slot]);
            }
            None => {
                out[slot] = match parent {
                    Some(parent) => out[parent],
                    None => IDENTITY_F64,
                };
                object[slot] = multiply_f64(out[slot], bind_object[slot]);
            }
        }
    }
    out
}

/// The worst forward error any lane of `skeleton` posed by `anim` at `phase`
/// has against its f64 reference, as a fraction of that lane's allowance at
/// tolerance one - so a value above `XMAP13_PALETTE_F32_TOLERANCE` is a
/// refusal - or the refusal the palette would give.
#[cfg(test)]
pub(crate) fn xmap13_palette_forward_error(
    skeleton: &[PdxJoint],
    anim: Option<&PdxAnimData>,
    phase: f32,
    vertex_extent: f32,
) -> Result<f32> {
    let mut by_slot: Vec<Option<&PdxJoint>> = vec![None; XMAP13_PALETTE_JOINTS];
    for joint in skeleton {
        by_slot[usize::try_from(joint.index)?] = Some(joint);
    }
    let mut inverse_bind = vec![IDENTITY; XMAP13_PALETTE_JOINTS];
    let mut bind_object = vec![IDENTITY; XMAP13_PALETTE_JOINTS];
    let mut inverse_bind_f64 = vec![IDENTITY_F64; XMAP13_PALETTE_JOINTS];
    let mut bind_object_f64 = vec![IDENTITY_F64; XMAP13_PALETTE_JOINTS];
    for (slot, joint) in by_slot.iter().enumerate() {
        let Some(joint) = joint else { continue };
        inverse_bind[slot] = affine_from_3x4(joint.transform_3x4);
        bind_object[slot] = affine_inverse(inverse_bind[slot]).ok_or("singular in f32")?;
        inverse_bind_f64[slot] = affine_from_3x4_f64(joint.transform_3x4);
        bind_object_f64[slot] = affine_inverse_f64(inverse_bind_f64[slot]).ok_or("singular in f64")?;
    }
    let sample = anim.map(|anim| sample_index(anim, phase));
    let channels = |joint: &PdxJoint| {
        anim.and_then(|anim| animated_channels(anim, &joint.name, sample.unwrap_or(0)))
    };
    let posed = compose_palette(&by_slot, &inverse_bind, &bind_object, |_, joint| {
        channels(joint).map(|(translation, quat, scale)| compose(translation, quat, scale))
    });
    let reference = compose_palette_f64(&by_slot, &inverse_bind_f64, &bind_object_f64, |joint| {
        channels(joint).map(|(translation, quat, scale)| compose_f64(translation, quat, scale))
    });
    let mut worst = 0.0f64;
    for (slot, joint) in by_slot.iter().enumerate() {
        if joint.is_none() {
            continue;
        }
        for lane in 0..16 {
            let expected = reference[slot][lane];
            let off = (f64::from(posed[slot][lane]) - expected).abs();
            if !off.is_finite() {
                return Err("non-finite".into());
            }
            let allowance = palette_lane_allowance(lane, vertex_extent)
                / f64::from(XMAP13_PALETTE_F32_TOLERANCE);
            worst = worst.max(off / allowance);
        }
    }
    Ok(worst as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdx_mesh::PdxAnimJoint;
    use std::collections::BTreeMap;

    /// The production palette for a mesh of unit extent, which every fixture
    /// here is unless it says otherwise.
    fn palette(skeleton: &[PdxJoint], anim: Option<&PdxAnimData>, phase: f32) -> Result<Vec<u8>> {
        xmap13_skin_palette(skeleton, anim, phase, 1.0)
    }

    fn forward_error(skeleton: &[PdxJoint], anim: Option<&PdxAnimData>, phase: f32) -> Result<f32> {
        xmap13_palette_forward_error(skeleton, anim, phase, 1.0)
    }

    /// Every float of a packed palette.
    fn lanes(palette: &[u8]) -> Vec<f32> {
        palette
            .chunks_exact(4)
            .map(|bytes| f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            .collect()
    }

    /// A palette at rest.
    ///
    /// A COMPUTED slot is compared to the precision composing two f32 matrices
    /// leaves: `bind_object * inverse_bind` is the identity in exact arithmetic
    /// and lands a unit in the last place away from it once the bind pose
    /// rotates. The claim there is that the mesh is not deformed, which is a
    /// numeric claim; determinism is asserted separately and IS exact.
    ///
    /// A slot NO joint claims is written straight out as the identity, so it is
    /// compared byte for byte. Allowing those a tolerance too let a palette put
    /// a small translation in every unused slot and pass.
    /// `at_rest` are the slots whose joint is unposed; `claimed` are the slots
    /// a joint occupies at all.
    ///
    /// Both are SETS, not ranges: production allows sparse joint indices, so a
    /// claimed COUNT would have assumed the occupied slots are a dense prefix
    /// and compared an occupied slot against the untouched identity. Every slot
    /// outside `claimed` is written straight out as the identity and is
    /// compared byte for byte; every slot in `at_rest` gets the tolerance that
    /// composing two f32 matrices needs.
    fn assert_at_rest(palette: &[u8], at_rest: &[usize], claimed: &[usize]) {
        assert_at_rest_with_posed(palette, at_rest, &[], claimed);
    }

    /// As above, with the slots the clip POSES named explicitly.
    ///
    /// Every claimed slot must be one or the other. A caller that simply left a
    /// claimed slot out of both lists was checking less and could not fail, so
    /// the accounting is asserted rather than assumed.
    fn assert_at_rest_with_posed(
        palette: &[u8],
        at_rest: &[usize],
        posed: &[usize],
        claimed: &[usize],
    ) {
        for slot in claimed {
            assert!(
                at_rest.contains(slot) || posed.contains(slot),
                "claimed slot {slot} is neither asserted at rest nor declared posed"
            );
        }
        let actual = lanes(palette);
        let identity = identity_palette();
        let expected = lanes(&identity);
        for slot in at_rest {
            assert!(claimed.contains(slot), "slot {slot} is at rest but unclaimed");
            for lane in slot * 16..slot * 16 + 16 {
                assert!(
                    (actual[lane] - expected[lane]).abs() < 1e-5,
                    "lane {lane} is {} and a mesh at rest wants {}",
                    actual[lane],
                    expected[lane]
                );
            }
        }
        for slot in 0..XMAP13_PALETTE_JOINTS {
            if claimed.contains(&slot) {
                continue;
            }
            assert_eq!(
                palette[slot * 64..slot * 64 + 64],
                identity[slot * 64..slot * 64 + 64],
                "slot {slot} is claimed by no joint and must be the exact identity"
            );
        }
    }

    /// Fifty identity float4x4, which is what a palette at rest must be.
    fn identity_palette() -> Vec<u8> {
        let mut out = vec![0u8; XMAP13_PALETTE_BYTES];
        for slot in 0..XMAP13_PALETTE_JOINTS {
            for lane in [0usize, 5, 10, 15] {
                let at = (slot * 16 + lane) * 4;
                out[at..at + 4].copy_from_slice(&1.0f32.to_le_bytes());
            }
        }
        out
    }

    fn skeleton(joints: usize) -> Vec<PdxJoint> {
        (0..joints)
            .map(|index| PdxJoint {
                name: format!("bone{index}"),
                index: index as i32,
                parent: (index > 0).then(|| index as i32 - 1),
                // A bind pose that is NOT the identity AND NOT DIAGONAL:
                // a rotation about z with a shear, plus a translation. A
                // diagonal linear part hides a transposed matrix inverse
                // entirely, which is how one lived here through a round.
                transform_3x4: [
                    0.8, 0.6, 0.0, -0.6, 0.8, 0.3, 0.1, 0.0, 1.0, index as f32, 0.5, 0.25,
                ],
                rcf: None,
            })
            .collect()
    }

    fn clip(joints: &[PdxJoint], sample_count: i32) -> PdxAnimData {
        let anim_joints: Vec<PdxAnimJoint> = joints
            .iter()
            .map(|joint| PdxAnimJoint {
                name: joint.name.clone(),
                channels: "tqs".into(),
                base_translation: [0.0, 0.0, 0.0],
                base_rotation_quat: [0.0, 0.0, 0.0, 1.0],
                base_scale: 1.0,
            })
            .collect();
        let count = joints.len();
        let mut samples_t = Vec::new();
        let mut samples_q = Vec::new();
        let mut samples_s = Vec::new();
        for sample in 0..sample_count as usize {
            for joint in 0..count {
                // Every sample differs, so a phase that landed on the wrong one
                // produces different bytes.
                let base = sample as f32 * 10.0 + joint as f32;
                samples_t.extend_from_slice(&[base, base + 1.0, base + 2.0]);
                samples_q.extend_from_slice(&[0.0, 0.0, 0.0, 1.0]);
                samples_s.push(1.0 + base / 100.0);
            }
        }
        PdxAnimData {
            fps: 10.0,
            sample_count,
            joints: anim_joints,
            samples_t,
            samples_q,
            samples_s,
        }
    }

    /// kills: a phase outside the clip, which would index a sample that is not
    /// there and pose the unit at whatever the clamp happened to reach.
    #[test]
    fn every_drawn_phase_is_inside_the_clip_it_is_drawn_for() {
        for length in [0.1f32, 1.0, 6.0, 1200.0] {
            for n in 0..200u64 {
                let phase = xmap13_pose_phase(xmap13_pose_seed(&format!("unit{n}")), length);
                assert!(
                    phase.is_finite() && phase >= 0.0 && phase < length,
                    "phase {phase} is outside [0, {length})"
                );
            }
        }
        // The largest draw the generator can make, which is where the clamp
        // earns its keep: the f64 quotient is strictly below 1.0, so the f64
        // product is strictly below the length - and the cast down to f32
        // ROUNDS IT UP to exactly the length, which would index one sample past
        // the clip. Two hundred sampled seeds never reach this; only the
        // extreme does, so it is pinned by value.
        for length in [0.1f32, 1.0, 6.0, 1200.0] {
            let phase = xmap13_pose_phase(u64::MAX, length);
            assert!(phase < length, "the largest draw landed on {length}");
        }
        // A clip with no length is not a draw at all.
        assert_eq!(xmap13_pose_phase(12345, 0.0), 0.0);
        assert_eq!(xmap13_pose_phase(12345, f32::NAN), 0.0);
        assert_eq!(xmap13_pose_phase(12345, -1.0), 0.0);
    }

    /// kills: a transposed matrix inverse, which every diagonal fixture hides.
    ///
    /// `affine_inverse` is the whole of how a joint's bind pose is recovered
    /// from its inverse bind, and a transpose there is invisible while the
    /// linear part is diagonal - it is only a rotation or a shear that tells
    /// the two apart. This checks the inverse directly, both ways round.
    #[test]
    fn the_affine_inverse_is_an_inverse_and_not_a_transpose_of_one() {
        // EVERY entry of the linear part is nonzero. A fixture with a zero
        // anywhere leaves an adjugate cross-product unexercised - two of them
        // were, and a term could be deleted with nothing failing.
        let m = [
            2.0, 0.7, 0.25, 0.0, //
            -0.4, 3.0, 0.5, 0.0, //
            0.9, -1.1, 4.0, 0.0, //
            3.0, -1.5, 0.25, 1.0,
        ];
        for lane in [0usize, 1, 2, 4, 5, 6, 8, 9, 10] {
            assert!(m[lane] != 0.0, "lane {lane} must be nonzero to exercise the adjugate");
        }
        // And it is not symmetric, so transposing the inverse is a real
        // difference rather than a no-op.
        assert!(
            (0..3).any(|column| (0..3).any(|row| m[column * 4 + row] != m[row * 4 + column])),
            "a symmetric fixture cannot tell an inverse from its transpose"
        );

        let inverse = affine_inverse(m).expect("a nonsingular affine");
        for product in [multiply(m, inverse), multiply(inverse, m)] {
            for (lane, (actual, expected)) in product.iter().zip(IDENTITY.iter()).enumerate() {
                assert!(
                    (actual - expected).abs() < 1e-5,
                    "lane {lane} of the product is {actual}, wanted {expected}"
                );
            }
        }

        // A SCALE, not a magnitude, decides singularity: a uniform 1e-7 scale
        // has determinant 1e-21 and inverts perfectly.
        let mut small = IDENTITY;
        for lane in [0usize, 5, 10] {
            small[lane] = 1e-7;
        }
        let inverse = affine_inverse(small).expect("a small uniform scale is not singular");
        assert!((inverse[0] - 1e7).abs() / 1e7 < 1e-4, "{}", inverse[0]);

        // A singular affine has no inverse and says so rather than returning
        // infinities - and so does one whose inverse f32 cannot carry.
        assert!(affine_inverse([0.0; 16]).is_none());
        let mut flattened = IDENTITY;
        flattened[5] = 0.0;
        assert!(affine_inverse(flattened).is_none(), "a collapsed axis has no inverse");

        // And the case the determinant CANNOT catch: two near-parallel columns,
        // whose determinant is a nonzero finite 2.0e-27 and whose adjugate
        // loses every significant bit to cancellation. The inverse comes out
        // FINITE and wrong - `m * inv` is off by about 2.0e3, not 0 - so the
        // determinant test and the is-finite test both admit it and only the
        // residual refuses it.
        let ill_conditioned = [
            6.876_945e-8,
            6.873_039_4e-8,
            -8.339_26e-6,
            0.0,
            -1.275_830_2e-6,
            -1.275_105_5e-6,
            -4.494_190_3e-6,
            0.0,
            8.940_697e-11,
            8.935_619e-11,
            -4.866_02e-6,
            0.0,
            -2.0,
            -1.0,
            2.0,
            1.0,
        ];
        assert!(
            affine_inverse(ill_conditioned).is_none(),
            "an inverse f32 cannot carry is not an inverse"
        );

        // ONE-SIDED is not enough, and this is the matrix that proves it: its
        // `m * inv` residual is exactly zero while `inv * m` is off by 32. The
        // palette multiplies `object * inverse_bind`, which is `inv * m`, so a
        // guard that checked only the other product admitted a joint whose
        // resting pose is deformed by that much.
        let one_sided = [
            0.044_454_18,
            0.467_001_17,
            -0.069_527_395,
            0.0,
            0.050_609_376,
            -2.289_297_6,
            -0.468_281_95,
            0.0,
            -2.353_407_9,
            -19.726_236,
            4.370_057,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
        ];
        assert!(
            affine_inverse(one_sided).is_none(),
            "an inverse that only works from one side is not an inverse"
        );

        // And the MIRROR of it, so neither direction can be the only one
        // checked: this matrix's `inv * m` residual is 6.1e-5 and inside the
        // tolerance while `m * inv` is 2.4e-3 and outside it. With only the two
        // fixtures above, dropping either half of the check still passed.
        let one_sided_other_way = [
            9.864_402e-9,
            6.217_003e-9,
            -9.055_9e-8,
            0.0,
            2.826_219_8e-8,
            1.210_110_2e-8,
            -9.659_24e-8,
            0.0,
            -9.347_099e-9,
            -6.901_026e-10,
            -6.185e-8,
            0.0,
            3.0,
            3.0,
            -1.0,
            1.0,
        ];
        assert!(
            affine_inverse(one_sided_other_way).is_none(),
            "an inverse that only works from the other side is not one either"
        );

        // And a NaN lane is refused rather than folded away: f32::max returns
        // the OTHER operand when one side is NaN, so a fold would have read a
        // NaN product as a small residual.
        assert_eq!(residual_from_identity([f32::NAN; 16]), None);
        assert_eq!(residual_from_identity([f32::INFINITY; 16]), None);
        assert_eq!(residual_from_identity(IDENTITY), Some(0.0));
    }

    /// kills: a palette that ignores the phase, which would make the whole
    /// derivation a constant dressed as a pose.
    #[test]
    fn the_palette_is_a_function_of_the_phase_and_of_the_skeleton() {
        let bones = skeleton(4);
        let anim = clip(&bones, 40);
        let at = |phase: f32| palette(&bones, Some(&anim), phase).expect("a palette");
        assert_ne!(at(0.0), at(1.0), "two phases must not pose alike");
        assert_eq!(at(0.0), at(0.0));
        // And the skeleton is read: one more joint changes the bytes.
        let longer = skeleton(5);
        assert_ne!(
            at(0.5),
            palette(&longer, Some(&clip(&longer, 40)), 0.5).expect("a palette")
        );
        // With no clip at all it is the rest palette, whatever the skeleton's
        // own bind transforms are - here a rotation with a shear, which is
        // exactly the case a transposed matrix inverse gets wrong.
        assert_at_rest(
            &palette(&bones, None, 0.0).expect("a palette"),
            &(0..bones.len()).collect::<Vec<_>>(),
            &(0..bones.len()).collect::<Vec<_>>(),
        );
    }

    /// kills: a joint the clip does not animate being posed from another
    /// joint's track.
    ///
    /// The two files index their own joints independently, so the match is by
    /// NAME. A positional match would pose `bone2` with `bone0`'s samples the
    /// moment a clip animates a subset, which is the normal case.
    #[test]
    fn a_joint_the_clip_does_not_animate_keeps_its_bind_pose() {
        let bones = skeleton(3);
        let mut partial = clip(&bones, 20);
        // Only the LAST joint is animated, and it keeps its own name.
        partial.joints.retain(|joint| joint.name == "bone2");
        partial.samples_t.truncate(20 * 3);
        partial.samples_q.truncate(20 * 4);
        partial.samples_s.truncate(20);
        let posed = palette(&bones, Some(&partial), 0.5).expect("a palette");
        // bone0 is not named by the clip, so it stays at its bind pose - which
        // is the identity skinning matrix, not its bind transform.
        assert_at_rest_with_posed(&posed, &[0, 1], &[2], &[0, 1, 2]);
        // bone2 is, so its slot moves.
        assert_ne!(posed[128..192], identity_palette()[128..192]);
    }

    /// kills: the palette being packed by VECTOR ORDER rather than by the
    /// joints' own indices, and: a parent reference read as a vector offset.
    ///
    /// A mesh's vertex bone indices address the palette by a joint's `index`.
    /// Packing by position points every weighted vertex at another bone's
    /// matrix while every size, digest and `skinned` check downstream passes -
    /// and reading `parent` as a position silently drops the composition when
    /// the vector is not authored parent-first.
    #[test]
    fn slots_are_the_joints_own_indices_and_parents_are_resolved_by_index() {
        // The CHILD is written first, so a reader that used positions would put
        // it in slot 0 and never compose it with its parent.
        let root = PdxJoint {
            name: "root".into(),
            index: 0,
            parent: None,
            transform_3x4: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, -2.0, 0.0, 0.0],
            rcf: None,
        };
        let child = PdxJoint {
            name: "child".into(),
            index: 1,
            parent: Some(0),
            transform_3x4: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, -5.0, 0.0, 0.0],
            rcf: None,
        };
        let reordered = vec![child.clone(), root.clone()];
        let ordered = vec![root, child];
        // The vector order cannot change the palette, because the slots are the
        // indices either way.
        assert_eq!(
            palette(&reordered, None, 0.0).expect("a palette"),
            palette(&ordered, None, 0.0).expect("a palette")
        );
        // And at rest both are the identity, which is the composition being
        // done rather than skipped.
        assert_at_rest(
            &palette(&reordered, None, 0.0).expect("a palette"),
            &(0..reordered.len()).collect::<Vec<_>>(),
            &(0..reordered.len()).collect::<Vec<_>>(),
        );

        // A clip that animates only the PARENT still moves the child, which is
        // the parent chain actually being walked.
        let mut anim = clip(&ordered, 8);
        anim.joints.retain(|joint| joint.name == "root");
        anim.samples_t.truncate(8 * 3);
        anim.samples_q.truncate(8 * 4);
        anim.samples_s.truncate(8);
        let posed = palette(&reordered, Some(&anim), 0.2).expect("a palette");
        assert_ne!(posed[64..128], identity_palette()[64..128], "the child follows its parent");
    }

    /// kills: a skeleton whose joints cannot address the palette being packed
    /// anyway.
    #[test]
    fn a_skeleton_that_cannot_address_the_palette_is_refused_by_name() {
        let joint = |name: &str, index: i32, parent: Option<i32>| PdxJoint {
            name: name.into(),
            index,
            parent,
            transform_3x4: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
            rcf: None,
        };
        for (label, skeleton, fragment) in [
            (
                "an index past the engine's slots",
                vec![joint("a", XMAP13_PALETTE_JOINTS as i32, None)],
                "palette slot",
            ),
            ("a negative index", vec![joint("a", -1, None)], "negative index"),
            (
                "two joints in one slot",
                vec![joint("a", 0, None), joint("b", 0, None)],
                "claim palette slot",
            ),
            (
                "a parent that is not an earlier joint",
                vec![joint("a", 0, Some(1)), joint("b", 1, None)],
                "not an earlier joint",
            ),
            (
                "a parent this skeleton does not carry",
                vec![joint("a", 0, None), joint("b", 2, Some(1))],
                "not an earlier joint",
            ),
            (
                "a singular inverse bind",
                vec![PdxJoint {
                    name: "flat".into(),
                    index: 0,
                    parent: None,
                    transform_3x4: [0.0; 12],
                    rcf: None,
                }],
                "singular inverse-bind",
            ),
        ] {
            let error = palette(&skeleton, None, 0.0)
                .expect_err(label)
                .to_string();
            assert!(error.contains(fragment), "{label}: {error}");
        }
    }

    /// kills: a palette built from one LOD's skeleton being drawn on another's.
    ///
    /// The palette is built before the camera has picked a LOD, so it has to
    /// serve every shape. A model whose shapes disagree has no such palette,
    /// and the disagreement is invisible downstream - the backend checks only
    /// that skin arrays and palette presence agree.
    #[test]
    fn a_model_whose_shapes_disagree_on_their_skeleton_has_no_one_palette() {
        let first = skeleton(3);
        let mut different = skeleton(3);
        different[1].name = "renamed".into();
        let empty: Vec<PdxJoint> = Vec::new();

        // Shapes without a skeleton say nothing, and the ones that agree are
        // one answer however many carry it.
        let agreed =
            xmap13_model_skeleton([empty.as_slice(), first.as_slice(), first.as_slice()])
                .expect("one skeleton");
        assert_eq!(agreed, first.as_slice());
        assert!(xmap13_model_skeleton([empty.as_slice()])
            .expect("no skeleton")
            .is_empty());

        let error = xmap13_model_skeleton([first.as_slice(), different.as_slice()])
            .expect_err("two skeletons")
            .to_string();
        assert!(error.contains("different skeletons"), "{error}");
    }

    /// kills: a skeleton that does not fit the engine's buffer being packed
    /// into it anyway, silently dropping joints.
    #[test]
    fn a_skeleton_too_large_for_the_engines_palette_is_refused_by_name() {
        let bones = skeleton(XMAP13_PALETTE_JOINTS + 1);
        let error = palette(&bones, None, 0.0).expect_err("too many joints");
        let text = error.to_string();
        assert!(text.contains("51"), "{text}");
        assert!(text.contains(&XMAP13_PALETTE_JOINTS.to_string()), "{text}");
        // And exactly the engine's count still fits.
        assert!(palette(&skeleton(XMAP13_PALETTE_JOINTS), None, 0.0).is_ok());
    }

    /// A two-joint chain whose child is parented to the root and carries the
    /// root's own inverse bind, which is the shape that makes the composed
    /// rest palette `inv * (m * inv) * m` for the child.
    fn chain_of_two(transform_3x4: [f32; 12]) -> Vec<PdxJoint> {
        (0..2)
            .map(|index| PdxJoint {
                name: format!("bone{index}"),
                index,
                parent: (index > 0).then_some(0),
                transform_3x4,
                rcf: None,
            })
            .collect()
    }

    /// The matrix one slot of a packed palette holds.
    fn slot_of(palette: &[u8], slot: usize) -> Vec<f32> {
        lanes(&palette[slot * 64..slot * 64 + 64])
    }

    /// kills: an unanimated joint's skinning matrix being RECONSTRUCTED through
    /// the chain rather than copied from its parent.
    ///
    /// Two fixtures that broke every reconstruction this module has had. The
    /// first: two identical ill-conditioned joints, child under root, each
    /// inverse within 4e-6 of the identity in both products, whose composed
    /// rest palette put the child's translation lane at 0.25. The second: a
    /// root whose inverse bind scales one axis by 10,000 over a child whose own
    /// composed residual is 6e-5, where a clip that poses the root at the
    /// identity should leave the child's palette EQUAL to the root's and a
    /// composed-rest guard admitted a child off by 0.16. Neither is a
    /// tolerance question now: an unanimated joint's matrix IS its parent's,
    /// byte for byte, and the identity at a root, so both fixtures come out
    /// exact and an inverse that fed them would be the mutation these catch.
    #[test]
    fn an_unanimated_joints_skinning_matrix_is_its_parents_exactly() {
        let identical = chain_of_two([
            -0.02, -0.04, 0.04, //
            0.20, -0.50, 0.50, //
            0.00002, -0.00004, 0.00005, //
            1.0, -2.0, -8.0,
        ]);
        let ib = affine_from_3x4(identical[0].transform_3x4);
        let inverse = affine_inverse(ib).expect("each inverse passes the per-matrix residual");
        assert!(residual_from_identity(multiply(ib, inverse)).unwrap() < 1e-5);
        assert!(residual_from_identity(multiply(inverse, ib)).unwrap() < 1e-5);
        // At rest: fifty EXACT identities, not fifty near-identities.
        assert_eq!(
            palette(&identical, None, 0.0).expect("a palette"),
            xmap13_rest_palette_bytes()
        );
        // Under a clip that names only the root, the child is its root.
        let mut root_only = clip(&identical, 4);
        root_only.joints.retain(|joint| joint.name == "bone0");
        root_only.samples_t.truncate(4 * 3);
        root_only.samples_q.truncate(4 * 4);
        root_only.samples_s.truncate(4);
        let posed = palette(&identical, Some(&root_only), 0.1).expect("a palette");
        assert_eq!(slot_of(&posed, 1), slot_of(&posed, 0));
        assert_ne!(slot_of(&posed, 0), lanes(&xmap13_rest_palette_bytes()[..64]));

        // The amplifying root: an identity local at the root, an unanimated
        // child, and the two palettes must be one.
        let mut amplifying = chain_of_two([
            0.02, -0.02, -0.05, //
            0.01, -0.02, 0.02, //
            4.0, -2.0, 4.0, //
            2.0, 8.0, 8.0,
        ]);
        amplifying[0].transform_3x4 = [1.0, 0.0, 0.0, 0.0, 10000.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
        let identity_root = PdxAnimData {
            fps: 10.0,
            sample_count: 2,
            joints: vec![PdxAnimJoint {
                name: "bone0".into(),
                channels: "".into(),
                base_translation: [0.0, 0.0, 0.0],
                base_rotation_quat: [0.0, 0.0, 0.0, 1.0],
                base_scale: 1.0,
            }],
            samples_t: Vec::new(),
            samples_q: Vec::new(),
            samples_s: Vec::new(),
        };
        let posed = palette(&amplifying, Some(&identity_root), 0.0).expect("a palette");
        assert_eq!(slot_of(&posed, 1), slot_of(&posed, 0), "the child is its root, exactly");
        assert_eq!(slot_of(&posed, 0), lanes(&{
            let mut bytes = Vec::new();
            for value in affine_from_3x4(amplifying[0].transform_3x4) {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            bytes
        }), "the root posed at the identity object is its own inverse bind");
        assert_eq!(palette(&amplifying, None, 0.0).expect("a palette"), xmap13_rest_palette_bytes());

        // And an ANIMATED child under an unanimated root still composes
        // through the root's bind object, which is what that inverse is for:
        // the child's own track moves it and the root stays exactly at rest.
        let mut child_only = clip(&amplifying, 4);
        child_only.joints.retain(|joint| joint.name == "bone1");
        child_only.samples_t.drain(..3);
        child_only.samples_t.truncate(4 * 3);
        child_only.samples_q.truncate(4 * 4);
        child_only.samples_s.truncate(4);
        let posed = palette(&amplifying, Some(&child_only), 0.2).expect("a palette");
        assert_eq!(slot_of(&posed, 0), lanes(&xmap13_rest_palette_bytes()[..64]));
        assert_ne!(slot_of(&posed, 1), slot_of(&posed, 0));
        assert!(slot_of(&posed, 1).iter().all(|value| value.is_finite()));
    }

    /// The shipped weighted group this lane tests the pick against:
    /// `infantry_rifle_entity`'s `attack` in `gfx/entities/units_infantry.asset`
    /// plays `charge_rifle` at chance 2 and `charge_rifle_shoot` at chance 1
    /// (`sample_weighted_group` in
    /// `extractor/native/runs/wo_xmap13_15_producers/CLIP_JOIN_CENSUS.json`,
    /// and asserted against the install itself by
    /// `building_files.rs::the_shipped_entity_states_select_their_clips_through_the_shipped_registries`).
    const SHIPPED_WEIGHTS: [f32; 2] = [2.0, 1.0];

    /// A two-joint chain whose child is parented to the root, each with its own
    /// inverse bind.
    fn parented(root: [f32; 12], child: [f32; 12]) -> Vec<PdxJoint> {
        vec![
            PdxJoint {
                name: "bone0".into(),
                index: 0,
                parent: None,
                transform_3x4: root,
                rcf: None,
            },
            PdxJoint {
                name: "bone1".into(),
                index: 1,
                parent: Some(0),
                transform_3x4: child,
                rcf: None,
            },
        ]
    }

    /// A clip that names only the child, with no channels, whose base values
    /// are the child's bind local under `root` - which for two joints sharing
    /// a linear part is the pure translation `t_root - t_child`.
    fn child_at_bind(root: [f32; 12], child: [f32; 12]) -> PdxAnimData {
        PdxAnimData {
            fps: 10.0,
            sample_count: 2,
            joints: vec![PdxAnimJoint {
                name: "bone1".into(),
                channels: "".into(),
                base_translation: [root[9] - child[9], root[10] - child[10], root[11] - child[11]],
                base_rotation_quat: [0.0, 0.0, 0.0, 1.0],
                base_scale: 1.0,
            }],
            samples_t: Vec::new(),
            samples_q: Vec::new(),
            samples_s: Vec::new(),
        }
    }

    /// A three-joint chain root -> middle -> grandchild with its own inverse
    /// binds.
    fn chain_of_three(root: [f32; 12], middle: [f32; 12], grandchild: [f32; 12]) -> Vec<PdxJoint> {
        let mut joints = parented(root, middle);
        joints.push(PdxJoint {
            name: "bone2".into(),
            index: 2,
            parent: Some(1),
            transform_3x4: grandchild,
            rcf: None,
        });
        joints
    }

    /// A clip naming `name` alone, with no channels, at the given local.
    fn one_joint_at(name: &str, translation: [f32; 3], scale: f32) -> PdxAnimJoint {
        PdxAnimJoint {
            name: name.into(),
            channels: "".into(),
            base_translation: translation,
            base_rotation_quat: [0.0, 0.0, 0.0, 1.0],
            base_scale: scale,
        }
    }

    fn clip_of(joints: Vec<PdxAnimJoint>) -> PdxAnimData {
        PdxAnimData {
            fps: 10.0,
            sample_count: 2,
            joints,
            samples_t: Vec::new(),
            samples_q: Vec::new(),
            samples_s: Vec::new(),
        }
    }

    /// kills: a pose whose f32 palette leaves the f64 evaluation of the same
    /// inputs by more than XMAP13_PALETTE_F32_TOLERANCE and is drawn anyway.
    ///
    /// The guard IS that difference, so what this pins is that it measures the
    /// pose actually built: four fixtures that escaped four earlier proxies
    /// are refused with their error measured, the same chains are exact at
    /// rest, a clip at exact bind on a chain that only deforms one ULP away
    /// is admitted, and the boundary is pinned from both sides so loosening
    /// to 1e-3 or tightening to 1e-5 fails a test.
    #[test]
    fn a_pose_whose_f32_palette_leaves_its_f64_reference_is_refused() {
        // Round 7: shared 2e-5-scale linear part, an animated child at its
        // exact bind translation, lane 14 at 0.125.
        let linear = [-0.02, -0.04, 0.04, 0.20, -0.50, 0.50, 0.00002, -0.00004, 0.00005];
        let r7_root = [linear[0], linear[1], linear[2], linear[3], linear[4], linear[5], linear[6], linear[7], linear[8], 1.0, -2.0, -8.0];
        let r7_child = [linear[0], linear[1], linear[2], linear[3], linear[4], linear[5], linear[6], linear[7], linear[8], 1.0, -3.0, -8.0];
        let r7 = parented(r7_root, r7_child);
        // Round 8: a bind local whose f64 rounding differs from the clip's own
        // f32 translation by one ULP, 4.9e-4 through a bind-pose proxy.
        let linear = [
            -0.9816235899925232, -1.1499571800231934, 0.005273930728435516,
            -0.008961031213402748, -0.0013505653478205204, 0.009216138161718845,
            0.002808044897392392, 0.004940689541399479, 0.0008221613243222237,
        ];
        let r8_root = [linear[0], linear[1], linear[2], linear[3], linear[4], linear[5], linear[6], linear[7], linear[8], 14.99167537689209, 9.626444816589355, 8.967620849609375];
        let r8_child = [linear[0], linear[1], linear[2], linear[3], linear[4], linear[5], linear[6], linear[7], linear[8], -10.619429588317871, -10.902976989746094, -13.760820388793945];
        let r8_rounding = parented(r8_root, r8_child);
        // Round 8: a chain exact at bind (1.9e-6) that amplifies a one-ULP
        // authored motion ten times.
        let linear = [
            0.9549171328544617, -0.0004958260105922818, 0.0050369203090667725,
            -4.4471259117126465, -0.005236878991127014, 7.975625038146973,
            0.008201169781386852, -0.004830608144402504, 0.006357064004987478,
        ];
        let amp_root = [linear[0], linear[1], linear[2], linear[3], linear[4], linear[5], linear[6], linear[7], linear[8], 5.25, -13.25, 6.75];
        let amp_child = [linear[0], linear[1], linear[2], linear[3], linear[4], linear[5], linear[6], linear[7], linear[8], 9.25, -12.25, -6.25];
        let r8_motion = parented(amp_root, amp_child);
        let one_ulp_off = clip_of(vec![one_joint_at("bone1", [-4.0, -1.0000001192092896, 13.0], 1.0)]);
        // Round 8: an animated root at uniform scale 10, an unanimated middle,
        // a grandchild at bind - the composition no pairwise check sees.
        let under_linear = [0.05, 0.03, 0.04, 1.0, 5.0, 5.0, -0.04, -0.03, -0.02];
        let under_root = [under_linear[0], under_linear[1], under_linear[2], under_linear[3], under_linear[4], under_linear[5], under_linear[6], under_linear[7], under_linear[8], 8.0, -1.0, 8.0];
        let under_child = [under_linear[0], under_linear[1], under_linear[2], under_linear[3], under_linear[4], under_linear[5], under_linear[6], under_linear[7], under_linear[8], 0.5, 2.0, -2.0];
        let identity_root = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
        let r8_deep = chain_of_three(identity_root, under_root, under_child);
        let deep_clip = clip_of(vec![
            one_joint_at("bone0", [0.0, 0.0, 0.0], 10.0),
            one_joint_at(
                "bone2",
                [under_root[9] - under_child[9], under_root[10] - under_child[10], under_root[11] - under_child[11]],
                1.0,
            ),
        ]);

        // Every per-matrix residual passes, which is what made these the
        // guard's cases and not the inverse's.
        for skeleton in [&r7, &r8_rounding, &r8_motion, &r8_deep] {
            for joint in skeleton {
                let ib = affine_from_3x4(joint.transform_3x4);
                let inverse = affine_inverse(ib).expect("each inverse passes the per-matrix residual");
                assert!(residual_from_identity(multiply(ib, inverse)).unwrap() <= 1e-4);
                assert!(residual_from_identity(multiply(inverse, ib)).unwrap() <= 1e-4);
            }
        }

        // REFUSED, each with the error it was found at.
        for (label, skeleton, anim, at_least, at_most) in [
            ("round 7 at bind", &r7, child_at_bind(r7_root, r7_child), 0.1f32, 0.2),
            ("round 8 rounding", &r8_rounding, child_at_bind(r8_root, r8_child), 1e-4, 5e-3),
            ("round 8 one ULP", &r8_motion, one_ulp_off.clone(), 1e-4, 5e-3),
            ("round 8 deep chain", &r8_deep, deep_clip.clone(), 1e-4, 5e-3),
        ] {
            let error = forward_error(skeleton, Some(&anim), 0.0).expect(label);
            assert!(
                error >= at_least && error <= at_most,
                "{label}: the f32 palette is {error} from its f64 reference"
            );
            let refusal = palette(skeleton, Some(&anim), 0.0)
                .expect_err(label)
                .to_string();
            assert!(refusal.contains("in f64") && refusal.contains("cannot be built"), "{label}: {refusal}");
        }
        // ...and the SAME chains at rest are exact, because nothing is
        // reconstructed there.
        for skeleton in [&r7, &r8_rounding, &r8_motion, &r8_deep] {
            assert_eq!(forward_error(skeleton, None, 0.0).expect("rest"), 0.0);
            assert_eq!(palette(skeleton, None, 0.0).expect("a palette"), xmap13_rest_palette_bytes());
        }

        // ADMITTED, with the boundary pinned: the one-ULP chain at exact bind
        // (1.9e-6), the round-8 middle/grandchild pair on its own at bind
        // (between 1e-5 and the tolerance), and a well-conditioned skeleton
        // under a full clip.
        let at_bind = child_at_bind(amp_root, amp_child);
        let error = forward_error(&r8_motion, Some(&at_bind), 0.0).expect("finite");
        assert!(error < 1e-5, "at exact bind the chain is {error} off");
        assert!(palette(&r8_motion, Some(&at_bind), 0.0).is_ok());
        let pair = parented(under_root, under_child);
        let error = forward_error(&pair, Some(&child_at_bind(under_root, under_child)), 0.0)
            .expect("finite");
        assert!(
            error > 1e-5 && error <= XMAP13_PALETTE_F32_TOLERANCE,
            "the just-under fixture must sit between 1e-5 and the tolerance: {error}"
        );
        assert!(palette(&pair, Some(&child_at_bind(under_root, under_child)), 0.0).is_ok());
        // A well-conditioned skeleton under the synthetic full clip, whose
        // translations run to three hundred units: the f32 translation lanes
        // are a few 1e-4 off, which on a mesh of unit extent is a refusal and
        // on a mesh whose vertices reach three hundred units is a hundredth of
        // one percent of its size and admitted. The extent is the mesh's, not
        // the lane's.
        let bones = skeleton(4);
        let full = clip(&bones, 30);
        let at_unit = forward_error(&bones, Some(&full), 1.5).expect("finite");
        assert!(at_unit > XMAP13_PALETTE_F32_TOLERANCE, "{at_unit}");
        assert!(palette(&bones, Some(&full), 1.5).is_err());
        let at_size = xmap13_palette_forward_error(&bones, Some(&full), 1.5, 300.0).expect("finite");
        assert!(at_size <= XMAP13_PALETTE_F32_TOLERANCE, "{at_size}");
        assert!(xmap13_skin_palette(&bones, Some(&full), 1.5, 300.0).is_ok());
        assert!(xmap13_skin_palette(&bones, Some(&full), 1.5, f32::NAN).is_err());
    }

    /// kills: a lane-relative allowance admitting a wrong VERTEX.
    ///
    /// Two lanes of 2^24 that cancel on a unit vertex: the f32 palette is
    /// wrong by about 2 on each and moves that vertex by 2, while every lane
    /// is within 1e-4 of its own magnitude. The bound is on the vertex, so it
    /// is refused at unit extent - and it is a refusal of this guard alone,
    /// every per-matrix residual being exactly zero.
    #[test]
    fn the_bound_is_the_vertex_displacement_and_not_the_lane() {
        let t = 16_777_216.0f32;
        let cancelling = vec![PdxJoint {
            name: "bone0".into(),
            index: 0,
            parent: None,
            transform_3x4: [t, t, 0.0, t, 2.0 * t, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
            rcf: None,
        }];
        let ib = affine_from_3x4(cancelling[0].transform_3x4);
        let inverse = affine_inverse(ib).expect("exactly invertible");
        assert_eq!(residual_from_identity(multiply(ib, inverse)), Some(0.0));
        assert_eq!(residual_from_identity(multiply(inverse, ib)), Some(0.0));
        let turn = clip_of(vec![PdxAnimJoint {
            name: "bone0".into(),
            channels: "".into(),
            base_translation: [t, -t, 0.0],
            base_rotation_quat: [0.0, 0.0, 0.5, 0.5],
            base_scale: 1.0,
        }]);
        let error = forward_error(&cancelling, Some(&turn), 0.0).expect("finite");
        assert!(error >= 1.0, "the vertex is moved by {error}");
        assert!(palette(&cancelling, Some(&turn), 0.0).is_err());
        // Not by making the mesh large: a vertex two units out is still
        // wrong by two.
        assert!(xmap13_skin_palette(&cancelling, Some(&turn), 0.0, 2.0).is_err());
        assert_eq!(palette(&cancelling, None, 0.0).expect("at rest"), xmap13_rest_palette_bytes());
    }

    /// kills: the f64 reference composing a rotation, a normalisation or a
    /// fallback differently from production, which would refuse a valid pose
    /// or admit a wrong one as "f32 error".
    ///
    /// Every earlier fixture rotated by the identity quaternion, so a sign
    /// flipped in the reference's rotation lanes passed everything. Each
    /// channel set here is composed both ways and the two must agree to f32
    /// rounding: a z-axis quarter turn, an x-axis turn, an UNNORMALISED
    /// quaternion, a DEGENERATE one (the identity fallback), a non-finite
    /// scale (the unit fallback) and a non-unit scale.
    #[test]
    fn the_f64_reference_composes_rotation_and_fallbacks_as_production_does() {
        let channels: [([f32; 3], [f32; 4], f32); 7] = [
            ([1.5, -2.0, 3.0], [0.0, 0.0, 0.70710677, 0.70710677], 1.0),
            ([0.0, 0.0, 0.0], [0.5, 0.0, 0.0, 0.8660254], 1.0),
            ([0.25, 0.25, 0.25], [0.0, 0.0, 1.0, 1.0], 1.0),
            ([0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 0.0], 1.0),
            ([0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0], f32::NAN),
            ([0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0], f32::INFINITY),
            ([-3.0, 0.5, 2.0], [0.3, -0.4, 0.5, 0.7], 2.5),
        ];
        for (translation, quat, scale) in channels {
            let production = compose(translation, quat, scale);
            let reference = compose_f64(translation, quat, scale);
            for lane in 0..16 {
                let off = (f64::from(production[lane]) - reference[lane]).abs();
                assert!(
                    off <= 4e-7 * reference[lane].abs().max(1.0),
                    "lane {lane} of {translation:?} {quat:?} {scale}: {} vs {}",
                    production[lane],
                    reference[lane]
                );
            }
            // A real rotation is really there, so the lanes a sign flip
            // would move are not zero.
            if quat[2] != 0.0 && quat.iter().any(|value| *value != 0.0) && quat != [0.0, 0.0, 0.0, 1.0] {
                assert!(reference[1] != 0.0 && reference[4] != 0.0, "{quat:?}");
            }
        }
        // And a clip that ROTATES is admitted: the skeleton fixture under a
        // quarter turn per sample is well within the tolerance.
        let bones = skeleton(3);
        let mut turning = clip(&bones, 8);
        for sample in 0..8usize {
            let angle = sample as f32 * std::f32::consts::FRAC_PI_4;
            for joint in 0..bones.len() {
                let at = (sample * bones.len() + joint) * 4;
                turning.samples_q[at..at + 4]
                    .copy_from_slice(&[0.0, (angle / 2.0).sin(), 0.0, (angle / 2.0).cos()]);
                let t = (sample * bones.len() + joint) * 3;
                turning.samples_t[t..t + 3].copy_from_slice(&[0.5, -0.25, 1.0]);
                turning.samples_s[sample * bones.len() + joint] = 1.0;
            }
        }
        assert!(forward_error(&bones, Some(&turning), 0.35).expect("finite") <= XMAP13_PALETTE_F32_TOLERANCE);
        assert!(palette(&bones, Some(&turning), 0.35).is_ok());
    }

    /// kills: a non-finite lane reaching the packer because a NaN compares
    /// false against every allowance.
    #[test]
    fn a_non_finite_pose_is_refused_rather_than_packed() {
        let bones = skeleton(1);
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let broken = clip_of(vec![one_joint_at("bone0", [value, 0.0, 0.0], 1.0)]);
            let error = palette(&bones, Some(&broken), 0.0)
                .expect_err("a non-finite lane is not a pose")
                .to_string();
            assert!(error.contains("cannot be built"), "{error}");
        }
    }

    /// kills: a clip opened somewhere other than beside the table that declares
    /// it, which is the only place the install puts one.
    #[test]
    fn a_declared_clip_lives_beside_its_table() {
        assert_eq!(
            authored_clip_logical_path("gfx/models/units/trains/flat_wagon.asset", "wagon_1_idle.anim"),
            "gfx/models/units/trains/wagon_1_idle.anim"
        );
        assert_eq!(
            authored_clip_logical_path("gfx\\models\\animation.asset", "a.anim"),
            "gfx/models/a.anim"
        );
        assert_eq!(authored_clip_logical_path("animation.asset", "a.anim"), "a.anim");
    }
}
