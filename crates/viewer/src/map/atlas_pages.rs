//! Equal cells on fixed-size texture pages, as many pages as the cells need.
//!
//! The flag atlases hold one cell per flag the map has drawn, and a mod can
//! ship a flag for every one of thousands of countries. One page cannot hold
//! every possible flag: a texture side is limited by the device, and most
//! flags are never drawn. So cells are handed out in load order onto pages of
//! a fixed size, and a new page starts whenever one is full. A page never
//! changes size, so a cell's normalised rectangle never moves once handed out.
//!
//! A drawn cell is sampled only inside its own rectangle: the icon layer
//! draws a flag 1:1 or minified onto whole device pixels, so every bilinear
//! footprint stays within the cell and what its neighbours hold (another flag
//! or nothing) does not reach a pixel.

use std::{
    io,
    sync::atomic::{AtomicU32, Ordering},
};

use eframe::wgpu;

/// The device's `max_texture_dimension_2d`, once a device has been asked for.
static DEVICE_LIMIT: AtomicU32 = AtomicU32::new(0);

/// Records the texture side limit of the device the viewer creates.
///
/// [`crate::render::device_descriptor`] requests the adapter's own limit, so
/// that is the limit every texture of this process is created under.
pub fn record_device_limit(limit: u32) {
    DEVICE_LIMIT.store(limit, Ordering::Relaxed);
}

/// The largest page side an atlas is built with.
///
/// The WebGPU default every supported device reaches (8192), lowered to the
/// device's own limit when a device has reported a smaller one.
pub fn page_limit() -> u32 {
    let default = wgpu::Limits::default().max_texture_dimension_2d;
    match DEVICE_LIMIT.load(Ordering::Relaxed) {
        0 => default,
        device => device.min(default),
    }
}

/// Cells per page side an atlas aims for: 1,024 cells a page keeps a
/// vanilla-sized set on one page without making the first page of a large
/// cell (82 x 52) much bigger than the flags a session draws.
const SIDE_CELLS: u32 = 32;

/// Where the cells of one atlas sit across its fixed-size pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellPages {
    /// One cell's size in texels.
    pub cell: [u32; 2],
    /// Cells per page row.
    pub columns: u32,
    /// Cell rows per page.
    pub rows: u32,
}

impl CellPages {
    /// Plans pages of `cell`-sized cells no side of which exceeds `limit`.
    /// Only a cell that is itself larger than a page is refused.
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

    /// Cells one page holds.
    pub fn cells_per_page(&self) -> u32 {
        self.columns * self.rows
    }

    /// Every page's size in texels.
    pub fn page_size(&self) -> [u32; 2] {
        [self.columns * self.cell[0], self.rows * self.cell[1]]
    }

    /// Bytes of one RGBA8 page.
    pub fn page_bytes(&self) -> usize {
        let [width, height] = self.page_size();
        width as usize * height as usize * 4
    }

    /// Pages `count` cells need.
    #[cfg(test)]
    pub fn pages_for(&self, count: u32) -> u32 {
        count.div_ceil(self.cells_per_page())
    }

    /// A cell's page and the texel of its top-left corner on that page.
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

    /// A cell's page and its rectangle there in normalised coordinates.
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

    /// The cell a page's normalised rectangle starts at, for reverse lookups.
    pub fn cell_at(&self, page: u32, uv: [f32; 4]) -> Option<u32> {
        let [width, height] = self.page_size().map(|side| side as f32);
        let column = (uv[0] * width / self.cell[0] as f32).round() as u32;
        let row = (uv[1] * height / self.cell[1] as f32).round() as u32;
        (column < self.columns && row < self.rows).then(|| {
            page.saturating_mul(self.cells_per_page())
                .saturating_add(row * self.columns + column)
        })
    }

    /// Copies one `cell`-sized RGBA8 image into its place on its page.
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
            let mut seen = std::collections::HashSet::new();
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
