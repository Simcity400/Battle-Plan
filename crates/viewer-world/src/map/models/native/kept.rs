//! A pass's retained draw lists: the shadow casters' list and one list for each lit walk of a
//! frame. The draws of one part in one view stand as one instanced draw over a run of rows the
//! instance index reaches, so each figure keeps a slot in every run of the groups its parts draw
//! in, and its blocks are the rows of those slots. A frame made from the same inputs ([`Made`])
//! walks nothing: it writes this frame's shared blocks (the scene blocks, and for a lit list the
//! light grid and the incoming pixel snapshot), the rows of actors that have a player (whose
//! poses move by nature) and of instances that changed, hands the GPU the bytes of those that
//! differ from the ones it holds, and records the kept draws. A figure that
//! joins, leaves, is given a player or no longer keeps its level of detail costs that figure: the
//! others keep their slots and their groups keep their bindings. A lit list also keeps how its
//! walk leaves the frame's constant state ([`Replay`]), which later passes read, and gives it
//! again every frame it is kept.
use super::*;
use std::ops::Range;

/// What a list's draws read besides each node's own blocks, in the order of the shared blocks:
/// each view's scene block, then for a lit list the light grid, then the incoming pixel
/// snapshot when the pass's nodes supply no userdata of their own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Reads {
    pub views: usize,
    pub grid: bool,
    pub pixel: bool,
    /// Each node's pixel snapshot is its own vertex block: the pass's nodes supply userdata.
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

/// Everything a walk reads that is not an actor's pose, a figure placed, or the shared blocks'
/// bytes. Any of it differing from the walk that made the list makes the list again, whole.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Made {
    /// [`ModelPass::revision`]: hidden actors, redirects, the content stack, a library that did
    /// more than grow.
    pub revision: u64,
    /// The map the walk's renderer was made for.
    pub map: MapSize,
    pub offsets: Vec<u32>,
    pub visible: Option<[u32; 4]>,
    pub alpha: u32,
    /// Each shared block's length; their bytes are written every frame.
    pub shared: Vec<usize>,
    pub reads: Reads,
    pub samples: u32,
    /// [`ResourceIdentities::replaced`]: textures a kept draw may have bound.
    pub textures: u64,
    /// The global textures a lit draw binds ([`ResourceStamp::globals`]).
    pub globals: u64,
    /// The bytes of the incoming object block a lit walk carries into its nodes' blocks.
    pub chain: Vec<u8>,
    /// The frame's game cull: the viewport's side planes move it with the camera still.
    pub cull: Option<GameCull>,
}

impl Made {
    #[allow(clippy::too_many_arguments, reason = "one value per input a walk reads")]
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

    /// The first input, in [`MADE_INPUTS`]' order, that differs from `other`'s.
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

/// What decides which gathered figures a frame draws: its own camera's cull and ground, by bits.
pub(super) type Drawn = (Option<GameCull>, Option<[u32; 4]>);

/// The inputs of [`Made`], in [`Made::first_difference`]'s order.
pub(super) const MADE_INPUTS: usize = 12;

/// Why a list is walked whole, for the flight record: never made; dropped by an earlier frame (a
/// refusal, a build waited on outside any figure); the field's change not followed; or this
/// frame's change, which the list could not take: the field reshaped or an actor born while the
/// camera moved, most figures to walk, no buffer, a walk not as planned, no room, a draw that
/// cannot be bound in the list, an error.
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

/// Why a list cannot be kept as it is ([`Kept::stale`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stale {
    Whole(Whole),
    /// Made from another value of this input ([`Made::first_difference`]).
    Input(usize),
}

/// Why a figure was walked into a kept list, for the flight record: no figure of its actor was
/// drawn in its view or any other; its actor was drawn there under another key (another mesh, or
/// another epoch); it was drawn only in another view; its key is placed more than once; its actor
/// was given a player; it waited on a build; it was demoted (its level of detail changed, or its
/// nodes did not write in place); its group came into the view; or its nodes failed to write in
/// place in this splice.
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

/// A node's constant block, where the walk recorded it or the list keeps it: its draws' rows
/// hold what their programs read of it, and the whole of it is what the frame's constant state
/// carries into the next pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Block {
    pub at: usize,
    pub length: usize,
}

/// Which of a node's blocks a run of rows holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Supply {
    Vertex,
    Skin,
}

/// A node the walk reached in one view. A live one (its actor has a player) is written again
/// every frame; another when its instance changes. Either way its row reaches the GPU only where
/// its bytes changed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Unit {
    pub live: bool,
    pub view: usize,
    pub group: usize,
    pub index: usize,
    pub row: Option<walk::Row>,
    /// The node's position in its mesh's descriptors.
    pub descriptor: usize,
    /// The level of detail it was drawn at; another one walks its figure again.
    pub lod: Option<usize>,
    /// Where its level of detail was chosen: its world position in its view.
    pub at: [f32; 3],
    pub vertex: Option<Block>,
    pub skin: Option<Block>,
    /// Palette entries its skinned parts read.
    pub bones: usize,
    /// The supplies its accepted parts read, as [`plan_tail::Supplies::present`] gives them.
    pub mask: u32,
    /// Its first draw's place among the list's instances.
    pub draws: usize,
    /// Its draws, which follow its own in the list.
    pub parts: usize,
}

