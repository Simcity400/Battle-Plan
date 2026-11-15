//! A pass's retained draw lists, by figure: the shadow casters' list and one list for each lit
//! walk of a frame. Each figure in each view keeps its nodes, its draws and a span of a buffer
//! the list owns, where its constant blocks sit and its draws are bound once. A frame made from
//! the same inputs ([`Made`]) walks nothing: it writes this frame's shared blocks (the scene
//! blocks, and for a lit list the light grid and the incoming pixel snapshot), the blocks of
//! actors that have a player (whose poses move by nature) and of instances that changed, and
//! records the kept draws. A figure that joins, leaves, is given a player or no longer keeps its
//! level of detail costs that figure: the others keep their spans and bindings, and only the
//! order is made again. A lit list also keeps how its walk leaves the frame's constant state
//! ([`Replay`]), which later passes read, and gives it again every frame it is kept.
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
    pub camera: Camera,
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
            camera: frame.camera,
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
        let Made { revision, camera, offsets, visible, alpha, shared, reads, samples, textures, globals, chain, cull } =
            self;
        [
            *revision != other.revision,
            *camera != other.camera,
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

/// Where a block was placed: its region (live or still) and offset there while walking, the
/// buffer's offset once placed; and its length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Block {
    pub live: bool,
    pub at: usize,
    pub length: usize,
}

/// A node the walk reached in one view. A live one (its actor has a player) is written again
/// every frame; another when its instance changes.
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
    /// Its draws, which follow its own in the list.
    pub parts: usize,
}

/// A draw found by the walk, bound once its blocks have their final place.
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
    region: usize,
}

/// A figure walked: its mark, and its units, draws and blocks in the recording.
struct Walked {
    mark: Mark,
    units: Range<usize>,
    pending: Range<usize>,
    region: Range<usize>,
}

/// One walk's blocks, draws and figures, in walk order.
#[derive(Default)]
pub(super) struct Recording {
    /// The live and the still blocks.
    regions: [Vec<u8>; 2],
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
            region: self.regions[usize::from(!live)].len(),
        });
    }

    /// Places `bytes` in the live or still region at the next `alignment`, zero-filled to
    /// `reserved`, as the arena places a constant block.
    pub(super) fn place(
        &mut self,
        live: bool,
        bytes: &[u8],
        reserved: usize,
        alignment: usize,
    ) -> Block {
        let region = &mut self.regions[usize::from(!live)];
        Block {
            live,
            at: place(region, bytes, reserved, alignment),
            length: bytes.len(),
        }
    }

    /// Each figure walked with its units, draws and blocks: a figure's blocks are one run of its
    /// region, from its first block's aligned start.
    fn walked(&self, alignment: usize) -> Vec<Walked> {
        let mut ends = [self.regions[0].len(), self.regions[1].len()];
        let mut walked: Vec<Walked> = Vec::with_capacity(self.marks.len());
        let (mut units, mut pending) = (self.units.len(), self.pending.len());
        for mark in self.marks.iter().rev() {
            let end = &mut ends[usize::from(!mark.live)];
            let start = align(mark.region, alignment).min(*end);
            walked.push(Walked {
                mark: *mark,
                units: mark.units..units,
                pending: mark.pending..pending,
                region: start..*end,
            });
            *end = mark.region;
            (units, pending) = (mark.units, mark.pending);
        }
        walked.reverse();
        walked
    }
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

