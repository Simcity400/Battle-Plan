use content_reader_gui::pdxshader::program::Stage;
use shader_pipeline::{binding, gpu};
use std::ops::Range;

pub fn scene(pipeline: &gpu::Pipeline) -> Result<(), String> {
    both(pipeline, 0, 416, &[0..348, 352..408])
}

pub fn tree_caster(pipeline: &gpu::Pipeline) -> Result<(), String> {
    both(
        pipeline,
        0,
        416,
        &[
            0..156,
            160..172,
            176..188,
            192..204,
            208..220,
            224..236,
            240..284,
            288..300,
            304..348,
            352..364,
            368..400,
            404..408,
        ],
    )
}

pub fn both(
    pipeline: &gpu::Pipeline,
    slot: u32,
    physical: u32,
    known: &[Range<u32>],
) -> Result<(), String> {
    for stage in [Stage::Vertex, Stage::Pixel] {
        pipeline.check_initialized(binding(slot, stage), physical, known)?;
    }
    Ok(())
}

use std::mem::offset_of;
use viewer_gpu::frame::FrameConstants;
use viewer_gpu::surface_motion::Motion;

const FOG: u32 = offset_of!(FrameConstants, fow_opacity_time_snow_mud_day_night) as u32;
const SCENE_TIMES: [u32; 2] = [
    offset_of!(FrameConstants, cam_look_at_dir_global_time) as u32 + 12,
    FOG + 4,
];

pub fn reads_time(buffers: &[shader_pipeline::Buffer], own: Option<(u32, u32)>) -> bool {
    buffers.iter().any(|buffer| {
        let reads = |at| buffer.reads.iter().any(|read| read.contains(&at));
        match buffer.binding / 2 {
            0 => SCENE_TIMES.iter().any(|&at| reads(at)),
            bound => own.is_some_and(|(slot, byte)| bound == slot && reads(byte)),
        }
    })
}

pub struct SurfaceClock {
    fog_on: Motion,
    fog_off: Motion,
    pixel: Motion,
}

pub struct RegionClock {
    idle: SurfaceClock,
    pulsing: SurfaceClock,
}

impl RegionClock {
    pub fn new(program: &shader_pipeline::Compiled, style_slot: u32) -> Self {
        Self {
            idle: SurfaceClock::classify(program, None, &[], Some(style_slot)),
            pulsing: SurfaceClock::classify(program, None, &[], None),
        }
    }

    pub fn for_styles(&self, pulses: bool) -> &SurfaceClock {
        if pulses { &self.pulsing } else { &self.idle }
    }
}

impl SurfaceClock {
    pub fn scene(program: &shader_pipeline::Compiled) -> Self {
        Self::classify(program, None, &[], None)
    }

    pub fn new(
        program: &shader_pipeline::Compiled,
        slot: u32,
        byte: u32,
        zeros: &[shader_pipeline::clock::Slot],
    ) -> Self {
        Self::classify(program, Some((slot, byte)), zeros, None)
    }

    fn classify(
        program: &shader_pipeline::Compiled,
        own: Option<(u32, u32)>,
        zeros: &[shader_pipeline::clock::Slot],
        style_slot: Option<u32>,
    ) -> Self {
        let pixel = if ["discard", "frag_depth", "sample_mask"]
            .iter()
            .any(|term| program.pixel.wgsl.contains(term))
        {
            Motion::Coverage
        } else {
            Motion::Shading
        };
        let depends = |fog: bool| {
            [
                (Stage::Vertex, &program.vertex),
                (Stage::Pixel, &program.pixel),
            ]
            .into_iter()
            .map(|(stage, shader)| {
                let scene = binding(0, stage);
                let read = |&(register, byte): &(u32, u32)| {
                    shader.buffers.iter().any(|b| {
                        b.binding == register && b.reads.iter().any(|range| range.contains(&byte))
                    })
                };
                let mut times = vec![(scene, SCENE_TIMES[0]), (scene, SCENE_TIMES[1])];
                times.extend(own.map(|(slot, byte)| (binding(slot, stage), byte)));
                if !times.iter().any(read) {
                    return Motion::Still;
                }
                let mut zeros = zeros.iter().copied().filter(read).collect::<Vec<_>>();
                if !fog {
                    let fade = offset_of!(FrameConstants, ambient_neg_x_fow_fade) as u32 + 12;
                    zeros.extend([(scene, FOG), (scene, fade)].into_iter().filter(read));
                }
                let channel = style_slot
                    .map(|slot| binding(slot, Stage::Pixel))
                    .filter(|slot| shader.textures.iter().any(|(binding, _)| binding == slot))
                    .map(|slot| (slot, 2));
                let live = (zeros.is_empty() && channel.is_none())
                    || shader_pipeline::clock::reads_with_zeros(
                        &shader.wgsl,
                        &times,
                        &zeros,
                        channel,
                    )
                    .unwrap_or(true);
                match (live, stage) {
                    (false, _) => Motion::Still,
                    (true, Stage::Pixel) => pixel,
                    (true, _) => Motion::Coverage,
                }
            })
            .max()
            .unwrap_or_default()
        };
        Self {
            fog_on: depends(true),
            fog_off: depends(false),
            pixel,
        }
    }