/// A draw found by the walk, bound with the group of its part once its rows have their place.
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

/// What a lit draw leaves in the frame's constant state: its part, the textures it bound and its
/// material.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Meta {
    pub key: usize,
    pub textures: u32,
    pub material: usize,
}

/// The actor a figure is: its mesh, seed, identity and epoch.
pub(super) type FigureKey = (usize, u32, u32, u32);

pub(super) fn figure_key(mesh: usize, instance: &ModelInstance) -> FigureKey {
    (
        mesh,
        instance.animation_seed,
        instance.animation_identity,
        instance.animation_epoch,
    )
}

/// Where the walk began a figure: its view and route, and the recording's lengths there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Mark {
    pub view: usize,
    pub position: usize,
    pub key: FigureKey,
    pub live: bool,
    /// A part of it waited on its build: drawn without it, and walked again next frame.
    pub waiting: bool,
    units: usize,
    pending: usize,
}

/// A figure walked: its mark, and its units and draws in the recording.
struct Walked {
    mark: Mark,
    units: Range<usize>,
    pending: Range<usize>,
}

/// One walk's blocks, draws and figures, in walk order.
#[derive(Default)]
pub(super) struct Recording {
    /// The recorded blocks' bytes, back to back in walk order.
    bytes: Vec<u8>,
    pub units: Vec<Unit>,
    pub pending: Vec<Pending>,
    /// Groups the walk passed the visibility test of, by view then group.
    pub visited: Vec<bool>,
    /// A part waiting on its builds: the frame draws its figure without it ([`Self::wait`]).
    pub incomplete: bool,
    marks: Vec<Mark>,
    /// For a walk of some figures: the routes it takes, by view and position in walk order,
    /// and the next one.
    only: Option<(Vec<(usize, usize)>, usize)>,
    /// The figures the game cull left out, by view and figure.
    pub culled: Vec<(usize, FigureKey)>,
    /// A static list's walk: every level of detail of every node, the frame drawing the one chosen.
    pub every_lod: bool,
}

impl Recording {
    /// A walk of the figures at these routes only, by view and position in walk order.
    pub(super) fn of(only: Vec<(usize, usize)>) -> Self {
        Self {
            only: Some((only, 0)),
            ..Self::default()
        }
    }

    /// A walk of some figures: it leaves no constant state for the passes after it.
    pub(super) fn partial(&self) -> bool {
        self.only.is_some()
    }

    /// Whether the walk takes the route at `position` in `view`; asked of every route, in walk
    /// order.
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

    /// A part of the figure being walked waits on its build: the figure is drawn without it and
    /// walked again next frame, alone; a walk outside any figure keeps no list.
    pub(super) fn wait(&mut self) {
        match self.marks.last_mut() {
            Some(mark) => mark.waiting = true,
            None => self.incomplete = true,
        }
    }

    /// Whether the walk left anything it can keep no list of.
    pub(super) fn incomplete(&self) -> bool {
        self.incomplete
    }

    /// The walk begins a figure: what it records next is that figure's, until the next begins.
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

    /// Records `bytes` as one node's block; the list keeps it and writes the rows its draws read.
    pub(super) fn place(&mut self, bytes: &[u8]) -> Block {
        let at = self.bytes.len();
        self.bytes.extend_from_slice(bytes);
        Block { at, length: bytes.len() }
    }

    /// A recorded block's bytes.
    fn recorded(&self, block: Block) -> &[u8] {
        &self.bytes[block.at..block.at + block.length]
    }

    /// Each unit's first draw among the walk's.
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

    /// Each figure walked with its units and draws.
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

/// The places a walk's visited flags set, by `view * groups + group`.
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

/// Room a laid-out area keeps for the figures that join it before the list is laid out again.
fn capacity(used: usize, alignment: usize) -> usize {
    align(used + (used / 4).max(64 << 10), alignment)
}

/// What a node's block in the list's own store is aligned to, so a block taken from the span of
/// one that left starts where a block of any stride may.
const BLOCK_ALIGNMENT: usize = 16;

/// Rows a laid-out group keeps for the instances that join it: its own quarter, and never fewer
/// than four, so a group of one or two takes more figures without the list being laid out again.
fn rows_capacity(instances: usize) -> usize {
    instances + (instances / 4).max(4)
}

/// The spans of one area of the buffer (the live or the still rows): the groups' runs are taken
/// from its free runs or above its top, and given back as groups empty or grow.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Area {
    start: usize,
    end: usize,
    /// Above it nothing is held.
    top: usize,
    /// Free runs below the top, in order, none touching another.
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

