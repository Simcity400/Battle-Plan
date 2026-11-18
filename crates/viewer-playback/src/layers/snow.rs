//! The snow and mud layer: today's snow fold of WeatherProvinces kept per entity ([`fold`]), run
//! on the layer's own worker, fed the steps themselves: ahead of the head as the host offers them,
//! or when the frame applies them. The worker keeps its own copy of the table from each step's
//! change set alone, folds it and builds the grid of the step's last hour with the bands of rows
//! where it differs from the grid made before it (of steps queued that the frame has applied
//! already, only the last makes one: its bands cover theirs). The frame side takes every grid up
//! to the hour it applied, in order, never one ahead of it, joining their bands, and the upload
//! writes them to the scene a band at a time. Nothing on the frame waits for it, except an
//! unlimited settle (compose, the proof).
//!
//! The scene's second snow texture, which no pass binds, is lent to the worker ([`Back`]): it
//! writes there the pages where a grid it made differs from what the texture holds, the grid due
//! next as the frames' pace runs, and hands the texture back. The frame that takes that grid swaps
//! it in for the drawn one, of its own epoch only (a grid of steps voided since is never shown),
//! and writes on the frame only the bands of the grids it took after it; the texture swapped out
//! is lent again. A grid that refreshes in a cross-fade is written by the frame as before.

mod fold;

use super::{
    Budget, Carried, Due, Fed, Gpu, Inputs, Layer, List, Num, Published, View, clock,
    owned::{Shape, Table},
};
use crate::playhead_host::mirror::{Mirror, Mirrors};
use fold::Fold;
use map_snow::{Refresh, SnowCells, SnowMud, Summary, winter::Winter};
use playhead::Step;
use replay_data::{Hour, rows::TableId};
use std::{
    collections::VecDeque,
    io,
    ops::Range,
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    time::Instant,
};
use viewer_foundation::map::presentation::Continuity;

/// Rows of the grid a band holds: the grain at which a change is kept.
const BAND: u32 = 8;
/// Bands one piece of an upload writes at most, as one rectangle: a write a call costs more than
/// its bytes, so neighbouring changed bands go together (at most 64 rows, 360 KB on the
/// 1,408-cell grid).
const RUN: u32 = 8;

/// The cells where one grid differs from another, as the columns changed in each band of
/// [`BAND`] rows (an empty range: none); or not known (a grid made whole, a product discarded,
/// a refusal), which is written whole. Two join exactly and cheaply: a band's columns are the
/// union's bound.
#[derive(Clone, Debug, PartialEq)]
enum Changed {
    Unknown,
    Bands(Vec<Range<u32>>),
}

impl Default for Changed {
    fn default() -> Self {
        Self::Bands(Vec::new())
    }
}

impl Changed {
    /// The bands holding `cells` of a `width`-cell row.
    fn of_cells(cells: &[u32], width: u32) -> Self {
        let mut changed = Self::default();
        for &cell in cells {
            let column = cell % width;
            changed.mark(cell / width / BAND, column..column + 1);
        }
        changed
    }

    fn mark(&mut self, band: u32, columns: Range<u32>) {
        let Self::Bands(bands) = self else { return };
        if columns.start >= columns.end {
            return;
        }
        let at = band as usize;
        if bands.len() <= at {
            bands.resize(at + 1, 0..0);
        }
        let held = &mut bands[at];
        *held = if held.start >= held.end { columns } else { held.start.min(columns.start)..held.end.max(columns.end) };
    }

    /// Every cell of `self` or `other`.
    fn join(&mut self, other: Changed) {
        match other {
            Self::Unknown => *self = Self::Unknown,
            Self::Bands(other) => {
                for (band, columns) in other.into_iter().enumerate() {
                    self.mark(band as u32, columns);
                }
            }
        }
    }

    /// Bands holding a change; every band of a grid `height` rows high when unknown.
    fn bands(&self, height: u32) -> usize {
        match self {
            Self::Unknown => height.div_ceil(BAND) as usize,
            Self::Bands(bands) => bands.iter().filter(|columns| columns.start < columns.end).count(),
        }
    }

    /// The next piece of a `width` by `height` grid: the rows from the first changed band through
    /// at most [`RUN`] changed bands in a row, and the bound of their columns.
    fn run(&self, [width, height]: [u32; 2]) -> Option<(Range<u32>, Range<u32>)> {
        let bands = height.div_ceil(BAND);
        let columns = |band: u32| match self {
            Self::Unknown => 0..width,
            Self::Bands(held) => held.get(band as usize).cloned().unwrap_or(0..0),
        };
        let first = (0..bands).find(|&band| !columns(band).is_empty())?;
        let mut last = first;
        let mut bound = columns(first);
        while last + 1 < bands && last + 1 < first + RUN && !columns(last + 1).is_empty() {
            last += 1;
            let next = columns(last);
            bound = bound.start.min(next.start)..bound.end.max(next.end);
        }
        Some((first * BAND..((last + 1) * BAND).min(height), bound.start.min(width)..bound.end.min(width)))
    }