    pub fn live(&self, fog: bool) -> Motion {
        if fog { self.fog_on } else { self.fog_off }
    }

    pub fn pixel_motion(&self, changing_input: bool) -> Motion {
        if changing_input {
            self.pixel
        } else {
            Motion::Still
        }
    }

    pub fn for_frames(&self, frames: &[&[u8]]) -> Motion {
        if self.fog_on == self.fog_off {
            return self.fog_on;
        }
        let fade = offset_of!(FrameConstants, ambient_neg_x_fow_fade) + 12;
        frames
            .iter()
            .map(|frame| {
                let fog = [FOG as usize, fade]
                    .iter()
                    .any(|&at| frame.get(at..at + 4) != Some(&[0; 4]));
                self.live(fog)
            })
            .max()
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy)]
pub enum WorldPixelConstants {
    LightGrid([f32; 4]),
    Border([f32; 8]),
}

impl WorldPixelConstants {
    pub fn time_read_by(&self, pipeline: &gpu::Pipeline) -> bool {
        matches!(self, Self::Border(_))
            && pipeline.constant_reads().iter().any(|buffer| {
                buffer.binding == binding(2, Stage::Pixel)
                    && buffer.reads.iter().any(|range| range.contains(&16))
            })
    }

    pub fn border(time: f32, selection: [f32; 4], alpha: f32) -> Self {
        Self::Border([
            selection[0],
            selection[1],
            selection[2],
            selection[3],
            time,
            alpha,
            0.,
            0.,
        ])
    }

    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::LightGrid(values) => bytemuck::bytes_of(values),
            Self::Border(values) => bytemuck::bytes_of(values),
        }
    }

    fn known(&self) -> Range<u32> {
        match self {
            Self::LightGrid(_) => 0..16,
            Self::Border(_) => 0..24,
        }
    }

    pub fn retain(&self, state: &mut super::shader_scene::State) {
        state.stream.bind_partial_constants(
            2,
            Stage::Pixel,
            Some(self.bytes()),
            std::slice::from_ref(&self.known()),
        );
    }

    pub fn check(&self, pipeline: &gpu::Pipeline) -> Result<(), String> {
        pipeline.check_initialized(
            binding(2, Stage::Pixel),
            self.bytes().len() as u32,
            std::slice::from_ref(&self.known()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_border_tail_is_unknown_not_out_of_bounds_zero() {
        let grid = WorldPixelConstants::LightGrid([2., 3., 5., 7.]);
        let border = WorldPixelConstants::border(11., [0.; 4], 1.);
        let read = |offset| shader_pipeline::Buffer {
            binding: binding(2, Stage::Pixel),
            size: 32,
            reads: std::iter::once(offset..offset + 4).collect(),
        };
        let check = |block: WorldPixelConstants, offset| {
            read(offset).check_initialized(block.bytes().len() as u32, &[block.known()])
        };
        assert!(check(grid, 24).is_ok());
        assert!(check(border, 20).is_ok());
        assert!(check(border, 24).unwrap_err().contains("24..28"));
        assert!(check(border, 32).is_ok());
        assert_eq!(&border.bytes()[16..20], &11_f32.to_le_bytes());
        assert_eq!(&border.bytes()[20..24], &1_f32.to_le_bytes());
    }
}