    /// A span of `length` bytes starting at a multiple of `alignment`, so a run of rows of one
    /// stride stands where the instance index reaches every row of it; none when the area has no
    /// such span.
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
            // The bytes passed over to reach the alignment are free, so the run given back joins
            // them and the top comes down past both.
            let passed = self.top..start;
            self.top = start + length;
            self.give(passed);
            start
        })
    }

    /// The area reaches to `end`, its store having grown that far.
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

/// One run of a group: the rows of one of its instances' blocks, every row of one stride from the
/// run's start, which is a multiple of that stride so the instance index reaches each of them.
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

/// The draws of one part in one view that one instanced draw stands for: its instances' rows, the
/// rows block the draw selects them from, and the draw's place in the list.
struct Group {
    key: usize,
    view: usize,
    live: bool,
    /// The band of the walk its instances stand in ([`Bands`]): its draw may hold only draws of
    /// one band, because the draws of another may not be stepped over.
    band: u32,
    /// Its draws keep the list's order: one instance, standing for one draw alone.
    alone: bool,
    /// Where its rows block stands; none when its part reads nothing per instance and its one
    /// instance's blocks are bound at their own offsets.
    rows: Option<usize>,
    runs: Vec<Run>,
    capacity: usize,
    /// Each slot's instance ([`Kept::instances`]).
    slots: Vec<usize>,
    /// Its draw's place in [`DrawList::draws`].
    at: usize,
}

impl Group {
    fn run(&self, supply: Supply, stride: Option<usize>) -> Option<&Run> {
        self.runs
            .iter()
            .find(|run| run.supply == supply && stride.is_none_or(|stride| run.stride == stride))
    }
}

/// What a draw of one part draws, beside the blocks its group binds for it.
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

/// The group a walked draw of one part, view, liveness and band joins, for the splice.
type Standing = std::collections::HashMap<(usize, usize, bool, u32), usize>;

/// One draw of the list: the group it joins and its slot in every run of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Instance {
    group: usize,
    slot: usize,
}

/// What a list needs of a part to lay its draws out and bind them.
#[derive(Clone, Copy)]
pub(super) struct PartProgram<'p> {
    pub program: &'p shader_pipeline::gpu::BindingProgram,
    /// What its draws and the list's others must agree on for their order among the figures to
    /// leave every pixel the same; none where no agreement makes it so
    /// ([`mesh_shader::Pass::order`]).
    pub order: Option<mesh_shader::Order>,
}

/// One figure in one view as the list draws it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Figure {
    pub view: usize,
    /// Its route's position in walk order.
    pub position: usize,
    pub key: FigureKey,
    pub live: bool,
    /// Drawn without a part that waited on its build: walked again next frame.
    pub waiting: bool,
    pub units: Range<usize>,
    /// Its draws among the list's instances.
    pub draws: Range<usize>,
}

/// A figure of this frame's walk order, visible, not hidden and drawn in the list's queue, at
/// its route.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Target {
    pub view: usize,
    pub position: usize,
    pub group: usize,
    pub index: usize,
    pub row: Option<walk::Row>,
    pub key: FigureKey,
    /// Its actor was given a player since the list read it.
    pub born: bool,
}

/// What instancing by part leaves of a list ([`Kept::part_counts`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct PartCounts {
    pub parts: usize,
    pub runs: usize,
    pub in_views: usize,
}

/// Which counts a list's walks and splices go to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Kind {
    #[default]
    Casters,
    Lit,
}

/// The shared blocks of a laid-out list: each one's place, the size its draws read, and how
/// its draws read them.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Shared {
    pub at: Vec<usize>,
    pub reserved: Vec<usize>,
    pub reads: Reads,
}

/// How a lit walk of the list leaves the frame's constant state: the binds it retains for later
/// passes (origins are the list's units, [`SHARED_PIXEL`] the incoming pixel snapshot), the
/// material whose textures those binds keep, the last node it wrote (whose block the next pass's
/// walk starts from) and its draws.
pub(super) struct Replay {
    pub sources: plans::PendingSources,
    pub material: Option<usize>,
    pub last: Option<usize>,
    pub drawn: u32,
}

/// The origin [`Replay`] gives the incoming pixel snapshot.
pub(super) const SHARED_PIXEL: usize = usize::MAX;

/// The levels of detail the flight record tells apart; the last holds every deeper one.
pub(super) const LODS: usize = 6;