    /// The bands of `rows` written, of a grid `height` rows high.
    fn written(&mut self, rows: Range<u32>, [width, height]: [u32; 2]) {
        if let Self::Unknown = self {
            *self = Self::Bands(vec![0..width; height.div_ceil(BAND) as usize]);
        }
        let Self::Bands(bands) = self else { return };
        for band in rows.start / BAND..rows.end.div_ceil(BAND) {
            if let Some(columns) = bands.get_mut(band as usize) {
                *columns = 0..0;
            }
        }
    }

    fn bytes(&self) -> usize {
        match self {
            Self::Unknown => 0,
            Self::Bands(bands) => bands.capacity() * 8,
        }
    }
}

/// What the uploads did, for the paced report: grids taken and how many bands each changed (and
/// how many were not known), pieces written, their bytes and their time; grids swapped in from the
/// back texture and the pages the worker wrote there for them.
#[derive(Clone, Copy, Debug, Default)]
struct Figures {
    grids: u64,
    bands: u64,
    unknown: u64,
    pieces: u64,
    written: u64,
    bytes: u64,
    ns: u64,
    swaps: u64,
    back_pages: u64,
    /// Swaps' and whole grids' nanoseconds on the frame, and the grids taken whole.
    swaps_ns: u64,
    wholes: u64,
    wholes_ns: u64,
}

/// A texture the scene does not draw, lent for the worker to fill ([`Target::lend`]).
pub(super) trait Plane: Send {
    /// Writes `bytes` (RGBA, four a texel) from texel `first` on, rows the grid's width.
    fn write(&mut self, first: usize, bytes: &[u8]);
    fn any(&self) -> &dyn std::any::Any;
}

struct ScenePlane {
    texture: eframe::wgpu::Texture,
    queue: eframe::wgpu::Queue,
    width: u32,
}

