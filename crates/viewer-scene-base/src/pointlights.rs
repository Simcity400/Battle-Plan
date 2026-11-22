
pub struct PointLightMaps {
    pub indices: Bound,
    pub data: Bound,
    written: Written,
}

#[derive(Default)]
struct Written {
    indices: Vec<u8>,
    data: Vec<[f32; 4]>,
}

impl Written {
    fn take(&mut self, tiles: &Tiles) -> [bool; 2] {
        let indices = self.indices != tiles.indices;
        if indices {
            self.indices.clone_from(&tiles.indices);
        }
        let data = bytemuck::cast_slice::<_, u8>(self.data.as_slice())
            != bytemuck::cast_slice::<_, u8>(tiles.data.as_slice());
        if data {
            self.data.clear();
            self.data.extend_from_slice(&tiles.data);
        }
        [indices, data]
    }
}

impl PointLightMaps {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> io::Result<Self> {
        use self::{DATA_WIDTH, INDEX_SIZE, Tiles};
        let tiles = Tiles::default();
        let mut written = Written::default();
        written.take(&tiles);
        Ok(Self {
            written,
            indices: data_texture(
                device,
                queue,
                "LightIndexMap",
                [INDEX_SIZE, INDEX_SIZE],
                wgpu::TextureFormat::Rgba8Unorm,
                &tiles.indices,
            )?,
            data: data_texture(
                device,
                queue,
                "LightDataMap",
                [DATA_WIDTH, 1],
                wgpu::TextureFormat::Rgba32Float,
                bytemuck::cast_slice(&tiles.data),
            )?,
        })
    }

    pub fn write(&mut self, queue: &wgpu::Queue, tiles: &Tiles) {
        use self::{DATA_WIDTH, INDEX_SIZE};
        let [indices, data] = self.written.take(tiles);
        if indices {
            self.indices
                .write(queue, [INDEX_SIZE, INDEX_SIZE], 4, &tiles.indices);
        }
        if data {
            self.data.write(
                queue,
                [DATA_WIDTH, 1],
                16,
                bytemuck::cast_slice(&tiles.data),
            );
        }
    }
}

use eframe::wgpu;
use std::io;
use viewer_gpu::camera::Camera;
#[cfg(test)]
use viewer_gpu::camera::MapDefines;
use viewer_gpu::textures::{Bound, data_texture};

pub const INDEX_SIZE: u32 = 64;
pub const DATA_WIDTH: u32 = 128;
const CELL: f32 = 8.0;
const MAX_LIGHTS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub position: [f32; 3],
    pub radius: f32,
    pub color: [f32; 3],
    pub intensity: f32,
    pub falloff: f32,
}

pub struct Tiles {
    pub grid: [f32; 4],
    pub indices: Vec<u8>,
    pub data: [[f32; 4]; DATA_WIDTH as usize],
    pub count: usize,
    laid: Option<([i32; 2], Vec<Light>)>,
}

impl Default for Tiles {
    fn default() -> Self {
        Self {
            grid: [0.0, 0.0, 0.125, 0.125],
            indices: vec![255; (INDEX_SIZE * INDEX_SIZE * 4) as usize],
            data: [[0.0; 4]; DATA_WIDTH as usize],
            count: 0,
            laid: None,
        }
    }
}

impl Tiles {
    pub fn rebuild(&mut self, camera: Camera, lights: &[Light]) {
        let eye = camera.eye();
        let forward = camera.look_direction();
        let origin = |eye: f32, forward: f32, size: u32| {
            (((eye + forward * 256.0) / CELL) as i32 - 32)
                .max(0)
                .min((size as f32 * 0.125).ceil() as i32 - 64)
        };
        self.rebuild_at(
            [
                origin(eye[0], forward[0], camera.map.width),
                origin(eye[2], forward[2], camera.map.height),
            ],
            lights,
        );
    }

