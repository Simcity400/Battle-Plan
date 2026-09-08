//! Retained pdx_anim bounds for the exact a729 build.
//! Source custody: extractor/native/runs/m9_animated_bounds/export_packet.py.
use crate::pdx_mesh::{PdxAnimData, PdxNode, PdxValues};
use std::sync::Arc;

type BoundsResult<T> = std::result::Result<T, String>;

#[derive(Clone)]
pub(crate) struct NativeBoundsSkeleton {
    parents: Vec<u8>,
    radii: Vec<f32>,
    inverse_bind: Vec<[f32; 16]>,
    scale: f32,
}

fn floats<'a>(node: &'a PdxNode, name: &str) -> BoundsResult<Option<&'a [f32]>> {
    match node.property(name).map(|p| &p.values) {
        None => Ok(None),
        Some(PdxValues::F32(v)) => Ok(Some(v)),
        _ => Err(format!("animated bounds: {name} is not float data")),
    }
}
fn ints<'a>(node: &'a PdxNode, name: &str) -> BoundsResult<Option<&'a [i32]>> {
    match node.property(name).map(|p| &p.values) {
        None => Ok(None),
        Some(PdxValues::I32(v)) => Ok(Some(v)),
        _ => Err(format!("animated bounds: {name} is not integer data")),
    }
}
fn named(node: &PdxNode, name: &str) -> bool { node.name.split('\0').next() == Some(name) }

impl NativeBoundsSkeleton {
    pub(crate) fn load(child: &PdxNode, skeleton: &PdxNode, scale: f32) -> BoundsResult<Self> {
        if !scale.is_finite() || scale <= 0.0 || skeleton.children.is_empty() {
            return Err("animated bounds: invalid skeleton/scale".into());
        }
        let mut parents = Vec::new();
        let mut inverse_bind = Vec::new();
        for (ordinal, joint) in skeleton.children.iter().enumerate() {
            let tx = floats(joint, "tx")?.ok_or("animated bounds: joint tx unavailable")?;
            let ix = ints(joint, "ix")?.ok_or("animated bounds: joint ix unavailable")?;
            if tx.len() != 12 || ix.len() != 1 || tx.iter().any(|v| !v.is_finite()) {
                return Err("animated bounds: malformed joint".into());
            }
            // 142250010 consumes source order; ix is read but does not reorder joints.
            let parent = match ints(joint, "pa")? {
                None => 0xff,
                Some([parent]) => *parent as u8,
                _ => return Err("animated bounds: malformed parent".into()),
            };
            if parent != 0xff && usize::from(parent) >= ordinal {
                return Err("animated bounds: parent does not precede child".into());
            }
            parents.push(parent);
            // 142250010 scales only tx translation; 1423997a0 copies all twelve
            // lanes to joint+8. 142399990 expands these rows before skinning.
            inverse_bind.push([tx[0], tx[1], tx[2], 0.0,
                tx[3], tx[4], tx[5], 0.0, tx[6], tx[7], tx[8], 0.0,
                tx[9] * scale, tx[10] * scale, tx[11] * scale, 1.0]);
        }
        let mut radii = vec![0.0f32; parents.len()];
        // 142250010 selects the first mesh under the construction's owning child.
        if let Some(mesh) = child.children.iter().find(|node| named(node, "mesh")) {
            if let (Some(skin), Some(positions)) =
                (mesh.children.iter().find(|node| named(node, "skin")), floats(mesh, "p")?)
            {
                if let (Some(indices), Some(weights)) = (ints(skin, "ix")?, floats(skin, "w")?) {
                    if positions.len() % 3 != 0 || indices.len() != positions.len() / 3 * 4
                        || weights.len() != indices.len() {
                        return Err("animated bounds: skin/position lengths differ".into());
                    }
                    for (vertex, position) in positions.chunks_exact(3).enumerate() {
                        let p: [f32; 3] = std::array::from_fn(|i| position[i] * scale);
                        if p.iter().any(|v| !v.is_finite()) {
                            return Err("animated bounds: scaled position is nonfinite".into());
                        }
                        // The loader copies its first tx row three times before 1424e7ba0.
                        // Its singular inverse leaves the zero-initialized radius center intact.
                        let d = [-p[0], -p[1], -p[2]];
                        let squared = (d[0] * d[0] + d[1] * d[1]) + d[2] * d[2];
                        for influence in vertex * 4..vertex * 4 + 4 {
                            let index = indices[influence];
                            if index >= 0 && (index as usize) < radii.len() && weights[influence] > 0.0 {
                                let radius = &mut radii[index as usize];
                                if *radius * *radius < squared { *radius = squared.sqrt(); }
                            }
                        }
                    }
                }
            }
        }
        Ok(Self { parents, radii, inverse_bind, scale })
    }
}

#[derive(Clone)]
struct ClipClock {
    entry: u64,
    clip: Arc<PdxAnimData>,
    phase: f64,
    speed: f64,
    looping: bool,
}
impl ClipClock {
    fn duration(&self) -> f64 {
        crate::production_features::xmap13_pose::xmap13_clip_length_seconds(&self.clip)
    }
    fn advance(&mut self, delta: f64) {
        self.phase += delta * self.speed;
        if self.looping {
            self.phase %= self.duration();
            if self.phase < 0.0 { self.phase += self.duration(); }
        } else {
            self.phase = self.phase.clamp(0.0, self.duration());
        }
    }
}