impl Plane for ScenePlane {
    fn write(&mut self, first: usize, bytes: &[u8]) {
        viewer_world::map::scene::write_snow_mud_texels(&self.queue, &self.texture, self.width, first, bytes);
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The plane lent, and the grid it holds whole (none: not known, so it is written whole).
pub(super) struct Back {
    plane: Box<dyn Plane>,
    holds: Option<Arc<SnowMud>>,
}

/// The back plane written whole with a grid made of `epoch`'s steps through `stamp`, the grid's
/// pages whose cells are shown unlike its bytes, and the pages written for it.
pub(super) struct Written {
    epoch: u64,
    stamp: Option<Hour>,
    back: Back,
    mismatched: Vec<usize>,
    pages: u64,
}

impl Written {
    fn grid(&self) -> &Arc<SnowMud> {
        self.back.holds.as_ref().expect("a written plane holds its grid")
    }
}

/// Where the back plane is.
enum Lease {
    /// Not lent: none made yet, or the last could not be swapped in.
    None,
    Away,
    Home(Written),
}

/// Writes into `back` the pages where `grid` differs from what it holds (every page when that is
/// not known); the pages written.
fn write_back(back: &mut Back, grid: &Arc<SnowMud>) -> u64 {
    let pages: Vec<usize> = match &back.holds {
        Some(held) if held.texels.len() == grid.texels.len() => grid.texels.changed_page_indices(&held.texels).collect(),
        _ => (0..grid.texels.pages().len()).collect(),
    };
    for &page in &pages {
        back.plane.write(page * map_display_data::PAGE_ITEMS / 4, grid.texels.page(page));
    }
    back.holds = Some(Arc::clone(grid));
    pages.len() as u64
}

/// `grid`'s pages holding a cell shown unlike its bytes, as taking it in rectangles finds them.
fn mismatched(grid: &SnowMud) -> Vec<usize> {
    grid.texels.pages().enumerate().filter(|(_, bytes)| bytes.chunks_exact(4).any(|cell| cell[0] != cell[3] || cell[2] != cell[1])).map(|(page, _)| page).collect()
}

/// Of the grids made (oldest first), the one to write into the back plane: of those past the hour
/// applied (`due`) and not the one it holds, the newest within the frames' `pace` of hours past
/// it, else the oldest.
fn pick(grids: &VecDeque<(Option<Hour>, Arc<SnowMud>)>, holds: Option<&Arc<SnowMud>>, due: Option<Hour>, pace: i64) -> Option<usize> {
    let reach = due.map(|due| i64::from(due) + pace);
    let (mut first, mut best) = (None, None);
    for (at, (stamp, grid)) in grids.iter().enumerate() {
        let ahead = due.is_none_or(|due| stamp.is_some_and(|stamp| stamp > due));
        if !ahead || holds.is_some_and(|held| Arc::ptr_eq(held, grid)) {
            continue;
        }
        first.get_or_insert(at);
        if reach.is_none_or(|reach| stamp.is_some_and(|stamp| i64::from(stamp) <= reach)) {
            best = Some(at);
        }
    }
    best.or(first)
}

/// The worker's side of the back plane: the plane while lent, the grids made since its epoch's
/// reset (each once, by allocation), and the hours the frames applied between two lends.
#[derive(Default)]
struct Lending {
    back: Option<Back>,
    grids: VecDeque<(Option<Hour>, Arc<SnowMud>)>,
    pace: i64,
    lent_at: Option<Hour>,
}

/// Grids the worker keeps to choose from: more than the steps the playhead builds ahead.
const KEPT: usize = 32;

/// Grids the frame remembers taking since the scene's: more than a frame takes at any speed, and
/// few, since each holds the pages it changed.
const TAKEN: usize = 8;

impl Lending {
    fn lend(&mut self, back: Back, due: Option<Hour>) {
        if let (Some(now), Some(was)) = (due, self.lent_at) {
            self.pace = i64::from(now) - i64::from(was);
        }
        (self.back, self.lent_at) = (Some(back), due);
    }

    fn made(&mut self, stamp: Option<Hour>, grid: &Arc<SnowMud>) {
        if self.grids.back().is_some_and(|(_, held)| Arc::ptr_eq(held, grid)) {
            return;
        }
        self.grids.push_back((stamp, Arc::clone(grid)));
        while self.grids.len() > KEPT {
            self.grids.pop_front();
        }
    }

    /// The grid due next written into the plane lent and handed back; false once the frame is gone.
    fn write(&mut self, epoch: u64, due: Option<Hour>, out: &Sender<Out>) -> bool {
        self.grids.retain(|(stamp, _)| due.is_none_or(|due| stamp.is_some_and(|stamp| stamp > due)));
        let Some(at) = self.back.as_ref().and_then(|back| pick(&self.grids, back.holds.as_ref(), due, self.pace)) else { return true };
        let (stamp, grid) = self.grids[at].clone();
        let mut back = self.back.take().expect("picked for a plane held");
        let pages = write_back(&mut back, &grid);
        out.send(Out::Written(Written { epoch, stamp, back, mismatched: mismatched(&grid), pages })).is_ok()
    }
}

/// Where the upload writes: the scene, or a test's stand-in.
trait Target {
    /// Whether `grid` can be written a band at a time as it would be taken whole.
    fn takes_parts(&self, grid: &SnowMud, continuity: Continuity) -> bool;
    fn whole(&mut self, grid: &Arc<SnowMud>, continuity: Continuity) -> io::Result<()>;
    fn begin(&mut self, grid: &SnowMud, continuity: Continuity) -> io::Result<()>;
    fn rect(&mut self, grid: &SnowMud, rows: Range<u32>, columns: Range<u32>) -> io::Result<()>;
    fn finish(&mut self, grid: &Arc<SnowMud>, continuity: Continuity) -> io::Result<()>;
    /// The texture no pass binds, of the drawn one's size, for the worker to fill.
    fn lend(&mut self) -> io::Result<Option<Box<dyn Plane>>>;
    /// Draws `back`, written whole with `grid`, in place of the drawn texture; false when it is
    /// not the texture lent or the grid cannot be taken in parts.
    fn swap(&mut self, back: &dyn Plane, grid: &Arc<SnowMud>, mismatched: Vec<usize>) -> io::Result<bool>;
}

struct OnScene<'a, 'b>(&'a mut Gpu<'b>);

impl Target for OnScene<'_, '_> {
    fn takes_parts(&self, grid: &SnowMud, continuity: Continuity) -> bool {
        self.0.scene.snow_mud_takes_parts(grid, continuity)
    }

    fn whole(&mut self, grid: &Arc<SnowMud>, continuity: Continuity) -> io::Result<()> {
        let gpu = &mut *self.0;
        gpu.scene.set_hour_layers(gpu.device, gpu.queue, viewer_world::map::scene::HourLayers { snow_mud: Some((grid, continuity)), ..Default::default() })
    }

    fn begin(&mut self, grid: &SnowMud, continuity: Continuity) -> io::Result<()> {
        self.0.scene.begin_snow_mud_parts(grid, continuity)
    }

    fn rect(&mut self, grid: &SnowMud, rows: Range<u32>, columns: Range<u32>) -> io::Result<()> {
        let gpu = &mut *self.0;
        gpu.scene.write_snow_mud_rect(gpu.queue, grid, rows, columns)
    }

    fn finish(&mut self, grid: &Arc<SnowMud>, continuity: Continuity) -> io::Result<()> {
        let gpu = &mut *self.0;
        gpu.scene.finish_snow_mud_parts(gpu.device, gpu.queue, grid, continuity)
    }

    fn lend(&mut self) -> io::Result<Option<Box<dyn Plane>>> {
        let gpu = &mut *self.0;
        let texture = gpu.scene.lend_snow_mud_back(gpu.device, gpu.queue)?;
        Ok(Some(Box::new(ScenePlane { width: texture.width(), texture, queue: gpu.queue.clone() })))
    }

    fn swap(&mut self, back: &dyn Plane, grid: &Arc<SnowMud>, mismatched: Vec<usize>) -> io::Result<bool> {
        let Some(plane) = back.any().downcast_ref::<ScenePlane>() else { return Ok(false) };
        let gpu = &mut *self.0;
        gpu.scene.swap_snow_mud(gpu.device, gpu.queue, &plane.texture, grid, mismatched)
    }
}

/// What the layer reads of the content: the cell composition and the winter volumes.
pub struct Content {
    cells: SnowCells,
    winter: Winter,
    /// Per province id: the volumes whose records name it.
    volumes: Vec<Vec<u32>>,
    boundary: Vec<u32>,
    /// Per cell: whether it is a boundary cell.
    on_boundary: Vec<bool>,
}

impl Content {
    pub fn new(cells: SnowCells, winter: Winter) -> Self {
        let mut volumes = vec![Vec::new(); cells.provinces()];
        for (volume, records) in winter.records.iter().enumerate() {
            for record in records {
                if let Some(held) = volumes.get_mut(record.province as usize)
                    && held.last() != Some(&(volume as u32))
                {
                    held.push(volume as u32);
                }
            }
        }
        let [width, height] = cells.dimensions();
        let boundary: Vec<u32> = if width > 0 && height > 0 { map_snow::boundary(width, height).collect() } else { Vec::new() };
        let mut on_boundary = vec![false; cells.cells()];
        for &cell in &boundary {
            on_boundary[cell as usize] = true;
        }
        Self { cells, winter, volumes, boundary, on_boundary }
    }

    fn bytes(&self) -> usize {
        self.volumes.capacity() * 24 + self.volumes.iter().map(|held| held.capacity() * 4).sum::<usize>() + self.boundary.capacity() * 4 + self.on_boundary.capacity()
    }
}

#[derive(Clone, Copy)]
struct Columns {
    snow: Num<i64>,
    display_snow: Num<u8>,
    display_mud: Num<u8>,
    display: List,
}

impl Columns {
    fn shape(&self) -> Shape {
        Shape::new(TableId::WeatherProvinces, &[self.snow.column, self.display_snow.column, self.display_mud.column], &[(self.display.column, false)])
    }
}

enum Job {
    /// The table whole (none when the hour's tables hold no mirror of it): the fold made anew,
    /// its grid stamped `stamp` (the hour applied when it starts over; none at a compose).
    Reset { epoch: u64, table: Option<Box<Table>>, columns: Columns, stamp: Option<Hour> },
    Step { epoch: u64, step: Arc<Step> },
    /// The back plane, to fill with the grid due next.
    Lend(Back),
}

/// What the worker hands the frame.
enum Out {
    Made(Made),
    Written(Written),
}

/// What the worker made of the steps it folded through the hour it is stamped with (none: the
/// compose's): the grid (none while refused), the bands of rows where it differs from the grid
/// made before it (none when made whole), why the hour is refused, the marks, and the grid's error
/// if it could not be built.
struct Made {
    epoch: u64,
    stamp: Option<Hour>,
    grid: Result<Option<Arc<SnowMud>>, String>,
    changed: Changed,
    refused: Option<String>,
    marked: [u64; 3],
    held: usize,
    ns: u64,
}

/// The next job is a step of `epoch` the frame has applied already, or a reset: the step folded
/// before it makes no grid (the next grid's bands cover its cells, which the fold keeps until a
/// grid is made).
fn superseded(next: Option<&Job>, epoch: u64, due: &Due) -> bool {
    match next {
        Some(Job::Reset { .. }) => true,
        Some(Job::Step { epoch: of, step }) => *of == epoch && due.reached(step.last()),
        Some(Job::Lend(_)) | None => false,
    }
}

fn run(content: Arc<Content>, jobs: Receiver<Job>, made: Sender<Out>, due: Due) {
    // The fold, its table, its marks as last handed back, and the hour folded through.
    let mut work: Option<(Fold, Option<Table>, [u64; 3], Option<Hour>)> = None;
    let mut epoch = 0;
    let mut lending = Lending::default();
    while let Ok(job) = jobs.recv() {
        let mut queue: VecDeque<Job> = std::iter::once(job).chain(jobs.try_iter()).collect();
        let mut started = Instant::now();
        while let Some(job) = queue.pop_front() {
            match job {
                Job::Reset { epoch: next, table, columns, stamp } => {
                    epoch = next;
                    lending.grids.clear();
                    work = Some((Fold::new(Arc::clone(&content), columns), table.map(|table| *table), [0; 3], stamp));
                }
                Job::Lend(back) => {
                    lending.lend(back, due.hour());
                    if !lending.write(epoch, due.hour(), &made) {
                        return;
                    }
                    continue;
                }
                Job::Step { epoch: of, step } if of == epoch => {
                    let Some((fold, table, _, stamp)) = &mut work else { continue };
                    *stamp = (*stamp).max(Some(step.last()));
                    if let Some(table) = table {
                        if step.gap() {
                            table.gap();
                        } else if let Some(changes) = step.table(TableId::WeatherProvinces.index())
                            && let Some(delta) = table.step(changes)
                        {
                            fold.apply(&delta, table);
                        }
                    }
                    drop(step);
                    if superseded(queue.front(), epoch, &due) {
                        continue;
                    }
                }
                Job::Step { .. } => continue,
            }
            let Some((fold, table, reported, stamp)) = &mut work else { continue };
            fold.settle(table.as_ref());
            let (grid, changed) = match fold.grid() {
                Ok((grid, Some(cells))) => (Ok(grid), Changed::of_cells(&cells, content.cells.dimensions()[0].max(1))),
                Ok((grid, None)) => (Ok(grid), Changed::Unknown),
                Err(error) => (Err(error.to_string()), Changed::Unknown),
            };
            let held = fold.held() + table.as_ref().map_or(0, Table::bytes);
            let now = fold.marked();
            let marked = std::array::from_fn(|at| now[at] - reported[at]);
            *reported = now;
            if let Ok(Some(grid)) = &grid {
                lending.made(*stamp, grid);
            }
            let sent = Made { epoch, stamp: *stamp, grid, changed, refused: fold.refused(), marked, held, ns: started.elapsed().as_nanos() as u64 };
            if made.send(Out::Made(sent)).is_err() || !lending.write(epoch, due.hour(), &made) {
                return;
            }
            started = Instant::now();
        }
    }
}

pub struct Snow {
    content: Result<Arc<Content>, String>,
    columns: Option<Columns>,
    jobs: Option<Sender<Job>>,
    made: Option<Receiver<Out>>,
    epoch: u64,
    /// The steps handed to the worker and the grids it made, each taken once its hour is applied.
    fed: Fed<Made>,
    /// What the worker last made.
    grid: Result<Option<Arc<SnowMud>>, String>,
    refused: Option<String>,
    marked: [u64; 3],
    worker_held: usize,
    /// The grid the scene holds whole, and whether the next upload starts the presentation over;
    /// where the newest grid differs from it, and the run of pieces under way on the scene.
    shown: Option<Arc<SnowMud>>,
    reset: bool,
    owed: Changed,
    parting: Option<Continuity>,
    /// The back plane; whether the grid written there was taken, and the bands of the grids taken
    /// after it.
    lease: Lease,
    back_taken: bool,
    after_back: Changed,
    /// The grids taken since the one the scene holds whole, oldest first, each with the bands where
    /// it differs from the one before it: a back plane written with one of them is swapped in with
    /// the bands of those after it.
    taken: VecDeque<(Arc<SnowMud>, Changed)>,
    figures: Figures,
    /// Nanoseconds on the frame since the layer was made: handing steps to the worker, exchanging
    /// with it, the scene's take; and on the worker, with its folds.
    spent: [u64; 3],
    worker: (u64, u64),
}

impl Snow {
    pub fn new(content: Result<Arc<Content>, String>) -> Self {
        Self {
            content,
            columns: None,
            jobs: None,
            made: None,
            epoch: 0,
            fed: Fed::default(),
            grid: Ok(None),
            refused: None,
            marked: [0; 3],
            worker_held: 0,
            shown: None,
            reset: true,
            owed: Changed::default(),
            parting: None,
            lease: Lease::None,
            back_taken: false,
            after_back: Changed::default(),
            taken: VecDeque::new(),
            figures: Figures::default(),
            spent: [0; 3],
            worker: (0, 0),
        }
    }

    pub fn fresh(&self) -> Self {
        Self { columns: self.columns, ..Self::new(self.content.clone()) }
    }

    /// Why the hour is refused, in today's words; none when it draws.
    pub fn refused(&self) -> Option<String> {
        self.refused.clone()
    }

    fn drawn(&self) -> Option<&SnowMud> {
        self.grid.as_ref().ok().and_then(Option::as_deref)
    }

    /// The texels, RGBA a cell; none while refused.
    pub fn texels(&self) -> Vec<[u8; 4]> {
        self.drawn().map_or_else(Vec::new, |grid| grid.texels.to_vec().chunks_exact(4).map(|texel| [texel[0], texel[1], texel[2], texel[3]]).collect())
    }

    pub fn unknown(&self) -> Vec<u8> {
        self.drawn().map_or_else(Vec::new, |grid| grid.unknown.to_vec())
    }

    /// Each province's snow word, none where no row holds it.
    pub fn province_snow(&self) -> impl Iterator<Item = Option<i64>> {
        self.drawn().map_or_else(Vec::new, |grid| grid.province_snow.to_vec()).into_iter()
    }

    pub fn factors(&self) -> Vec<Option<f32>> {
        self.drawn().and_then(|grid| grid.post_factors.as_ref()).map_or_else(Vec::new, |factors| factors.to_vec())
    }

    pub fn refresh(&self) -> Option<Refresh> {
        self.drawn().and_then(|grid| grid.refresh)
    }

    pub fn summary(&self) -> Option<Summary> {
        self.drawn().map(|grid| grid.summary)
    }

    /// The notice that part of the snow and mud drawn is unknown, which the map hatches, as today's.
    pub fn note(&self) -> Option<&'static str> {
        self.summary().and_then(|summary| summary.note())
    }

    pub fn marked(&self) -> [u64; 3] {
        self.marked
    }

    /// Microseconds on the frame, by part: handing steps to the worker, exchanging with it, the
    /// scene's take.
    pub fn spent(&self) -> [u64; 3] {
        self.spent.map(|ns| ns / 1000)
    }

    /// Microseconds on the worker, and its folds, since the layer was made.
    pub fn worker(&self) -> (u64, u64) {
        (self.worker.0 / 1000, self.worker.1)
    }

    pub fn content_bytes(&self) -> usize {
        self.content.as_ref().map_or(0, |content| content.bytes())
    }

    /// The newest hour its worker has made a grid of, as received on the frame.
    #[cfg(test)]
    pub fn made_through(&self) -> Option<Hour> {
        self.fed.made_through()
    }

    fn take(&mut self, made: Made) {
        if let Ok(Some(grid)) = &made.grid {
            self.figures.grids += 1;
            self.figures.bands += made.changed.bands(grid.height) as u64;
            self.figures.unknown += u64::from(made.changed == Changed::Unknown);
            if self.written_is(grid) {
                (self.back_taken, self.after_back) = (true, Changed::default());
            } else if self.back_taken {
                self.after_back.join(made.changed.clone());
            }
            if self.taken.len() == TAKEN {
                self.taken.pop_front();
            }
            self.taken.push_back((Arc::clone(grid), made.changed.clone()));
            self.owed.join(made.changed);
        } else {
            // While refused nothing is written: what the scene holds is not known against the next.
            self.owed = Changed::Unknown;
            self.back_taken = false;
            self.taken.clear();
        }
        (self.grid, self.refused, self.worker_held) = (made.grid, made.refused, made.held);
        for (held, made) in self.marked.iter_mut().zip(made.marked) {
            *held += made;
        }
        self.worker.0 += made.ns;
        self.worker.1 += 1;
    }

    /// Receives what the worker made and takes every grid due, in order: those of the hours applied,
    /// each marking its bands, so the bands owed join those of every grid passed over. With `wait`,
    /// until the grid of the last hour applied has arrived.
    fn exchange(&mut self, wait: bool) {
        if let Some(receive) = self.made.take() {
            loop {
                let out = if wait && !self.fed.arrived() { receive.recv().ok() } else { receive.try_recv().ok() };
                match out {
                    None => break,
                    Some(Out::Made(made)) => {
                        if made.epoch == self.epoch {
                            self.fed.receive(made.stamp, made);
                        }
                    }
                    Some(Out::Written(written)) => self.home(written),
                }
            }
            self.made = Some(receive);
        }
        let mut due = Vec::new();
        self.fed.take(|made| due.push(made));
        for made in due {
            self.take(made);
        }
    }
}

impl Snow {
    /// The back plane home, written with a grid: taken already when it is one of the grids taken
    /// since the scene's, with the bands of those taken after it.
    fn home(&mut self, written: Written) {
        let at = (written.epoch == self.epoch).then(|| self.taken.iter().rposition(|(grid, _)| Arc::ptr_eq(grid, written.grid()))).flatten();
        self.back_taken = at.is_some();
        self.after_back = Changed::default();
        if let Some(at) = at {
            for (_, changed) in self.taken.iter().skip(at + 1) {
                self.after_back.join(changed.clone());
            }
        }
        self.lease = Lease::Home(written);
    }