    fn rebuild_at(&mut self, origin: [i32; 2], lights: &[Light]) {
        if self.laid.as_ref().is_some_and(|(laid, was)| *laid == origin && was == lights) {
            return;
        }
        self.laid = Some((origin, lights.to_vec()));
        self.grid = [
            origin[0] as f32 * CELL,
            origin[1] as f32 * CELL,
            0.125,
            0.125,
        ];
        self.indices.fill(255);
        self.count = 0;
        for light in lights {
            let [x, _, z] = light.position;
            let r = light.radius;
            let start = [((x - r) / CELL) as i32, ((z - r) / CELL) as i32];
            let end = [1 - ((x + r) * -0.125) as i32, 1 - ((z + r) * -0.125) as i32];
            let mut admitted = false;
            for cz in start[1].max(origin[1])..end[1].min(origin[1] + 64) {
                for cx in start[0].max(origin[0])..end[0].min(origin[0] + 64) {
                    let offset = (((cz - origin[1]) * 64 + cx - origin[0]) * 4) as usize;
                    let cell = &mut self.indices[offset..offset + 4];
                    if cell[3] != 255 {
                        continue;
                    }
                    let dx = cx as f32 * CELL + 4.0 - x;
                    let dz = cz as f32 * CELL + 4.0 - z;
                    if (dx * dx + dz * dz).sqrt() - 32.0_f32.sqrt() <= r {
                        *cell
                            .iter_mut()
                            .find(|v| **v == 255)
                            .expect("free fourth slot") = self.count as u8;
                        admitted = true;
                    }
                }
            }
            if admitted {
                self.data[self.count * 2] = [x, light.position[1], z, r];
                self.data[self.count * 2 + 1] = [
                    light.color[0] * light.intensity,
                    light.color[1] * light.intensity,
                    light.color[2] * light.intensity,
                    light.falloff,
                ];
                self.count += 1;
                if self.count == MAX_LIGHTS {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use viewer_gpu::camera::MapSize;

    #[test]
    fn tiles_equal_to_those_written_are_not_written_again() {
        let mut written = Written::default();
        let mut tiles = Tiles::default();
        assert_eq!(written.take(&tiles), [true, true], "the textures hold nothing yet");
        assert_eq!(written.take(&tiles), [false, false], "the same tiles write nothing");
        tiles.indices[0] = 3;
        assert_eq!(written.take(&tiles), [true, false], "a changed index map alone");
        tiles.data[1][2] = 0.5;
        assert_eq!(written.take(&tiles), [false, true], "changed light data alone");
    }

    fn light(x: f32, z: f32) -> Light {
        Light {
            position: [x, 10.0, z],
            radius: 3.0,
            color: [1.0, 0.3, 0.1],
            intensity: 2.0,
            falloff: 2.0,
        }
    }

    #[test]
    fn first_four_win_and_an_unadmitted_light_uses_no_data_slot() {
        let mut lights = vec![light(4.0, 4.0); 5];
        lights.push(light(20.0, 20.0));
        let mut tiles = Tiles::default();
        tiles.rebuild_at([0, 0], &lights);
        assert_eq!(&tiles.indices[..4], &[0, 1, 2, 3]);
        assert_eq!(tiles.count, 5);
        assert_eq!(tiles.data[8], [20.0, 10.0, 20.0, 3.0]);
        assert_eq!(tiles.data[9], [2.0, 0.6, 0.2, 2.0]);
    }

    #[test]
    fn a_camera_inside_its_cell_with_the_same_lights_lays_the_grid_once() {
        let mut tiles = Tiles::default();
        tiles.rebuild_at([0, 0], &[light(4.0, 4.0)]);
        tiles.indices[400] = 7;
        tiles.rebuild_at([0, 0], &[light(4.0, 4.0)]);
        assert_eq!(tiles.indices[400], 7, "the same cell and the same lights lay nothing again");
        tiles.rebuild_at([0, 0], &[light(5.0, 4.0)]);
        assert_eq!(tiles.indices[400], 255, "a light that moved lays the grid again");
        tiles.indices[400] = 7;
        tiles.rebuild_at([1, 0], &[light(5.0, 4.0)]);
        assert_eq!(tiles.indices[400], 255, "a camera that left its cell lays the grid again");
    }

    #[test]
    fn clearing_indices_makes_stale_data_unreachable() {
        let mut tiles = Tiles::default();
        tiles.rebuild_at([0, 0], &[light(4.0, 4.0)]);
        tiles.rebuild_at([0, 0], &[]);
        assert!(tiles.indices.iter().all(|&v| v == 255));
        assert_eq!(tiles.count, 0);
        assert_eq!(tiles.data[0][3], 3.0);
    }

    #[test]
    fn global_cap_and_camera_edge_clamp() {
        let lights: Vec<_> = (0..80)
            .map(|i| light((i % 16) as f32 * 24.0 + 4.0, (i / 16) as f32 * 24.0 + 4.0))
            .collect();
        let mut tiles = Tiles::default();
        tiles.rebuild(
            Camera {
                center: [-100.0, -100.0],
                height: 300.0,
                map: MapSize::TEST,
                defines: MapDefines::test(),
                pose: viewer_gpu::camera::Pose::Map,
            },
            &lights,
        );
        assert_eq!(tiles.grid, [0.0, 0.0, 0.125, 0.125]);
        assert_eq!(tiles.count, 64);
        assert!(tiles.indices.iter().all(|&v| v < 64 || v == 255));
        tiles.rebuild(
            Camera {
                center: [150.0, 50.0],
                height: 300.0,
                map: MapSize {
                    width: 300,
                    height: 100,
                },
                defines: MapDefines::test(),
                pose: viewer_gpu::camera::Pose::Map,
            },
            &lights,
        );
        assert_eq!(tiles.grid, [-26.0 * 8.0, -51.0 * 8.0, 0.125, 0.125]);
    }
}
