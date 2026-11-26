use crate::native::*;
use std::ops::Range;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Reads {
    pub views: usize,
    pub grid: bool,
    pub pixel: bool,
    pub own_pixel: bool,
}

impl Reads {
    fn grid_at(self) -> usize {
        self.views
    }

    fn pixel_at(self) -> usize {
        self.views + usize::from(self.grid)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Made {
    pub revision: u64,
    pub map: MapSize,
    pub offsets: Vec<u32>,
    pub visible: Option<[u32; 4]>,
    pub alpha: u32,
    pub shared: Vec<usize>,
    pub reads: Reads,
    pub samples: u32,
    pub textures: u64,
    pub globals: u64,
    pub chain: Vec<u8>,
    pub cull: Option<GameCull>,
}

impl Made {
    #[expect(clippy::too_many_arguments, reason = "one value per input a walk reads")]
    pub(super) fn new(
        revision: u64,
        frame: &ModelFrame<'_>,
        shared: &[&[u8]],
        reads: Reads,
        samples: u32,
        textures: u64,
        globals: u64,
        chain: &[u8],
        cull: Option<GameCull>,
    ) -> Self {
        Self {
            revision,
            map: frame.camera.map,
            offsets: frame.offsets.iter().map(|offset| offset.to_bits()).collect(),
            visible: frame.visible.map(|visible| visible.map(f32::to_bits)),
            alpha: frame.alpha.to_bits(),
            shared: shared.iter().map(|bytes| bytes.len()).collect(),
            reads,
            samples,
            textures,
            globals,
            chain: chain.to_vec(),
            cull,
        }
    }

    pub(super) fn first_difference(&self, other: &Made) -> Option<usize> {
        let Made { revision, map, offsets, visible, alpha, shared, reads, samples, textures, globals, chain, cull } =
            self;
        [
            *revision != other.revision,
            *map != other.map,
            *offsets != other.offsets,
            *visible != other.visible,
            *alpha != other.alpha,
            *shared != other.shared,
            *reads != other.reads,
            *samples != other.samples,
            *textures != other.textures,
            *globals != other.globals,
            *chain != other.chain,
            *cull != other.cull,
        ]
        .iter()
        .position(|differs| *differs)
    }
}

pub(super) type Drawn = (Option<GameCull>, Option<[u32; 4]>);

pub(super) const MADE_INPUTS: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Whole {
    First,
    Refused,
    Waiting,
    NotFollowed,
    MostWalked,
    NoBuffer,
    Unplanned,
    NoRoom,
    Unbound,
    Error,
}

pub(super) const WHOLES: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stale {
    Whole(Whole),
    Input(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Entered {
    New,
    OtherView,
    KeyMesh,
    KeyEpoch,
    Twice,
    Born,
    Waiting,
    Demoted,
    Rewrite,
}

pub(super) const ENTERED: usize = 9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Block {
    pub at: usize,
    pub length: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Supply {
    Vertex,
    Skin,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Unit {
    pub live: bool,
    pub view: usize,
    pub group: usize,
    pub index: usize,
    pub row: Option<walk::Row>,
    pub descriptor: usize,
    pub lod: Option<usize>,
    pub at: [f32; 3],
    pub vertex: Option<Block>,
    pub skin: Option<Block>,
    pub bones: usize,
    pub mask: u32,
    pub draws: usize,
    pub parts: usize,
}

pub(super) struct Pending {
    pub key: usize,
    pub view: usize,
    pub vertex: Block,
    pub skin: Option<Block>,
    pub reference: u32,
    pub vertices: VertexKey,
    pub indices: usize,
    pub first: u32,
    pub count: u32,
    pub base: i32,
    pub textures: u32,
    pub material: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Meta {
    pub key: usize,
    pub textures: u32,
    pub material: usize,
}

pub(super) type FigureKey = (usize, u32, u32, u32);

pub(super) fn figure_key(mesh: usize, instance: &ModelInstance) -> FigureKey {
    (
        mesh,
        instance.animation_seed,
        instance.animation_identity,
        instance.animation_epoch,
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Mark {
    pub view: usize,
    pub position: usize,
    pub key: FigureKey,
    pub live: bool,
    pub waiting: bool,
    units: usize,
    pending: usize,
}

struct Walked {
    mark: Mark,
    units: Range<usize>,
    pending: Range<usize>,
}

#[derive(Default)]
pub(super) struct Recording {
    bytes: Vec<u8>,
    pub units: Vec<Unit>,
    pub pending: Vec<Pending>,
    pub visited: Vec<bool>,
    pub incomplete: bool,
    marks: Vec<Mark>,
    only: Option<(Vec<(usize, usize)>, usize)>,
    pub culled: Vec<(usize, FigureKey)>,
    pub every_lod: bool,
}

impl Recording {
    pub(super) fn of(only: Vec<(usize, usize)>) -> Self {
        Self {
            only: Some((only, 0)),
            ..Self::default()
        }
    }

    pub(super) fn partial(&self) -> bool {
        self.only.is_some()
    }

    pub(super) fn takes(&mut self, view: usize, position: usize) -> bool {
        let Some((only, next)) = &mut self.only else {
            return true;
        };
        if only.get(*next) == Some(&(view, position)) {
            *next += 1;
            true
        } else {
            false
        }
    }

    pub(super) fn wait(&mut self) {
        match self.marks.last_mut() {
            Some(mark) => mark.waiting = true,
            None => self.incomplete = true,
        }
    }

    pub(super) fn incomplete(&self) -> bool {
        self.incomplete
    }

    pub(super) fn begin(&mut self, view: usize, position: usize, key: FigureKey, live: bool) {
        self.marks.push(Mark {
            view,
            position,
            key,
            live,
            waiting: false,
            units: self.units.len(),
            pending: self.pending.len(),
        });
    }

    pub(super) fn place(&mut self, bytes: &[u8]) -> Block {
        let at = self.bytes.len();
        self.bytes.extend_from_slice(bytes);
        Block { at, length: bytes.len() }
    }

    fn recorded(&self, block: Block) -> &[u8] {
        &self.bytes[block.at..block.at + block.length]
    }

    fn first_draws(&self) -> Vec<usize> {
        let mut at = 0;
        self.units
            .iter()
            .map(|unit| {
                let first = at;
                at += unit.parts;
                first
            })
            .collect()
    }

    fn walked(&self) -> Vec<Walked> {
        let mut walked: Vec<Walked> = Vec::with_capacity(self.marks.len());
        let (mut units, mut pending) = (self.units.len(), self.pending.len());
        for mark in self.marks.iter().rev() {
            walked.push(Walked {
                mark: *mark,
                units: mark.units..units,
                pending: mark.pending..pending,
            });
            (units, pending) = (mark.units, mark.pending);
        }
        walked.reverse();
        walked
    }
}

fn gathered(visited: &[bool]) -> Vec<usize> {
    visited.iter().enumerate().filter(|(_, visited)| **visited).map(|(at, _)| at).collect()
}

fn align(value: usize, alignment: usize) -> usize {
    value.div_ceil(alignment) * alignment
}

fn place(region: &mut Vec<u8>, bytes: &[u8], reserved: usize, alignment: usize) -> usize {
    let at = align(region.len(), alignment);
    region.resize(at, 0);
    region.extend_from_slice(bytes);
    region.resize(at + reserved.max(bytes.len()), 0);
    at
}

fn capacity(used: usize, alignment: usize) -> usize {
    align(used + (used / 4).max(64 << 10), alignment)
}

const BLOCK_ALIGNMENT: usize = 16;

fn rows_capacity(instances: usize) -> usize {
    instances + (instances / 4).max(4)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Area {
    start: usize,
    end: usize,
    top: usize,
    free: Vec<Range<usize>>,
}

impl Area {
    fn new(start: usize, top: usize, end: usize) -> Self {
        Self {
            start,
            end,
            top,
            free: Vec::new(),
        }
    }

    pub(super) fn take(&mut self, length: usize, alignment: usize) -> Option<usize> {
        for at in 0..self.free.len() {
            let run = self.free[at].clone();
            let start = align(run.start, alignment);
            if start + length > run.end {
                continue;
            }
            self.free.remove(at);
            self.give(run.start..start);
            self.give(start + length..run.end);
            return Some(start);
        }
        let start = align(self.top, alignment);
        (start + length <= self.end).then(|| {
            let passed = self.top..start;
            self.top = start + length;
            self.give(passed);
            start
        })
    }

    fn grow(&mut self, end: usize) {
        self.end = end;
    }

    pub(super) fn give(&mut self, span: Range<usize>) {
        if span.is_empty() {
            return;
        }
        let at = self.free.partition_point(|run| run.start < span.start);
        let mut run = span;
        if at < self.free.len() && self.free[at].start == run.end {
            run.end = self.free.remove(at).end;
        }
        if at > 0 && self.free[at - 1].end == run.start {
            self.free[at - 1].end = run.end;
        } else {
            self.free.insert(at, run);
        }
        if self.free.last().is_some_and(|last| last.end == self.top) {
            self.top = self.free.pop().expect("tested").start;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Run {
    supply: Supply,
    stride: usize,
    at: usize,
}

impl Run {
    fn span(&self, capacity: usize) -> Range<usize> {
        self.at..self.at + capacity * self.stride
    }
}

struct Group {
    key: usize,
    view: usize,
    live: bool,
    band: u32,
    alone: bool,
    rows: Option<usize>,
    runs: Vec<Run>,
    capacity: usize,
    slots: Vec<usize>,
    at: usize,
}

impl Group {
    fn run(&self, supply: Supply, stride: Option<usize>) -> Option<&Run> {
        self.runs
            .iter()
            .find(|run| run.supply == supply && stride.is_none_or(|stride| run.stride == stride))
    }
}

#[derive(Clone, Copy)]
pub(super) struct DrawGeometry {
    pub view: usize,
    pub reference: u32,
    pub vertices: VertexKey,
    pub indices: usize,
    pub first: u32,
    pub count: u32,
    pub base: i32,
}

impl From<&Pending> for DrawGeometry {
    fn from(pending: &Pending) -> Self {
        Self {
            view: pending.view,
            reference: pending.reference,
            vertices: pending.vertices,
            indices: pending.indices,
            first: pending.first,
            count: pending.count,
            base: pending.base,
        }
    }
}

type Standing = std::collections::HashMap<(usize, usize, bool, u32), usize>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Instance {
    group: usize,
    slot: usize,
}

#[derive(Clone, Copy)]
pub(super) struct PartProgram<'p> {
    pub program: &'p shader_pipeline::gpu::BindingProgram,
    pub order: Option<mesh_shader::Order>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Figure {
    pub view: usize,
    pub position: usize,
    pub key: FigureKey,
    pub live: bool,
    pub waiting: bool,
    pub units: Range<usize>,
    pub draws: Range<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Target {
    pub view: usize,
    pub position: usize,
    pub group: usize,
    pub index: usize,
    pub row: Option<walk::Row>,
    pub key: FigureKey,
    pub born: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct PartCounts {
    pub parts: usize,
    pub runs: usize,
    pub in_views: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Kind {
    #[default]
    Casters,
    Lit,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Shared {
    pub at: Vec<usize>,
    pub reserved: Vec<usize>,
    pub reads: Reads,
}

pub(super) struct Replay {
    pub sources: plans::PendingSources,
    pub material: Option<usize>,
    pub last: Option<usize>,
    pub drawn: u32,
}

pub(super) const SHARED_PIXEL: usize = usize::MAX;

pub(super) const LODS: usize = 6;

#[derive(Default)]
pub(super) struct Kept {
    kind: Kind,
    made: Option<Made>,
    dropped: Option<Whole>,
    read: Option<u64>,
    pub draws: DrawList,
    metas: Vec<Meta>,
    figures: Vec<Figure>,
    units: Vec<Unit>,
    instances: Vec<Instance>,
    groups: Vec<Option<Group>>,
    order: Vec<usize>,
    bands: u32,
    live: Vec<usize>,
    by_instance: std::cell::OnceCell<Vec<(usize, usize)>>,
    gathered: Vec<usize>,
    dirty: Vec<(usize, usize)>,
    shared: Shared,
    bytes: Vec<u8>,
    blocks: Vec<u8>,
    block_area: Area,
    live_area: Area,
    still_area: Area,
    buffer: Option<wgpu::Buffer>,
    pub replay: Option<Replay>,
    counts: Counts,
    lods: [u64; LODS],
    moved: [u64; 2],
    culled: std::collections::HashSet<(usize, FigureKey)>,
    checks: u64,
    lods_for: Option<[f32; 3]>,
    drawn: Option<Drawn>,
    posed: u64,
    wrote: Wrote,
    resident: Vec<u8>,
    shared_reads: Vec<Vec<Range<usize>>>,
    fresh: bool,
    repeated: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Wrote {
    pub rows: u64,
    pub changed: u64,
    pub bytes: u64,
    pub changed_bytes: u64,
    pub staged: u64,
    pub staged_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Counts {
    pub walks: u64,
    pub kept: u64,
    pub spliced: u64,
    pub figures: u64,
}

pub(super) struct LiveBytes<'a> {
    pub world: &'a [[f32; 4]; 4],
    pub userdata: Option<[f32; 8]>,
    pub bones: Option<&'a [u8]>,
}

impl Kept {
    pub(super) fn new(kind: Kind) -> Self {
        Self {
            kind,
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(super) fn fresh(&self, made: &Made, since: Option<u64>) -> bool {
        self.stale(made, since).is_none()
    }

    pub(super) fn stale(&self, made: &Made, since: Option<u64>) -> Option<Stale> {
        match &self.made {
            None => Some(Stale::Whole(self.dropped.unwrap_or(Whole::First))),
            Some(kept) if kept != made => Some(Stale::Input(kept.first_difference(made).unwrap_or(0))),
            Some(_) if since.is_none() || self.read != since => Some(Stale::Whole(Whole::NotFollowed)),
            Some(_) => None,
        }
    }

    pub(super) fn drop_for(&mut self, why: Whole) {
        self.made = None;
        self.dropped = Some(why);
    }

    pub(super) fn dropped(&self) -> Option<Whole> {
        self.dropped.filter(|_| self.made.is_none())
    }

    pub(super) fn unmade_for(&mut self, why: Whole) {
        if self.made.is_none() {
            self.dropped = Some(why);
        }
    }

    pub(super) fn read(&mut self, frame: u64) {
        self.read = Some(frame);
    }

    pub(super) fn forget(&mut self) {
        self.made = None;
        self.drawn = None;
    }

    pub(super) fn drawn_by(&self) -> Option<Drawn> {
        self.drawn
    }

    pub(super) fn chosen_by(&mut self, drawn: Drawn) {
        self.drawn = Some(drawn);
    }

    pub(super) fn live(&self) -> &[usize] {
        &self.live
    }

    pub(super) fn unit(&self, at: usize) -> Unit {
        self.units[at]
    }

    pub(super) fn units(&self) -> &[Unit] {
        &self.units
    }

    pub(super) fn lods_for(&self) -> Option<[f32; 3]> {
        self.lods_for
    }

    pub(super) fn levelled_at(&mut self, eye: [f32; 3]) {
        self.lods_for = Some(eye);
    }

    pub(super) fn posed(&self) -> u64 {
        self.posed
    }

    pub(super) fn posed_for(&mut self, epoch: u64) {
        self.posed = epoch;
    }

    pub(super) fn was_culled(&self, view: usize, key: FigureKey) -> bool {
        self.culled.contains(&(view, key))
    }

    pub(super) fn set_culled(&mut self, culled: Vec<(usize, FigureKey)>) {
        self.culled = culled.into_iter().collect();
    }

    pub(super) fn units_of(&self, instance: usize) -> impl Iterator<Item = usize> + '_ {
        let by_instance = self.by_instance.get_or_init(|| {
            let mut by_instance: Vec<_> =
                self.units.iter().enumerate().map(|(at, unit)| (unit.index, at)).collect();
            by_instance.sort_unstable();
            by_instance
        });
        let from = by_instance.partition_point(|(of, _)| *of < instance);
        by_instance[from..]
            .iter()
            .take_while(move |(of, _)| *of == instance)
            .map(|(_, unit)| *unit)
    }

    pub(super) fn figures(&self) -> usize {
        self.figures.len()
    }

    pub(super) fn units_of_figure(&self, at: usize) -> Range<usize> {
        self.figures[at].units.clone()
    }

    pub(super) fn figure_of(&self, unit: usize) -> usize {
        self.figures.partition_point(|figure| figure.units.end <= unit)
    }

    pub(super) fn figure_live(&self, at: usize) -> bool {
        self.figures[at].live
    }

    pub(super) fn waiting(&self) -> Option<Vec<bool>> {
        self.figures
            .iter()
            .any(|figure| figure.waiting)
            .then(|| self.figures.iter().map(|figure| figure.waiting).collect())
    }

    pub(super) fn order_free(&self) -> bool {
        self.bands <= 1
    }

    pub(super) fn bands(&self) -> u32 {
        self.bands
    }

    pub(super) fn visited(&self, view: usize, group: usize, groups: usize) -> bool {
        self.gathered.binary_search(&(view * groups + group)).is_ok()
    }

    pub(super) fn gathered(&self) -> &[usize] {
        &self.gathered
    }

    pub(super) fn places(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.figures.iter().map(|figure| (figure.view, figure.position))
    }

    pub(super) fn holds(&self, view: usize, position: usize) -> bool {
        self.figures.binary_search_by_key(&(view, position), |figure| (figure.view, figure.position)).is_ok()
    }

    fn block_bytes(&self, block: Block) -> &[u8] {
        &self.blocks[block.at..block.at + block.length]
    }

    pub(super) fn vertex_bytes(&self, unit: usize) -> Option<&[u8]> {
        Some(self.block_bytes(self.units.get(unit)?.vertex?))
    }

    pub(super) fn skin_bytes(&self, unit: usize) -> Option<&[u8]> {
        Some(self.block_bytes(self.units.get(unit)?.skin?))
    }

    pub(super) fn write_unit(&mut self, at: usize, bytes: LiveBytes<'_>) -> bool {
        let unit = self.units[at];
        match (unit.skin, bytes.bones) {
            (Some(skin), Some(bones)) if bones.len() == skin.length => {}
            (None, None) => {}
            _ => return false,
        }
        let world: &[u8] = bytemuck::bytes_of(bytes.world);
        let userdata = bytes.userdata;
        if let Some(block) = unit.vertex {
            let into = &mut self.blocks[block.at..block.at + block.length];
            let end = world.len().min(into.len());
            into[..end].copy_from_slice(&world[..end]);
            if let Some(data) = &userdata
                && into.len() >= 96
            {
                into[64..96].copy_from_slice(bytemuck::bytes_of(data));
            }
        }
        if let (Some(block), Some(bones)) = (unit.skin, bytes.bones) {
            self.blocks[block.at..block.at + block.length].copy_from_slice(bones);
        }
        for instance in unit.draws..unit.draws + unit.parts {
            let Instance { group, slot } = self.instances[instance];
            for run in 0..self.groups[group].as_ref().expect("an instance keeps its group").runs.len() {
                let run = self.groups[group].as_ref().expect("held above").runs[run];
                let row = run.at + slot * run.stride;
                let (written, changed) = match run.supply {
                    Supply::Vertex => {
                        let length = run.stride;
                        let end = world.len().min(length);
                        let mut changed = self.bytes[row..row + end] != world[..end];
                        if changed {
                            self.bytes[row..row + end].copy_from_slice(&world[..end]);
                        }
                        match &userdata {
                            Some(data) if length >= 96 => {
                                let data = bytemuck::bytes_of(data);
                                if self.bytes[row + 64..row + 96] != *data {
                                    self.bytes[row + 64..row + 96].copy_from_slice(data);
                                    changed = true;
                                }
                                (96, changed)
                            }
                            _ => (end, changed),
                        }
                    }
                    Supply::Skin => {
                        let bones = bytes.bones.expect("a skinned run has its palette");
                        let end = run.stride.min(bones.len());
                        let changed = self.bytes[row..row + end] != bones[..end];
                        if changed {
                            self.bytes[row..row + end].copy_from_slice(&bones[..end]);
                        }
                        (end, changed)
                    }
                };
                self.wrote.rows += 1;
                self.wrote.bytes += written as u64;
                if changed && written != 0 {
                    self.wrote.changed += 1;
                    self.wrote.changed_bytes += written as u64;
                    self.dirty.push((row, written));
                }
            }
        }
        true
    }

    pub(super) fn write_shared(&mut self, shared: &[&[u8]]) {
        for (&place, bytes) in self.shared.at.iter().zip(shared) {
            let into = place..place + bytes.len();
            if self.bytes[into.clone()] != **bytes {
                self.bytes[into].copy_from_slice(bytes);
                self.dirty.push((place, bytes.len()));
            }
        }
        self.counts.kept += 1;
    }

    fn whole(&self) -> [Range<usize>; 2] {
        [0..self.live_area.top, self.still_area.start..self.still_area.top]
    }

    fn stage(
        &mut self,
        arena: &shader_pipeline::gpu::Arena,
        buffer: &wgpu::Buffer,
        range: Range<usize>,
    ) {
        arena.write(buffer, range.start as u64, &self.bytes[range.clone()]);
        if !self.resident.is_empty() {
            self.resident[range.clone()].copy_from_slice(&self.bytes[range.clone()]);
        }
        self.wrote.staged += 1;
        self.wrote.staged_bytes += (range.end - range.start) as u64;
    }

    pub(super) fn staged_difference(&self) -> Option<usize> {
        if self.resident.is_empty() {
            return None;
        }
        self.whole().into_iter().find_map(|range| {
            let at = self.resident[range.clone()]
                .iter()
                .zip(&self.bytes[range.clone()])
                .position(|(held, wanted)| held != wanted)?;
            Some(range.start + at)
        })
    }

    pub(super) fn judge_staged(&self, what: &'static str) {
        if self.resident.is_empty() {
            return;
        }
        match self.staged_difference() {
            Some(at) => {
                viewer_scene_base::actor_check::differs(what, &at, &(self.resident[at], self.bytes[at]))
            }
            None => viewer_scene_base::actor_check::checked(
                what,
                self.whole().iter().map(|range| range.end - range.start).sum::<usize>() as u64,
            ),
        }
    }

    pub(super) fn dirty_ranges(&mut self) -> Vec<Range<usize>> {
        self.dirty.sort_unstable();
        let mut ranges: Vec<Range<usize>> = Vec::new();
        for (at, length) in self.dirty.drain(..) {
            let start = at / 4 * 4;
            let end = (at + length).next_multiple_of(4);
            match ranges.last_mut() {
                Some(last) if start <= last.end + 1024 => last.end = last.end.max(end),
                _ => ranges.push(start..end),
            }
        }
        ranges
    }

    pub(super) fn keep(&mut self, arena: &shader_pipeline::gpu::Arena, shared: &[&[u8]]) {
        self.repeated = !std::mem::take(&mut self.fresh)
            && self.dirty.is_empty()
            && self.shared_reads_hold(shared);
        self.write_shared(shared);
        let ranges = self.dirty_ranges();
        if let Some(buffer) = self.buffer.clone() {
            for range in ranges {
                self.stage(arena, &buffer, range);
            }
        }
        let name = match self.kind {
            Kind::Casters => crate::Count::CasterKept,
            Kind::Lit => crate::Count::LitKept,
        };
        crate::count(name, 1);
    }

    pub(super) fn repeated(&self) -> bool {
        self.repeated
    }

    fn shared_reads_hold(&self, shared: &[&[u8]]) -> bool {
        self.shared.at.len() == shared.len()
            && self.shared.at.iter().zip(shared).zip(&self.shared_reads).all(
                |((&place, bytes), reads)| {
                    reads.iter().all(|range| {
                        let range = range.start.min(bytes.len())..range.end.min(bytes.len());
                        self.bytes[place + range.start..place + range.end] == bytes[range]
                    })
                },
            )
    }

    fn note_reads<'p>(&mut self, parts: impl Fn(usize) -> Option<PartProgram<'p>>) {
        let reads = self.shared.reads;
        let mut held = vec![Vec::new(); self.shared.at.len()];
        for group in self.groups.iter().flatten() {
            let Some(part) = parts(group.key) else {
                held.iter_mut().for_each(|ranges| ranges.push(0..usize::MAX));
                continue;
            };
            for spec in part.program.constant_reads() {
                let at = match plan_tail::selector(spec.binding) {
                    Some(plan_tail::FRAME) => group.view,
                    Some(plan_tail::GRID) if reads.grid => reads.grid_at(),
                    Some(plan_tail::PIXEL) if reads.pixel && !reads.own_pixel => reads.pixel_at(),
                    _ => continue,
                };
                if let Some(ranges) = held.get_mut(at) {
                    ranges.extend(spec.reads.iter().map(|read| read.start as usize..read.end as usize));
                }
            }
        }
        for ranges in &mut held {
            ranges.sort_unstable_by_key(|range| (range.start, range.end));
            let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
            for range in ranges.drain(..) {
                match merged.last_mut() {
                    Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
                    _ => merged.push(range),
                }
            }
            *ranges = merged;
        }
        self.shared_reads = held;
        self.fresh = true;
        self.repeated = false;
    }

    pub(super) fn forget_writes(&mut self) {
        self.dirty.clear();
    }

    #[cfg(test)]
    pub(super) fn plan(&self, targets: &[Target], demoted: &[bool]) -> Vec<Option<usize>> {
        self.plan_counting(targets, demoted, None)
    }

    pub(super) fn plan_counting(
        &self,
        targets: &[Target],
        demoted: &[bool],
        mut entered: Option<&mut [u64; ENTERED]>,
    ) -> Vec<Option<usize>> {
        let mut named = vec![false; if entered.is_some() { targets.len() } else { 0 }];
        let mut old: Vec<((usize, FigureKey), usize)> = self
            .figures
            .iter()
            .enumerate()
            .map(|(at, figure)| ((figure.view, figure.key), at))
            .collect();
        old.sort_unstable();
        let mut new: Vec<((usize, FigureKey), usize)> = targets
            .iter()
            .enumerate()
            .map(|(at, target)| ((target.view, target.key), at))
            .collect();
        new.sort_unstable();
        let mut next = vec![None; targets.len()];
        let (mut a, mut b) = (0, 0);
        let run = |keys: &[((usize, FigureKey), usize)], from: usize| {
            keys[from..].iter().take_while(|entry| entry.0 == keys[from].0).count()
        };
        while a < old.len() && b < new.len() {
            match old[a].0.cmp(&new[b].0) {
                std::cmp::Ordering::Less => a += 1,
                std::cmp::Ordering::Greater => b += 1,
                std::cmp::Ordering::Equal => {
                    let (olds, news) = (run(&old, a), run(&new, b));
                    let (figure, target) = (old[a].1, new[b].1);
                    let lowered = demoted.get(figure).copied().unwrap_or(false);
                    let born = targets[target].born && !self.figures[figure].live;
                    if olds == 1 && news == 1 && !lowered && !born {
                        next[target] = Some(figure);
                    } else if let Some(entered) = entered.as_deref_mut() {
                        let why = if olds != 1 || news != 1 {
                            Entered::Twice
                        } else if lowered && self.figures[figure].waiting {
                            Entered::Waiting
                        } else if lowered {
                            Entered::Demoted
                        } else {
                            Entered::Born
                        };
                        entered[why as usize] += news as u64;
                        for entry in &new[b..b + news] {
                            named[entry.1] = true;
                        }
                    }
                    a += olds;
                    b += news;
                }
            }
        }
        if let Some(entered) = entered {
            let mut actors: Vec<((usize, u32, u32), (usize, u32))> = self
                .figures
                .iter()
                .map(|figure| ((figure.view, figure.key.1, figure.key.2), (figure.key.0, figure.key.3)))
                .collect();
            actors.sort_unstable();
            let mut anywhere: Vec<(u32, u32)> = actors.iter().map(|((_, seed, identity), _)| (*seed, *identity)).collect();
            anywhere.sort_unstable();
            for (at, target) in targets.iter().enumerate() {
                if next[at].is_some() || named[at] {
                    continue;
                }
                let (mesh, seed, identity, _) = target.key;
                let here = (target.view, seed, identity);
                let from = actors.partition_point(|entry| entry.0 < here);
                let there: Vec<_> = actors[from..].iter().take_while(|entry| entry.0 == here).collect();
                let why = if there.iter().any(|entry| entry.1.0 != mesh) {
                    Entered::KeyMesh
                } else if !there.is_empty() {
                    Entered::KeyEpoch
                } else if anywhere.binary_search(&(seed, identity)).is_ok() {
                    Entered::OtherView
                } else {
                    Entered::New
                };
                entered[why as usize] += 1;
            }
        }
        next
    }

    pub(super) fn follow(&mut self, figure: usize, target: &Target) {
        for unit in &mut self.units[self.figures[figure].units.clone()] {
            unit.group = target.group;
            unit.index = target.index;
            unit.row = target.row;
        }
        self.figures[figure].position = target.position;
        self.by_instance = std::cell::OnceCell::new();
    }

    fn found<'p>(
        recording: &Recording,
        parts: &impl Fn(usize) -> Option<PartProgram<'p>>,
    ) -> io::Result<(Vec<Found>, u32)> {
        let mut found: Vec<Found> = Vec::new();
        let mut bands = Bands::default();
        for unit in &recording.units {
            for at in unit.draws..unit.draws + unit.parts {
                let key = recording.pending[at].key;
                let part = parts(key).ok_or_else(missing)?;
                let band = bands.next(part.order);
                let alone = part.program.instanced_rows().is_empty();
                let group = (!alone)
                    .then(|| {
                        found.iter().position(|held| {
                            !held.alone
                                && held.key == key
                                && held.view == unit.view
                                && held.live == unit.live
                                && held.band == band
                        })
                    })
                    .flatten()
                    .unwrap_or_else(|| {
                        found.push(Found {
                            key,
                            view: unit.view,
                            live: unit.live,
                            band,
                            alone,
                            instances: Vec::new(),
                        });
                        found.len() - 1
                    });
                found[group].instances.push(at);
            }
        }
        Ok((found, bands.count()))
    }

    fn runs_of(
        part: PartProgram<'_>,
        reads: Reads,
        alignment: usize,
    ) -> io::Result<(bool, Vec<(Supply, usize)>)> {
        let mut runs: Vec<(Supply, usize)> = Vec::new();
        let rows = !part.program.instanced_rows().is_empty();
        if rows {
            for &(binding, stride) in part.program.instanced_rows() {
                let Some(supply) = supply_of(binding, reads) else {
                    return Err(invalid("a native draw reads a shared block per instance"));
                };
                let stride = stride as usize;
                if !runs.contains(&(supply, stride)) {
                    runs.push((supply, stride));
                }
            }
        } else {
            let mut sizes = [0usize; 2];
            for spec in part.program.constant_reads() {
                if let Some(supply) = supply_of(spec.binding, reads) {
                    let slot = &mut sizes[usize::from(supply == Supply::Skin)];
                    *slot = (*slot).max(spec.size as usize);
                }
            }
            for (slot, supply) in [Supply::Vertex, Supply::Skin].into_iter().enumerate() {
                if sizes[slot] != 0 {
                    runs.push((supply, align(sizes[slot], alignment)));
                }
            }
        }
        Ok((rows, runs))
    }

    pub(super) fn part_counts(&self) -> PartCounts {
        let mut parts = std::collections::HashSet::new();
        let mut in_views = std::collections::HashSet::new();
        let mut runs = 0;
        let mut last = None;
        for unit in &self.units {
            for meta in &self.metas[unit.draws..unit.draws + unit.parts] {
                let part = (unit.view, meta.key);
                parts.insert(meta.key);
                in_views.insert(part);
                if last != Some(part) {
                    runs += 1;
                    last = Some(part);
                }
            }
        }
        PartCounts { parts: parts.len(), runs, in_views: in_views.len() }
    }

    pub(super) fn lods(&self) -> [u64; LODS] {
        self.lods
    }

    pub(super) fn take_wrote(&mut self) -> Wrote {
        std::mem::take(&mut self.wrote)
    }

    pub(super) fn take_moved(&mut self) -> [u64; 2] {
        std::mem::take(&mut self.moved)
    }

    fn index(&mut self) {
        self.lods = [0; LODS];
        for unit in &self.units {
            if let (Some(lod), true) = (unit.lod, unit.parts != 0) {
                self.lods[lod.min(LODS - 1)] += 1;
            }
        }
        self.live.clear();
        self.live
            .extend((0..self.units.len()).filter(|&at| self.units[at].live));
        self.by_instance = std::cell::OnceCell::new();
    }

    fn area(&mut self, live: bool) -> &mut Area {
        if live { &mut self.live_area } else { &mut self.still_area }
    }

    #[expect(clippy::too_many_arguments, reason = "the group's part, its place and its runs")]
    fn make_group(
        &mut self,
        key: usize,
        view: usize,
        live: bool,
        band: u32,
        alone: bool,
        rows: bool,
        specs: &[(Supply, usize)],
        alignment: usize,
    ) -> Option<usize> {
        let capacity = if alone { 1 } else { rows_capacity(1) };
        let rows = match rows {
            true => Some(self.area(live).take(shader_pipeline::gpu::ROWS_BYTES, alignment)?),
            false => None,
        };
        let mut runs: Vec<Run> = Vec::with_capacity(specs.len());
        for &(supply, stride) in specs {
            match self.area(live).take(capacity * stride, stride) {
                Some(at) => runs.push(Run { supply, stride, at }),
                None => {
                    let area = self.area(live);
                    for run in &runs {
                        area.give(run.span(capacity));
                    }
                    if let Some(rows) = rows {
                        area.give(rows..rows + shader_pipeline::gpu::ROWS_BYTES);
                    }
                    return None;
                }
            }
        }
        let at = self.groups.iter().position(Option::is_none).unwrap_or_else(|| {
            self.groups.push(None);
            self.groups.len() - 1
        });
        self.groups[at] = Some(Group {
            key,
            view,
            live,
            band,
            alone,
            rows,
            runs,
            capacity,
            slots: Vec::new(),
            at: 0,
        });
        Some(at)
    }

    fn free_group(&mut self, group: usize) {
        let Some(held) = self.groups[group].take() else {
            return;
        };
        let area = self.area(held.live);
        for run in &held.runs {
            area.give(run.span(held.capacity));
        }
        if let Some(rows) = held.rows {
            area.give(rows..rows + shader_pipeline::gpu::ROWS_BYTES);
        }
    }

    fn grow(&mut self, group: usize) -> Option<bool> {
        let held = self.groups[group].as_ref().expect("a group being filled");
        if held.slots.len() < held.capacity {
            return Some(false);
        }
        let (live, count, capacity) = (held.live, held.slots.len(), held.capacity);
        let runs = held.runs.clone();
        let wanted = rows_capacity(count + 1);
        let mut taken: Vec<usize> = Vec::with_capacity(runs.len());
        for run in &runs {
            match self.area(live).take(wanted * run.stride, run.stride) {
                Some(at) => taken.push(at),
                None => {
                    let area = self.area(live);
                    for (run, at) in runs.iter().zip(&taken) {
                        area.give(*at..*at + wanted * run.stride);
                    }
                    return None;
                }
            }
        }
        for (run, &at) in runs.iter().zip(&taken) {
            let held = count * run.stride;
            self.bytes.copy_within(run.at..run.at + held, at);
            if held != 0 {
                self.dirty.push((at, held));
            }
            self.area(live).give(run.span(capacity));
        }
        let held = self.groups[group].as_mut().expect("held above");
        for (run, at) in held.runs.iter_mut().zip(taken) {
            run.at = at;
        }
        held.capacity = wanted;
        Some(true)
    }

    fn release(&mut self, gone: &[usize]) {
        let mut freed: Vec<Vec<usize>> = self.groups.iter().map(|_| Vec::new()).collect();
        for &instance in gone {
            let Instance { group, slot } = self.instances[instance];
            freed[group].push(slot);
        }
        for group in 0..freed.len() {
            if freed[group].is_empty() {
                continue;
            }
            freed[group].sort_unstable();
            let held = self.groups[group].as_ref().expect("an instance keeps its group");
            let (runs, count) = (held.runs.clone(), held.slots.len());
            let keep: Vec<usize> = (0..count)
                .filter(|slot| freed[group].binary_search(slot).is_err())
                .collect();
            if keep.is_empty() {
                self.free_group(group);
                continue;
            }
            let slots: Vec<usize> = keep.iter().map(|&old| held.slots[old]).collect();
            for (new, &old) in keep.iter().enumerate() {
                if new == old {
                    continue;
                }
                for run in &runs {
                    let (from, into) = (run.at + old * run.stride, run.at + new * run.stride);
                    self.bytes.copy_within(from..from + run.stride, into);
                    self.dirty.push((into, run.stride));
                }
                self.instances[slots[new]].slot = new;
            }
            self.groups[group].as_mut().expect("held above").slots = slots;
        }
    }

    fn take_slot(&mut self, group: usize) -> Option<(usize, bool)> {
        let moved = self.grow(group)?;
        let held = self.groups[group].as_mut().expect("a group being filled");
        held.slots.push(usize::MAX);
        Some((held.slots.len() - 1, moved))
    }

    fn write_rows_block(
        &mut self,
        group: usize,
        program: &shader_pipeline::gpu::BindingProgram,
    ) -> io::Result<()> {
        let held = self.groups[group].as_ref().expect("a group of the list");
        let Some(at) = held.rows else {
            return Ok(());
        };
        let block = program
            .rows_block(&bases(held, program, self.shared.reads)?)
            .map_err(invalid)?;
        self.bytes[at..at + block.len()].copy_from_slice(&block);
        self.dirty.push((at, block.len()));
        Ok(())
    }

    #[expect(clippy::too_many_arguments, reason = "the walk, its inputs and what it was made from")]
    pub(super) fn finish<'p>(
        &mut self,
        arena: &shader_pipeline::gpu::Arena,
        mut recording: Recording,
        mut draws: DrawList,
        shared: &[&[u8]],
        reads: Reads,
        parts: impl Fn(usize) -> Option<PartProgram<'p>>,
        made: Option<Made>,
    ) -> io::Result<()> {
        self.fresh = true;
        self.repeated = false;
        let device = arena.device();
        let alignment = alignment(device);
        let recorded = std::mem::take(&mut recording.bytes);
        let source = |block: Block| &recorded[block.at..block.at + block.length];
        let firsts = recording.first_draws();
        for (unit, first) in recording.units.iter_mut().zip(firsts) {
            unit.draws = first;
        }
        let mut reserved: Vec<usize> = shared.iter().map(|bytes| bytes.len()).collect();
        for pending in &recording.pending {
            for spec in parts(pending.key).ok_or_else(missing)?.program.constant_reads() {
                let at = match plan_tail::selector(spec.binding) {
                    Some(plan_tail::FRAME) => pending.view,
                    Some(plan_tail::GRID) if reads.grid => reads.grid_at(),
                    Some(plan_tail::PIXEL) if reads.pixel && !reads.own_pixel => reads.pixel_at(),
                    _ => continue,
                };
                if let Some(reserve) = reserved.get_mut(at) {
                    *reserve = (*reserve).max(spec.size as usize);
                }
            }
        }
        let (found, bands) = Self::found(&recording, &parts)?;
        let mut bytes = std::mem::take(&mut self.bytes);
        bytes.clear();
        let shared_at: Vec<usize> = shared
            .iter()
            .zip(&reserved)
            .map(|(block, reserve)| place(&mut bytes, block, *reserve, alignment))
            .collect();
        let mut groups: Vec<Option<Group>> = found.iter().map(|_| None).collect();
        let mut cursor = align(bytes.len(), alignment);
        let mut areas = Vec::with_capacity(2);
        for live in [true, false] {
            let start = cursor;
            for (at, group) in found.iter().enumerate() {
                if group.live != live {
                    continue;
                }
                let part = parts(group.key).ok_or_else(missing)?;
                let (rows, specs) = Self::runs_of(part, reads, alignment)?;
                let capacity = if group.alone { 1 } else { rows_capacity(group.instances.len()) };
                let rows = rows.then(|| {
                    let at = align(cursor, alignment);
                    cursor = at + shader_pipeline::gpu::ROWS_BYTES;
                    at
                });
                let runs = specs
                    .into_iter()
                    .map(|(supply, stride)| {
                        let at = align(cursor, stride);
                        cursor = at + capacity * stride;
                        Run { supply, stride, at }
                    })
                    .collect();
                groups[at] = Some(Group {
                    key: group.key,
                    view: group.view,
                    live,
                    band: group.band,
                    alone: group.alone,
                    rows,
                    runs,
                    capacity,
                    slots: Vec::new(),
                    at: 0,
                });
            }
            let end = start + capacity(cursor - start, alignment);
            areas.push(Area::new(start, cursor, end));
            cursor = end;
        }
        bytes.resize(cursor, 0);
        let mut instances: Vec<Instance> = recording
            .pending
            .iter()
            .map(|_| Instance { group: 0, slot: 0 })
            .collect();
        let mut dirty = Vec::new();
        for (at, group) in found.iter().enumerate() {
            let held = groups[at].as_mut().expect("made above");
            for (slot, &pending) in group.instances.iter().enumerate() {
                held.slots.push(pending);
                instances[pending] = Instance { group: at, slot };
            }
            let held = groups[at].as_ref().expect("made above");
            for (slot, &pending) in group.instances.iter().enumerate() {
                let draw = &recording.pending[pending];
                write_rows(&mut bytes, &mut dirty, held, slot, Supply::Vertex, source(draw.vertex));
                if let Some(skin) = draw.skin {
                    write_rows(&mut bytes, &mut dirty, held, slot, Supply::Skin, source(skin));
                }
            }
        }
        let mut blocks = std::mem::take(&mut self.blocks);
        blocks.clear();
        for unit in &mut recording.units {
            for block in [&mut unit.vertex, &mut unit.skin] {
                if let Some(block) = block {
                    let at = blocks.len();
                    blocks.extend_from_slice(&recorded[block.at..block.at + block.length]);
                    *block = Block { at, length: block.length };
                }
            }
        }
        let blocks_read = Shared {
            at: shared_at,
            reserved,
            reads,
        };
        for group in groups.iter().flatten() {
            if let Some(at) = group.rows {
                let program = parts(group.key).ok_or_else(missing)?.program;
                let block = program
                    .rows_block(&bases(group, program, reads)?)
                    .map_err(invalid)?;
                bytes[at..at + block.len()].copy_from_slice(&block);
            }
        }
        let limits = device.limits();
        let needed = (bytes.len() as u64).max(wgpu::COPY_BUFFER_ALIGNMENT);
        let rows = groups.iter().flatten().any(|group| group.rows.is_some());
        if rows && needed > u64::from(limits.max_storage_buffer_binding_size) {
            return Err(invalid(format!(
                "native draw list: {needed} bytes of rows exceed the GPU's {}-byte storage binding",
                limits.max_storage_buffer_binding_size
            )));
        }
        if self.buffer.as_ref().is_none_or(|buffer| buffer.size() < needed) {
            let size = (needed + needed / 4).next_multiple_of(alignment as u64);
            if size > limits.max_buffer_size {
                return Err(invalid(format!(
                    "native draw list: {size}-byte constant buffer exceeds GPU limit {}",
                    limits.max_buffer_size
                )));
            }
            self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("native draw list constants"),
                size,
                usage: wgpu::BufferUsages::UNIFORM
                    | wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let buffer = self.buffer.clone().expect("made above");
        draws.draws.clear();
        let mut order = Vec::with_capacity(groups.len());
        for at in 0..groups.len() {
            let group = groups[at].as_mut().expect("made above");
            group.at = draws.draws.len();
            let (key, first, instances) = (group.key, group.slots[0], group.slots.len() as u32);
            order.push(at);
            let draw = bind_group(
                arena,
                &buffer,
                parts(key).ok_or_else(missing)?,
                (&bytes, &blocks_read),
                groups[at].as_ref().expect("made above"),
                (&recording.pending[first]).into(),
                instances,
            )?
            .ok_or_else(|| invalid("a shared block is reserved for every read of its walk"))?;
            draws.draws.push(draw);
        }
        let made = made.filter(|_| !recording.incomplete());
        let still = areas.pop().expect("two areas");
        let live = areas.pop().expect("two areas");
        self.take(
            bytes,
            blocks,
            (live, still),
            groups,
            order,
            bands,
            recording,
            instances,
            draws,
            blocks_read,
            made,
        );
        self.resident = match viewer_scene_base::actor_check::every() {
            Some(_) => vec![0; self.bytes.len()],
            None => Vec::new(),
        };
        for range in self.whole() {
            if !range.is_empty() {
                self.stage(arena, &buffer, range);
            }
        }
        let name = match self.kind {
            Kind::Casters => crate::Count::CasterWalks,
            Kind::Lit => crate::Count::LitWalks,
        };
        crate::count(name, 1);
        self.note_reads(parts);
        Ok(())
    }

    fn plan_bands<'p>(
        &self,
        next: &[Option<usize>],
        recording: &Recording,
        walked: &[Walked],
        parts: &impl Fn(usize) -> Option<PartProgram<'p>>,
    ) -> io::Result<(Vec<u32>, Vec<u32>, u32)> {
        let mut bands = Bands::default();
        let mut kept = vec![0; self.instances.len()];
        let mut walks = vec![0; recording.pending.len()];
        let mut walking = walked.iter();
        for figure in next {
            match figure {
                Some(at) => {
                    for instance in self.figures[*at].draws.clone() {
                        let key = self.metas[instance].key;
                        kept[instance] = bands.next(parts(key).ok_or_else(missing)?.order);
                    }
                }
                None => {
                    let walk = walking.next().expect("counted by the caller");
                    for pending in walk.pending.clone() {
                        let key = recording.pending[pending].key;
                        walks[pending] = bands.next(parts(key).ok_or_else(missing)?.order);
                    }
                }
            }
        }
        Ok((walks, kept, bands.count()))
    }

    fn copy_rows(&mut self, from: usize, slot: usize, into: usize, into_slot: usize) {
        let source: Vec<Run> =
            self.groups[from].as_ref().expect("a group of the list").runs.clone();
        let runs = self.groups[into].as_ref().expect("a group of the list").runs.clone();
        for (from, run) in source.iter().zip(&runs) {
            let (from, at) = (from.at + slot * run.stride, run.at + into_slot * run.stride);
            self.bytes.copy_within(from..from + run.stride, at);
            self.dirty.push((at, run.stride));
        }
    }

    fn reband<'p>(
        &mut self,
        kept: &[u32],
        alignment: usize,
        parts: &impl Fn(usize) -> Option<PartProgram<'p>>,
        created: &mut Vec<bool>,
        geometry: &mut Vec<Option<DrawGeometry>>,
        standing: &mut Standing,
    ) -> io::Result<bool> {
        for group in 0..self.groups.len() {
            let Some(held) = self.groups[group].as_ref() else {
                continue;
            };
            let Some(&first) = held.slots.first() else {
                continue;
            };
            let band = kept[first];
            if held.alone || held.slots.iter().all(|&instance| kept[instance] == band) {
                let held = self.groups[group].as_mut().expect("held above");
                held.band = band;
                if !held.alone {
                    standing.insert((held.key, held.view, held.live, band), group);
                }
                continue;
            }
            let (key, view, live, was) = (held.key, held.view, held.live, held.at);
            let was = self.draws.draws[was].geometry(view);
            let part = parts(key).ok_or_else(missing)?;
            let rows = !part.program.instanced_rows().is_empty();
            let (_, specs) = Self::runs_of(part, self.shared.reads, alignment)?;
            let cut: Vec<(usize, u32)> = self.groups[group]
                .as_ref()
                .expect("held above")
                .slots
                .iter()
                .enumerate()
                .filter(|(_, instance)| kept[**instance] != band)
                .map(|(slot, instance)| (slot, kept[*instance]))
                .collect();
            let (mut made, mut moved, mut landed) = (Vec::new(), Vec::new(), Vec::new());
            for (slot, cut) in cut {
                let instance = self.groups[group].as_ref().expect("held above").slots[slot];
                let into = match made.iter().find(|(band, _)| *band == cut) {
                    Some((_, into)) => *into,
                    None => {
                        let Some(into) =
                            self.make_group(key, view, live, cut, false, rows, &specs, alignment)
                        else {
                            return Ok(false);
                        };
                        created.resize(self.groups.len(), false);
                        geometry.resize(self.groups.len(), None);
                        created[into] = true;
                        geometry[into] = Some(was);
                        standing.insert((key, view, live, cut), into);
                        made.push((cut, into));
                        into
                    }
                };
                let Some((into_slot, _)) = self.take_slot(into) else {
                    return Ok(false);
                };
                self.copy_rows(group, slot, into, into_slot);
                moved.push(instance);
                landed.push((instance, into, into_slot));
            }
            self.groups[group].as_mut().expect("held above").band = band;
            standing.insert((key, view, live, band), group);
            self.release(&moved);
            for (instance, into, slot) in landed {
                self.instances[instance] = Instance { group: into, slot };
                let held = self.groups[into].as_mut().expect("made above");
                if held.slots.len() <= slot {
                    held.slots.resize(slot + 1, usize::MAX);
                }
                held.slots[slot] = instance;
            }
        }
        Ok(true)
    }

    pub(super) fn splice<'p>(
        &mut self,
        arena: &shader_pipeline::gpu::Arena,
        next: &[Option<usize>],
        recording: &Recording,
        visited: Vec<bool>,
        alignment: usize,
        parts: impl Fn(usize) -> Option<PartProgram<'p>>,
    ) -> io::Result<bool> {
        self.fresh = true;
        self.repeated = false;
        let walked = recording.walked();
        let wanted = next.iter().filter(|figure| figure.is_none()).count();
        let routes = walked.iter().map(|walk| (walk.mark.view, walk.mark.position));
        if walked.len() != wanted
            || recording.only.as_ref().is_some_and(|(only, _)| !only.iter().copied().eq(routes))
        {
            self.dropped = Some(Whole::Unplanned);
            return Ok(false);
        }
        let Some(buffer) = self.buffer.clone() else {
            self.dropped = Some(Whole::NoBuffer);
            return Ok(false);
        };
        let mut kept = vec![false; self.figures.len()];
        for &figure in next.iter().flatten() {
            kept[figure] = true;
        }
        let left = kept.iter().filter(|kept| !**kept).count();
        let gone: Vec<usize> = self
            .figures
            .iter()
            .zip(&kept)
            .filter(|(_, kept)| !**kept)
            .flat_map(|(figure, _)| figure.draws.clone())
            .collect();
        let (walked_bands, kept_bands, bands) =
            self.plan_bands(next, recording, &walked, &parts)?;
        self.release(&gone);
        let mut created = vec![false; self.groups.len()];
        let mut grown = vec![false; self.groups.len()];
        let mut geometry: Vec<Option<DrawGeometry>> = vec![None; self.groups.len()];
        let mut standing = Standing::default();
        if !self.reband(&kept_bands, alignment, &parts, &mut created, &mut geometry, &mut standing)?
        {
            self.dropped = Some(Whole::NoRoom);
            return Ok(false);
        }
        grown.resize(self.groups.len(), false);
        self.bands = bands;
        let mut placed: Vec<Instance> = recording
            .pending
            .iter()
            .map(|_| Instance { group: 0, slot: 0 })
            .collect();
        let reads = self.shared.reads;
        let firsts = recording.first_draws();
        for walk in &walked {
            for at in walk.units.clone() {
                let unit = recording.units[at];
                for pending in firsts[at]..firsts[at] + unit.parts {
                    let key = recording.pending[pending].key;
                    let part = parts(key).ok_or_else(missing)?;
                    let rows = !part.program.instanced_rows().is_empty();
                    let alone = !rows;
                    let band = walked_bands[pending];
                    let standing_at = (key, unit.view, unit.live, band);
                    let group = match (!alone).then(|| standing.get(&standing_at).copied()).flatten()
                    {
                        Some(group) => group,
                        None => {
                            let (_, specs) = Self::runs_of(part, reads, alignment)?;
                            let Some(group) = self.make_group(
                                key,
                                unit.view,
                                unit.live,
                                band,
                                alone,
                                rows,
                                &specs,
                                alignment,
                            ) else {
                                self.dropped = Some(Whole::NoRoom);
                                return Ok(false);
                            };
                            created.resize(self.groups.len(), false);
                            grown.resize(self.groups.len(), false);
                            geometry.resize(self.groups.len(), None);
                            created[group] = true;
                            geometry[group] = Some((&recording.pending[pending]).into());
                            if !alone {
                                standing.insert(standing_at, group);
                            }
                            group
                        }
                    };
                    let Some((slot, moved)) = self.take_slot(group) else {
                        self.dropped = Some(Whole::NoRoom);
                        return Ok(false);
                    };
                    grown[group] |= moved;
                    placed[pending] = Instance { group, slot };
                    let held = self.groups[group].as_ref().expect("made above");
                    let draw = &recording.pending[pending];
                    let rows = (&mut self.bytes, &mut self.dirty);
                    write_rows(rows.0, rows.1, held, slot, Supply::Vertex, recording.recorded(draw.vertex));
                    if let Some(skin) = draw.skin {
                        let rows = (&mut self.bytes, &mut self.dirty);
                        write_rows(rows.0, rows.1, held, slot, Supply::Skin, recording.recorded(skin));
                    }
                }
            }
        }
        for group in 0..self.groups.len() {
            if (created[group] || grown[group]) && self.groups[group].is_some() {
                let key = self.groups[group].as_ref().expect("tested").key;
                let program = parts(key).ok_or_else(missing)?.program;
                self.write_rows_block(group, program)?;
            }
        }
        let mut bound: Vec<Option<RecordedDraw>> = self.groups.iter().map(|_| None).collect();
        for group in 0..self.groups.len() {
            if !created[group] {
                continue;
            }
            let held = self.groups[group].as_ref().expect("made in this splice");
            let (key, instances) = (held.key, held.slots.len() as u32);
            let Some(draw) = bind_group(
                arena,
                &buffer,
                parts(key).ok_or_else(missing)?,
                (&self.bytes, &self.shared),
                held,
                geometry[group].expect("a group made this splice is bound from its draw"),
                instances,
            )?
            else {
                self.dropped = Some(Whole::Unbound);
                return Ok(false);
            };
            bound[group] = Some(draw);
        }
        self.rebuild(next, &kept, recording, &walked, &placed, &firsts);
        self.gathered = gathered(&visited);
        self.order = if self.order_free() {
            let held: Vec<usize> = std::mem::take(&mut self.order)
                .into_iter()
                .filter(|group| self.groups[*group].is_some() && !created[*group])
                .collect();
            held.into_iter()
                .chain((0..self.groups.len()).filter(|group| created[*group]))
                .collect()
        } else {
            let mut seen = vec![false; self.groups.len()];
            let mut order = Vec::with_capacity(self.groups.len());
            for instance in &self.instances {
                if !std::mem::replace(&mut seen[instance.group], true) {
                    order.push(instance.group);
                }
            }
            order
        };
        let mut old: Vec<Option<RecordedDraw>> =
            std::mem::take(&mut self.draws.draws).into_iter().map(Some).collect();
        let mut draws = Vec::with_capacity(self.order.len());
        for at in 0..self.order.len() {
            let group = self.order[at];
            let held = self.groups[group].as_mut().expect("the order holds its groups");
            let instances = held.slots.len() as u32;
            let was = held.at;
            held.at = at;
            let draw = match bound[group].take() {
                Some(draw) => draw,
                None => {
                    let mut draw = old[was].take().expect("a group is drawn once");
                    if draw.instances != instances {
                        draw.instances = instances;
                        draw.id = draw_id();
                    }
                    draw
                }
            };
            draws.push(draw);
        }
        self.draws.draws = draws;
        self.draws.unplanned = true;
        self.index();
        self.counts.spliced += 1;
        self.counts.figures += wanted as u64;
        self.moved[0] += wanted as u64;
        self.moved[1] += left as u64;
        use crate::Count;
        let (spliced, walked, gone) = match self.kind {
            Kind::Casters => (Count::CasterSpliced, Count::CasterFiguresWalked, Count::CasterFiguresLeft),
            Kind::Lit => (Count::LitSpliced, Count::LitFiguresWalked, Count::LitFiguresLeft),
        };
        crate::count(spliced, 1);
        crate::count(walked, wanted as u64);
        crate::count(gone, left as u64);
        self.note_reads(parts);
        Ok(true)
    }

    fn hold_block(&mut self, from: &[u8], block: Block) -> Block {
        let at = loop {
            if let Some(at) = self.block_area.take(block.length, BLOCK_ALIGNMENT) {
                break at;
            }
            let end = capacity(self.blocks.len() + block.length + BLOCK_ALIGNMENT, BLOCK_ALIGNMENT);
            self.blocks.resize(end, 0);
            self.block_area.grow(end);
        };
        self.blocks[at..at + block.length].copy_from_slice(&from[block.at..block.at + block.length]);
        Block { at, length: block.length }
    }

    fn rebuild(
        &mut self,
        next: &[Option<usize>],
        kept: &[bool],
        recording: &Recording,
        walked: &[Walked],
        placed: &[Instance],
        firsts: &[usize],
    ) {
        let figures = std::mem::take(&mut self.figures);
        let held = std::mem::take(&mut self.units);
        let instanced = std::mem::take(&mut self.instances);
        let metas = std::mem::take(&mut self.metas);
        let mut units = Vec::with_capacity(held.len());
        let mut instances = Vec::with_capacity(instanced.len());
        let mut made = Vec::with_capacity(next.len());
        self.metas = Vec::with_capacity(metas.len());
        for figure in figures.iter().zip(kept).filter(|(_, kept)| !**kept).map(|(figure, _)| figure) {
            for unit in &held[figure.units.clone()] {
                for block in [unit.vertex, unit.skin].into_iter().flatten() {
                    self.block_area.give(block.at..block.at + block.length);
                }
            }
        }
        let mut walked = walked.iter();
        for figure in next {
            let start = (units.len(), instances.len());
            match figure {
                Some(at) => {
                    let old = &figures[*at];
                    for unit in &held[old.units.clone()] {
                        let first = instances.len();
                        instances
                            .extend_from_slice(&instanced[unit.draws..unit.draws + unit.parts]);
                        units.push(Unit { draws: first, ..*unit });
                    }
                    self.metas.extend_from_slice(&metas[old.draws.clone()]);
                    made.push(Figure {
                        units: start.0..units.len(),
                        draws: start.1..instances.len(),
                        ..old.clone()
                    });
                }
                None => {
                    let walk = walked.next().expect("counted by the caller");
                    for (at, unit) in walk.units.clone().zip(&recording.units[walk.units.clone()]) {
                        let first = instances.len();
                        instances.extend_from_slice(&placed[firsts[at]..firsts[at] + unit.parts]);
                        let (vertex, skin) = (
                            unit.vertex.map(|block| self.hold_block(&recording.bytes, block)),
                            unit.skin.map(|block| self.hold_block(&recording.bytes, block)),
                        );
                        units.push(Unit { draws: first, vertex, skin, ..*unit });
                    }
                    for pending in &recording.pending[walk.pending.clone()] {
                        self.metas.push(Meta {
                            key: pending.key,
                            textures: pending.textures,
                            material: pending.material,
                        });
                    }
                    let mark = walk.mark;
                    made.push(Figure {
                        view: mark.view,
                        position: mark.position,
                        key: mark.key,
                        live: mark.live,
                        waiting: mark.waiting,
                        units: start.0..units.len(),
                        draws: start.1..instances.len(),
                    });
                }
            }
        }
        self.figures = made;
        self.units = units;
        self.instances = instances;
        for group in self.groups.iter_mut().flatten() {
            group.slots.clear();
        }
        for at in 0..self.instances.len() {
            let Instance { group, slot } = self.instances[at];
            let group = self.groups[group].as_mut().expect("an instance keeps its group");
            if group.slots.len() <= slot {
                group.slots.resize(slot + 1, usize::MAX);
            }
            group.slots[slot] = at;
        }
    }

    pub(super) fn checks(&mut self) -> &mut u64 {
        &mut self.checks
    }

    pub(super) fn judge<'p>(
        &self,
        what: &'static str,
        parts: impl Fn(usize) -> Option<PartProgram<'p>>,
    ) {
        let mut bands = Bands::default();
        let mut walked = vec![0; self.instances.len()];
        let mut drawn = vec![false; self.groups.len()];
        for (at, unit) in self.units.iter().enumerate() {
            for instance in unit.draws..unit.draws + unit.parts {
                let meta = self.metas[instance];
                let Some(part) = parts(meta.key) else {
                    return viewer_scene_base::actor_check::differs(what, &instance, &"its part is gone");
                };
                let band = bands.next(part.order);
                walked[instance] = band;
                let Instance { group, slot } = self.instances[instance];
                let Some(held) = self.groups[group].as_ref() else {
                    return viewer_scene_base::actor_check::differs(what, &instance, &"its group is gone");
                };
                let held = (held.key, held.view, held.live, held.band, held.slots.get(slot));
                let wanted = (meta.key, unit.view, unit.live, band, Some(&instance));
                if held != wanted {
                    return viewer_scene_base::actor_check::differs(what, &instance, &(held, wanted));
                }
                let block = |block: Option<Block>, supply| {
                    let (Some(block), Some(run)) = (block, self.groups[group].as_ref()?.run(supply, None))
                    else {
                        return None;
                    };
                    let end = run.stride.min(block.length);
                    let row = run.at + slot * run.stride;
                    (self.bytes[row..row + end] != self.blocks[block.at..block.at + end])
                        .then_some(supply)
                };
                let unit = self.units[at];
                if let Some(supply) = block(unit.vertex, Supply::Vertex)
                    .or_else(|| block(unit.skin, Supply::Skin))
                {
                    return viewer_scene_base::actor_check::differs(what, &instance, &("rows", supply));
                }
            }
        }
        let mut last = 0;
        for &group in &self.order {
            let Some(held) = self.groups[group].as_ref() else {
                return viewer_scene_base::actor_check::differs(what, &group, &"a drawn group is gone");
            };
            if std::mem::replace(&mut drawn[group], true) {
                return viewer_scene_base::actor_check::differs(what, &group, &"drawn twice");
            }
            if held.band < last {
                return viewer_scene_base::actor_check::differs(what, &group, &(held.band, last));
            }
            last = held.band;
        }
        for (group, held) in self.groups.iter().enumerate() {
            let held = held.as_ref().is_some_and(|held| !held.slots.is_empty());
            if held != drawn[group] {
                return viewer_scene_base::actor_check::differs(what, &group, &"held but not drawn");
            }
        }
        for (instance, &band) in walked.iter().enumerate() {
            let held = self.groups[self.instances[instance].group].as_ref();
            if held.is_none_or(|held| held.band != band) {
                return viewer_scene_base::actor_check::differs(what, &instance, &"banded apart");
            }
        }
        viewer_scene_base::actor_check::checked(what, self.instances.len() as u64);
    }

    #[expect(clippy::too_many_arguments, reason = "the walk, its layout and what it was made from")]
    fn take(
        &mut self,
        bytes: Vec<u8>,
        blocks: Vec<u8>,
        areas: (Area, Area),
        groups: Vec<Option<Group>>,
        order: Vec<usize>,
        bands: u32,
        recording: Recording,
        instances: Vec<Instance>,
        draws: DrawList,
        shared: Shared,
        made: Option<Made>,
    ) {
        self.moved[1] += self.figures.len() as u64;
        self.dropped = None;
        self.culled = recording.culled.iter().copied().collect();
        self.figures.clear();
        for walk in recording.walked() {
            let draws = recording.units[walk.units.clone()]
                .iter()
                .map(|unit| unit.parts)
                .sum::<usize>();
            let first = recording.units[walk.units.clone()]
                .first()
                .map_or(instances.len(), |unit| unit.draws);
            self.figures.push(Figure {
                view: walk.mark.view,
                position: walk.mark.position,
                key: walk.mark.key,
                live: walk.mark.live,
                waiting: walk.mark.waiting,
                units: walk.units,
                draws: first..first + draws,
            });
        }
        self.moved[0] += self.figures.len() as u64;
        self.metas.clear();
        self.metas.extend(recording.pending.iter().map(|pending| Meta {
            key: pending.key,
            textures: pending.textures,
            material: pending.material,
        }));
        self.draws = draws;
        self.draws.unplanned = true;
        self.made = made;
        self.units = recording.units;
        self.instances = instances;
        self.groups = groups;
        self.order = order;
        self.bands = bands;
        self.index();
        self.gathered = gathered(&recording.visited);
        self.dirty.clear();
        self.shared = shared;
        self.bytes = bytes;
        self.block_area = Area::new(0, blocks.len(), blocks.len());
        self.blocks = blocks;
        (self.live_area, self.still_area) = areas;
        self.counts.walks += 1;
    }

    pub(super) fn replay(&self, binds: impl Fn(usize) -> (u32, bool)) -> Replay {
        let mut sources = plans::PendingSources::default();
        let (mut material, mut last, mut drawn) = (None, None, 0);
        for (at, unit) in self.units.iter().enumerate() {
            if unit.lod.is_some() && unit.vertex.is_some() {
                last = Some(at);
            }
            if unit.parts == 0 {
                continue;
            }
            let pixel = if self.shared.reads.own_pixel {
                Some(at)
            } else {
                self.shared.reads.pixel.then_some(SHARED_PIXEL)
            };
            sources.constants_mask_at(unit.mask, unit.view, at, pixel, at, 0);
            for meta in &self.metas[unit.draws..unit.draws + unit.parts] {
                sources.state.record_part(meta.key, binds(meta.key), meta.textures);
                material = Some(meta.material);
            }
            drawn += unit.parts;
        }
        let unshown: usize = self
            .order
            .iter()
            .zip(&self.draws.draws)
            .filter(|(_, draw)| !draw.shown)
            .filter_map(|(group, _)| self.groups[*group].as_ref())
            .map(|group| group.slots.len())
            .sum();
        Replay {
            sources,
            material,
            last,
            drawn: drawn.saturating_sub(unshown) as u32,
        }
    }

    #[cfg(test)]
    pub(super) fn counts(&self) -> Counts {
        self.counts
    }

    #[cfg(test)]
    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[cfg(test)]
    pub(super) fn slots_at(&self, at: usize) -> &[usize] {
        &self.groups[self.order[at]].as_ref().expect("a drawn group").slots
    }

    #[cfg(test)]
    pub(super) fn key_at(&self, at: usize) -> usize {
        self.groups[self.order[at]].as_ref().expect("a drawn group").key
    }
}

#[derive(Default)]
struct Bands {
    held: Option<Option<mesh_shader::Order>>,
    band: u32,
}

impl Bands {
    fn next(&mut self, order: Option<mesh_shader::Order>) -> u32 {
        let agrees = matches!((self.held, order), (Some(Some(held)), Some(order)) if held == order);
        if self.held.is_some() && !agrees {
            self.band += 1;
        }
        self.held = Some(order);
        self.band
    }

    fn count(&self) -> u32 {
        self.band + u32::from(self.held.is_some())
    }
}

struct Found {
    key: usize,
    view: usize,
    live: bool,
    band: u32,
    alone: bool,
    instances: Vec<usize>,
}

fn missing() -> io::Error {
    invalid("native program missing after its walk")
}

pub(super) fn supply_of(binding: u32, reads: Reads) -> Option<Supply> {
    match plan_tail::selector(binding) {
        Some(plan_tail::VERTEX) => Some(Supply::Vertex),
        Some(plan_tail::PIXEL) if reads.own_pixel => Some(Supply::Vertex),
        Some(plan_tail::SKIN) => Some(Supply::Skin),
        _ => None,
    }
}

fn write_rows(
    bytes: &mut [u8],
    dirty: &mut Vec<(usize, usize)>,
    group: &Group,
    slot: usize,
    supply: Supply,
    source: &[u8],
) {
    for run in group.runs.iter().filter(|run| run.supply == supply) {
        let row = run.at + slot * run.stride;
        let end = run.stride.min(source.len());
        bytes[row..row + end].copy_from_slice(&source[..end]);
        if end != 0 {
            dirty.push((row, end));
        }
    }
}

fn bases(
    group: &Group,
    program: &shader_pipeline::gpu::BindingProgram,
    reads: Reads,
) -> io::Result<Vec<u32>> {
    program
        .instanced_rows()
        .iter()
        .map(|&(binding, stride)| {
            let supply = supply_of(binding, reads)
                .ok_or_else(|| invalid("a native draw reads a shared block per instance"))?;
            let run = group
                .run(supply, Some(stride as usize))
                .ok_or_else(|| invalid("a native draw reads a row its group has not"))?;
            u32::try_from(run.at / run.stride)
                .map_err(|_| invalid("a native row stands past the rows a draw reaches"))
        })
        .collect()
}

fn bind_group(
    arena: &shader_pipeline::gpu::Arena,
    buffer: &wgpu::Buffer,
    part: PartProgram<'_>,
    (bytes, shared): (&[u8], &Shared),
    group: &Group,
    geometry: DrawGeometry,
    instances: u32,
) -> io::Result<Option<RecordedDraw>> {
    let reads = shared.reads;
    let program = part.program;
    let mut blocks = Vec::with_capacity(program.constant_reads().len());
    for spec in program.constant_reads() {
        let stride = program
            .instanced_rows()
            .iter()
            .find(|(binding, _)| *binding == spec.binding)
            .map(|(_, stride)| *stride as usize);
        let shared_at = |at: usize| -> io::Result<Option<(usize, usize)>> {
            let (Some(&place), Some(&reserved)) = (shared.at.get(at), shared.reserved.get(at))
            else {
                return Err(invalid("a native draw reads a shared block its list has not"));
            };
            Ok((spec.size as usize <= reserved).then_some((place, spec.size as usize)))
        };
        let own = |supply: Supply| {
            group
                .run(supply, stride)
                .map(|run| (run.at, run.stride))
                .ok_or_else(|| invalid("a native draw reads a block its group has no run of"))
        };
        let place = match plan_tail::selector(spec.binding) {
            Some(plan_tail::FRAME) => shared_at(geometry.view)?,
            Some(plan_tail::GRID) if reads.grid => shared_at(reads.grid_at())?,
            Some(plan_tail::PIXEL) if reads.own_pixel => Some(own(Supply::Vertex)?),
            Some(plan_tail::PIXEL) if reads.pixel => shared_at(reads.pixel_at())?,
            Some(plan_tail::VERTEX) => Some(own(Supply::Vertex)?),
            Some(plan_tail::SKIN) => Some(own(Supply::Skin).map_err(|_| {
                invalid("a native draw reads a palette its part does not have")
            })?),
            _ => return Err(invalid("a native draw reads a block its list does not supply")),
        };
        let Some((place, length)) = place else {
            return Ok(None);
        };
        let offset = u32::try_from(place).map_err(|_| invalid("constant offset exceeds u32"))?;
        blocks.push((&bytes[place..place + length], offset));
    }
    let bindings = match group.rows {
        Some(rows) => {
            let rows = u32::try_from(rows).map_err(|_| invalid("rows offset exceeds u32"))?;
            program.bind_rows(arena, buffer, &blocks, rows)
        }
        None => program.bind_buffer(arena, buffer, &blocks),
    };
    Ok(Some(RecordedDraw {
        reads_clock: viewer_scene_base::shader_constants::reads_time(program.constant_reads(), None),
        id: draw_id(),
        shown: true,
        bindings: bindings.map_err(invalid)?,
        reference: geometry.reference,
        vertices: geometry.vertices,
        indices: geometry.indices,
        first: geometry.first,
        count: geometry.count,
        base: geometry.base,
        instances,
    }))
}

fn alignment(device: &wgpu::Device) -> usize {
    let limits = device.limits();
    limits
        .min_uniform_buffer_offset_alignment
        .max(limits.min_storage_buffer_offset_alignment)
        .max(wgpu::COPY_BUFFER_ALIGNMENT as u32) as usize
}

pub(super) fn block_alignment(arena: &shader_pipeline::gpu::Arena) -> usize {
    alignment(arena.device())
}

#[cfg(test)]
mod tests;
