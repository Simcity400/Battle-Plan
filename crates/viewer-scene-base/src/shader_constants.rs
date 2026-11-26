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

pub fn reads_time(pipeline: &gpu::Pipeline, slot: u32, byte: u32) -> bool {
    use std::mem::offset_of;
    use viewer_gpu::frame::FrameConstants;
    let scene_times = [
        offset_of!(FrameConstants, cam_look_at_dir_global_time) + 3 * size_of::<f32>(),
        offset_of!(FrameConstants, fow_opacity_time_snow_mud_day_night) + size_of::<f32>(),
    ];
    pipeline.constant_reads().iter().any(|buffer| {
        let reads = |at| buffer.reads.iter().any(|read| read.contains(&at));
        match buffer.binding / 2 {
            0 => scene_times.iter().any(|&at| reads(at as u32)),
            bound => bound == slot && reads(byte),
        }
    })
}

#[derive(Clone, Copy)]
pub enum WorldPixelConstants {
    LightGrid([f32; 4]),
    Border([f32; 8]),
}

impl WorldPixelConstants {
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
