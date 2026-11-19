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
//! The worker holds no device and no queue: every texel the scene is shown is written by the
//! frame's own upload, from the grid the worker handed over.
//!
//! **Only the bands the camera shows are written.** Zoomed in, 3% of the grid is on screen and the
//! whole of it was written for every grid (23 MB a second of queue writes on the stress test); a
//! band the camera does not show stays owed ([`Changed`]) and is written by the first upload that
//! shows it, from the newest grid there is. Nothing is left stale on screen: the layer owes an
//! install while a band in view is owed ([`Layer::uploaded`]), so a camera that moves while the
//! hours stand still is answered by the next frame.

mod fold;

use super::{
    Carried, Settling, Due, Fed, Gpu, Inputs, Layer, List, Num, Published, View,
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
    /// at most [`RUN`] changed bands in a row, and the bound of their columns, among the cells
    /// `shown` holds (none: the whole grid). A band the camera does not show is passed over and
    /// stays changed, for the first upload that shows it.
    fn run_in(&self, [width, height]: [u32; 2], shown: Option<&Shown>) -> Option<(Range<u32>, Range<u32>)> {
        let bands = height.div_ceil(BAND);
        let columns = |band: u32| {
            let held = match self {
                Self::Unknown => 0..width,
                Self::Bands(held) => held.get(band as usize).cloned().unwrap_or(0..0),
            };
            match shown {
                Some(shown) if !shown.shows(band, &held) => 0..0,
                _ => held,
            }
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

/// The grid's cells the camera shows, in grid rows and columns: what an upload writes of a grid.
/// A band whose rows or columns fall outside is written by the first upload that shows it.
#[derive(Clone, Debug, PartialEq)]
struct Shown {
    rows: Range<u32>,
    /// The columns the camera shows: one range, or the two a view across the world's seam shows.
    columns: [Range<u32>; 2],
}

impl Shown {
    /// The cells of a grid `[width, height]` cells across that `camera` shows at `viewport`, each
    /// cell being [`map_snow::SNOW_MUD_BLOCK`] map pixels of world and the world's z growing with
    /// the row. None: the camera shows ground the whole grid may cover (it is above the map, or
    /// its view is wider than the world), where every band is written as before.
    fn of(
        camera: &viewer_foundation::map::camera::Camera,
        viewport: [u32; 2],
        [width, height]: [u32; 2],
    ) -> Option<Self> {
        let ground = camera.visible_ground(viewport, MARGIN)?;
        let block = map_snow::SNOW_MUD_BLOCK as f32;
        let cells = |from: f32, to: f32| {
            let (from, to) = ((from / block).floor(), (to / block).ceil() + 1.0);
            (from.is_finite() && to.is_finite() && to > from).then_some((from as i64, to as i64))
        };
        let (first, last) = cells(ground[1], ground[3])?;
        let rows = first.clamp(0, i64::from(height)) as u32..last.clamp(0, i64::from(height)) as u32;
        // The world wraps in x: the columns a view across the seam shows are at both ends.
        let (first, last) = cells(ground[0], ground[2])?;
        let (span, width) = (last - first, i64::from(width));
        if span >= width {
            return None;
        }
        let start = first.rem_euclid(width);
        let columns = if start + span <= width {
            [start as u32..(start + span) as u32, 0..0]
        } else {
            [start as u32..width as u32, 0..(start + span - width) as u32]
        };
        Some(Self { rows, columns })
    }

    /// Whether the camera shows any cell of the band at `band` whose changed columns are `columns`.
    fn shows(&self, band: u32, columns: &Range<u32>) -> bool {
        let rows = band * BAND..(band + 1) * BAND;
        rows.start < self.rows.end
            && self.rows.start < rows.end
            && self
                .columns
                .iter()
                .any(|shown| columns.start < shown.end && shown.start < columns.end)
    }
}

/// World units of ground kept beyond the view's own, so a band reaching the edge of the picture is
/// written before the camera reaches it: one band of grid rows.
const MARGIN: f32 = (BAND * map_snow::SNOW_MUD_BLOCK) as f32;

/// What the uploads did, for the paced report: grids taken and how many bands each changed (and
/// how many were not known), pieces written, their bytes and their time; and the grids taken whole.
#[derive(Clone, Copy, Debug, Default)]
struct Figures {
    grids: u64,
    /// Bands owed when an upload ended, because the camera does not show them.
    unshown: u64,
    bands: u64,
    unknown: u64,
    pieces: u64,
    written: u64,
    bytes: u64,
    ns: u64,
    wholes: u64,
    wholes_ns: u64,
}

/// Where the upload writes: the scene, or a test's stand-in.
trait Target {
    /// Whether `grid` can be written a band at a time as it would be taken whole.
    fn takes_parts(&self, grid: &SnowMud, continuity: Continuity) -> bool;
    fn whole(&mut self, grid: &Arc<SnowMud>, continuity: Continuity) -> io::Result<()>;
    fn begin(&mut self, grid: &SnowMud, continuity: Continuity) -> io::Result<()>;
    fn rect(&mut self, grid: &SnowMud, rows: Range<u32>, columns: Range<u32>) -> io::Result<()>;
    fn finish(&mut self, grid: &Arc<SnowMud>, continuity: Continuity) -> io::Result<()>;
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
}

/// What the layer reads of the content: the cell composition and the winter volumes.
pub struct Content {
    cells: SnowCells,
    winter: Winter,
    /// The map's size and camera defines, for the ground a view's camera shows.
    size: viewer_foundation::map::camera::MapSize,
    defines: viewer_foundation::map::camera::MapDefines,
    /// Per province id: the volumes whose records name it.
    volumes: Vec<Vec<u32>>,
    boundary: Vec<u32>,
    /// Per cell: whether it is a boundary cell.
    on_boundary: Vec<bool>,
}

impl Content {
    pub fn new(cells: SnowCells, winter: Winter, size: viewer_foundation::map::camera::MapSize, defines: viewer_foundation::map::camera::MapDefines) -> Self {
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
        Self { cells, winter, size, defines, volumes, boundary, on_boundary }
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
        None => false,
    }
}

fn run(content: Arc<Content>, jobs: Receiver<Job>, made: Sender<Made>, due: Due) {
    // The fold, its table, its marks as last handed back, and the hour folded through.
    let mut work: Option<(Fold, Option<Table>, [u64; 3], Option<Hour>)> = None;
    let mut epoch = 0;
    while let Ok(job) = jobs.recv() {
        let mut queue: VecDeque<Job> = std::iter::once(job).chain(jobs.try_iter()).collect();
        let mut started = Instant::now();
        while let Some(job) = queue.pop_front() {
            match job {
                Job::Reset { epoch: next, table, columns, stamp } => {
                    epoch = next;
                    work = Some((Fold::new(Arc::clone(&content), columns), table.map(|table| *table), [0; 3], stamp));
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
            let sent = Made { epoch, stamp: *stamp, grid, changed, refused: fold.refused(), marked, held, ns: started.elapsed().as_nanos() as u64 };
            if made.send(sent).is_err() {
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
    made: Option<Receiver<Made>>,
    epoch: u64,
    /// The steps handed to the worker and the grids it made, each taken once its hour is applied.
    fed: Fed<Made>,
    /// What the worker last made.
    grid: Result<Option<Arc<SnowMud>>, String>,
    refused: Option<String>,
    marked: [u64; 3],
    worker_held: usize,
    /// The cells the camera shows, as the last view placed it; none before a frame has placed the
    /// camera, where the whole grid is written as before.
    in_view: Option<Shown>,
    /// The grid the scene holds whole, and whether the next upload starts the presentation over;
    /// where the newest grid differs from it, and the run of pieces under way on the scene.
    shown: Option<Arc<SnowMud>>,
    reset: bool,
    owed: Changed,
    parting: Option<Continuity>,
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
            in_view: None,
            shown: None,
            reset: true,
            owed: Changed::default(),
            parting: None,
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
    #[cfg(any(test, feature = "test-support"))]
    pub fn made_through(&self) -> Option<Hour> {
        self.fed.made_through()
    }

    fn take(&mut self, made: Made) {
        if let Ok(Some(grid)) = &made.grid {
            self.figures.grids += 1;
            self.figures.bands += made.changed.bands(grid.height) as u64;
            self.figures.unknown += u64::from(made.changed == Changed::Unknown);
            self.owed.join(made.changed);
        } else {
            // While refused nothing is written: what the scene holds is not known against the next.
            self.owed = Changed::Unknown;
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
                let made = if wait && !self.fed.arrived() { receive.recv().ok() } else { receive.try_recv().ok() };
                let Some(made) = made else { break };
                if made.epoch == self.epoch {
                    self.fed.receive(made.stamp, made);
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

    /// The newest grid written to `target`: whole when it cannot be taken in parts, else every
    /// band where it differs from the grid there.
    fn push(&mut self, target: &mut dyn Target) -> io::Result<()> {
        let grid = match &self.grid {
            Err(why) => return Err(io::Error::other(why.clone())),
            Ok(grid) => grid.clone(),
        };
        let started = Instant::now();
        let Some(grid) = grid.filter(|_| !self.uploaded()) else {
            self.spent[2] += started.elapsed().as_nanos() as u64;
            return Ok(());
        };
        let pushed = self.push_grid(target, &grid);
        self.spent[2] += started.elapsed().as_nanos() as u64;
        pushed
    }

    fn push_grid(&mut self, target: &mut dyn Target, grid: &Arc<SnowMud>) -> io::Result<()> {
        let size = [grid.width, grid.height];
        // A run under way whose grid can no longer be taken a band at a time is taken over whole.
        if self.parting.is_some_and(|run| run == Continuity::Continue && (grid.refresh.is_some() || self.shown.as_ref().is_some_and(|shown| [shown.width, shown.height] != size))) {
            (self.parting, self.reset) = (None, true);
        }
        if self.parting.is_none() {
            let continuity = if self.reset { Continuity::Reset } else { Continuity::Continue };
            if !target.takes_parts(grid, continuity) {
                let started = Instant::now();
                target.whole(grid, continuity)?;
                (self.figures.wholes, self.figures.wholes_ns) = (self.figures.wholes + 1, self.figures.wholes_ns + Instant::now().duration_since(started).as_nanos() as u64);
                (self.shown, self.reset, self.owed) = (Some(Arc::clone(grid)), false, Changed::default());
                return Ok(());
            }
            if self.reset {
                self.owed = Changed::Unknown;
            }
            target.begin(grid, continuity)?;
            (self.parting, self.reset) = (Some(continuity), false);
        }
        while let Some((rows, columns)) = self.owed.run_in(size, self.in_view.as_ref()) {
            let started = Instant::now();
            let bytes = rows.len() * columns.len() * 4;
            target.rect(grid, rows.clone(), columns)?;
            self.figures.pieces += 1;
            self.figures.written += u64::from(rows.end.div_ceil(BAND) - rows.start / BAND);
            self.owed.written(rows, size);
            self.figures.bytes += bytes as u64;
            self.figures.ns += Instant::now().duration_since(started).as_nanos() as u64;
        }
        let continuity = self.parting.take().unwrap_or(Continuity::Continue);
        target.finish(grid, continuity)?;
        self.figures.unshown += self.owed.bands(grid.height) as u64;
        self.shown = Some(Arc::clone(grid));
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
        "bands_left_outside_the_view": figures.unshown,
        "grids_taken_whole": figures.wholes,
        "us_a_whole_grid_mean": per(figures.wholes_ns, figures.wholes * 1000),
    })
}

/// The snow layer of a loaded recording's content: its cell composition and winter volumes.
pub fn for_document(document: &crate::document::Document) -> Box<dyn Layer> {
    let size = viewer_foundation::map::camera::MapSize::of(&document.content.map);
    let defines = document.content.map_defines;
    let content = document.content.products.snow_content().map(|content| Arc::new(Content::new(content.cells.clone(), content.winter.clone(), size, defines))).map_err(|error| error.to_string());
    Box::new(Snow::new(content))
}

impl Layer for Snow {
    fn name(&self) -> &'static str {
        "snow"
    }

    /// The cells the camera shows, taken here: a camera that moves writes the bands it reaches.
    fn placed(&mut self, view: &crate::layers::View) -> Option<u64> {
        let size = self.content.as_ref().map(|content| content.cells.dimensions()).unwrap_or([0, 0]);
        self.in_view = match (view.camera, self.content.as_ref()) {
            (Some(placement), Ok(content)) if view.viewport != [0, 0] && size != [0, 0] => {
                Shown::of(&placement.on(content.size, content.defines), view.viewport, size)
            }
            _ => None,
        };
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

    fn settle(&mut self, _mirrors: &Mirrors, _settling: Settling) -> io::Result<()> {
        let started = Instant::now();
        self.exchange(true);
        self.spent[1] += started.elapsed().as_nanos() as u64;
        Ok(())
    }

    fn upload(&mut self, gpu: &mut Gpu<'_>) -> io::Result<()> {
        let _push = replay_frame::cpu::scope("snow.push", Some("snow"));
        self.push(&mut OnScene(gpu))
    }

    /// Everything the camera shows is on the GPU: a band it does not show stays owed and is
    /// written by the first upload that shows it, so a camera that moves is answered at once.
    fn uploaded(&self) -> bool {
        match (&self.grid, &self.shown) {
            (Ok(Some(grid)), Some(shown)) => Arc::ptr_eq(grid, shown) && self.owed.run_in([grid.width, grid.height], self.in_view.as_ref()).is_none(),
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
        self.worker_held + self.owed.bytes()
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests;