    /// Whether `grid` is the one the back plane was written with, of this epoch.
    fn written_is(&self, grid: &Arc<SnowMud>) -> bool {
        matches!(&self.lease, Lease::Home(written) if written.epoch == self.epoch && Arc::ptr_eq(written.grid(), grid))
    }

    fn lend(&mut self, back: Back) {
        self.lease = match &self.jobs {
            Some(jobs) if jobs.send(Job::Lend(back)).is_ok() => Lease::Away,
            _ => Lease::None,
        };
    }

    /// The back plane's turn in an upload. Written with a grid taken, of this epoch, while the
    /// scene holds a whole grid drawn at once: swapped in, the bands of the grids taken after it
    /// left owed, and the texture swapped out lent again holding what the scene showed. Written
    /// with a grid not yet due: kept. Otherwise lent again to write the grid due next; lent first
    /// once the scene holds a grid drawn at once.
    fn swap_or_lend(&mut self, target: &mut dyn Target) -> io::Result<()> {
        let Ok(Some(grid)) = &self.grid else { return Ok(()) };
        let at_once = grid.refresh.is_none();
        match std::mem::replace(&mut self.lease, Lease::None) {
            Lease::Away => self.lease = Lease::Away,
            Lease::None => {
                if at_once
                    && self.shown.is_some()
                    && self.jobs.is_some()
                    && let Some(plane) = target.lend()?
                {
                    self.lend(Back { plane, holds: None });
                }
            }
            Lease::Home(written) => {
                let current = written.epoch == self.epoch;
                let swappable = current && self.back_taken && self.parting.is_none() && !self.reset && self.shown.as_ref().is_some_and(|shown| !Arc::ptr_eq(shown, written.grid()));
                if swappable {
                    self.back_taken = false;
                    let held = Arc::clone(written.grid());
                    let started = clock::now();
                    let swapped = target.swap(&*written.back.plane, &held, written.mismatched.clone())?;
                    self.figures.swaps_ns += clock::now().duration_since(started).as_nanos() as u64;
                    if swapped {
                        self.figures.swaps += 1;
                        self.figures.back_pages += written.pages;
                        if let Some(at) = self.taken.iter().rposition(|(grid, _)| Arc::ptr_eq(grid, &held)) {
                            self.taken.drain(..=at);
                        }
                        let shown = self.shown.replace(held);
                        self.owed = std::mem::take(&mut self.after_back);
                        drop(written);
                        if let Some(plane) = target.lend()? {
                            self.lend(Back { plane, holds: shown });
                        }
                    }
                    return Ok(());
                }
                if current && !self.back_taken && written.stamp > self.fed.applied_hour() {
                    self.lease = Lease::Home(written);
                    return Ok(());
                }
                self.back_taken = false;
                if at_once {
                    self.lend(written.back);
                }
            }
        }
        Ok(())
    }