/// The retained list: its figures, the groups their draws stand in, their rows' bytes as the GPU
/// holds them, and what the walk that made it read.
#[derive(Default)]
pub(super) struct Kept {
    kind: Kind,
    made: Option<Made>,
    /// What dropped the list, while it is not made.
    dropped: Option<Whole>,
    /// The frame whose change of the field the list last followed ([`ModelPass::frame_change`]).
    read: Option<u64>,
    pub draws: DrawList,
    metas: Vec<Meta>,
    figures: Vec<Figure>,
    units: Vec<Unit>,
    instances: Vec<Instance>,
    /// The groups, by the place their instances name; emptied ones are taken again.
    groups: Vec<Option<Group>>,
    /// The groups' draws in the order the list records them.
    order: Vec<usize>,
    /// The bands its draws stand in ([`Bands`]): one where every part agrees on the order its
    /// draws may stand in among the figures, more where they do not.
    bands: u32,
    /// The live units.
    live: Vec<usize>,
    /// Each unit by its instance, for the instances that change, made when first asked after the
    /// units change.
    by_instance: std::cell::OnceCell<Vec<(usize, usize)>>,
    /// The groups its walk gathered, as `view * groups + group`, sorted: what a frame that draws
    /// by another rule tests again.
    gathered: Vec<usize>,
    /// Rows and shared blocks whose bytes this frame changed, to reach the GPU with it.
    dirty: Vec<(usize, usize)>,
    shared: Shared,
    bytes: Vec<u8>,
    /// Every node's block, whole: its rows hold what their programs read of it. A splice leaves
    /// the blocks of the nodes it keeps where they stand and takes the spans of those that left.
    blocks: Vec<u8>,
    block_area: Area,
    live_area: Area,
    still_area: Area,
    buffer: Option<wgpu::Buffer>,
    /// A lit list's [`Replay`], made again with its order.
    pub replay: Option<Replay>,
    counts: Counts,
    /// The nodes drawn at each level of detail ([`LODS`]).
    lods: [u64; LODS],
    /// Figures walked into the list and figures that left it since the last take.
    moved: [u64; 2],
    /// The figures the game cull left out when the list was walked or spliced, by view and figure:
    /// one that comes within the cull joins the list.
    culled: std::collections::HashSet<(usize, FigureKey)>,
    /// Frames the list was prepared on, so its judge runs at its own rate.
    checks: u64,
    /// The eye its figures' levels of detail stand for. A kept frame whose eye differs gives
    /// every figure the level that eye chooses ([`ModelPass::relevelled`]).
    lods_for: Option<[f32; 3]>,
    /// The drawn cull and ground its figures were chosen by ([`Drawn`]). A kept frame that
    /// draws by others tests every figure it gathered again.
    drawn: Option<Drawn>,
    /// The pose set its live rows were written for ([`ModelPass::posed`]). A kept frame whose
    /// set is the same writes no standing figure's row again: its limbs stand where they stood.
    posed: u64,
    /// What this frame wrote and staged, taken by the record each frame.
    wrote: Wrote,
    /// The bytes the list has staged, held only while the judge of them runs
    /// ([`Self::staged_difference`]).
    resident: Vec<u8>,
    /// What its draws' programs read of each shared block, by block, sorted and merged.
    shared_reads: Vec<Vec<Range<usize>>>,
    /// Its draws were walked or spliced since the last frame kept it.
    fresh: bool,
    /// The last frame kept it with every byte its draws read as the frame before left them.
    repeated: bool,
}

/// What one frame of a list wrote in place and staged for the GPU ([`Kept::take_wrote`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Wrote {
    /// Rows written in place, and those of them whose bytes differed from what the GPU holds.
    pub rows: u64,
    pub changed: u64,
    pub bytes: u64,
    pub changed_bytes: u64,
    /// The writes and the bytes the frame staged.
    pub staged: u64,
    pub staged_bytes: u64,
}

/// Full walks laid out, frames kept, lists spliced by figure and the figures walked into them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Counts {
    pub walks: u64,
    pub kept: u64,
    pub spliced: u64,
    pub figures: u64,
}