#[derive(Clone)]
pub(crate) struct NativeAnimatedBounds {
    next_entry: u64,
    skeleton: Arc<NativeBoundsSkeleton>,
    current: Option<ClipClock>,
    previous: Option<ClipClock>,
    blend_elapsed: f64,
    manager_time: f64,
    blend_started: f64,
    blend_duration: f64,
    eligible: bool,
    bounds: BoundsResult<[[f32; 3]; 2]>,
    evaluated_pose: Option<NativeEvaluatedPose>,
}

/// Only an eligible native evaluation establishes these matrices. In particular,
/// constructor allocation does not establish a rest palette (142399610).
#[derive(Clone)]
pub(crate) struct NativeEvaluatedPose {
    pub(crate) entry: u64,
    pub(crate) clip: Arc<PdxAnimData>,
    pub(crate) phase: f64,
    pub(crate) speed: f64,
    pub(crate) previous: Option<(Arc<PdxAnimData>, f64)>,
    pub(crate) previous_entry: Option<u64>,
    pub(crate) blend_factor: Option<f32>,
    pub(crate) palette: Vec<[f32; 16]>,
}
impl NativeAnimatedBounds {
    pub(crate) fn new(skeleton: NativeBoundsSkeleton) -> Self {
        Self { next_entry: 0, skeleton: Arc::new(skeleton), current: None, previous: None,
            blend_elapsed: 0.0, manager_time: 0.0, blend_started: 0.0,
            blend_duration: 0.0, eligible: true, bounds: Ok([[0.0; 3]; 2]),
            evaluated_pose: None }
    }
    pub(crate) fn set_eligible(&mut self, eligible: bool) { self.eligible = eligible; }
    pub(crate) fn current_entry(&self) -> Option<u64> { self.current.as_ref().map(|clock| clock.entry) }
    pub(crate) fn retained_entries(&self) -> [Option<u64>; 4] {
        [self.current_entry(), self.previous.as_ref().map(|clock| clock.entry),
            self.evaluated_pose.as_ref().map(|pose| pose.entry),
            self.evaluated_pose.as_ref().and_then(|pose| pose.previous_entry)]
    }
    pub(crate) fn read(&self) -> BoundsResult<[[f32; 3]; 2]> { self.bounds.clone() }
    pub(crate) fn read_pose(&self) -> BoundsResult<NativeEvaluatedPose> {
        self.bounds.as_ref().map_err(Clone::clone)?;
        self.evaluated_pose.clone().ok_or_else(||
            "animated pose: no eligible native evaluation has written the palette".into())
    }

    pub(crate) fn play(&mut self, clip: Option<Arc<PdxAnimData>>, phase: f64,
        speed: f64, looping: bool, blend: f32) {
        let Some(clip) = clip else { self.current = None; return; };
        if clip.joints.len() != self.skeleton.parents.len() {
            self.current = None;
            return;
        }
        if clip.sample_count < 1 || !clip.fps.is_finite() || clip.fps <= 0.0
            || !phase.is_finite() || !speed.is_finite() || !blend.is_finite() {
            self.bounds = Err("animated bounds: invalid clip clock".into());
            return;
        }
        let was_active = self.current.is_some();
        if was_active && blend > 0.0 {
            self.previous = self.current.clone();
            self.blend_elapsed = 0.0;
            self.blend_started = self.manager_time;
            self.blend_duration = f64::from(blend);
        }
        let entry = self.next_entry;
        let Some(next_entry) = entry.checked_add(1) else {
            self.bounds = Err("animated pose: retained entry identity exhausted".into()); return;
        };
        self.next_entry = next_entry;
        let mut clock = ClipClock { entry, clip, phase, speed, looping };
        clock.advance(0.0);
        self.current = Some(clock);
        // 14239b250 evaluates a first play immediately; replacing an active clip
        // leaves evaluation to the update while preserving the existing bounds.
        if !was_active && self.eligible {
            self.blend_elapsed = self.manager_time - self.blend_started;
            if self.blend_elapsed >= self.blend_duration { self.previous = None; }
            self.evaluate();
        }
    }

    pub(crate) fn advance(&mut self, delta: f64, entity_speed: f64) {
        if !delta.is_finite() || !entity_speed.is_finite() {
            self.bounds = Err("animated bounds: nonfinite update clock".into()); return;
        }
        self.manager_time += delta;
        let Some(current) = &mut self.current else { return; };
        current.advance(delta * entity_speed);
        if let Some(previous) = &mut self.previous {
            self.blend_elapsed = self.manager_time - self.blend_started;
            if self.blend_elapsed < self.blend_duration {
                previous.advance(delta * entity_speed);
            } else {
                self.previous = None;
            }
        }
        if self.eligible { self.evaluate(); }
    }