    /// The newest grid written to `target`: whole when it cannot be taken in parts, else the bands
    /// where it differs from the grid there, a run of neighbouring bands a budget check; the
    /// first piece of a call always goes (the host's free unit), the rest is carried.
    fn push(&mut self, target: &mut dyn Target, budget: &mut Budget) -> io::Result<()> {
        let grid = match &self.grid {
            Err(why) => return Err(io::Error::other(why.clone())),
            Ok(grid) => grid.clone(),
        };
        let started = Instant::now();
        self.swap_or_lend(target)?;
        let Some(grid) = grid.filter(|_| !self.uploaded()) else {
            self.spent[2] += started.elapsed().as_nanos() as u64;
            return Ok(());
        };
        let pushed = self.push_grid(target, &grid, budget);
        self.spent[2] += started.elapsed().as_nanos() as u64;
        pushed
    }

    fn push_grid(&mut self, target: &mut dyn Target, grid: &Arc<SnowMud>, budget: &mut Budget) -> io::Result<()> {
        let size = [grid.width, grid.height];
        // A run under way whose grid can no longer be taken a band at a time is taken over whole.
        if self.parting.is_some_and(|run| run == Continuity::Continue && (grid.refresh.is_some() || self.shown.as_ref().is_some_and(|shown| [shown.width, shown.height] != size))) {
            (self.parting, self.reset) = (None, true);
        }
        let mut free = false;
        if self.parting.is_none() {
            let continuity = if self.reset { Continuity::Reset } else { Continuity::Continue };
            if !budget.left() {
                return Ok(());
            }
            if !target.takes_parts(grid, continuity) {
                let started = clock::now();
                target.whole(grid, continuity)?;
                (self.figures.wholes, self.figures.wholes_ns) = (self.figures.wholes + 1, self.figures.wholes_ns + clock::now().duration_since(started).as_nanos() as u64);
                (self.shown, self.reset, self.owed) = (Some(Arc::clone(grid)), false, Changed::default());
                self.taken.clear();
                return Ok(());
            }
            if self.reset {
                self.owed = Changed::Unknown;
            }
            target.begin(grid, continuity)?;
            (self.parting, self.reset, free) = (Some(continuity), false, true);
        }
        while let Some((rows, columns)) = self.owed.run(size) {
            if !std::mem::take(&mut free) && !budget.left() {
                return Ok(());
            }
            let started = clock::now();
            let bytes = rows.len() * columns.len() * 4;
            target.rect(grid, rows.clone(), columns)?;
            self.figures.pieces += 1;
            self.figures.written += u64::from(rows.end.div_ceil(BAND) - rows.start / BAND);
            self.owed.written(rows, size);
            self.figures.bytes += bytes as u64;
            self.figures.ns += clock::now().duration_since(started).as_nanos() as u64;
        }
        if !std::mem::take(&mut free) && !budget.left() {
            return Ok(());
        }
        let continuity = self.parting.take().unwrap_or(Continuity::Continue);
        target.finish(grid, continuity)?;
        self.shown = Some(Arc::clone(grid));
        self.taken.clear();
        Ok(())
    }
}

/// The snow uploads' figures for the paced report: per grid taken, the bands of rows it changed
/// (of 64 on a 512-row grid), and per piece written its bytes and microseconds.
pub fn summary(side: &crate::playhead_host::MapSide) -> serde_json::Value {
    let Some(layer) = side.parts().and_then(|(_, layers)| layers.iter().find_map(|layer| layer.any().downcast_ref::<Snow>())) else { return serde_json::Value::Null };
    let figures = layer.figures;
    let per = |sum: u64, count: u64| (count > 0).then(|| sum as f64 / count as f64);
    serde_json::json!({
        "grids_taken": figures.grids,
        "bands_changed_a_grid_mean": per(figures.bands, figures.grids),
        "grids_not_known_against_the_last": figures.unknown,
        "pieces_written": figures.pieces,
        "bytes_a_piece_mean": per(figures.bytes, figures.pieces),
        "us_a_piece_mean": per(figures.ns, figures.pieces * 1000),
        "bands_written": figures.written,
        "us_a_band_written_mean": per(figures.ns, figures.written * 1000),
        "grids_swapped_in": figures.swaps,
        "back_pages_written_a_swap_mean": per(figures.back_pages, figures.swaps),
        "us_a_swap_mean": per(figures.swaps_ns, figures.swaps * 1000),
        "grids_taken_whole": figures.wholes,
        "us_a_whole_grid_mean": per(figures.wholes_ns, figures.wholes * 1000),
    })
}

/// The snow layer of a loaded recording's content: its cell composition and winter volumes.
pub fn for_document(document: &crate::document::Document) -> Box<dyn Layer> {
    let content = document.content.products.snow_content().map(|content| Arc::new(Content::new(content.cells.clone(), content.winter.clone()))).map_err(|error| error.to_string());
    Box::new(Snow::new(content))
}

impl Layer for Snow {
    fn name(&self) -> &'static str {
        "snow"
    }