/// The bytes a kept frame changes, by node.
pub(super) struct LiveBytes<'a> {
    pub world: &'a [[f32; 4]; 4],
    /// A lit node's own userdata, after its world in its vertex block.
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

    /// Whether the list was made from these inputs and followed every change of the field read
    /// before this frame's (`since`).
    #[cfg(test)]
    pub(super) fn fresh(&self, made: &Made, since: Option<u64>) -> bool {
        self.stale(made, since).is_none()
    }

    /// Why the list cannot be kept from `made` after the field read before this frame's
    /// (`since`), or none when it can ([`Self::fresh`]).
    pub(super) fn stale(&self, made: &Made, since: Option<u64>) -> Option<Stale> {
        match &self.made {
            None => Some(Stale::Whole(self.dropped.unwrap_or(Whole::First))),
            Some(kept) if kept != made => Some(Stale::Input(kept.first_difference(made).unwrap_or(0))),
            Some(_) if since.is_none() || self.read != since => Some(Stale::Whole(Whole::NotFollowed)),
            Some(_) => None,
        }
    }

    /// Dropped for `why`: walked whole next.
    pub(super) fn drop_for(&mut self, why: Whole) {
        self.made = None;
        self.dropped = Some(why);
    }

    /// What dropped the list, while it is not made.
    pub(super) fn dropped(&self) -> Option<Whole> {
        self.dropped.filter(|_| self.made.is_none())
    }

    /// A whole walk kept no inputs (it refused, or waited on a build outside any figure): `why`.
    pub(super) fn unmade_for(&mut self, why: Whole) {
        if self.made.is_none() {
            self.dropped = Some(why);
        }
    }

    /// The list followed the field's change of `frame`.
    pub(super) fn read(&mut self, frame: u64) {
        self.read = Some(frame);
    }

    pub(super) fn forget(&mut self) {
        self.made = None;
        self.drawn = None;
    }

    /// The drawn cull and ground the list's figures were chosen by.
    pub(super) fn drawn_by(&self) -> Option<Drawn> {
        self.drawn
    }

    /// The list's figures are the ones `drawn` chooses.
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

    /// The eye the list's levels of detail stand for.
    pub(super) fn lods_for(&self) -> Option<[f32; 3]> {
        self.lods_for
    }

    /// The list's levels of detail are the ones `eye` chooses.
    pub(super) fn levelled_at(&mut self, eye: [f32; 3]) {
        self.lods_for = Some(eye);
    }

    /// The pose set this list's live rows hold.
    pub(super) fn posed(&self) -> u64 {
        self.posed
    }

    /// The list's live rows hold the poses of set `epoch`.
    pub(super) fn posed_for(&mut self, epoch: u64) {
        self.posed = epoch;
    }

    /// Whether the game cull left out the figure of `key` in `view` when the list was made.
    pub(super) fn was_culled(&self, view: usize, key: FigureKey) -> bool {
        self.culled.contains(&(view, key))
    }

    /// The figures the game cull left out of this splice.
    pub(super) fn set_culled(&mut self, culled: Vec<(usize, FigureKey)>) {
        self.culled = culled.into_iter().collect();
    }

    /// The units of `instance`.
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

    /// The figures in the list.
    pub(super) fn figures(&self) -> usize {
        self.figures.len()
    }

    /// The units of the figure at `at`.
    pub(super) fn units_of_figure(&self, at: usize) -> Range<usize> {
        self.figures[at].units.clone()
    }

    /// The figure a unit is of.
    pub(super) fn figure_of(&self, unit: usize) -> usize {
        self.figures.partition_point(|figure| figure.units.end <= unit)
    }

    pub(super) fn figure_live(&self, at: usize) -> bool {
        self.figures[at].live
    }

    /// The figures drawn without a part that waited on its build, by place; none when none did.
    pub(super) fn waiting(&self) -> Option<Vec<bool>> {
        self.figures
            .iter()
            .any(|figure| figure.waiting)
            .then(|| self.figures.iter().map(|figure| figure.waiting).collect())
    }

    /// Whether the list's parts all let their draws stand in any order among its figures.
    /// Whether every part of the list agrees on the order its draws may stand in among the
    /// figures, so the whole list is one band.
    pub(super) fn order_free(&self) -> bool {
        self.bands <= 1
    }

    pub(super) fn bands(&self) -> u32 {
        self.bands
    }

    /// Whether the walk gathered `group` in `view`.
    pub(super) fn visited(&self, view: usize, group: usize, groups: usize) -> bool {
        self.gathered.binary_search(&(view * groups + group)).is_ok()
    }

    /// The groups the walk gathered, as `view * groups + group`, sorted.
    pub(super) fn gathered(&self) -> &[usize] {
        &self.gathered
    }

    /// Each figure's view and its route's position in walk order.
    pub(super) fn places(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.figures.iter().map(|figure| (figure.view, figure.position))
    }

    /// Whether the list holds the figure at `position` in `view`, units or none: a caster
    /// figure whose descriptors all draw in the alternate queue is held without any.
    pub(super) fn holds(&self, view: usize, position: usize) -> bool {
        self.figures.binary_search_by_key(&(view, position), |figure| (figure.view, figure.position)).is_ok()
    }

    /// The bytes of a block the list holds, whole.
    fn block_bytes(&self, block: Block) -> &[u8] {
        &self.blocks[block.at..block.at + block.length]
    }

    /// The bytes of a unit's vertex block, whole.
    pub(super) fn vertex_bytes(&self, unit: usize) -> Option<&[u8]> {
        Some(self.block_bytes(self.units.get(unit)?.vertex?))
    }

    /// The bytes of a unit's palette.
    pub(super) fn skin_bytes(&self, unit: usize) -> Option<&[u8]> {
        Some(self.block_bytes(self.units.get(unit)?.skin?))
    }

    /// Writes a node's world, userdata and palette into every row its draws read, in place;
    /// false when its palette's length moved, which only a walk of its figure can place.
    pub(super) fn write_unit(&mut self, at: usize, bytes: LiveBytes<'_>) -> bool {
        let unit = self.units[at];
        match (unit.skin, bytes.bones) {
            (Some(skin), Some(bones)) if bones.len() == skin.length => {}
            (None, None) => {}
            _ => return false,
        }
        let world: &[u8] = bytemuck::bytes_of(bytes.world);
        let userdata = bytes.userdata;
        // The block the list keeps, whole: what the next pass's walk starts from.
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

    /// Writes this frame's shared blocks, staging the ones whose bytes changed.
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

    /// The ranges of the list's bytes its draws read: the shared blocks with the live rows, then
    /// the still rows. A whole write stages these, and a kept frame the rows that changed.
    fn whole(&self) -> [Range<usize>; 2] {
        [0..self.live_area.top, self.still_area.start..self.still_area.top]
    }

    /// Stages `range` of the list's bytes, which the GPU holds once the arena flushes.
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

    /// Where what the list has staged differs from the bytes a whole write would have staged,
    /// over the ranges its draws read; none while they are the same, and none at all while the
    /// judge is off and the list holds no copy of them.
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

    /// The judge of what the GPU holds ([`crate::map::actor_check`]): the bytes the list has
    /// staged are the bytes a whole write would have staged. Names the first that differs.
    pub(super) fn judge_staged(&self, what: &'static str) {
        if self.resident.is_empty() {
            return;
        }
        match self.staged_difference() {
            Some(at) => {
                crate::map::actor_check::differs(what, &at, &(self.resident[at], self.bytes[at]))
            }
            None => crate::map::actor_check::checked(
                what,
                self.whole().iter().map(|range| range.end - range.start).sum::<usize>() as u64,
            ),
        }
    }

    /// The rows written this frame as ranges to stage, near ones joined.
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

    /// Keeps the list for this frame: every row and shared block whose bytes it changed goes to
    /// the arena's flush, and a frame that changed none of them stages nothing.
    pub(super) fn keep(&mut self, arena: &shader_pipeline::gpu::Arena, shared: &[&[u8]]) {
        // The rows a kept frame changed are dirty already; the shared blocks are compared here.
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
            Kind::Casters => super::super::Count::CasterKept,
            Kind::Lit => super::super::Count::LitKept,
        };
        super::super::count(name, 1);
    }

    /// Whether the last frame kept the list with its draws reading what they read the frame
    /// before: the same draws over the same bytes draw the same picture.
    pub(super) fn repeated(&self) -> bool {
        self.repeated
    }

    /// Whether `shared` holds, wherever the list's draws read it, the bytes the list holds.
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

    /// Notes what the list's draws read of each shared block, after a walk or splice made them.
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

    /// Drops the writes of a frame that walks after all.
    pub(super) fn forget_writes(&mut self) {
        self.dirty.clear();
    }

    /// Each target's figure in the list, or none to walk it: a figure is kept in the view it
    /// was drawn in when its actor is placed once there in both, was not `demoted`, and has
    /// not been given a player since it was walked without one.
    #[cfg(test)]
    pub(super) fn plan(&self, targets: &[Target], demoted: &[bool]) -> Vec<Option<usize>> {
        self.plan_counting(targets, demoted, None)
    }

    /// The plan above (`plan`), counting into `entered` why each target is walked ([`Entered`]).
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
            // Each other target by what the list drew of its actor: in its view under another key,
            // only in another view, or nothing.
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

    /// A kept figure follows its actor to its route in this frame's field.
    pub(super) fn follow(&mut self, figure: usize, target: &Target) {
        for unit in &mut self.units[self.figures[figure].units.clone()] {
            unit.group = target.group;
            unit.index = target.index;
            unit.row = target.row;
        }
        self.figures[figure].position = target.position;
        self.by_instance = std::cell::OnceCell::new();
    }

    /// The groups the walk's draws stand in, in walk order, each with its instances' draws: the
    /// draws of one part in one view and liveness stand as one inside a band of the walk
    /// ([`Bands`]) where the part reads its blocks per instance, and one draw an instance where
    /// it does not. The bands the walk left, beside them.
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

    /// The runs one group needs: for a part that reads its blocks per instance, one for each
    /// distinct block and stride its program reads them at; for one that does not, one row of
    /// each block its instance fills, at the alignment a block binds at.
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

    /// What instancing by part leaves of the list's draws: the distinct parts drawn, the runs
    /// of consecutive draws of one part in one view (walk order kept), and the distinct parts in
    /// each view (one draw each, order across figures given up).
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

    /// The nodes drawn at each level of detail, the last holding every deeper one.
    pub(super) fn lods(&self) -> [u64; LODS] {
        self.lods
    }

    /// The figures walked into the list and the figures that left it since the last call.
    /// What this frame wrote in place and staged, cleared for the next.
    pub(super) fn take_wrote(&mut self) -> Wrote {
        std::mem::take(&mut self.wrote)
    }

    pub(super) fn take_moved(&mut self) -> [u64; 2] {
        std::mem::take(&mut self.moved)
    }

    /// The live units, each unit by its instance and the nodes drawn at each level of detail,
    /// from the units.
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

    /// The area a group of `live` instances takes its runs from.
    fn area(&mut self, live: bool) -> &mut Area {
        if live { &mut self.live_area } else { &mut self.still_area }
    }

    /// A group of `specs`' runs taken from its area, with room for the instances that join it;
    /// none when the area has no run that long.
    #[allow(clippy::too_many_arguments, reason = "the group's part, its place and its runs")]
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

    /// Gives a group's runs and rows block back to its area.
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

    /// Makes room in `group` for one more instance, moving its runs where they no longer hold
    /// them all; whether they moved, or none when its area has no run that long.
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

    /// Gives the slots of the instances that are leaving back: the instances above a hole in
    /// each group move down into it, so a group's draw covers all of them from the first, and a
    /// group left with none gives its runs back.
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

    /// The slot one more instance of `group` takes and whether its runs moved to hold it, the
    /// group grown where it was full; none when its area has no run long enough to grow into.
    fn take_slot(&mut self, group: usize) -> Option<(usize, bool)> {
        let moved = self.grow(group)?;
        let held = self.groups[group].as_mut().expect("a group being filled");
        held.slots.push(usize::MAX);
        Some((held.slots.len() - 1, moved))
    }

    /// Writes a group's rows block: the base row of each block its part reads per instance.
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

    /// Lays out a full walk's rows, binds one draw for each group and writes the bytes.
    /// `parts` gives a draw's resident program and whether its draws may be reordered; `shared`
    /// are the shared blocks' bytes as `reads` orders them.
    #[allow(clippy::too_many_arguments, reason = "the walk, its inputs and what it was made from")]
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
        // Each shared block's place is reserved for the largest read of it, as the arena
        // reserves one; a block read per instance keeps a row of its own stride instead.
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
        // Each group's rows block, then one run of its instances' rows for each block its part
        // reads, the live groups before the still ones, each area with room for what joins it.
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
        // Every instance takes its slot, and its blocks are written to the rows of it.
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
        // Every node's block, whole, as the list keeps it.
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
        self.resident = match crate::map::actor_check::every() {
            Some(_) => vec![0; self.bytes.len()],
            None => Vec::new(),
        };
        for range in self.whole() {
            if !range.is_empty() {
                self.stage(arena, &buffer, range);
            }
        }
        let name = match self.kind {
            Kind::Casters => super::super::Count::CasterWalks,
            Kind::Lit => super::super::Count::LitWalks,
        };
        super::super::count(name, 1);
        self.note_reads(parts);
        Ok(())
    }

    /// The band each draw will stand in once the splice has left the list in walk order
    /// ([`Bands`]): by the instance a kept draw is today, by the draw the recording holds for a
    /// walked one, and how many bands that leaves.
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

    /// Copies one instance's rows from a slot of one group into a slot of another holding the
    /// same runs.
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

    /// Cuts every group a band boundary now runs through: its instances of each band after the
    /// first move into a group of that band, bound from the geometry their draw had. False when
    /// the list has no room for the groups the cut needs.
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

    /// Makes the list this frame's figures, in walk order (`next`: each target's kept figure, or
    /// none for the next the walk recorded): the figures that left give their slots back and the
    /// instances above them move down into the holes; the walked ones take slots in the groups
    /// their parts draw in, have their blocks written there and their groups' draws bound. False,
    /// the list to be laid out again, when a walked figure cannot be placed or bound, a part's
    /// draws cannot stand in the list's order, or the walk is not the one planned.
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
        // Each walked draw joins the group of its part in its band, made when the list holds none.
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
        // A group made or grown reads its rows from another base.
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
            // Bands stand in walk order, so each group's draw stands where its first instance does.
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
        use super::super::Count;
        let (spliced, walked, gone) = match self.kind {
            Kind::Casters => (Count::CasterSpliced, Count::CasterFiguresWalked, Count::CasterFiguresLeft),
            Kind::Lit => (Count::LitSpliced, Count::LitFiguresWalked, Count::LitFiguresLeft),
        };
        super::super::count(spliced, 1);
        super::super::count(walked, wanted as u64);
        super::super::count(gone, left as u64);
        self.note_reads(parts);
        Ok(true)
    }

    /// A block of the list's store holding `block` of `from`: the span of a node that left, or
    /// room above everything held, the store grown when neither has it.
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

    /// The units, instances, draws' metadata and figures of a splice, in this frame's walk order,
    /// with each node reading its block from the widest row that holds it.
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

    /// Calls of the judge of the list's draws, so it runs at its own rate.
    pub(super) fn checks(&mut self) -> &mut u64 {
        &mut self.checks
    }

    /// The judge of the list's draws ([`crate::map::actor_check`]): what the list submits
    /// against what a walk of the same figures would have, built here as the list's own walk
    /// order. Every instance stands at its own slot of a group of its part, its view, its
    /// liveness and its band; the groups stand in the order their bands were walked in and are
    /// drawn once each; and every instance's rows are the bytes of its node's own block, which
    /// is what a whole walk writes. Names what differs and the draw it differs at.
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
                    return crate::map::actor_check::differs(what, &instance, &"its part is gone");
                };
                let band = bands.next(part.order);
                walked[instance] = band;
                let Instance { group, slot } = self.instances[instance];
                let Some(held) = self.groups[group].as_ref() else {
                    return crate::map::actor_check::differs(what, &instance, &"its group is gone");
                };
                let held = (held.key, held.view, held.live, held.band, held.slots.get(slot));
                let wanted = (meta.key, unit.view, unit.live, band, Some(&instance));
                if held != wanted {
                    return crate::map::actor_check::differs(what, &instance, &(held, wanted));
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
                    return crate::map::actor_check::differs(what, &instance, &("rows", supply));
                }
            }
        }
        let mut last = 0;
        for &group in &self.order {
            let Some(held) = self.groups[group].as_ref() else {
                return crate::map::actor_check::differs(what, &group, &"a drawn group is gone");
            };
            if std::mem::replace(&mut drawn[group], true) {
                return crate::map::actor_check::differs(what, &group, &"drawn twice");
            }
            if held.band < last {
                return crate::map::actor_check::differs(what, &group, &(held.band, last));
            }
            last = held.band;
        }
        for (group, held) in self.groups.iter().enumerate() {
            let held = held.as_ref().is_some_and(|held| !held.slots.is_empty());
            if held != drawn[group] {
                return crate::map::actor_check::differs(what, &group, &"held but not drawn");
            }
        }
        for (instance, &band) in walked.iter().enumerate() {
            let held = self.groups[self.instances[instance].group].as_ref();
            if held.is_none_or(|held| held.band != band) {
                return crate::map::actor_check::differs(what, &instance, &"banded apart");
            }
        }
        crate::map::actor_check::checked(what, self.instances.len() as u64);
    }

    /// Takes a laid-out walk as the list. `made` is kept only when the walk refused nothing and
    /// waited on no build.
    #[allow(clippy::too_many_arguments, reason = "the walk, its layout and what it was made from")]
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

    /// How a lit walk of the list, in its order, leaves the frame's constant state: the same
    /// fold over its nodes and draws as the walk makes ([`plans::PendingSources`]), with the
    /// list's units as the origins of their blocks.
    /// `binds` gives a part's sampler slots and whether it is skinned.
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
        // Instances of a group whose draw is not encoded are not drawn.
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

    /// Full walks, kept frames, splices and figures walked into them.
    #[cfg(test)]
    pub(super) fn counts(&self) -> Counts {
        self.counts
    }

    /// The list's bytes, as its draws read them.
    #[cfg(test)]
    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The walk's draws the draw at `at` stands for, by slot.
    #[cfg(test)]
    pub(super) fn slots_at(&self, at: usize) -> &[usize] {
        &self.groups[self.order[at]].as_ref().expect("a drawn group").slots
    }

    /// The part the draw at `at` draws.
    #[cfg(test)]
    pub(super) fn key_at(&self, at: usize) -> usize {
        self.groups[self.order[at]].as_ref().expect("a drawn group").key
    }
}