    fn evaluate(&mut self) {
        let evaluated = (|| {
            let current = self.current.as_ref().ok_or("animated bounds: no active clip")?;
            let now = sample(current)?;
            let previous = self.previous.as_ref().map(sample).transpose()?;
            let blend = (self.blend_elapsed / self.blend_duration).clamp(0.0, 1.0) as f32;
            let mut objects = Vec::<[f32; 16]>::new();
            let mut bounds = [[f32::MAX; 3], [-f32::MAX; 3]];
            for (i, mut pose) in now.into_iter().enumerate() {
                if let Some(previous) = &previous {
                    let old = previous[i];
                    pose.translation = std::array::from_fn(|axis|
                        pose.translation[axis] * blend + old.translation[axis] * (1.0 - blend));
                    pose.rotation = slerp(old.rotation, pose.rotation, blend)?;
                    pose.scale = pose.scale * blend + old.scale * (1.0 - blend);
                }
                // 14239a015..145 and the first-play blend branch multiply the
                // blended translation by blended joint scale. The unblended
                // branch instead multiplies translation by skeleton scale.
                let translation_scale = if previous.is_some() { pose.scale } else { self.skeleton.scale };
                let mut object = pose.matrix(translation_scale);
                let parent = self.skeleton.parents[i];
                if parent != 0xff { object = multiply(object, objects[usize::from(parent)]); }
                let radius = self.skeleton.radii[i];
                for axis in 0..3 {
                    let near = object[12 + axis] - radius;
                    let far = radius + object[12 + axis];
                    bounds[0][axis] = if bounds[0][axis] < near { bounds[0][axis] } else { near };
                    bounds[1][axis] = if bounds[1][axis] > far { bounds[1][axis] } else { far };
                }
                objects.push(object);
            }
            let palette = objects.into_iter().enumerate().map(|(i, object)|
                multiply(self.skeleton.inverse_bind[i], object)).collect();
            Ok((bounds, NativeEvaluatedPose {
                entry: current.entry,
                clip: current.clip.clone(), phase: current.phase, speed: current.speed,
                previous: self.previous.as_ref().map(|clock| (clock.clip.clone(), clock.phase)),
                previous_entry: self.previous.as_ref().map(|clock| clock.entry),
                blend_factor: previous.as_ref().map(|_| blend), palette,
            }))
        })();
        match evaluated {
            Ok((bounds, pose)) => {
                self.bounds = Ok(bounds);
                self.evaluated_pose = Some(pose);
            }
            Err(reason) => self.bounds = Err(reason),
        }
    }
}

#[derive(Clone, Copy)]
struct JointPose { translation: [f32; 3], rotation: [f32; 4], scale: f32 }
impl JointPose {
    fn matrix(self, translation_scale: f32) -> [f32; 16] {
        let [x,y,z,w] = self.rotation;
        let twice = |x: f32| x + x;
        let r = [1.0-twice(y*y+z*z), twice(w*z+y*x), twice(x*z-y*w),
            twice(y*x-w*z), 1.0-twice(x*x+z*z), twice(w*x+y*z),
            twice(y*w+x*z), twice(y*z-w*x), 1.0-twice(x*x+y*y)];
        [r[0]*self.scale,r[1]*self.scale,r[2]*self.scale,0.0,
         r[3]*self.scale,r[4]*self.scale,r[5]*self.scale,0.0,
         r[6]*self.scale,r[7]*self.scale,r[8]*self.scale,0.0,
         translation_scale*self.translation[0],translation_scale*self.translation[1],
         translation_scale*self.translation[2],1.0]
    }
}
fn multiply(local: [f32;16], parent: [f32;16]) -> [f32;16] {
    std::array::from_fn(|i| {
        let row = (i/4)*4;
        let col = i%4;
        ((local[row+1]*parent[4+col] + local[row]*parent[col])
            + local[row+2]*parent[8+col]) + local[row+3]*parent[12+col]
    })
}

fn sample(clock: &ClipClock) -> BoundsResult<Vec<JointPose>> {
    let clip = &clock.clip;
    let step = 1.0 / f64::from(clip.fps);
    let raw_frame = (clock.phase / step) as i32;
    let mix = ((clock.phase - f64::from(raw_frame)*step)/step).clamp(0.0,1.0) as f32;
    let first = if raw_frame == clip.sample_count { 0 } else { raw_frame };
    let second = if first+1 == clip.sample_count { 0 } else { first+1 };
    if first < 0 || second < 0 { return Err("animated bounds: negative sample".into()); }
    let mut channel_ordinal = [0usize;3];
    let counts: [usize;3] = ['t','q','s'].map(|flag| clip.joints.iter()
        .filter(|joint| joint.channels.contains(flag)).count());
    let mut poses = Vec::new();
    for joint in &clip.joints {
        let mut pose = JointPose { translation: joint.base_translation,
            rotation: joint.base_rotation_quat, scale: joint.base_scale };
        for (channel, flag) in ['t','q','s'].into_iter().enumerate() {
            if !joint.channels.contains(flag) { continue; }
            let width = [3,4,1][channel];
            let data = [&clip.samples_t, &clip.samples_q, &clip.samples_s][channel];
            let values = |frame: i32| -> BoundsResult<&[f32]> {
                let at = (frame as usize * counts[channel] + channel_ordinal[channel])*width;
                data.get(at..at+width).ok_or_else(|| "animated bounds: missing sample channels".into())
            };
            let a = values(first)?;
            let b = values(second)?;
            match channel {
                0 => pose.translation = std::array::from_fn(|i| (1.0-mix)*a[i]+mix*b[i]),
                1 => pose.rotation = slerp(a.try_into().unwrap(), b.try_into().unwrap(), mix)?,
                _ => pose.scale = (1.0-mix)*a[0]+mix*b[0],
            }
            channel_ordinal[channel] += 1;
        }
        poses.push(pose);
    }
    Ok(poses)
}

