//! The snow upload writes the bands of rows a step changed, a run of neighbouring bands a budget
//! check, and ends with the scene holding what taking the grid whole leaves.

use super::{BAND, Back, Changed, Lease, Made, Plane, RUN, Snow, Target, Written, pick, write_back};
use crate::layers::{Budget, Layer};
use map_snow::SnowMud;
use std::{
    collections::VecDeque,
    io,
    ops::Range,
    sync::{Arc, Mutex},
    time::Duration,
};
use viewer_foundation::map::presentation::Continuity;

const WIDTH: u32 = 64;
const HEIGHT: u32 = 200;

/// A stand-in for the scene: its texture's bytes, and what each call wrote; the texture no pass
/// binds, by the number of the lend that made it, and the swaps made.
#[derive(Default)]
struct Counted {
    texels: Vec<u8>,
    held: Option<[u32; 2]>,
    wholes: usize,
    rects: Vec<(Range<u32>, Range<u32>)>,
    written: usize,
    begun: usize,
    finished: usize,
    back: Option<(u64, Arc<Mutex<Vec<u8>>>)>,
    lends: u64,
    swaps: usize,
}

/// A lent texture's bytes, shared with the stand-in scene, and which it is.
struct Lent {
    id: u64,
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl Plane for Lent {
    fn write(&mut self, first: usize, bytes: &[u8]) {
        self.bytes.lock().unwrap()[first * 4..first * 4 + bytes.len()].copy_from_slice(bytes);
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Target for Counted {
    fn takes_parts(&self, grid: &SnowMud, continuity: Continuity) -> bool {
        continuity == Continuity::Reset || (self.held == Some([grid.width, grid.height]) && grid.refresh.is_none())
    }

    fn whole(&mut self, grid: &Arc<SnowMud>, _continuity: Continuity) -> io::Result<()> {
        self.texels = grid.texels.to_vec();
        self.held = Some([grid.width, grid.height]);
        self.wholes += 1;
        self.written += self.texels.len();
        Ok(())
    }

    fn begin(&mut self, grid: &SnowMud, _continuity: Continuity) -> io::Result<()> {
        self.texels.resize(grid.texels.len(), 0);
        self.held = Some([grid.width, grid.height]);
        self.begun += 1;
        Ok(())
    }

    fn rect(&mut self, grid: &SnowMud, rows: Range<u32>, columns: Range<u32>) -> io::Result<()> {
        for row in rows.clone() {
            let at = (row * grid.width + columns.start) as usize * 4;
            let end = at + columns.len() * 4;
            for byte in at..end {
                self.texels[byte] = grid.texels[byte];
            }
            self.written += end - at;
        }
        self.rects.push((rows, columns));
        Ok(())
    }

    fn finish(&mut self, _grid: &Arc<SnowMud>, _continuity: Continuity) -> io::Result<()> {
        self.finished += 1;
        Ok(())
    }

    fn lend(&mut self) -> io::Result<Option<Box<dyn Plane>>> {
        if self.back.as_ref().is_none_or(|(_, bytes)| bytes.lock().unwrap().len() != self.texels.len()) {
            self.lends += 1;
            self.back = Some((self.lends, Arc::new(Mutex::new(vec![0; self.texels.len()]))));
        }
        let (id, bytes) = self.back.clone().unwrap();
        Ok(Some(Box::new(Lent { id, bytes })))
    }

    fn swap(&mut self, back: &dyn Plane, grid: &Arc<SnowMud>, _mismatched: Vec<usize>) -> io::Result<bool> {
        let lent = back.any().downcast_ref::<Lent>().unwrap();
        if self.back.as_ref().is_none_or(|(id, _)| *id != lent.id) || !self.takes_parts(grid, Continuity::Continue) {
            return Ok(false);
        }
        std::mem::swap(&mut self.texels, &mut *lent.bytes.lock().unwrap());
        self.swaps += 1;
        Ok(true)
    }
}

fn grid(seed: u8) -> Arc<SnowMud> {
    let texels: Vec<u8> = (0..WIDTH * HEIGHT).flat_map(|cell| [seed, cell as u8, cell as u8, seed]).collect();
    Arc::new(SnowMud {
        width: WIDTH,
        height: HEIGHT,
        texels: texels.into(),
        unknown: vec![0; (WIDTH * HEIGHT) as usize].into(),
        province_snow: vec![].into(),
        post_factors: None,
        build: Duration::ZERO,
        summary: Default::default(),
        refresh: None,
    })
}

/// `grid` with `cells` given the texel `[value; 4]`.
fn edited(grid: &SnowMud, cells: &[u32], value: u8) -> Arc<SnowMud> {
    let texels = grid.texels.edited(cells.iter().flat_map(|&cell| (0..4).map(move |channel| (cell as usize * 4 + channel, value))));
    Arc::new(SnowMud { texels, ..grid.clone() })
}

/// What the worker hands over with a grid: the bands where it differs from the one before.
fn made(grid: &Arc<SnowMud>, cells: Option<&[u32]>) -> Made {
    made_at(None, grid, cells)
}

/// [`made`], stamped with the hour it is current at.
fn made_at(stamp: Option<i32>, grid: &Arc<SnowMud>, cells: Option<&[u32]>) -> Made {
    Made { epoch: 0, stamp, grid: Ok(Some(Arc::clone(grid))), changed: cells.map_or(Changed::Unknown, |cells| Changed::of_cells(cells, WIDTH)), refused: None, marked: [0; 3], held: 0, ns: 0 }
}

fn layer() -> Snow {
    Snow::new(Err("no content: the upload alone is tested".into()))
}

/// The layer holding `first`, sent to the scene whole.
fn sent(first: &Arc<SnowMud>, scene: &mut Counted) -> Snow {
    let mut layer = layer();
    layer.take(made(first, None));
    layer.push(scene, &mut Budget::unlimited()).unwrap();
    assert!(layer.uploaded(), "an unlimited upload sends everything");
    assert_eq!(scene.texels, first.texels.to_vec(), "the scene holds the first grid");
    layer
}

/// One cell's snow changes: the upload writes that cell's band and columns alone, not the grid,
/// and the scene ends with the new grid. Before, every new grid was handed to the scene whole and
/// its presentation compared, copied and wrote every page the change touched.
#[test]
fn snow_upload_writes_only_the_band_a_changed_cell_lies_in() {
    let first = grid(1);
    let mut scene = Counted::default();
    let mut layer = sent(&first, &mut scene);
    assert_eq!((scene.wholes, scene.rects.len(), scene.written), (0, HEIGHT.div_ceil(BAND).div_ceil(RUN) as usize, first.texels.len()), "composed, every band is written once, a run of bands a piece");

    let cell = 13 * WIDTH + 10;
    let second = edited(&first, &[cell], 200);
    let (rects, written) = (scene.rects.len(), scene.written);
    layer.take(made(&second, Some(&[cell])));
    assert!(!layer.uploaded(), "the changed band is owed to the scene");
    layer.push(&mut scene, &mut Budget::unlimited()).unwrap();
    assert_eq!(&scene.rects[rects..], [(8..16, 10..11)], "one band's one column, of {} bands", HEIGHT.div_ceil(BAND));
    assert_eq!(scene.written - written, BAND as usize * 4, "{} bytes, not the grid's {}", BAND * 4, first.texels.len());
    assert_eq!((scene.wholes, scene.finished), (0, 2));
    assert_eq!(scene.texels, second.texels.to_vec(), "the scene holds the new grid");
    assert!(layer.uploaded());
}

/// A step that changes nothing the picture reads hands back the grid the scene holds: nothing is
/// owed and nothing written. A grid made again with no cell changed (a snow word, a factor) is
/// finished without a band written.
#[test]
fn snow_a_step_that_changes_no_cell_writes_no_band() {
    let first = grid(1);
    let mut scene = Counted::default();
    let mut layer = sent(&first, &mut scene);
    let (rects, written, finished) = (scene.rects.len(), scene.written, scene.finished);
    layer.take(made(&first, Some(&[])));
    assert!(layer.uploaded(), "the grid the scene holds is owed nothing");
    layer.push(&mut scene, &mut Budget::unlimited()).unwrap();
    assert_eq!((scene.rects.len(), scene.written, scene.finished), (rects, written, finished), "nothing is written");

    let words = Arc::new(SnowMud { province_snow: vec![Some(5)].into(), ..SnowMud::clone(&first) });
    layer.take(made(&words, Some(&[])));
    layer.push(&mut scene, &mut Budget::new(Duration::ZERO).first_free()).unwrap();
    assert_eq!((scene.rects.len(), scene.written, scene.finished), (rects, written, finished + 1), "taken as one piece, no band written");
    assert!(layer.uploaded());
}

/// With every band changed and its budget spent, each upload writes one piece (the first band
/// with the run's start, a band, or the finish) and carries the rest; a grid arriving during the
/// run adds its bands, and the scene ends with the newest grid, as taking it whole would.
#[test]
fn snow_a_whole_change_is_carried_across_spent_budgets_and_ends_equal_to_whole() {
    let first = grid(1);
    let mut scene = Counted::default();
    let mut layer = sent(&first, &mut scene);
    let every: Vec<u32> = (0..WIDTH * HEIGHT).collect();
    let second = edited(&first, &every, 9);
    layer.take(made(&second, Some(&every)));
    let pieces = HEIGHT.div_ceil(BAND).div_ceil(RUN) as usize;
    let mut uploads = 0;
    let mut third = None;
    while !layer.uploaded() {
        let (rects, finished) = (scene.rects.len(), scene.finished);
        layer.push(&mut scene, &mut Budget::new(Duration::ZERO).first_free()).unwrap();
        uploads += 1;
        assert_eq!(scene.rects.len() - rects + scene.finished - finished, 1, "upload {uploads} writes one piece");
        assert_eq!(scene.wholes, 0, "never one indivisible write");
        if uploads == 2 {
            let cell = 2 * WIDTH + 3;
            let newest = edited(&second, &[cell], 44);
            layer.take(made(&newest, Some(&[cell])));
            third = Some(newest);
        }
        assert!(uploads <= pieces + 2, "the upload never finished");
    }
    let third = third.unwrap();
    assert_eq!(uploads, pieces + 2, "a run an upload, band 0 written again for the grid that came during the run, then the finish");
    assert_eq!(scene.texels, third.texels.to_vec(), "the pieces leave the newest grid, as a whole upload does");
    assert_eq!(scene.begun, 2, "one run for the composed grid, one for the change");
}

/// Two steps folded ahead, each grid with the bands where it differs from the one before it; the
/// frame takes both in one frame (it applied both steps): the bands uploaded are the union of the
/// two grids' bands, and the scene ends holding the second grid, as a whole write of it would.
/// A grid ahead of the hour applied is not taken. Before, the frame took one grid a batch.
#[test]
fn snow_two_grids_taken_in_one_frame_upload_the_union_of_their_bands() {
    let first = grid(1);
    let mut scene = Counted::default();
    let mut layer = sent(&first, &mut scene);
    let (early, late) = (3 * WIDTH + 4, 30 * WIDTH + 50);
    let second = edited(&first, &[early], 70);
    let third = edited(&second, &[late], 90);
    let fourth = edited(&third, &[early], 110);
    layer.fed.receive(Some(4), made_at(Some(4), &second, Some(&[early])));
    layer.fed.receive(Some(8), made_at(Some(8), &third, Some(&[late])));
    layer.fed.receive(Some(12), made_at(Some(12), &fourth, Some(&[early])));
    layer.fed.applied_through(8);
    layer.exchange(false);
    let rects = scene.rects.len();
    layer.push(&mut scene, &mut Budget::unlimited()).unwrap();
    let mut written = scene.rects[rects..].to_vec();
    written.sort_by_key(|(rows, _)| rows.start);
    assert_eq!(written, [(0..8, 4..5), (24..32, 50..51)], "the first grid's band and the second's, not only the second's");
    assert_eq!(scene.wholes, 0);
    assert_eq!(scene.texels, third.texels.to_vec(), "the scene equals a whole write of the second grid, not the one made ahead of it");
    assert!(layer.uploaded());
}

/// Changed sets join exactly: a band's columns are the bound of both, and not knowing absorbs
/// everything; the pieces come in runs of neighbouring changed bands, whole when not known.
#[test]
fn snow_changed_sets_join_and_are_written_in_runs_of_neighbouring_bands() {
    let cells = [WIDTH * 9 + 5, WIDTH * 15 + 30, 3, WIDTH * 39 + 63];
    let mut changed = Changed::of_cells(&cells, WIDTH);
    assert_eq!(changed, Changed::Bands(vec![3..4, 5..31, 0..0, 0..0, 63..64]));
    changed.join(Changed::of_cells(&[WIDTH * 17 + 40], WIDTH));
    assert_eq!(changed, Changed::Bands(vec![3..4, 5..31, 40..41, 0..0, 63..64]), "a third band, joined");
    let size = [WIDTH, 40];
    assert_eq!(changed.run(size), Some((0..24, 3..41)), "bands 0 to 2 together, their columns' bound");
    changed.written(0..24, size);
    assert_eq!(changed.run(size), Some((32..40, 63..64)));
    changed.written(32..40, size);
    assert_eq!((changed.run(size), changed.bands(40)), (None, 0));
    changed.join(Changed::Unknown);
    assert_eq!(changed, Changed::Unknown, "not known absorbs");
    changed.join(Changed::of_cells(&[1], WIDTH));
    assert_eq!((changed.run(size), changed.bands(40)), (Some((0..40, 0..WIDTH)), 5), "not known is written whole");
}

/// The back texture, lent and written as the worker writes it ([`write_back`]) with `grid`, made
/// of `epoch`'s steps through `stamp`.
fn written(scene: &mut Counted, holds: Option<&Arc<SnowMud>>, grid: &Arc<SnowMud>, epoch: u64, stamp: i32) -> Written {
    let plane = scene.lend().unwrap().unwrap();
    let held = holds.cloned();
    if let Some(held) = &held {
        // What the texture holds: the worker writes only where the grid differs from it.
        scene.back.as_ref().unwrap().1.lock().unwrap().copy_from_slice(&held.texels.to_vec());
    }
    let mut back = Back { plane, holds: held };
    let pages = write_back(&mut back, grid);
    Written { epoch, stamp: Some(stamp), back, mismatched: Vec::new(), pages }
}

/// The worker wrote the grid of hour 4 into the back texture; the frame applies hours 4 and 8 and
/// takes both grids: the back is swapped in, and the frame writes only the band of the grid after
/// it, not the hour-4 grid's band. Before, the frame wrote both grids' bands into the drawn texture.
#[test]
fn snow_a_grid_written_to_the_back_is_swapped_in_and_the_frame_writes_only_what_came_after_it() {
    let first = grid(1);
    let mut scene = Counted::default();
    let mut layer = sent(&first, &mut scene);
    let (early, late) = (3 * WIDTH + 4, 30 * WIDTH + 50);
    let second = edited(&first, &[early], 70);
    let third = edited(&second, &[late], 90);
    layer.lease = Lease::Home(written(&mut scene, None, &second, layer.epoch, 4));
    layer.fed.receive(Some(4), made_at(Some(4), &second, Some(&[early])));
    layer.fed.receive(Some(8), made_at(Some(8), &third, Some(&[late])));
    layer.fed.applied_through(8);
    layer.exchange(false);
    let (rects, wrote) = (scene.rects.len(), scene.written);
    layer.push(&mut scene, &mut Budget::unlimited()).unwrap();
    assert_eq!(scene.swaps, 1, "the hour-4 grid is swapped in");
    assert_eq!(&scene.rects[rects..], [(24..32, 50..51)], "the frame writes the hour-8 grid's band alone");
    assert_eq!(scene.written - wrote, BAND as usize * 4);
    assert_eq!(scene.texels, third.texels.to_vec(), "the scene ends holding the hour-8 grid, as a whole write would");
    assert!(layer.uploaded());

    // The grid due next is written where it differs from what the swapped-out texture holds.
    let fourth = edited(&third, &[early], 120);
    let next = written(&mut scene, Some(&third), &fourth, layer.epoch, 12);
    assert_eq!(next.pages, 1, "one page differs from what the texture held");
    layer.home(next);
    layer.fed.receive(Some(12), made_at(Some(12), &fourth, Some(&[early])));
    layer.fed.applied_through(12);
    layer.exchange(false);
    let (rects, wrote) = (scene.rects.len(), scene.written);
    layer.push(&mut scene, &mut Budget::unlimited()).unwrap();
    assert_eq!((scene.swaps, scene.rects.len() - rects, scene.written - wrote), (2, 0, 0), "swapped in, nothing written on the frame");
    assert_eq!(scene.texels, fourth.texels.to_vec());
}

/// A grid written to the back for steps voided since (a change of pace restarted the worker) is
/// never swapped in, even taken at its hour: the frame writes its bands as before.
#[test]
fn snow_a_back_written_for_steps_voided_since_is_never_swapped_in() {
    let first = grid(1);
    let mut scene = Counted::default();
    let mut layer = sent(&first, &mut scene);
    let cell = 9 * WIDTH + 2;
    let second = edited(&first, &[cell], 33);
    layer.lease = Lease::Home(written(&mut scene, None, &second, layer.epoch, 4));
    layer.epoch += 1;
    layer.fed.void();
    let mut made = made_at(Some(4), &second, Some(&[cell]));
    made.epoch = layer.epoch;
    layer.fed.receive(Some(4), made);
    layer.fed.applied_through(4);
    layer.exchange(false);
    let rects = scene.rects.len();
    layer.push(&mut scene, &mut Budget::unlimited()).unwrap();
    assert_eq!(scene.swaps, 0, "a plane of another epoch is never drawn");
    assert_eq!(&scene.rects[rects..], [(8..16, 2..3)], "the frame writes the band");
    assert_eq!(scene.texels, second.texels.to_vec());
    assert!(!matches!(layer.lease, Lease::Home(_)), "the plane is lent again, not kept");
}

/// The worker writes the grid due next: past the hour applied, the newest within the frames' pace,
/// never the one the texture holds; and only the pages where it differs from that one.
#[test]
fn snow_the_back_takes_the_grid_due_next_and_the_pages_that_differ() {
    let grids: VecDeque<(Option<i32>, Arc<SnowMud>)> = [4, 8, 12, 16].into_iter().enumerate().map(|(at, hour)| (Some(hour), grid(at as u8 + 2))).collect();
    assert_eq!(pick(&grids, None, Some(6), 0), Some(1), "no pace known: the oldest past the hour applied");
    assert_eq!(pick(&grids, None, Some(6), 7), Some(2), "the newest the next frame reaches");
    assert_eq!(pick(&grids, Some(&grids[2].1), Some(6), 7), Some(1), "not the grid it holds");
    assert_eq!(pick(&grids, None, Some(16), 24), None, "none past the hour applied");
    assert_eq!(pick(&grids, None, Some(2), 100), Some(3));

    let first = grid(1);
    let cell = 150 * WIDTH + 7;
    let second = edited(&first, &[cell], 5);
    let bytes = Arc::new(Mutex::new(vec![0; first.texels.len()]));
    let mut back = Back { plane: Box::new(Lent { id: 1, bytes: Arc::clone(&bytes) }), holds: None };
    assert_eq!(write_back(&mut back, &first), first.texels.pages().len() as u64, "holding nothing known, every page");
    assert_eq!(write_back(&mut back, &second), 1, "the changed cell's page alone");
    assert_eq!(*bytes.lock().unwrap(), second.texels.to_vec());
    assert!(Arc::ptr_eq(back.holds.as_ref().unwrap(), &second));
}

/// The back written with the hour-4 grid comes home after the frame took the hour-4 and hour-8
/// grids: it is swapped in all the same, and the frame writes the hour-8 grid's band alone. Before,
/// a back plane home with any grid but the newest taken was lent again and the frame wrote both.
#[test]
fn snow_a_back_home_after_its_grid_was_taken_is_swapped_in_with_the_bands_after_it() {
    let first = grid(1);
    let mut scene = Counted::default();
    let mut layer = sent(&first, &mut scene);
    let (early, late) = (3 * WIDTH + 4, 30 * WIDTH + 50);
    let second = edited(&first, &[early], 70);
    let third = edited(&second, &[late], 90);
    layer.fed.receive(Some(4), made_at(Some(4), &second, Some(&[early])));
    layer.fed.receive(Some(8), made_at(Some(8), &third, Some(&[late])));
    layer.fed.applied_through(8);
    layer.exchange(false);
    let home = written(&mut scene, None, &second, layer.epoch, 4);
    layer.home(home);
    let (rects, wrote) = (scene.rects.len(), scene.written);
    layer.push(&mut scene, &mut Budget::unlimited()).unwrap();
    assert_eq!(scene.swaps, 1, "the hour-4 grid, taken before its plane came home, is swapped in");
    assert_eq!(&scene.rects[rects..], [(24..32, 50..51)], "the frame writes the hour-8 grid's band alone");
    assert_eq!(scene.written - wrote, BAND as usize * 4);
    assert_eq!(scene.texels, third.texels.to_vec());
}