/// A group the walk found, with its instances' draws in walk order.
/// Cuts a walk's draws into bands: maximal runs whose parts agree on what their draws must agree
/// on for their order among the list's figures to leave every pixel the same
/// ([`mesh_shader::Order`]). Draws of one part may stand as one instanced draw only inside a band,
/// because a draw that agrees with nothing - or with another order - may not be stepped over. A
/// part that allows no order at all is a band of its own.
#[derive(Default)]
struct Bands {
    /// The order of the draw before, once one has been given.
    held: Option<Option<mesh_shader::Order>>,
    band: u32,
}

impl Bands {
    /// The band of the next draw in walk order.
    fn next(&mut self, order: Option<mesh_shader::Order>) -> u32 {
        let agrees = matches!((self.held, order), (Some(Some(held)), Some(order)) if held == order);
        if self.held.is_some() && !agrees {
            self.band += 1;
        }
        self.held = Some(order);
        self.band
    }

    /// How many bands the walk left.
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

/// Which of a node's blocks a binding reads; none for a block the whole list shares.
pub(super) fn supply_of(binding: u32, reads: Reads) -> Option<Supply> {
    match plan_tail::selector(binding) {
        Some(plan_tail::VERTEX) => Some(Supply::Vertex),
        Some(plan_tail::PIXEL) if reads.own_pixel => Some(Supply::Vertex),
        Some(plan_tail::SKIN) => Some(Supply::Skin),
        _ => None,
    }
}

/// Writes what their programs read of one block into `slot`'s row of every run of `supply` in
/// `group`.
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

/// The base row of each block a group's part reads per instance, in the order
/// [`shader_pipeline::gpu::BindingProgram::instanced_rows`] gives them.
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

/// A group's draw bound through `buffer`: each of its program's reads at its place in `bytes` —
/// a shared block at its own offset, as `shared` lays it out, and a block of the node's own as
/// the run its instances' rows stand in, which the rows block the draw selects reaches from its
/// base. None when it reads more of a shared block than was reserved for it (`shared.reserved`,
/// when the list was laid out before the draw).
#[allow(clippy::too_many_arguments, reason = "the draw, its group and the buffers they sit in")]
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

/// The block alignment a walk places with.
pub(super) fn block_alignment(arena: &shader_pipeline::gpu::Arena) -> usize {
    alignment(arena.device())
}

#[cfg(test)]
mod tests;