// 14254e038 positive acosf domain reached after the slerp sign correction.
fn acos_positive(x: f32) -> BoundsResult<f32> {
    if !x.is_finite() || !(0.0..=1.0).contains(&x) { return Err("animated bounds: acos domain".into()); }
    let exponent = (x.to_bits() >> 23) & 255;
    if exponent <= 100 { return Ok(f32::from_bits(0x3fc90fdb)); }
    if x == 1.0 { return Ok(0.0); }
    let z = if exponent < 126 { x*x } else { (1.0-x)*0.5 };
    let f = |bits| f32::from_bits(bits);
    let p = ((((f(0xbc5b3fe1)-z*f(0x3b81ce6b))*z-f(0x3d678bdd))*z+f(0x3e3c94dc))*z)
        /(f(0x3f8d6fa5)-z*f(0x3f561f0d));
    if exponent < 126 {
        Ok((f64::from_bits(0x3ff921fb54442d18)
            - (f64::from(x) - (f64::from_bits(0x3c91a62633145c07)-f64::from(p*x)))) as f32)
    } else {
        let root = z.sqrt();
        let high = f32::from_bits(root.to_bits() & 0xffff0000);
        let tail = (z-high*high)/(high+root);
        Ok((((root+root)*p+tail)+tail)+high*2.0)
    }
}
fn slerp(a: [f32;4], mut b: [f32;4], mix: f32) -> BoundsResult<[f32;4]> {
    let mut dot = ((b[1]*a[1]+b[0]*a[0])+b[2]*a[2])+b[3]*a[3];
    if dot < 0.0 { dot = -dot; b = b.map(|v| -v); }
    if dot <= f32::from_bits(0x3f7fdf3b) {
        use portable_render_stream::exact_math::{native_cos, native_sin, viewer_uses_fma};
        let angle = acos_positive(dot)?;
        let fma = viewer_uses_fma();
        let sin = |x| native_sin(x, fma).map_err(|e| format!("animated bounds: sine {e:?}"));
        let right = sin(angle*mix)? / sin(angle)?;
        let left = native_cos(angle*mix, fma).map_err(|e| format!("animated bounds: cosine {e:?}"))? - right*dot;
        Ok(std::array::from_fn(|i| a[i]*left+b[i]*right))
    } else {
        let q: [f32;4] = std::array::from_fn(|i| (b[i]-a[i])*mix+a[i]);
        let length = (((q[0]*q[0]+q[1]*q[1])+q[2]*q[2])+q[3]*q[3]).sqrt();
        Ok(if length == 0.0 { [0.0;4] } else { q.map(|v| v/length) })
    }
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::*;
    std::arch::global_asm!(include_str!("native_animated_bounds_oracle.asm"));
    // Instruction composition reuses exact_math's separately tested CRT leaves.
    // It cannot independently detect a defect shared with those leaves.
    #[no_mangle]
    extern "win64" fn m9_oracle_native_sin(value: f32) -> f32 {
        use portable_render_stream::exact_math::{native_sin, viewer_uses_fma};
        native_sin(value, viewer_uses_fma()).expect("oracle sine domain")
    }
    #[no_mangle]
    extern "win64" fn m9_oracle_native_cos(value: f32) -> f32 {
        use portable_render_stream::exact_math::{native_cos, viewer_uses_fma};
        native_cos(value, viewer_uses_fma()).expect("oracle cosine domain")
    }
    extern "win64" {
        fn m9_oracle_142399990(object: *mut u8);
        fn m9_oracle_14239b250(object: *mut u8, clip: *mut u8, play: *const u8) -> u8;
        fn m9_oracle_14254e038(value: f32) -> f32;
        fn m9_oracle_1424e7f50(quaternion: *const f32, output: *mut f32) -> *mut f32;
        fn m9_oracle_1424e8d50(first: *const f32, output: *mut f32, mix: f32, second: *mut f32) -> *mut f32;
        fn m9_oracle_radius_center(joint: *mut u8);
        fn m9_oracle_clip_duration(clip: *mut u8);
    }
    #[repr(C, align(16))]
    struct Block<const N: usize>([u8;N]);
    #[repr(C, align(16))]
    struct AlignedFloats<const N: usize>([f32;N]);
    impl<const N: usize> Block<N> {
        fn new() -> Self { Self([0;N]) }
        fn u32(&mut self, offset: usize, value: u32) { self.0[offset..offset+4].copy_from_slice(&value.to_le_bytes()); }
        fn f32(&mut self, offset: usize, value: f32) { self.u32(offset,value.to_bits()); }
        fn f64(&mut self, offset: usize, value: f64) { self.0[offset..offset+8].copy_from_slice(&value.to_le_bytes()); }
        fn ptr<T>(&mut self, offset: usize, value: *const T) { self.0[offset..offset+8].copy_from_slice(&(value as usize).to_le_bytes()); }
        fn floats(&mut self, offset: usize, values: &[f32]) { for (i,value) in values.iter().enumerate() { self.f32(offset+4*i,*value); } }
        fn bounds(&self) -> [[f32;3];2] {
            std::array::from_fn(|row| std::array::from_fn(|axis|
                f32::from_le_bytes(self.0[0x28+row*12+axis*4..0x2c+row*12+axis*4].try_into().unwrap())))
        }
    }
    fn clip() -> Arc<PdxAnimData> {
        Arc::new(PdxAnimData { fps:2.0, sample_count:2,
            joints:vec![crate::pdx_mesh::PdxAnimJoint { name:"root".into(), channels:"t".into(),
                base_translation:[0.,1.,0.],base_rotation_quat:[0.,0.,0.,1.],base_scale:1.0 }],
            samples_t:vec![0.,1.,0.,0.,2.,0.],samples_q:vec![],samples_s:vec![] })
    }
    fn retained() -> NativeAnimatedBounds {
        NativeAnimatedBounds::new(NativeBoundsSkeleton { parents:vec![255],radii:vec![3.0],
            inverse_bind:vec![identity()],scale:3.0 })
    }
    fn identity() -> [f32;16] {
        [1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.,0.,0.,0.,0.,1.]
    }
    #[test]
    fn clip_duration_matches_native_loader_instructions() {
        for fps in [1.0, 3.0, 15.0, 29.97, 60.0, 0.0001] {
            for samples in [1, 2, 4, 1001, i32::MAX] {
                let mut raw = Block::<128>::new();
                raw.u32(4, samples as u32);
                raw.f32(8, fps);
                unsafe { m9_oracle_clip_duration(raw.0.as_mut_ptr()); }
                let expected = u64::from_le_bytes(raw.0[0x18..0x20].try_into().unwrap());
                let mut animation = (*clip()).clone();
                animation.fps = fps;
                animation.sample_count = samples;
                assert_eq!(crate::production_features::xmap13_pose::xmap13_clip_length_seconds(&animation)
                    .to_bits(), expected, "fps={fps} samples={samples}");
            }
        }
    }
    fn oracle(phase: f64, elapsed: f64, eligible: bool, initial_play: bool) -> [[f32;3];2] {
        oracle_fields(phase,elapsed,eligible,initial_play,false)
    }
    fn oracle_fields(phase: f64, elapsed: f64, eligible: bool, initial_play: bool,
        previous: bool) -> [[f32;3];2] {
        oracle_scaled_fields(phase, elapsed, eligible, initial_play, previous, 1.0, 1.0).0
    }
    fn oracle_scaled_fields(phase: f64, elapsed: f64, eligible: bool, initial_play: bool,
        previous: bool, current_scale: f32, previous_scale: f32) -> ([[f32;3];2], [f32;16]) {
        let mut manager = Block::<32>::new();
        manager.f64(0x18,elapsed);
        let mut bone = Block::<56>::new();
        bone.u32(0,255); bone.f32(4,3.0);
        bone.floats(8,&[1.,0.,0.,0.,1.,0.,0.,0.,1.,0.,0.,0.]);
        let mut skeleton = Block::<24>::new();
        skeleton.u32(0,1); skeleton.f32(4,3.0); skeleton.ptr(8,bone.0.as_ptr());
        let mut defaults = Block::<32>::new();
        defaults.floats(0,&[0.,0.,0.,1.,0.,1.,0.,current_scale]);
        let mut previous_defaults = Block::<32>::new();
        previous_defaults.floats(0,&[0.,0.,0.,1.,0.,1.,0.,previous_scale]);
        let flags = [1u8];
        let samples = [0f32,1.,0.,0.,2.,0.];
        let mut animation = Block::<384>::new();
        animation.u32(0,1); animation.u32(4,2); animation.f32(8,2.0);
        animation.f64(0x10,0.5); animation.f64(0x18,0.5);
        animation.ptr(0x20,defaults.0.as_ptr()); animation.ptr(0x28,flags.as_ptr());
        animation.ptr(0x30,samples.as_ptr()); animation.u32(0x38,3);
        let mut previous_animation = Block::<384>::new();
        previous_animation.0.copy_from_slice(&animation.0);
        previous_animation.ptr(0x20,previous_defaults.0.as_ptr());
        let mut posed = Block::<64>::new();
        let mut palette = Block::<64>::new();
        let mut object = Block::<168>::new();
        object.ptr(0,manager.0.as_ptr()); object.ptr(8,skeleton.0.as_ptr());
        object.ptr(0x10,posed.0.as_mut_ptr()); object.ptr(0x18,palette.0.as_mut_ptr());
        object.0[0x20] = u8::from(eligible); object.u32(0x24,1);
        object.f64(0x58,1.0); object.f64(0x68,phase);
        if previous {
            object.ptr(0x70,previous_animation.0.as_ptr());
            object.f64(0x80,1.0); object.f64(0x90,0.0); object.f64(0xa0,0.5);
        }
        if initial_play {
            let mut play = Block::<16>::new();
            play.floats(0,&[1.0,phase as f32,0.0,0.0]);
            assert_ne!(unsafe { m9_oracle_14239b250(object.0.as_mut_ptr(), animation.0.as_mut_ptr(),play.0.as_ptr()) },0);
        } else {
            object.ptr(0x48,animation.0.as_mut_ptr());
            unsafe { m9_oracle_142399990(object.0.as_mut_ptr()); }
        }
        (object.bounds(), std::array::from_fn(|i|
            f32::from_le_bytes(palette.0[i*4..i*4+4].try_into().unwrap())))
    }
    fn bits(bounds: [[f32;3];2]) -> [[u32;3];2] { bounds.map(|row| row.map(f32::to_bits)) }

    #[test]
    fn hierarchical_bounds_match_native_parent_rotation_translation_and_scale() {
        for phase in [0.0, 0.125, 0.25, 0.5] {
            let mut manager = Block::<32>::new();
            manager.f64(0x18, 0.0);
            let mut bones = Block::<112>::new();
            let inverse_bind = [
                [1.,0.25,0.,0., 0.,2.,0.,0., 0.,0.,0.5,0., 3.,-6.,9.,1.],
                [0.,1.,0.,0., -1.,0.,0.,0., 0.,0.,1.,0., -3.,6.,0.,1.],
            ];
            for (ordinal, parent, radius) in [(0, 255, 3.0), (1, 0, 1.5)] {
                bones.u32(ordinal * 56, parent);
                bones.f32(ordinal * 56 + 4, radius);
                let tx: Vec<f32> = inverse_bind[ordinal].chunks_exact(4)
                    .flat_map(|row| row[..3].iter().copied()).collect();
                bones.floats(ordinal * 56 + 8, &tx);
            }
            let mut skeleton = Block::<24>::new();
            skeleton.u32(0,2); skeleton.f32(4,3.0); skeleton.ptr(8,bones.0.as_ptr());
            let mut defaults = Block::<64>::new();
            defaults.floats(0,&[0.,0.,0.6,0.8,0.,1.,0.,2.0]);
            defaults.floats(32,&[0.,0.,0.,1.,2.,0.,0.,0.5]);
            let flags = [1u8,1];
            let samples = [0f32,1.,0., 2.,0.,0., 0.,2.,0., 4.,0.,1.];
            let mut animation = Block::<384>::new();
            animation.u32(0,2); animation.u32(4,2); animation.f32(8,2.0);
            animation.f64(0x10,0.5); animation.f64(0x18,0.5);
            animation.ptr(0x20,defaults.0.as_ptr()); animation.ptr(0x28,flags.as_ptr());
            animation.ptr(0x30,samples.as_ptr()); animation.u32(0x38,6);
            let mut posed = Block::<128>::new();
            let mut palette = Block::<128>::new();
            let mut object = Block::<168>::new();
            object.ptr(0,manager.0.as_ptr()); object.ptr(8,skeleton.0.as_ptr());
            object.ptr(0x10,posed.0.as_mut_ptr()); object.ptr(0x18,palette.0.as_mut_ptr());
            object.0[0x20]=1; object.u32(0x24,1);
            object.ptr(0x48,animation.0.as_ptr()); object.f64(0x58,1.0); object.f64(0x68,phase);
            unsafe { m9_oracle_142399990(object.0.as_mut_ptr()); }

            let mut authored = (*clip()).clone();
            authored.joints[0].base_rotation_quat=[0.,0.,0.6,0.8];
            authored.joints[0].base_scale=2.0;
            let mut child = authored.joints[0].clone();
            child.name="child".into(); child.base_rotation_quat=[0.,0.,0.,1.];
            child.base_translation=[2.,0.,0.]; child.base_scale=0.5;
            authored.joints.push(child); authored.samples_t=samples.to_vec();
            let mut retained = NativeAnimatedBounds::new(NativeBoundsSkeleton {
                parents:vec![255,0], radii:vec![3.0,1.5],
                inverse_bind:inverse_bind.to_vec(), scale:3.0 });
            retained.play(Some(Arc::new(authored)),phase,1.0,false,0.0);
            assert_eq!(bits(retained.read().unwrap()),bits(object.bounds()),"phase={phase}");
            let evaluated = retained.read_pose().unwrap();
            let bytes: Vec<u8> = evaluated.palette.iter().flatten()
                .flat_map(|lane| lane.to_le_bytes()).collect();
            assert_eq!(bytes, palette.0, "native palette phase={phase}");
            assert_eq!(evaluated.phase.to_bits(), phase.to_bits());
        }
    }

    #[test]
    fn native_initial_play_and_update_match_retained_interpolated_bounds() {
        for phase in [0.0,0.125,0.25,0.5] {
            let mut state = retained();
            state.play(Some(clip()),phase,1.0,false,0.0);
            assert_eq!(bits(state.read().unwrap()),bits(oracle(phase,0.0,true,true)));
            assert_eq!(bits(state.read().unwrap()),bits(oracle(phase,0.0,true,false)));
            state.advance(0.0625,1.0);
            assert_eq!(bits(state.read().unwrap()),bits(oracle(phase,0.0625,true,false)));
        }
    }
    #[test]
    fn constructor_disabled_evaluation_and_no_clip_retain_native_bounds() {
        let mut state = retained();
        assert_eq!(bits(state.read().unwrap()),[[0;3];2]);
        assert!(state.read_pose().is_err());
        state.set_eligible(false);
        state.play(Some(clip()),0.125,1.0,false,0.0);
        state.advance(0.125,1.0);
        assert_eq!(bits(state.read().unwrap()),bits(oracle(0.125,0.125,false,false)));
        state.set_eligible(true);
        state.advance(0.0,1.0);
        let previous = state.read().unwrap();
        assert_eq!(bits(previous),bits(oracle(0.125,0.125,true,false)));
        state.play(None,0.0,1.0,false,0.0);
        state.advance(10.0,1.0);
        assert_eq!(bits(state.read().unwrap()),bits(previous));
    }
    #[test]
    fn native_blend_uses_both_retained_clip_clocks_until_expiry() {
        for elapsed in [0.0,0.0625,0.125,0.25,0.5] {
            let mut state = retained();
            state.play(Some(clip()),0.0,1.0,false,0.0);
            let first = state.read().unwrap();
            state.play(Some(clip()),0.25,1.0,false,0.5);
            assert_eq!(bits(state.read().unwrap()),bits(first),"replacement retains prior bounds until update");
            state.advance(elapsed,1.0);
            assert_eq!(bits(state.read().unwrap()),bits(oracle_fields(0.25,elapsed,true,false,true)));
        }
    }
    #[test]
    fn interrupted_blend_uses_manager_time_when_a_clip_restarts() {
        for elapsed in [0.125,1.0] {
            let mut state = retained();
            state.play(Some(clip()),0.0,1.0,false,0.0);
            state.play(Some(clip()),0.125,1.0,false,0.5);
            state.play(None,0.0,1.0,false,0.0);
            state.advance(elapsed,1.0);
            state.play(Some(clip()),0.25,1.0,false,0.0);
            assert_eq!(bits(state.read().unwrap()),bits(oracle_fields(0.25,elapsed,true,true,true)));
        }
    }
    #[test]
    fn blended_joint_scale_controls_translation_and_palette_until_blend_expires() {
        for (old_scale, current_scale) in [(0.5,2.0),(2.0,0.5)] {
            for elapsed in [0.0,0.0625,0.125,0.25,0.5] {
                let scaled_clip = |scale| {
                    let mut authored = (*clip()).clone();
                    authored.joints[0].base_scale = scale;
                    Arc::new(authored)
                };
                let mut state = retained();
                state.play(Some(scaled_clip(old_scale)),0.0,1.0,false,0.0);
                state.play(Some(scaled_clip(current_scale)),0.25,1.0,false,0.5);
                state.advance(elapsed,1.0);
                let (bounds,palette) = oracle_scaled_fields(0.25,elapsed,true,false,true,current_scale,old_scale);
                assert_eq!(bits(state.read().unwrap()),bits(bounds),"scales={old_scale}/{current_scale}, elapsed={elapsed}");
                assert_eq!(state.read_pose().unwrap().palette[0].map(f32::to_bits),palette.map(f32::to_bits));
            }
        }
    }
    #[test]
    fn same_clip_entry_restarts_clock_but_retains_bounds_until_eligible_update() {
        let shared = clip();
        let mut state = retained();
        state.play(Some(shared.clone()), 0.125, 1.0, false, 0.0);
        let before = bits(state.read().unwrap());
        state.set_eligible(false);
        state.play(Some(shared), 0.25, 1.0, false, 0.0);
        assert_eq!(bits(state.read().unwrap()), before);
        state.advance(0.0, 1.0);
        assert_eq!(bits(state.read().unwrap()), before);
        state.set_eligible(true);
        state.advance(0.0, 1.0);
        let after = bits(state.read().unwrap());
        assert_eq!(after, bits(oracle(0.25, 0.0, true, false)));
        assert_ne!(after, before);
    }
    #[test]
    fn retained_palette_identity_and_phase_follow_evaluation_not_entry_or_clock() {
        let first = clip();
        let second = Arc::new((*first).clone());
        let mut state = retained();
        state.play(Some(first.clone()), 0.125, 1.0, false, 0.0);
        let before = state.read_pose().unwrap();
        state.set_eligible(false);
        state.play(Some(second.clone()), 0.25, 1.0, false, 0.5);
        state.advance(0.0625, 1.0);
        let deferred = state.read_pose().unwrap();
        assert!(Arc::ptr_eq(&deferred.clip, &first));
        assert_eq!(deferred.phase.to_bits(), before.phase.to_bits());
        assert_eq!(deferred.palette, before.palette);
        state.set_eligible(true);
        state.advance(0.0, 1.0);
        let evaluated = state.read_pose().unwrap();
        assert!(Arc::ptr_eq(&evaluated.clip, &second));
        assert!(Arc::ptr_eq(&evaluated.previous.as_ref().unwrap().0, &first));
        assert_eq!(evaluated.previous.as_ref().unwrap().1.to_bits(), 0.1875f64.to_bits());
        assert_eq!(evaluated.blend_factor, Some(0.125));
        assert_eq!(evaluated.phase.to_bits(), 0.3125f64.to_bits());
        assert_ne!(evaluated.palette, before.palette);
        state.play(None, 0.0, 1.0, false, 0.0);
        state.advance(10.0, 1.0);
        let cleared = state.read_pose().unwrap();
        assert!(Arc::ptr_eq(&cleared.clip, &second));
        assert_eq!(cleared.phase.to_bits(), evaluated.phase.to_bits());
        assert_eq!(cleared.palette, evaluated.palette);
    }
    #[test]
    fn loader_instruction_slice_leaves_radius_center_zero_for_repeated_rows() {
        for first_row in [[1.,0.,0.],[0.3,0.6,0.9],[f32::MAX,1.,0.],[0.,0.,0.]] {
            let mut joint = Block::<100>::new();
            joint.floats(0,&first_row);
            joint.floats(0x24,&[100.,-33.,8.]);
            unsafe { m9_oracle_radius_center(joint.0.as_mut_ptr()); }
            assert_eq!(&joint.0[0x30..0x3c], &[0u8;12]);
        }
    }
    #[test]
    fn positive_acos_and_quaternion_matrix_match_exported_instructions() {
        for x in [0.0001,0.01,0.25,0.4999,0.5,0.75,0.999,1.0] {
            assert_eq!(acos_positive(x).unwrap().to_bits(),unsafe { m9_oracle_14254e038(x) }.to_bits());
        }
        for q in [[0.,0.,0.,1.],[0.1,-0.3,0.6,0.9],[-0.,0.,-0.,1.]] {
            let aligned_q = AlignedFloats(q);
            let mut native = AlignedFloats([0f32;9]);
            unsafe { m9_oracle_1424e7f50(aligned_q.0.as_ptr(),native.0.as_mut_ptr()); }
            let matrix = JointPose { translation:[0.;3],rotation:q,scale:1.0 }.matrix(1.0);
            for row in 0..3 { for col in 0..3 {
                assert_eq!(matrix[row*4+col].to_bits(),native.0[row*3+col].to_bits());
            }}
        }
    }
    #[test]
    fn quaternion_interpolation_matches_native_composition_across_branches() {
        for (a,b) in [([0.,0.,0.,1.],[0.001,0.,0.,0.9999995]),
            ([0.,0.,0.,1.],[0.,0.,0.,-1.]), ([0.,0.,0.,1.],[0.,0.,0.,1.]),
            ([0.,0.,0.,1.],[0.6,0.,0.,0.8]),
            ([0.,0.,0.,1.],[-0.6,0.,0.,-0.8]),
            ([0.,0.,0.,1.],[1.,0.,0.,0.])] {
            for mix in [0.0,0.125,0.5,1.0] {
                let first = AlignedFloats(a);
                let mut second = AlignedFloats(b);
                let mut native = AlignedFloats([0.0;4]);
                unsafe { m9_oracle_1424e8d50(first.0.as_ptr(),native.0.as_mut_ptr(),mix,second.0.as_mut_ptr()); }
                assert_eq!(slerp(a,b,mix).unwrap().map(f32::to_bits),native.0.map(f32::to_bits));
            }
        }
    }
    #[test]
    fn authenticated_mesh_shape_uses_positive_influences_and_first_mesh_only() {
        use crate::pdx_mesh::test_fixtures::*;
        let mut bytes = synthetic_skinned_mesh_bytes();
        push_object(&mut bytes,3,"skeleton");
        push_object(&mut bytes,4,"root");
        push_prop_i(&mut bytes,"ix",&[7]);
        push_prop_f(&mut bytes,"tx",&[1.,0.25,0.,0.,2.,0.,0.,0.,0.5,20.,-2.,1.]);
        let tree = crate::pdx_mesh::parse_pdx_asset(&bytes).unwrap();
        let child = &tree.root.children[0].children[0];
        let skeleton = child.children.iter().find(|n| named(n,"skeleton")).unwrap();
        let source = NativeBoundsSkeleton::load(child,skeleton,3.0).unwrap();
        assert_eq!(source.radii,[3.0]);
        assert_eq!(source.parents,[255]);
        assert_eq!(source.inverse_bind, [[1.,0.25,0.,0., 0.,2.,0.,0.,
            0.,0.,0.5,0., 60.,-6.,3.,1.]]);
        let mut malformed = child.clone();
        let skin = malformed.children.iter_mut().find(|n| named(n,"mesh")).unwrap()
            .children.iter_mut().find(|n| named(n,"skin")).unwrap();
        skin.properties.iter_mut().find(|p| p.name=="w").unwrap().values=PdxValues::F32(vec![1.]);
        assert!(NativeBoundsSkeleton::load(&malformed,skeleton,3.0).is_err());
    }
}
