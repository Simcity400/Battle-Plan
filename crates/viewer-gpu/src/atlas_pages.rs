
#[cfg(test)]
use std::collections::HashSet;
use std::{
    io,
    sync::atomic::{AtomicU32, Ordering},
};

use eframe::wgpu;

static DEVICE_LIMIT: AtomicU32 = AtomicU32::new(0);

pub fn record_device_limit(limit: u32) {
    DEVICE_LIMIT.store(limit, Ordering::Relaxed);
}

pub fn page_limit() -> u32 {
    let default = wgpu::Limits::default().max_texture_dimension_2d;
    match DEVICE_LIMIT.load(Ordering::Relaxed) {
        0 => default,
        device => device.min(default),
    }
}

const SIDE_CELLS: u32 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellPages {
    pub cell: [u32; 2],
    pub columns: u32,
    pub rows: u32,
}

impl CellPages {
    pub fn plan(cell: [u32; 2], limit: u32, what: &str) -> io::Result<Self> {
        if cell[0] == 0 || cell[1] == 0 || cell[0] > limit || cell[1] > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{what}: a {} x {} cell does not fit a {limit} px texture",
                    cell[0], cell[1]
                ),
            ));
        }
        Ok(Self {
            cell,
            columns: (limit / cell[0]).clamp(1, SIDE_CELLS),
            rows: (limit / cell[1]).clamp(1, SIDE_CELLS),
        })
    }

    pub fn cells_per_page(&self) -> u32 {
        self.columns * self.rows
    }

    pub fn page_size(&self) -> [u32; 2] {
        [self.columns * self.cell[0], self.rows * self.cell[1]]
    }

    pub fn page_bytes(&self) -> usize {
        let [width, height] = self.page_size();
        width as usize * height as usize * 4
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn pages_for(&self, count: u32) -> u32 {
        count.div_ceil(self.cells_per_page())
    }

    pub fn locate(&self, index: u32) -> (u32, [u32; 2]) {
        let page = index / self.cells_per_page();
        let within = index % self.cells_per_page();
        (
            page,
            [
                within % self.columns * self.cell[0],
                within / self.columns * self.cell[1],
            ],
        )
    }

    pub fn uv(&self, index: u32) -> (u32, [f32; 4]) {
        let (page, [x, y]) = self.locate(index);
        let [width, height] = self.page_size().map(|side| side as f32);
        (
            page,
            [
                x as f32 / width,
                y as f32 / height,
                (x + self.cell[0]) as f32 / width,
                (y + self.cell[1]) as f32 / height,
            ],
        )
    }

    pub fn cell_at(&self, page: u32, uv: [f32; 4]) -> Option<u32> {
        let [width, height] = self.page_size().map(|side| side as f32);
        let column = (uv[0] * width / self.cell[0] as f32).round() as u32;
        let row = (uv[1] * height / self.cell[1] as f32).round() as u32;
        (column < self.columns && row < self.rows).then(|| {
            page.saturating_mul(self.cells_per_page())
                .saturating_add(row * self.columns + column)
        })
    }

    pub fn blit(&self, page: &mut [u8], at: [u32; 2], pixels: &[u8]) {
        let page_width = self.page_size()[0] as usize;
        let row_bytes = self.cell[0] as usize * 4;
        for row in 0..self.cell[1] as usize {
            let from = row * row_bytes;
            let to = ((at[1] as usize + row) * page_width + at[0] as usize) * 4;
            page[to..to + row_bytes].copy_from_slice(&pixels[from..from + row_bytes]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vanilla_sized_set_fits_one_page_of_each_cell() {
        for (cell, size) in [
            ([10, 7], [320, 224]),
            ([41, 26], [1312, 832]),
            ([82, 52], [2624, 1664]),
        ] {
            let pages = CellPages::plan(cell, 8192, "flags").unwrap();
            assert_eq!(pages.page_size(), size);
            assert_eq!(pages.pages_for(900), 1);
        }
    }

    #[test]
    fn sixty_thousand_cells_page_under_a_small_limit_and_stay_addressable() {
        let limit = 2048;
        for cell in [[10, 7], [41, 26], [82, 52]] {
            let pages = CellPages::plan(cell, limit, "flags").unwrap();
            let count = 60_000;
            assert!(pages.pages_for(count) > 1, "{cell:?} needs several pages");
            let [width, height] = pages.page_size();
            assert!(width <= limit && height <= limit, "{cell:?}");
            let mut seen = HashSet::new();
            for index in 0..count {
                let (page, [x, y]) = pages.locate(index);
                assert!(page < pages.pages_for(count));
                assert!(x + cell[0] <= width && y + cell[1] <= height);
                assert!(seen.insert((page, x, y)), "cell {index} overlaps");
                let (uv_page, uv) = pages.uv(index);
                assert_eq!(uv_page, page);
                assert!(uv.iter().all(|value| (0.0..=1.0).contains(value)));
                assert_eq!(pages.cell_at(page, uv), Some(index));
            }
        }
    }

    #[test]
    fn a_cell_larger_than_a_page_is_refused() {
        assert!(CellPages::plan([9000, 7], 8192, "wide").is_err());
        assert!(CellPages::plan([0, 7], 8192, "empty").is_err());
        let one = CellPages::plan([8192, 8192], 8192, "whole").unwrap();
        assert_eq!((one.columns, one.rows), (1, 1));
    }

    #[test]
    fn blits_land_in_their_page_rectangles() {
        let pages = CellPages::plan([2, 2], 4, "tiny").unwrap();
        assert_eq!((pages.cells_per_page(), pages.pages_for(9)), (4, 3));
        let mut page = vec![0; pages.page_bytes()];
        let (_, at) = pages.locate(3);
        pages.blit(&mut page, at, &[7; 16]);
        assert_eq!(&page[40..48], &[7; 8]);
        assert_eq!(&page[56..64], &[7; 8]);
        assert!(page[..40].iter().all(|byte| *byte == 0));
    }
}