/// The spans of one area of the buffer (the live or the still blocks): the figures' spans are
/// taken from its free runs or above its top, and given back as figures leave.
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

    /// A span of `length` bytes, a multiple of the alignment every span starts at; none when
    /// the area is full.
    pub(super) fn take(&mut self, length: usize) -> Option<usize> {
        if let Some(at) = self.free.iter().position(|run| run.len() >= length) {
            let start = self.free[at].start;
            if self.free[at].len() == length {
                self.free.remove(at);
            } else {
                self.free[at].start += length;
            }
            return Some(start);
        }
        (self.top + length <= self.end).then(|| {
            self.top += length;
            self.top - length
        })
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

/// A full walk's blocks in their final order: the shared blocks, the live area, then the still
/// one, each area with room for figures that join.
pub(super) struct Layout {
    pub bytes: Vec<u8>,
    pub shared: Vec<usize>,
    live: Area,
    still: Area,
}

impl Layout {
    /// `reserved` is each shared block's size, the largest a draw's program reads of it.
    pub(super) fn new(
        mut bytes: Vec<u8>,
        recording: &mut Recording,
        shared: &[&[u8]],
        reserved: &[usize],
        alignment: usize,
    ) -> Self {
        bytes.clear();
        let shared = shared
            .iter()
            .zip(reserved)
            .map(|(block, reserved)| place(&mut bytes, block, *reserved, alignment))
            .collect();
        let mut area = |region: &[u8]| {
            let start = align(bytes.len(), alignment);
            bytes.resize(start, 0);
            bytes.extend_from_slice(region);
            let end = start + capacity(region.len(), alignment);
            bytes.resize(end, 0);
            Area::new(start, start + align(region.len(), alignment), end)
        };
        let live = area(&recording.regions[0]);
        let still = area(&recording.regions[1]);
        let layout = Self {
            bytes,
            shared,
            live,
            still,
        };
        for unit in &mut recording.units {
            unit.vertex = unit.vertex.map(|block| layout.absolute(block));
            unit.skin = unit.skin.map(|block| layout.absolute(block));
        }
        layout
    }

    /// A walk's block at its place in the buffer.
    pub(super) fn absolute(&self, block: Block) -> Block {
        let area = if block.live { &self.live } else { &self.still };
        Block {
            at: block.at + area.start,
            ..block
        }
    }
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
    pub draws: Range<usize>,
    /// Its blocks in the buffer, in the live or still area.
    pub span: Range<usize>,
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

/// What instancing by part would leave of a list ([`Kept::part_counts`]).
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

/// The retained list: its figures, their draws, their blocks' bytes as the GPU holds them, and
/// what the walk that made it read.
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
    /// The live units.
    live: Vec<usize>,
    /// Each unit by its instance, for the instances that change, made when first asked after the
    /// units change.
    by_instance: std::cell::OnceCell<Vec<(usize, usize)>>,
    visited: Vec<bool>,
    /// Still blocks written this frame, to reach the GPU with it.
    dirty: Vec<(usize, usize)>,
    shared: Shared,
    bytes: Vec<u8>,
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
    }

    pub(super) fn live(&self) -> &[usize] {
        &self.live
    }

    pub(super) fn unit(&self, at: usize) -> Unit {
        self.units[at]
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

    /// Whether the walk drew `group` in `view`.
    pub(super) fn visited(&self, view: usize, group: usize, groups: usize) -> bool {
        self.visited.get(view * groups + group).copied().unwrap_or(false)
    }

    /// The bytes of a unit's vertex block, as its draws read them.
    pub(super) fn vertex_bytes(&self, unit: usize) -> Option<&[u8]> {
        let block = self.units.get(unit)?.vertex?;
        Some(&self.bytes[block.at..block.at + block.length])
    }

    /// The bytes of a unit's palette.
    pub(super) fn skin_bytes(&self, unit: usize) -> Option<&[u8]> {
        let block = self.units.get(unit)?.skin?;
        Some(&self.bytes[block.at..block.at + block.length])
    }

    /// Writes a node's world, userdata and palette in place; false when its palette's length
    /// moved, which only a walk of its figure can place.
    pub(super) fn write_unit(&mut self, at: usize, bytes: LiveBytes<'_>) -> bool {
        let unit = self.units[at];
        match (unit.skin, bytes.bones) {
            (Some(skin), Some(bones)) if bones.len() == skin.length => {
                self.bytes[skin.at..skin.at + skin.length].copy_from_slice(bones);
                if !unit.live {
                    self.dirty.push((skin.at, skin.length));
                }
            }
            (None, None) => {}
            _ => return false,
        }
        if let Some(vertex) = unit.vertex {
            self.bytes[vertex.at..vertex.at + 64].copy_from_slice(bytemuck::bytes_of(bytes.world));
            let mut written = 64;
            if let Some(userdata) = bytes.userdata {
                self.bytes[vertex.at + 64..vertex.at + 96]
                    .copy_from_slice(bytemuck::bytes_of(&userdata));
                written = 96;
            }
            if !unit.live {
                self.dirty.push((vertex.at, written));
            }
        }
        true
    }

    /// Writes this frame's shared blocks; returns the bytes a kept frame hands the GPU from the
    /// start: the shared blocks and the live area.
    pub(super) fn kept_bytes(&mut self, shared: &[&[u8]]) -> &[u8] {
        for (&at, bytes) in self.shared.at.iter().zip(shared) {
            self.bytes[at..at + bytes.len()].copy_from_slice(bytes);
        }
        self.counts.kept += 1;
        &self.bytes[..self.live_area.top]
    }

    /// The still blocks written this frame as ranges to write, near ones joined.
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

    /// Keeps the list for this frame: the shared and live blocks, and the still blocks written
    /// this frame, go to the arena's flush.
    pub(super) fn keep(&mut self, arena: &shader_pipeline::gpu::Arena, shared: &[&[u8]]) {
        let ranges = self.dirty_ranges();
        let buffer = self.buffer.clone();
        let bytes = self.kept_bytes(shared);
        if let Some(buffer) = buffer {
            if !bytes.is_empty() {
                arena.write(&buffer, 0, bytes);
            }
            for range in ranges {
                arena.write(&buffer, range.start as u64, &self.bytes[range]);
            }
        }
        let name = match self.kind {
            Kind::Casters => super::super::Count::CasterKept,
            Kind::Lit => super::super::Count::LitKept,
        };
        super::super::count(name, 1);
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

    /// Makes the list this frame's figures, in walk order (`next`: each target's kept figure, or
    /// none for the next the walk recorded): the figures left give their spans back, the
    /// walked ones take spans, have their blocks copied there and their draws bound by `bind`
    /// (given the bytes, the shared blocks, the draw and its blocks' places; none when it cannot
    /// be bound in this layout). False, the list to be laid out again, when a walked figure
    /// cannot be placed or bound, or the walk is not the one planned.
    pub(super) fn splice(
        &mut self,
        next: &[Option<usize>],
        recording: &Recording,
        visited: Vec<bool>,
        alignment: usize,
        mut bind: impl FnMut(&[u8], &Shared, &Pending, Block, Option<Block>) -> io::Result<Option<RecordedDraw>>,
    ) -> io::Result<bool> {
        let walked = recording.walked(alignment);
        let wanted = next.iter().filter(|figure| figure.is_none()).count();
        let routes = walked.iter().map(|walk| (walk.mark.view, walk.mark.position));
        if walked.len() != wanted
            || recording.only.as_ref().is_some_and(|(only, _)| !only.iter().copied().eq(routes))
        {
            self.dropped = Some(Whole::Unplanned);
            return Ok(false);
        }
        let mut kept = vec![false; self.figures.len()];
        for &figure in next.iter().flatten() {
            kept[figure] = true;
        }
        for (figure, kept) in self.figures.iter().zip(&kept) {
            if !kept {
                let area = if figure.live { &mut self.live_area } else { &mut self.still_area };
                area.give(figure.span.clone());
            }
        }
        let left = kept.iter().filter(|kept| !**kept).count();
        let figures = std::mem::take(&mut self.figures);
        let mut draws: Vec<Option<RecordedDraw>> = std::mem::take(&mut self.draws.draws).into_iter().map(Some).collect();
        let mut units = Vec::with_capacity(self.units.len());
        let mut placed = Vec::with_capacity(draws.len());
        let mut metas = Vec::with_capacity(draws.len());
        let mut walked = walked.into_iter();
        for figure in next {
            let made = match figure {
                Some(at) => {
                    let old = &figures[*at];
                    let start = (units.len(), placed.len());
                    units.extend_from_slice(&self.units[old.units.clone()]);
                    metas.extend_from_slice(&self.metas[old.draws.clone()]);
                    placed.extend(draws[old.draws.clone()].iter_mut().map(|draw| {
                        draw.take().expect("a figure is kept once")
                    }));
                    Figure {
                        units: start.0..units.len(),
                        draws: start.1..placed.len(),
                        ..old.clone()
                    }
                }
                None => {
                    let walk = walked.next().expect("counted above");
                    let mark = walk.mark;
                    let region = &recording.regions[usize::from(!mark.live)][walk.region.clone()];
                    let span = if region.is_empty() {
                        0..0
                    } else {
                        let length = align(region.len(), alignment);
                        let area = if mark.live { &mut self.live_area } else { &mut self.still_area };
                        let Some(at) = area.take(length) else {
                            self.dropped = Some(Whole::NoRoom);
                            return Ok(false);
                        };
                        self.bytes[at..at + region.len()].copy_from_slice(region);
                        if !mark.live {
                            self.dirty.push((at, region.len()));
                        }
                        at..at + length
                    };
                    let from = walk.region.start;
                    let absolute = |block: Block| Block {
                        at: block.at - from + span.start,
                        ..block
                    };
                    let start = (units.len(), placed.len());
                    units.extend(recording.units[walk.units.clone()].iter().map(|unit| Unit {
                        vertex: unit.vertex.map(absolute),
                        skin: unit.skin.map(absolute),
                        ..*unit
                    }));
                    for pending in &recording.pending[walk.pending.clone()] {
                        let Some(draw) = bind(
                            &self.bytes,
                            &self.shared,
                            pending,
                            absolute(pending.vertex),
                            pending.skin.map(absolute),
                        )?
                        else {
                            self.dropped = Some(Whole::Unbound);
                            return Ok(false);
                        };
                        placed.push(draw);
                        metas.push(Meta {
                            key: pending.key,
                            textures: pending.textures,
                            material: pending.material,
                        });
                    }
                    Figure {
                        view: mark.view,
                        position: mark.position,
                        key: mark.key,
                        live: mark.live,
                        waiting: mark.waiting,
                        units: start.0..units.len(),
                        draws: start.1..placed.len(),
                        span,
                    }
                }
            };
            self.figures.push(made);
        }
        self.units = units;
        self.metas = metas;
        self.draws.draws = placed;
        self.draws.unplanned = true;
        self.visited = visited;
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
        Ok(true)
    }

    /// What instancing by part would leave of the list's draws: the distinct parts drawn, the runs
    /// of consecutive draws of one part in one view (walk order kept), and the distinct parts in
    /// each view (one draw each, order across figures given up).
    pub(super) fn part_counts(&self) -> PartCounts {
        let mut parts = std::collections::HashSet::new();
        let mut in_views = std::collections::HashSet::new();
        let mut runs = 0;
        let mut last = None;
        let mut draw = 0;
        for unit in &self.units {
            for meta in &self.metas[draw..draw + unit.parts] {
                let part = (unit.view, meta.key);
                parts.insert(meta.key);
                in_views.insert(part);
                if last != Some(part) {
                    runs += 1;
                    last = Some(part);
                }
            }
            draw += unit.parts;
        }
        PartCounts { parts: parts.len(), runs, in_views: in_views.len() }
    }

    /// The nodes drawn at each level of detail, the last holding every deeper one.
    pub(super) fn lods(&self) -> [u64; LODS] {
        self.lods
    }

    /// The figures walked into the list and the figures that left it since the last call.
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

    /// Takes a laid-out walk as the list. `made` is kept only when the walk refused nothing and
    /// waited on no build.
    fn take(
        &mut self,
        layout: Layout,
        recording: Recording,
        draws: DrawList,
        shared: Shared,
        alignment: usize,
        made: Option<Made>,
    ) {
        self.moved[1] += self.figures.len() as u64;
        self.dropped = None;
        self.culled = recording.culled.iter().copied().collect();
        self.figures.clear();
        for walk in recording.walked(alignment) {
            let span = if walk.region.is_empty() {
                0..0
            } else {
                let area = if walk.mark.live { &layout.live } else { &layout.still };
                area.start + walk.region.start..area.start + align(walk.region.end, alignment)
            };
            self.figures.push(Figure {
                view: walk.mark.view,
                position: walk.mark.position,
                key: walk.mark.key,
                live: walk.mark.live,
                waiting: walk.mark.waiting,
                units: walk.units,
                draws: walk.pending,
                span,
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
        self.index();
        self.visited = recording.visited;
        self.dirty.clear();
        self.shared = Shared {
            at: layout.shared,
            ..shared
        };
        self.bytes = layout.bytes;
        self.live_area = layout.live;
        self.still_area = layout.still;
        self.counts.walks += 1;
    }

    /// Lays out a full walk's blocks, binds every draw there and writes the bytes. `programs`
    /// gives a draw's resident program; `shared` are the shared blocks' bytes as `reads` orders
    /// them.
    #[allow(clippy::too_many_arguments, reason = "the walk, its inputs and what it was made from")]
    pub(super) fn finish<'p>(
        &mut self,
        arena: &shader_pipeline::gpu::Arena,
        mut recording: Recording,
        mut draws: DrawList,
        shared: &[&[u8]],
        reads: Reads,
        programs: impl Fn(usize) -> Option<&'p shader_pipeline::gpu::BindingProgram>,
        made: Option<Made>,
    ) -> io::Result<()> {
        let missing = || invalid("native program missing after its walk");
        let alignment = alignment(arena.device());
        let mut reserved: Vec<usize> = shared.iter().map(|bytes| bytes.len()).collect();
        for pending in &recording.pending {
            for spec in programs(pending.key).ok_or_else(missing)?.constant_reads() {
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
        let layout = Layout::new(
            std::mem::take(&mut self.bytes),
            &mut recording,
            shared,
            &reserved,
            alignment,
        );
        let device = arena.device();
        let needed = (layout.bytes.len() as u64).max(wgpu::COPY_BUFFER_ALIGNMENT);
        if self.buffer.as_ref().is_none_or(|buffer| buffer.size() < needed) {
            let size = (needed + needed / 4).next_multiple_of(alignment as u64);
            if size > device.limits().max_buffer_size {
                return Err(invalid(format!(
                    "native draw list: {size}-byte constant buffer exceeds GPU limit {}",
                    device.limits().max_buffer_size
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
        let blocks = Shared {
            at: layout.shared.clone(),
            reserved,
            reads,
        };
        draws.draws.clear();
        for pending in &recording.pending {
            let program = programs(pending.key).ok_or_else(missing)?;
            let draw = bind(
                arena,
                &buffer,
                program,
                (&layout.bytes, &blocks),
                pending,
                layout.absolute(pending.vertex),
                pending.skin.map(|block| layout.absolute(block)),
            )?
            .ok_or_else(|| invalid("a shared block is reserved for every read of its walk"))?;
            draws.draws.push(draw);
        }
        for range in [0..layout.live.top, layout.still.start..layout.still.top] {
            if !range.is_empty() {
                arena.write(&buffer, range.start as u64, &layout.bytes[range]);
            }
        }
        let made = made.filter(|_| !recording.incomplete());
        self.take(layout, recording, draws, blocks, alignment, made);
        let name = match self.kind {
            Kind::Casters => super::super::Count::CasterWalks,
            Kind::Lit => super::super::Count::LitWalks,
        };
        super::super::count(name, 1);
        Ok(())
    }

    /// The buffer the list's draws are bound through.
    pub(super) fn buffer(&self) -> Option<wgpu::Buffer> {
        self.buffer.clone()
    }

    /// How a lit walk of the list, in its order, leaves the frame's constant state: the same
    /// fold over its nodes and draws as the walk makes ([`plans::PendingSources`]), with the
    /// list's units as the origins of their blocks.
    /// `binds` gives a part's sampler slots and whether it is skinned.
    pub(super) fn replay(&self, binds: impl Fn(usize) -> (u32, bool)) -> Replay {
        let mut sources = plans::PendingSources::default();
        let (mut material, mut last, mut draw) = (None, None, 0);
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
            for meta in &self.metas[draw..draw + unit.parts] {
                sources.state.record_part(meta.key, binds(meta.key), meta.textures);
                material = Some(meta.material);
            }
            draw += unit.parts;
        }
        let unshown = self.draws.draws.iter().filter(|draw| !draw.shown).count();
        Replay {
            sources,
            material,
            last,
            drawn: draw.saturating_sub(unshown) as u32,
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
}

/// A draw bound through `buffer` at its blocks' places in `bytes`, reading the shared blocks as
/// `shared` lays them out; none when it reads more of
/// a shared block than was reserved for it (`shared.reserved`, when the list was laid out
/// before the draw).
#[allow(clippy::too_many_arguments, reason = "the draw, its blocks and the buffers they sit in")]
pub(super) fn bind(
    arena: &shader_pipeline::gpu::Arena,
    buffer: &wgpu::Buffer,
    program: &shader_pipeline::gpu::BindingProgram,
    (bytes, shared): (&[u8], &Shared),
    pending: &Pending,
    vertex: Block,
    skin: Option<Block>,
) -> io::Result<Option<RecordedDraw>> {
    let reads = shared.reads;
    let mut blocks = Vec::with_capacity(program.constant_reads().len());
    for spec in program.constant_reads() {
        let at = match plan_tail::selector(spec.binding) {
            Some(plan_tail::FRAME) => Some(pending.view),
            Some(plan_tail::VERTEX) => None,
            Some(plan_tail::PIXEL) if reads.own_pixel => None,
            Some(plan_tail::PIXEL) if reads.pixel => Some(reads.pixel_at()),
            Some(plan_tail::GRID) if reads.grid => Some(reads.grid_at()),
            Some(plan_tail::SKIN) => None,
            _ => return Err(invalid("a native draw reads a block its list does not supply")),
        };
        let (place, length) = match at {
            Some(at) => {
                let (Some(&place), Some(&reserved)) = (shared.at.get(at), shared.reserved.get(at))
                else {
                    return Err(invalid("a native draw reads a shared block its list has not"));
                };
                if spec.size as usize > reserved {
                    return Ok(None);
                }
                (place, spec.size as usize)
            }
            None if plan_tail::selector(spec.binding) == Some(plan_tail::SKIN) => {
                let skin = skin
                    .ok_or_else(|| invalid("a native draw reads a palette its part does not have"))?;
                (skin.at, skin.length)
            }
            None => (vertex.at, vertex.length),
        };
        let offset = u32::try_from(place).map_err(|_| invalid("constant offset exceeds u32"))?;
        blocks.push((&bytes[place..place + length], offset));
    }
    let bindings = program.bind_buffer(arena, buffer, &blocks);
    Ok(Some(RecordedDraw {
        id: draw_id(),
        shown: true,
        bindings: bindings.map_err(invalid)?,
        reference: pending.reference,
        vertices: pending.vertices,
        indices: pending.indices,
        first: pending.first,
        count: pending.count,
        base: pending.base,
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