    fn placed(&mut self, _view: &crate::layers::View) -> Option<u64> {
        Some(0)
    }

    fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        if let Err(why) = &self.content {
            return Err(io::Error::other(format!("The snow layer is refused: its content cannot be read: {why}")));
        }
        let table = TableId::WeatherProvinces;
        inputs.table(table, true)?;
        self.columns = Some(Columns {
            snow: inputs.number(table, "snow")?,
            display_snow: inputs.number(table, "display_snow")?,
            display_mud: inputs.number(table, "display_mud")?,
            display: inputs.list(table, "display")?,
        });
        Ok(())
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        let (Ok(content), Some(columns)) = (&self.content, self.columns) else { return Ok(()) };
        if self.jobs.is_none() {
            let (jobs, receive) = mpsc::channel();
            let (send, made) = mpsc::channel();
            let (content, due) = (Arc::clone(content), self.fed.due());
            std::thread::Builder::new().name("snow fold".into()).spawn(move || {
                viewer_foundation::worker_priority::background_priority();
                run(content, receive, send, due)
            })?;
            (self.jobs, self.made) = (Some(jobs), Some(made));
        }
        self.epoch += 1;
        // What was made ahead for the old epoch is dropped unseen; the scene starts over whole.
        self.fed.compose(mirrors.get(TableId::WeatherProvinces).and_then(Mirror::head));
        let table = mirrors.get(TableId::WeatherProvinces).map(|mirror| Box::new(Table::from_mirror(mirror, &columns.shape())));
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(Job::Reset { epoch: self.epoch, table, columns, stamp: None });
        }
        (self.shown, self.reset, self.owed, self.parting) = (None, true, Changed::Unknown, None);
        self.taken.clear();
        self.exchange(true);
        Ok(())
    }

    fn apply(&mut self, step: &Step, _mirrors: &Mirrors) {
        if self.columns.is_some() {
            self.fed.applied(step);
        }
    }

    fn wants_ahead(&self) -> bool {
        true
    }

    /// The worker starts over from the mirrors at the hour applied; its first grid there is whole,
    /// and stands for that hour: a void in the frame that applied a step hands no step after it.
    fn void_ahead(&mut self, mirrors: &Mirrors, _kept: Option<Hour>) {
        let (Some(columns), Some(jobs)) = (self.columns, &self.jobs) else { return };
        self.epoch += 1;
        self.fed.void();
        let table = mirrors.get(TableId::WeatherProvinces).map(|mirror| Box::new(Table::from_mirror(mirror, &columns.shape())));
        let _ = jobs.send(Job::Reset { epoch: self.epoch, table, columns, stamp: self.fed.applied_hour() });
    }

    fn ahead(&mut self, steps: &[Arc<Step>]) {
        let started = Instant::now();
        if let Some(jobs) = &self.jobs {
            for step in self.fed.hand(steps) {
                let _ = jobs.send(Job::Step { epoch: self.epoch, step: Arc::clone(step) });
            }
        }
        self.spent[0] += started.elapsed().as_nanos() as u64;
    }

    fn settle(&mut self, _mirrors: &Mirrors, budget: &mut Budget) -> io::Result<()> {
        let started = Instant::now();
        self.exchange(budget.unlimited_now());
        self.spent[1] += started.elapsed().as_nanos() as u64;
        Ok(())
    }

    fn upload(&mut self, gpu: &mut Gpu<'_>, budget: &mut Budget) -> io::Result<()> {
        self.push(&mut OnScene(gpu), budget)
    }

    fn uploaded(&self) -> bool {
        match (&self.grid, &self.shown) {
            (Ok(Some(grid)), Some(shown)) => Arc::ptr_eq(grid, shown),
            (Ok(Some(_)), None) => false,
            _ => true,
        }
    }

    fn carried(&self) -> Carried {
        // Its settle sends whatever it gathered: what the worker holds is published, not carried.
        Carried::default()
    }

    fn published(&self) -> Published {
        self.fed.published()
    }

    fn held(&self) -> usize {
        self.worker_held + self.owed.bytes() + self.after_back.bytes() + self.taken.iter().map(|(_, changed)| changed.bytes() + 16).sum::<usize>()
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests;
