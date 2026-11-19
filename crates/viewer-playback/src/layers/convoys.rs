//! The viewed country's trade-route lines and the convoys the unit models sail on them, from the
//! mirrors: the values of the rows today's builder reads ([`trade_routes::build_lines`]) kept per
//! row from change sets, the product made again when a row it reads changed, and each route's
//! stops (the content's search) kept while the route and the provinces' ports and states are
//! unchanged.
//!
//! Which country is viewed is the vision's (P6's layer): the window's layer reads it from the
//! vision layer, the proof gives a vision whole ([`Convoys::set_vision`]); without one the layer is
//! refused by name.

use super::{Carried, Settling, Inputs, Layer, List, Num, Published, Text, Upstream, View};
use crate::playhead_host::mirror::{Cell, Mirror, Mirrors, UNREAD_REASON};
use playhead::{Step, changes::Changes};
use replay_columnar::manifest::Status;
use replay_data::{Hour, rows::TableId};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    time::Instant,
};
use viewer_foundation::map::{
    perspective::{Choice, Vision},
    province_map::HeightField,
    ribbon_content::{ArrowType, OrderDraw, OrderField, OrderVertex},
};
use viewer_world::map::{
    map_mode::visibility,
    trade_routes::{self, ClientFields, Countries, Emit, ExchangeOrder, LinksGap, OriginFields, RegionalFields, RouteFields, RouteKey, Shaped, Stops, Tables, TradeRoutes},
};

/// A `ResourceOrigins` row's values the lines read.
#[derive(Clone, Debug, PartialEq)]
pub struct Origin {
    country: u32,
    state: u32,
    list: u8,
    ordinal: u32,
    bad: bool,
    convoys_assigned: i32,
}

/// A `ConvoyClients` row's values the lines read.
#[derive(Clone, Debug, PartialEq)]
pub struct Client {
    handle: u32,
    category: u8,
    owner: u32,
    other: u32,
    route_class: u8,
    route_country: u32,
    route_other: u32,
    bad: bool,
    convoys_assigned: i32,
    owner_ordinal: Option<u32>,
    other_ordinal: Option<u32>,
}

/// A `TradeRoutes` row's values the lines and their stops read.
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    country: u32,
    class: u8,
    other: u32,
    kind: u8,
    from: u32,
    to: u32,
    path: Vec<u32>,
    source_state: Option<u32>,
    destination_state: Option<u32>,
}

/// A `RegionalConvoyEfficiency` row's values.
#[derive(Clone, Debug, PartialEq)]
pub struct Regional {
    country: u32,
    region: u32,
    efficiency: i64,
}

macro_rules! fields {
    ($trait:ident for $type:ty { $($name:ident: $ret:ty),+ $(,)? }) => {
        impl $trait for $type {
            $(fn $name(&self) -> $ret {
                self.$name
            })+
        }
    };
}

fields!(OriginFields for Origin { country: u32, state: u32, list: u8, ordinal: u32, bad: bool, convoys_assigned: i32 });
fields!(ClientFields for Client {
    handle: u32, category: u8, owner: u32, other: u32, route_class: u8, route_country: u32, route_other: u32,
    bad: bool, convoys_assigned: i32, owner_ordinal: Option<u32>, other_ordinal: Option<u32>,
});
fields!(RegionalFields for Regional { country: u32, region: u32, efficiency: i64 });

impl RouteFields for Route {
    fn country(&self) -> u32 {
        self.country
    }
    fn class(&self) -> u8 {
        self.class
    }
    fn other(&self) -> u32 {
        self.other
    }
    fn kind(&self) -> u8 {
        self.kind
    }
    fn from(&self) -> u32 {
        self.from
    }
    fn to(&self) -> u32 {
        self.to
    }
    fn path(&self) -> &[u32] {
        &self.path
    }
    fn source_state(&self) -> Option<u32> {
        self.source_state
    }
    fn destination_state(&self) -> Option<u32> {
        self.destination_state
    }
}

/// The provinces' ports and states, which a route's stops read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Provinces {
    pub ports: BTreeMap<u32, u32>,
    pub state_of: BTreeMap<u32, u32>,
}

/// The lines' tables, as the builder reads them; each why it cannot be read whole.
pub type Read<'a> = Result<Tables<'a, Origin, Client, Route, Regional>, String>;

/// Today's builder over the content: a route's stops and the lines and convoys of the tables.
pub trait Routes: Send + Sync {
    fn stops(&self, route: &Route, reverse: bool, provinces: &Provinces) -> Result<Option<Stops>, String>;

    #[allow(clippy::too_many_arguments, reason = "the builder's tables, countries, exchanges, culture, provinces and stops are each the layer's")]
    fn build(
        &self,
        ours: u32,
        tables: Read<'_>,
        countries: &Countries<'_>,
        exchanges: &BTreeMap<u32, ExchangeOrder>,
        culture: &dyn Fn(&Countries<'_>) -> Result<String, String>,
        provinces: Result<&Provinces, String>,
        stops: &mut dyn FnMut(&Route, bool) -> Result<Option<Stops>, String>,
        emit: &mut Emit<'_>,
    ) -> TradeRoutes;
}

impl Routes for crate::document::Content {
    fn stops(&self, route: &Route, reverse: bool, provinces: &Provinces) -> Result<Option<Stops>, String> {
        let (graph, geography) = self.icons.raids.mission_topology().map_err(|why| format!("the content's province graph is unavailable: {why}"))?;
        let position = |province: u32| {
            let slot = if geography.land.contains(&province) && provinces.ports.contains_key(&province) {
                viewer_foundation::map::unit_rules::SHIP_IN_PORT
            } else {
                viewer_foundation::map::unit_rules::STANDSTILL
            };
            self.counters.anchor(province, slot)
        };
        let pixels = |province| self.province_geometry.pixel_count(province);
        let crossing = |a, b| self.movement.as_ref()?.map.neighbour_crossing(a, b);
        let content = trade_routes::RouteContent {
            graph,
            geography,
            state_of: &provinces.state_of,
            ports: &provinces.ports,
            pixels: &pixels,
            crossing: &crossing,
            region_of: &self.icons.raids.regions,
            region_provinces: &self.icons.raids.region_provinces,
            state_provinces: &self.icons.raids.trigger_content.state_provinces,
            position: &position,
        };
        trade_routes::directed_stops(route, &content, reverse)
    }

    fn build(
        &self,
        ours: u32,
        tables: Read<'_>,
        countries: &Countries<'_>,
        exchanges: &BTreeMap<u32, ExchangeOrder>,
        culture: &dyn Fn(&Countries<'_>) -> Result<String, String>,
        provinces: Result<&Provinces, String>,
        stops: &mut dyn FnMut(&Route, bool) -> Result<Option<Stops>, String>,
        emit: &mut Emit<'_>,
    ) -> TradeRoutes {
        let mut out = TradeRoutes::default();
        let (graph, geography) = match self.icons.raids.mission_topology() {
            Ok(topology) => topology,
            Err(why) => {
                out.report.unavailable = Some(format!("the content's province graph is unavailable: {why}"));
                return out;
            }
        };
        let provinces = match provinces {
            Ok(provinces) => provinces,
            Err(why) => {
                out.report.unavailable = Some(why);
                return out;
            }
        };
        let position = |province: u32| {
            let slot = if geography.land.contains(&province) && provinces.ports.contains_key(&province) {
                viewer_foundation::map::unit_rules::SHIP_IN_PORT
            } else {
                viewer_foundation::map::unit_rules::STANDSTILL
            };
            self.counters.anchor(province, slot)
        };
        let pixels = |province| self.province_geometry.pixel_count(province);
        let crossing = |a, b| self.movement.as_ref()?.map.neighbour_crossing(a, b);
        let content = trade_routes::RouteContent {
            graph,
            geography,
            state_of: &provinces.state_of,
            ports: &provinces.ports,
            pixels: &pixels,
            crossing: &crossing,
            region_of: &self.icons.raids.regions,
            region_provinces: &self.icons.raids.region_provinces,
            state_provinces: &self.icons.raids.trigger_content.state_provinces,
            position: &position,
        };
        let heights = viewer_foundation::map::province_map::HeightField { width: self.assets.heightmap.width, height: self.assets.heightmap.height, values: self.assets.heightmap_texels() };
        let declared = |name: &str| self.unit_models.declares(name);
        let inputs = trade_routes::Inputs { orders: &self.orders, defines: self.trade_routes.as_ref().map_err(String::as_str), content: &content, heights: &heights, ours, declared: &declared };
        let checked = (|| {
            inputs.defines.map_err(|reason| format!("the trade-route defines cannot be read: {reason}"))?;
            inputs.orders.types.get(viewer_foundation::map::ribbon_content::TRADE_ROUTE_TYPE_NAME).ok_or_else(|| {
                format!("{} declares no arrowType {}", viewer_foundation::map::ribbon_content::MAPARROWS, viewer_foundation::map::ribbon_content::TRADE_ROUTE_TYPE_NAME)
            })?;
            let tables = tables?;
            trade_routes::build_lines(&inputs, &tables, countries, exchanges, culture, stops, emit, &mut out)
        })();
        if let Err(reason) = checked {
            out.report.unavailable = Some(reason);
        }
        out
    }
}

/// What a line's ribbon is made from: its arrow, colours and shaped controls.
type RibbonKey = (String, [u32; 4], [u32; 4], Vec<[i64; 2]>, Vec<u8>);

/// The rows a product reads, as the frame held them when it was asked for.
type Owned = (Vec<Origin>, Vec<Client>, Vec<Route>, Vec<Regional>);

/// What the worker makes one product from.
struct Job {
    /// The hour of the mirrors the rows were taken from.
    hour: Option<Hour>,
    ours: u32,
    tables: Result<Owned, String>,
    slots: Arc<BTreeMap<u32, u32>>,
    gap: LinksGap,
    exchanges: BTreeMap<u32, ExchangeOrder>,
    culture: Result<String, String>,
    provinces: Arc<Provinces>,
    provinces_read: Result<(), String>,
    /// Bumped by every change of a province's state or port: the stops found before are dropped.
    provinces_version: u64,
}

/// A product, the jobs it answers (newer ones replace older ones not started), and what it cost.
struct Made {
    product: TradeRoutes,
    /// The hour of the mirrors the product was made from.
    hour: Option<Hour>,
    jobs: usize,
    reused: usize,
    made: usize,
    micros: usize,
    bytes: usize,
}

/// The layer's own worker: makes the products off the frame, keeping each route's stops and each
/// drawn line's ribbon from one product to the next.
struct Maker {
    routes: Arc<dyn Routes>,
    /// Each route's stops by key and direction, with the route they were found for.
    stops: HashMap<(RouteKey, bool), (Route, Result<Option<Stops>, String>)>,
    /// Each drawn line's ribbon by what it is made from, as the last product drew them.
    ribbons: HashMap<RibbonKey, Option<Arc<Ribbon>>>,
    provinces_version: u64,
}

impl Maker {
    fn spawn(routes: Arc<dyn Routes>) -> io::Result<(Sender<Job>, Receiver<Made>)> {
        let (jobs, receive) = mpsc::channel::<Job>();
        let (send, made) = mpsc::channel::<Made>();
        let mut maker = Self { routes, stops: HashMap::new(), ribbons: HashMap::new(), provinces_version: 0 };
        std::thread::Builder::new().name("convoys".into()).spawn(move || {
            viewer_foundation::worker_priority::background_priority();
            while let Ok(mut job) = receive.recv() {
                let mut answered = 1;
                while let Ok(newer) = receive.try_recv() {
                    (job, answered) = (newer, answered + 1);
                }
                if send.send(maker.make(job, answered)).is_err() {
                    return;
                }
            }
        })?;
        Ok((jobs, made))
    }

    fn make(&mut self, job: Job, answered: usize) -> Made {
        let started = Instant::now();
        let hour = job.hour;
        if job.provinces_version != self.provinces_version {
            self.stops.clear();
            self.provinces_version = job.provinces_version;
        }
        let countries = Countries::over(&job.slots, job.gap.clone(), job.ours);
        let tables: Read<'_> = match &job.tables {
            Ok((origins, clients, routes, regional)) => Ok(Tables { origins, clients, routes, regional }),
            Err(why) => Err(why.clone()),
        };
        let culture = |_: &Countries<'_>| job.culture.clone();
        let provinces = job.provinces_read.clone().map(|()| &*job.provinces);
        let (routes, cache) = (&self.routes, &mut self.stops);
        let mut stops = |route: &Route, reverse: bool| {
            let key = (route.country, route.class, route.other);
            match cache.get(&(key, reverse)) {
                Some((held, stops)) if held == route => stops.clone(),
                _ => {
                    let stops = routes.stops(route, reverse, &job.provinces);
                    cache.insert((key, reverse), (route.clone(), stops.clone()));
                    stops
                }
            }
        };
        let (held, mut kept) = (std::mem::take(&mut self.ribbons), HashMap::new());
        let (mut reused, mut made) = (0, 0);
        let mut emit = |field: &mut OrderField, arrow: &ArrowType, shaped: &Shaped, color: [f32; 4], hotspot: [f32; 4], heights: &HeightField<'_>| {
            let key: RibbonKey = (arrow.name.clone(), color.map(f32::to_bits), hotspot.map(f32::to_bits), shaped.controls.clone(), shaped.variants.clone());
            let ribbon = match held.get(&key).or_else(|| kept.get(&key)) {
                Some(ribbon) => {
                    reused += 1;
                    ribbon.clone()
                }
                None => {
                    made += 1;
                    let mut drawn = OrderField::default();
                    let points = trade_routes::emit(&mut drawn, arrow, shaped, color, hotspot, heights);
                    points.map(|points| Arc::new(Ribbon { vertices: drawn.vertices, native: drawn.native_vertices, draw: drawn.draws.pop().expect("a drawn ribbon"), points }))
                }
            };
            kept.insert(key, ribbon.clone());
            let ribbon = ribbon?;
            let base = field.native_vertices.len() as u32;
            field.vertices.extend_from_slice(&ribbon.vertices);
            field.native_vertices.extend_from_slice(&ribbon.native);
            let mut draw = ribbon.draw.clone();
            for strip in &mut draw.native_strips {
                (strip.start, strip.end) = (strip.start + base, strip.end + base);
            }
            field.draws.push(draw);
            Some(ribbon.points.clone())
        };
        let product = routes.build(job.ours, tables, &countries, &job.exchanges, &culture, provinces, &mut stops, &mut emit);
        self.ribbons = kept;
        let ribbons: usize = self.ribbons.values().flatten().map(|ribbon| ribbon.vertices.capacity() * std::mem::size_of::<OrderVertex>() + ribbon.native.capacity() * 32 + ribbon.points.capacity() * 8 + 256).sum();
        let stops: usize = self.stops.values().map(|(route, stops)| route.path.capacity() * 4 + stops.as_ref().ok().and_then(Option::as_ref).map_or(0, |stops| (stops.provinces.capacity() + stops.regions.capacity()) * 4) + 96).sum();
        Made { product, hour, jobs: answered, reused, made, micros: started.elapsed().as_micros() as usize, bytes: ribbons + stops }
    }
}

/// `CountryLinks`' status in the builder's words.
fn links_gap(links: Option<&Mirror>) -> LinksGap {
    match links.and_then(Mirror::status) {
        Some(Status::Complete) => LinksGap::Complete,
        Some(Status::Truncated { missing }) => LinksGap::Truncated { missing },
        Some(Status::Unavailable { .. }) => LinksGap::Unavailable { reason: links.and_then(Mirror::reason).unwrap_or(UNREAD_REASON).to_owned() },
        Some(Status::NotRecorded) | None => LinksGap::NotRecorded { mode: "this".into() },
    }
}

/// The tables whose rows the layer holds; a changed row that cannot touch the lines waits by key.
const DEFERRED: [TableId; 5] = [TableId::ResourceOrigins, TableId::ConvoyClients, TableId::TradeRoutes, TableId::RegionalConvoyEfficiency, TableId::ResourceExchanges];

fn deferred_at(table: TableId) -> Option<usize> {
    DEFERRED.iter().position(|&deferred| deferred == table)
}

/// One line's ribbon as `emit` drew it: its vertices, its native vertices with its strips relative
/// to them, its draw, and its drawn centre line.
#[derive(Clone, Debug)]
struct Ribbon {
    vertices: Vec<OrderVertex>,
    native: Vec<viewer_foundation::map::ribbon_content::Vertex>,
    draw: OrderDraw,
    points: Vec<[f32; 2]>,
}

/// One table's rows' values, sorted by key: the builder reads them as a slice in key order.
#[derive(Debug)]
struct Rows<T> {
    keys: Vec<u128>,
    values: Vec<T>,
}

impl<T> Default for Rows<T> {
    fn default() -> Self {
        Self { keys: Vec::new(), values: Vec::new() }
    }
}

impl<T: PartialEq> Rows<T> {
    /// Whether the row's values changed.
    fn put(&mut self, key: u128, value: T) -> bool {
        if self.keys.last().is_none_or(|&last| last < key) {
            self.keys.push(key);
            self.values.push(value);
            return true;
        }
        match self.keys.binary_search(&key) {
            Ok(at) if self.values[at] == value => false,
            Ok(at) => {
                self.values[at] = value;
                true
            }
            Err(at) => {
                self.keys.insert(at, key);
                self.values.insert(at, value);
                true
            }
        }
    }

    fn remove(&mut self, key: u128) -> bool {
        match self.keys.binary_search(&key) {
            Ok(at) => {
                self.keys.remove(at);
                self.values.remove(at);
                true
            }
            Err(_) => false,
        }
    }

    fn clear(&mut self) {
        self.keys.clear();
        self.values.clear();
    }

    fn get(&self, key: u128) -> Option<&T> {
        self.keys.binary_search(&key).ok().map(|at| &self.values[at])
    }

    fn bytes(&self) -> usize {
        self.keys.capacity() * 16 + self.values.capacity() * std::mem::size_of::<T>()
    }
}

#[derive(Clone, Copy)]
struct Columns {
    origin_list: Num<u8>,
    origin_bad: Num<bool>,
    origin_convoys: Num<i32>,
    origin_ordinal: Num<u32>,
    client_category: Num<u8>,
    client_owner: Num<u32>,
    client_other: Num<u32>,
    client_route_class: Num<u8>,
    client_route_country: Num<u32>,
    client_route_other: Num<u32>,
    client_bad: Num<bool>,
    client_convoys: Num<i32>,
    client_owner_ordinal: Num<u32>,
    client_other_ordinal: Num<u32>,
    route_kind: Num<u8>,
    route_from: Num<u32>,
    route_to: Num<u32>,
    route_path: List,
    route_source: Num<u32>,
    route_destination: Num<u32>,
    efficiency: Num<i64>,
    exchange_resource: Num<u32>,
    exchange_import: Num<u32>,
    exchange_export: Num<u32>,
    link_slot: Num<u32>,
    culture: Text,
    province_state: Num<u32>,
    province_port: Num<i32>,
}

/// The tables the layer reads, each a change of which may change the lines.
const TABLES: [TableId; 8] = [
    TableId::ResourceOrigins,
    TableId::ConvoyClients,
    TableId::TradeRoutes,
    TableId::RegionalConvoyEfficiency,
    TableId::ResourceExchanges,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::Provinces,
];

/// Why `mirror`'s table cannot be read whole at the head, in the words of today's reader where the
/// mirror holds them.
fn incomplete(mirror: Option<&Mirror>, table: TableId) -> Result<(), String> {
    let Some(mirror) = mirror else { return Err(format!("{table:?} is not read yet")) };
    let hour = mirror.head().unwrap_or_default();
    match mirror.status() {
        Some(Status::Complete) => Ok(()),
        Some(Status::Truncated { missing }) => Err(format!("hour {hour} {table:?} is truncated ({missing} missing rows)")),
        Some(Status::Unavailable { .. }) => Err(format!("hour {hour} {table:?} is unavailable: {}", mirror.reason().unwrap_or(UNREAD_REASON))),
        Some(Status::NotRecorded) => Err(format!("hour {hour} {table:?} is not recorded at this hour")),
        None => Err(format!("hour {hour} {table:?} is not recorded here: a gap in the recording")),
    }
}

fn path_of(mirror: &Mirror, column: List, slot: u32) -> Vec<u32> {
    match mirror.list(column, slot) {
        Cell::Value(bytes) => bytes.chunks_exact(4).map(|item| u32::from_le_bytes([item[0], item[1], item[2], item[3]])).collect(),
        _ => Vec::new(),
    }
}

/// The trade-route lines' and convoys' product and what keeps it.
pub struct Convoys {
    routes: Arc<dyn Routes>,
    given: Mutex<Option<(Choice, Arc<Vision>)>>,
    fresh_vision: AtomicBool,
    /// The perspective and the viewing country's slot the product is made for: all it reads of a
    /// vision.
    vision: Option<(Choice, u32)>,
    columns: Option<Columns>,
    refused: Option<String>,
    origins: Rows<Origin>,
    clients: Rows<Client>,
    trade: Rows<Route>,
    regional: Rows<Regional>,
    exchanges: Rows<ExchangeOrder>,
    links: Arc<BTreeMap<u32, u32>>,
    /// A link inserted, changed or removed since the last product: which references name the
    /// viewed country may have changed.
    links_changed: bool,
    provinces: Arc<Provinces>,
    provinces_version: u64,
    /// Changed rows not taken, by [`DEFERRED`] table: they could not touch the lines then.
    behind: [HashSet<u128>; 5],
    /// Raw country references that link to the viewed country, the routes its lines look up and
    /// its clients' handles, as the last product found them: what a changed row must touch to
    /// change the lines.
    viewed: HashSet<u32>,
    /// Whether CountryLinks' rows were held: without them every reference is unresolved.
    links_held: bool,
    referenced: HashSet<RouteKey>,
    serving: HashSet<u32>,
    worker: Option<(Sender<Job>, Receiver<Made>)>,
    /// Jobs sent and not answered yet.
    in_flight: usize,
    /// What the worker's stops and ribbons held at its last product.
    worker_bytes: usize,
    /// Steps applied since the last job was sent: a product that names its hour is old on arrival.
    moved: bool,
    /// Registered in the window: its vision is read from the vision layer before it.
    window: bool,
    /// Bumped by every product taken, for the window's merge of the ribbons.
    product_seq: u64,
    /// The newest hour of the mirrors the product stands for: made from them, or settled at them
    /// with nothing marked or out. While a job is out, what it publishes.
    product_hour: Option<Hour>,
    /// The viewed country's graphical culture, until its appearance or the viewed links change.
    culture: Option<Result<String, String>>,
    /// Rows a step changed, by table index: slots written or inserted, keys deleted.
    pending: Vec<(usize, Vec<u32>, Vec<u128>)>,
    stale: bool,
    /// Made whole by the worker and shared from there: the frame's window takes it by reference
    /// count and the scene reads that one.
    product: Arc<TradeRoutes>,
    statuses: HashMap<usize, Status>,
    /// The map mode draws no trade route: it declares, holds and carries nothing.
    hidden: bool,
    frame: u64,
    since: Option<u64>,
    /// Rows changed, products made, ribbons kept and made, and the products' microseconds, since
    /// the layer was made.
    marked: [usize; 5],
}

const NOT_DRAWN: &str = "no trade route is drawn in this map mode";

impl Convoys {
    pub fn new(routes: Arc<dyn Routes>) -> Self {
        Self {
            routes,
            given: Mutex::new(None),
            fresh_vision: AtomicBool::new(false),
            vision: None,
            columns: None,
            refused: Some("not composed yet".into()),
            origins: Rows::default(),
            clients: Rows::default(),
            trade: Rows::default(),
            regional: Rows::default(),
            exchanges: Rows::default(),
            links: Arc::default(),
            links_changed: false,
            provinces: Arc::default(),
            provinces_version: 0,
            behind: Default::default(),
            viewed: HashSet::new(),
            links_held: false,
            referenced: HashSet::new(),
            serving: HashSet::new(),
            worker: None,
            in_flight: 0,
            worker_bytes: 0,
            moved: false,
            window: false,
            product_seq: 0,
            product_hour: None,
            culture: None,
            pending: Vec::new(),
            stale: false,
            product: Arc::default(),
            statuses: HashMap::new(),
            hidden: false,
            frame: 0,
            since: None,
            marked: [0; 5],
        }
    }

    /// A layer of the same content, inputs and view, made from scratch.
    pub fn fresh(&self) -> Self {
        Self { columns: self.columns, hidden: self.hidden, vision: self.vision.clone(), stale: self.vision.is_some(), ..Self::new(Arc::clone(&self.routes)) }
    }

    /// What the viewer sees at the head: whose lines are drawn; the product is made again with it.
    pub fn set_vision(&self, choice: Choice, vision: Arc<Vision>) {
        *self.given.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((choice, vision));
        self.fresh_vision.store(true, Ordering::Release);
    }

    /// The perspective of the vision the product is made with.
    pub fn perspective(&self) -> Option<&Choice> {
        self.vision.as_ref().map(|(choice, _)| choice)
    }

    pub fn refused(&self) -> Option<&str> {
        self.refused.as_deref()
    }

    /// The map mode draws no trade route: today draws none there either.
    pub fn hidden(&self) -> bool {
        self.hidden
    }

    /// Hidden: everything it holds dropped, as a layer made anew holds it, its worker let go; the
    /// vision it was given, its place in the window and its counters kept.
    fn release(&mut self) {
        let empty = Self::new(Arc::clone(&self.routes));
        let kept = std::mem::replace(self, empty);
        (self.given, self.fresh_vision, self.vision, self.window) = (kept.given, kept.fresh_vision, kept.vision, kept.window);
        (self.product_seq, self.frame, self.marked) = (kept.product_seq, kept.frame, kept.marked);
        self.hidden = true;
        self.refused = Some(NOT_DRAWN.into());
    }

    /// Registered in the window: its vision is read from the vision layer before it.
    pub fn in_window(mut self) -> Self {
        self.window = true;
        self
    }

    /// Bumped by every product taken.
    pub fn product_seq(&self) -> u64 {
        self.product_seq
    }

    /// The lines, convoys and report at the head.
    pub fn product(&self) -> &TradeRoutes {
        &self.product
    }

    /// The product the frame's window takes: the same one, by reference count.
    pub fn product_shared(&self) -> Arc<TradeRoutes> {
        Arc::clone(&self.product)
    }

    /// Rows changed, products made, ribbons kept and made, and the products' microseconds.
    pub fn marked(&self) -> [usize; 5] {
        self.marked
    }

    fn columns(&self) -> Columns {
        self.columns.expect("the convoys layer is declared before it composes")
    }

    fn read_origin(&self, mirror: &Mirror, slot: u32) -> Origin {
        let c = self.columns();
        let key = mirror.key(slot).unwrap_or_default();
        Origin {
            country: key as u32,
            state: (key >> 32) as u32,
            list: mirror.number(c.origin_list, slot).value().unwrap_or_default(),
            ordinal: mirror.number(c.origin_ordinal, slot).value().unwrap_or_default(),
            bad: mirror.number(c.origin_bad, slot).value().unwrap_or_default(),
            convoys_assigned: mirror.number(c.origin_convoys, slot).value().unwrap_or_default(),
        }
    }

    fn read_client(&self, mirror: &Mirror, slot: u32) -> Client {
        let c = self.columns();
        let number = |column: Num<u32>| mirror.number(column, slot).value().unwrap_or_default();
        Client {
            handle: mirror.key(slot).unwrap_or_default() as u32,
            category: mirror.number(c.client_category, slot).value().unwrap_or_default(),
            owner: number(c.client_owner),
            other: number(c.client_other),
            route_class: mirror.number(c.client_route_class, slot).value().unwrap_or_default(),
            route_country: number(c.client_route_country),
            route_other: number(c.client_route_other),
            bad: mirror.number(c.client_bad, slot).value().unwrap_or_default(),
            convoys_assigned: mirror.number(c.client_convoys, slot).value().unwrap_or_default(),
            owner_ordinal: mirror.number(c.client_owner_ordinal, slot).value(),
            other_ordinal: mirror.number(c.client_other_ordinal, slot).value(),
        }
    }

    fn read_route(&self, mirror: &Mirror, slot: u32) -> Route {
        let c = self.columns();
        let key = mirror.key(slot).unwrap_or_default();
        let number = |column: Num<u32>| mirror.number(column, slot).value().unwrap_or_default();
        Route {
            country: (key >> 40) as u32,
            class: (key >> 32) as u8,
            other: key as u32,
            kind: mirror.number(c.route_kind, slot).value().unwrap_or_default(),
            from: number(c.route_from),
            to: number(c.route_to),
            path: path_of(mirror, c.route_path, slot),
            source_state: mirror.number(c.route_source, slot).value(),
            destination_state: mirror.number(c.route_destination, slot).value(),
        }
    }

    fn read_regional(&self, mirror: &Mirror, slot: u32) -> Regional {
        let key = mirror.key(slot).unwrap_or_default();
        Regional { country: (key >> 32) as u32, region: key as u32, efficiency: mirror.number(self.columns().efficiency, slot).value().unwrap_or_default() }
    }

    fn read_exchange(&self, mirror: &Mirror, slot: u32) -> ExchangeOrder {
        let c = self.columns();
        let number = |column: Num<u32>| mirror.number(column, slot).value().unwrap_or_default();
        ExchangeOrder { resource: number(c.exchange_resource), import_ordinal: number(c.exchange_import), export_ordinal: number(c.exchange_export) }
    }

    /// One row of `table` at `slot` (live) or `key` (deleted) taken again; whether anything changed.
    fn take_row(&mut self, mirrors: &Mirrors, table: TableId, slot: Option<u32>, key: u128) -> bool {
        let Some(mirror) = mirrors.get(table) else { return false };
        if let Some(at) = deferred_at(table) {
            self.behind[at].remove(&key);
        }
        let live = slot.filter(|&slot| mirror.live(slot));
        match (table, live) {
            (TableId::ResourceOrigins, Some(slot)) => self.origins.put(key, self.read_origin(mirror, slot)),
            (TableId::ResourceOrigins, None) => self.origins.remove(key),
            (TableId::ConvoyClients, Some(slot)) => self.clients.put(key, self.read_client(mirror, slot)),
            (TableId::ConvoyClients, None) => self.clients.remove(key),
            (TableId::TradeRoutes, Some(slot)) => self.trade.put(key, self.read_route(mirror, slot)),
            (TableId::TradeRoutes, None) => self.trade.remove(key),
            (TableId::RegionalConvoyEfficiency, Some(slot)) => self.regional.put(key, self.read_regional(mirror, slot)),
            (TableId::RegionalConvoyEfficiency, None) => self.regional.remove(key),
            (TableId::ResourceExchanges, Some(slot)) => self.exchanges.put(key, self.read_exchange(mirror, slot)),
            (TableId::ResourceExchanges, None) => self.exchanges.remove(key),
            (TableId::CountryLinks, Some(slot)) => {
                let canonical = mirror.number(self.columns().link_slot, slot).value();
                let changed = match canonical {
                    Some(canonical) => self.links.get(&(key as u32)) != Some(&canonical) && Arc::make_mut(&mut self.links).insert(key as u32, canonical) != Some(canonical),
                    None => self.links.contains_key(&(key as u32)) && Arc::make_mut(&mut self.links).remove(&(key as u32)).is_some(),
                };
                self.links_changed |= changed;
                changed
            }
            (TableId::CountryLinks, None) => {
                let changed = self.links.contains_key(&(key as u32)) && Arc::make_mut(&mut self.links).remove(&(key as u32)).is_some();
                self.links_changed |= changed;
                changed
            }
            (TableId::Provinces, live) => {
                let c = self.columns();
                let id = key as u32;
                let (state, port) = live.map_or((None, None), |slot| (mirror.number(c.province_state, slot).value(), mirror.number(c.province_port, slot).value().filter(|&port| port > 0)));
                let port = port.map(|port| port as u32);
                if self.provinces.state_of.get(&id).copied() == state && self.provinces.ports.get(&id).copied() == port {
                    return false;
                }
                let provinces = Arc::make_mut(&mut self.provinces);
                let state_changed = match state {
                    Some(state) => provinces.state_of.insert(id, state) != Some(state),
                    None => provinces.state_of.remove(&id).is_some(),
                };
                let port_changed = match port {
                    Some(port) => provinces.ports.insert(id, port) != Some(port),
                    None => provinces.ports.remove(&id).is_some(),
                };
                self.provinces_version += 1;
                state_changed || port_changed
            }
            (TableId::CountryAppearance, _) => {
                self.culture = None;
                true
            }
            _ => false,
        }
    }

    /// Whether a country reference may change the viewed country's lines: it links to it, or it
    /// links nowhere (a refusal the report counts).
    fn names(&self, country: u32) -> bool {
        country != 0 && (self.viewed.contains(&country) || !self.links_held || !self.links.contains_key(&country))
    }

    fn serves(&self, client: &Client) -> bool {
        !matches!(client.category, trade_routes::SUPPLY_TRANSFER | trade_routes::IMPORT | trade_routes::LEND_LEASE | trade_routes::MARKET)
            || self.names(client.owner)
            || self.names(client.other)
    }

    /// Whether the row of `table` at `key`, as held now, may change the viewed country's lines.
    fn touches(&self, table: TableId, key: u128) -> bool {
        match table {
            TableId::ResourceOrigins => self.names(key as u32),
            TableId::ConvoyClients => self.clients.get(key).is_some_and(|client| self.serves(client)),
            TableId::TradeRoutes => self.referenced.contains(&((key >> 40) as u32, (key >> 32) as u8, key as u32)),
            TableId::RegionalConvoyEfficiency => self.names((key >> 32) as u32),
            TableId::ResourceExchanges => self.serving.contains(&((key >> 32) as u32)),
            TableId::CountryAppearance => self.viewed.contains(&(key as u32)),
            _ => true,
        }
    }

    /// Every row read again from the mirrors.
    fn read_all(&mut self, mirrors: &Mirrors) {
        self.origins.clear();
        self.clients.clear();
        self.trade.clear();
        self.regional.clear();
        self.exchanges.clear();
        self.links = Arc::default();
        self.links_changed = true;
        self.provinces = Arc::default();
        self.provinces_version += 1;
        self.behind = Default::default();
        self.pending.clear();
        self.statuses = mirrors.iter().filter_map(|mirror| mirror.status().map(|status| (mirror.table().index(), status))).collect();
        for table in TABLES {
            let Some(mirror) = mirrors.get(table) else { continue };
            let mut rows: Vec<(u32, u128)> = mirror.rows().filter_map(|slot| Some((slot, mirror.key(slot)?))).collect();
            rows.sort_unstable_by_key(|&(_, key)| key);
            for (slot, key) in rows {
                self.take_row(mirrors, table, Some(slot), key);
            }
        }
        self.stale = true;
    }

    /// The rows behind in `tables` that the lines now read, taken from the mirrors.
    fn take_behind(&mut self, mirrors: &Mirrors, tables: &[TableId]) {
        for &table in tables {
            let (Some(at), Some(mirror)) = (deferred_at(table), mirrors.get(table)) else { continue };
            let keys: Vec<u128> = self.behind[at].iter().copied().collect();
            for key in keys {
                let slot = mirror.slot(key);
                let wanted = match slot {
                    Some(slot) => self.touches(table, key) || self.would_touch(table, mirror, slot),
                    None => true,
                };
                if wanted {
                    self.take_row(mirrors, table, slot, key);
                }
            }
        }
    }

    /// Whether the row at `slot` of `table`, as the mirror holds it now, may change the lines: a
    /// client by its new countries (every other row by key, [`Self::touches`]).
    fn would_touch(&self, table: TableId, mirror: &Mirror, slot: u32) -> bool {
        if table != TableId::ConvoyClients {
            return false;
        }
        let c = self.columns();
        let number = |column: Num<u32>| mirror.number(column, slot).value().unwrap_or_default();
        let category = mirror.number(c.client_category, slot).value().unwrap_or_default();
        !matches!(category, trade_routes::SUPPLY_TRANSFER | trade_routes::IMPORT | trade_routes::LEND_LEASE | trade_routes::MARKET) || self.names(number(c.client_owner)) || self.names(number(c.client_other))
    }

    /// The viewed country's graphical culture from its appearance row.
    fn read_culture(&self, mirrors: &Mirrors, countries: &Countries<'_>) -> Result<String, String> {
        let appearance = mirrors.get(TableId::CountryAppearance);
        incomplete(appearance, TableId::CountryAppearance)?;
        let appearance = appearance.expect("checked above");
        let row = appearance
            .rows()
            .filter_map(|slot| Some((appearance.key(slot)?, slot)))
            .filter(|&(key, _)| countries.viewed(key as u32) == Ok(true))
            .min()
            .ok_or("the viewed country has no CountryAppearance row")?;
        let bytes = match appearance.text(self.columns().culture, row.1) {
            Cell::Value(id) => appearance.string(id).map(<[u8]>::to_vec).unwrap_or_default(),
            _ => Vec::new(),
        };
        String::from_utf8(bytes).map_err(|_| "the viewed country's graphical culture is not text".to_owned())
    }

    /// Asks the worker for the product of the values: the rows that can change the viewed lines,
    /// whose lines, why a table cannot be read; rows behind that the lines now read are taken first.
    fn make(&mut self, mirrors: &Mirrors) {
        self.marked[1] += 1;
        let ours = self.vision.as_ref().map_or(0, |&(_, ours)| ours);
        let links = mirrors.get(TableId::CountryLinks);
        let held = links.is_some_and(|links| matches!(links.status(), Some(Status::Complete | Status::Truncated { .. })));
        let viewed: HashSet<u32> = if held { self.links.iter().filter(|&(_, &slot)| slot == ours).map(|(&raw, _)| raw).collect() } else { HashSet::new() };
        let renamed = held != self.links_held || viewed != self.viewed || std::mem::take(&mut self.links_changed);
        if viewed != self.viewed {
            self.culture = None;
        }
        self.links_held = held;
        self.viewed = viewed;
        if renamed {
            self.take_behind(mirrors, &[TableId::ResourceOrigins, TableId::ConvoyClients, TableId::RegionalConvoyEfficiency]);
        }
        // Only rows that can change the lines: the builder skips every other one without a word.
        let origins: Vec<Origin> = self.origins.values.iter().filter(|origin| self.names(origin.country)).cloned().collect();
        let clients: Vec<Client> = self.clients.values.iter().filter(|client| self.serves(client)).cloned().collect();
        let regional: Vec<Regional> = self.regional.values.iter().filter(|row| self.names(row.country)).cloned().collect();
        let referenced: HashSet<RouteKey> = origins.iter().map(|origin| (origin.country, trade_routes::ORIGIN_ROUTE_CLASS, origin.state)).chain(clients.iter().map(|client| (client.route_country, client.route_class, client.route_other))).collect();
        let serving: HashSet<u32> = clients.iter().map(|client| client.handle).collect();
        let routes_wanted: Vec<u128> = referenced.difference(&self.referenced).map(|&(country, class, other)| u128::from(country) << 40 | u128::from(class) << 32 | u128::from(other)).collect();
        let newly_serving = serving.difference(&self.serving).next().is_some();
        (self.referenced, self.serving) = (referenced, serving);
        if let (Some(at), Some(mirror)) = (deferred_at(TableId::TradeRoutes), mirrors.get(TableId::TradeRoutes)) {
            for key in routes_wanted {
                if self.behind[at].contains(&key) {
                    self.take_row(mirrors, TableId::TradeRoutes, mirror.slot(key), key);
                }
            }
        }
        if newly_serving {
            self.take_behind(mirrors, &[TableId::ResourceExchanges]);
        }
        let routes: Vec<Route> = self.trade.values.iter().filter(|route| self.referenced.contains(&(route.country, route.class, route.other))).cloned().collect();
        let tables = [TableId::ResourceOrigins, TableId::ConvoyClients, TableId::TradeRoutes, TableId::RegionalConvoyEfficiency].into_iter().try_for_each(|table| incomplete(mirrors.get(table), table)).map(|()| (origins, clients, routes, regional));
        let exchanges: BTreeMap<u32, ExchangeOrder> = match incomplete(mirrors.get(TableId::ResourceExchanges), TableId::ResourceExchanges) {
            Ok(()) => self.exchanges.keys.iter().zip(&self.exchanges.values).map(|(&key, &order)| ((key >> 32) as u32, order)).collect(),
            Err(_) => BTreeMap::new(),
        };
        let slots = if held { Arc::clone(&self.links) } else { Arc::default() };
        if self.culture.is_none() {
            let culture = self.read_culture(mirrors, &Countries::over(&slots, links_gap(links), ours));
            self.culture = Some(culture);
        }
        let job = Job {
            hour: TABLES.iter().find_map(|&table| mirrors.get(table).and_then(Mirror::head)),
            ours,
            tables,
            slots,
            gap: links_gap(links),
            exchanges,
            culture: self.culture.clone().expect("read above"),
            provinces: Arc::clone(&self.provinces),
            provinces_read: incomplete(mirrors.get(TableId::Provinces), TableId::Provinces),
            provinces_version: self.provinces_version,
        };
        if self.worker.is_none() {
            match Maker::spawn(Arc::clone(&self.routes)) {
                Ok(worker) => self.worker = Some(worker),
                Err(error) => {
                    self.refused = Some(format!("The convoys layer is refused: its worker cannot start: {error}"));
                    return;
                }
            }
        }
        if let Some((jobs, _)) = &self.worker
            && jobs.send(job).is_ok()
        {
            self.in_flight += 1;
            self.moved = false;
        }
    }

    /// Takes the products the worker made; `wait` waits for every job in flight.
    fn exchange(&mut self, wait: bool) {
        let Some((_, made)) = &self.worker else { return };
        while self.in_flight > 0 {
            let next = if wait { made.recv().ok() } else { made.try_recv().ok() };
            let Some(made) = next else { break };
            self.in_flight = self.in_flight.saturating_sub(made.jobs);
            self.product = Arc::new(made.product);
            self.product_hour = made.hour;
            self.product_seq += 1;
            self.marked[2] += made.reused;
            self.marked[3] += made.made;
            self.marked[4] += made.micros;
            self.worker_bytes = made.bytes;
        }
        // Today's words for a table that cannot be read name the hour.
        self.stale |= self.moved && self.product.report.unavailable.is_some();
    }

    fn settle_dirty(&mut self, mirrors: &Mirrors, wait: bool) {
        self.frame += 1;
        // A table back after a view that drew none returns as a reset, which composes the layer.
        if self.columns.is_none() || TABLES.iter().any(|&table| mirrors.get(table).is_none()) {
            return;
        }
        if self.fresh_vision.swap(false, Ordering::AcqRel) {
            self.vision = self.given.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take().map(|(choice, vision)| (choice, vision.ours()));
            self.stale = true;
        }
        if self.vision.is_none() {
            self.refused = Some("The convoys layer is refused: it is given no vision of the viewing country yet".into());
            return;
        }
        if self.refused.take().is_some() {
            self.read_all(mirrors);
        }
        // Nothing out and nothing marked: the product stands for the rows the mirrors are at.
        if self.in_flight == 0 && !self.stale && self.pending.is_empty() {
            self.product_hour = TABLES.iter().find_map(|&table| mirrors.get(table).and_then(Mirror::head)).or(self.product_hour);
        }
        for (table, slots, deleted) in std::mem::take(&mut self.pending) {
            let table = TableId::ALL[table];
            let Some(mirror) = mirrors.get(table) else { continue };
            let mut changed = false;
            let behind = deferred_at(table);
            for key in deleted {
                if mirror.slot(key).is_none() {
                    if self.touches(table, key) {
                        changed |= self.take_row(mirrors, table, None, key);
                    } else if let Some(at) = behind {
                        self.behind[at].insert(key);
                    }
                }
            }
            for slot in slots {
                let Some(key) = mirror.key(slot) else { continue };
                if self.touches(table, key) || self.would_touch(table, mirror, slot) {
                    changed |= self.take_row(mirrors, table, Some(slot), key);
                } else if let Some(at) = behind {
                    self.behind[at].insert(key);
                }
            }
            self.stale |= changed;
        }
        self.exchange(wait);
        if self.stale {
            self.stale = false;
            self.make(mirrors);
            self.exchange(wait);
        }
        self.since = self.stale.then(|| self.since.unwrap_or(self.frame));
    }

    /// The products of `self` and `other` compared: the convoys, the lines drawn and the report.
    pub fn differ(&self, other: &Self) -> (u64, Option<String>) {
        differ(&self.product, &other.product, self.refused(), other.refused())
    }
}

/// Two products compared: convoys, the drawn lines' count and the report; the first difference.
pub fn differ(ours: &TradeRoutes, theirs: &TradeRoutes, refused: Option<&str>, their_refusal: Option<&str>) -> (u64, Option<String>) {
    let mut differing = 0;
    let mut first = None;
    let mut note = |what: String| {
        differing += 1;
        first.get_or_insert(what);
    };
    if refused != their_refusal {
        note(format!("refused: {refused:?} against {their_refusal:?}"));
    }
    if ours.convoys != theirs.convoys {
        note(format!("convoys: {} lines against {}", ours.convoys.len(), theirs.convoys.len()));
    }
    if ours.report != theirs.report {
        note(format!("report: {:?} against {:?}", ours.report, theirs.report));
    }
    if ours.field.draws.len() != theirs.field.draws.len() || ours.field.trade_route_hovers != theirs.field.trade_route_hovers {
        note(format!("lines: {} draws against {}", ours.field.draws.len(), theirs.field.draws.len()));
    }
    (differing, first)
}

impl Layer for Convoys {
    fn name(&self) -> &'static str {
        "convoys"
    }

    fn placed(&mut self, _view: &crate::layers::View) -> Option<u64> {
        Some(0)
    }

    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        if self.window {
            inputs.after("vision");
        }
        if !visibility::of(&view.map_mode).trade_routes {
            self.release();
            return Ok(());
        }
        self.hidden = false;
        for table in TABLES {
            inputs.table(table, true)?;
        }
        self.columns = Some(Columns {
            origin_list: inputs.number(TableId::ResourceOrigins, "list")?,
            origin_bad: inputs.number(TableId::ResourceOrigins, "bad")?,
            origin_convoys: inputs.number(TableId::ResourceOrigins, "convoys_assigned")?,
            origin_ordinal: inputs.number(TableId::ResourceOrigins, "ordinal")?,
            client_category: inputs.number(TableId::ConvoyClients, "category")?,
            client_owner: inputs.number(TableId::ConvoyClients, "owner")?,
            client_other: inputs.number(TableId::ConvoyClients, "other")?,
            client_route_class: inputs.number(TableId::ConvoyClients, "route_class")?,
            client_route_country: inputs.number(TableId::ConvoyClients, "route_country")?,
            client_route_other: inputs.number(TableId::ConvoyClients, "route_other")?,
            client_bad: inputs.number(TableId::ConvoyClients, "bad")?,
            client_convoys: inputs.number(TableId::ConvoyClients, "convoys_assigned")?,
            client_owner_ordinal: inputs.number(TableId::ConvoyClients, "owner_ordinal")?,
            client_other_ordinal: inputs.number(TableId::ConvoyClients, "other_ordinal")?,
            route_kind: inputs.number(TableId::TradeRoutes, "kind")?,
            route_from: inputs.number(TableId::TradeRoutes, "from")?,
            route_to: inputs.number(TableId::TradeRoutes, "to")?,
            route_path: inputs.list(TableId::TradeRoutes, "path")?,
            route_source: inputs.number(TableId::TradeRoutes, "source_state")?,
            route_destination: inputs.number(TableId::TradeRoutes, "destination_state")?,
            efficiency: inputs.number(TableId::RegionalConvoyEfficiency, "efficiency")?,
            exchange_resource: inputs.number(TableId::ResourceExchanges, "resource")?,
            exchange_import: inputs.number(TableId::ResourceExchanges, "import_ordinal")?,
            exchange_export: inputs.number(TableId::ResourceExchanges, "export_ordinal")?,
            link_slot: inputs.number(TableId::CountryLinks, "canonical_slot")?,
            culture: inputs.text(TableId::CountryAppearance, "graphical_culture")?,
            province_state: inputs.number(TableId::Provinces, "state")?,
            province_port: inputs.number(TableId::Provinces, "port_sea")?,
        });
        Ok(())
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        if self.hidden {
            self.release();
            return Ok(());
        }
        if self.columns.is_none() {
            return Ok(());
        }
        let marked = self.marked;
        self.refused = Some("composing".into());
        self.settle_dirty(mirrors, true);
        self.marked = marked;
        Ok(())
    }

    fn apply(&mut self, step: &Step, mirrors: &Mirrors) {
        if step.gap() || self.columns.is_none() {
            return;
        }
        let c = self.columns();
        let read: [&[usize]; 8] = [
            &[c.origin_list.column, c.origin_bad.column, c.origin_convoys.column, c.origin_ordinal.column],
            &[
                c.client_category.column,
                c.client_owner.column,
                c.client_other.column,
                c.client_route_class.column,
                c.client_route_country.column,
                c.client_route_other.column,
                c.client_bad.column,
                c.client_convoys.column,
                c.client_owner_ordinal.column,
                c.client_other_ordinal.column,
            ],
            &[c.route_kind.column, c.route_from.column, c.route_to.column, c.route_path.column, c.route_source.column, c.route_destination.column],
            &[c.efficiency.column],
            &[c.exchange_resource.column, c.exchange_import.column, c.exchange_export.column],
            &[c.link_slot.column],
            &[c.culture.column],
            &[c.province_state.column, c.province_port.column],
        ];
        for changes in step.tables() {
            let Some(at) = TABLES.iter().position(|table| table.index() == changes.table()) else { continue };
            if mirrors.get(TABLES[at]).is_none() {
                continue;
            }
            if changes.status().is_some_and(|(status, _)| self.statuses.insert(changes.table(), status) != Some(status)) {
                self.stale = true;
            }
            self.mark(changes, read[at]);
        }
        self.moved = true;
        self.stale |= self.product.report.unavailable.is_some();
    }

    fn settle(&mut self, mirrors: &Mirrors, settling: Settling) -> io::Result<()> {
        self.settle_dirty(mirrors, settling.waits());
        Ok(())
    }

    fn settle_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>, settling: Settling) -> io::Result<()> {
        // Only whose lines or plan are drawn: the viewing country, taken when it changes.
        if self.window
            && let Some(vision) = upstream.get::<crate::layers::vision::Vision>()
            && let Some(ours) = vision.ours()
            && self.vision.as_ref().is_none_or(|(choice, held)| *held != ours || choice != vision.choice())
        {
            // Held at once: a mode that draws no convoys settles nothing, so a given vision would wait.
            self.vision = Some((vision.choice().clone(), ours));
            self.stale |= self.columns.is_some();
        }
        self.settle_dirty(mirrors, settling.waits());
        Ok(())
    }


    fn carried(&self) -> Carried {
        // A job out is the worker's: counted here it froze the shown hour while playing.
        let entities = self.pending.iter().map(|(_, slots, deleted)| slots.len() + deleted.len()).sum::<usize>() + usize::from(self.stale);
        Carried { entities }
    }

    /// Current once no job is out; while one is, the newest hour its product stands for, or
    /// nothing before its first product.
    fn published(&self) -> Published {
        if self.in_flight == 0 {
            return Published::Current;
        }
        self.product_hour.map_or(Published::Nothing, Published::At)
    }

    fn held(&self) -> usize {
        let paths: usize = self.trade.values.iter().map(|route| route.path.capacity() * 4).sum();
        let behind: usize = self.behind.iter().map(|keys| keys.capacity() * 24).sum();
        self.origins.bytes() + self.clients.bytes() + self.trade.bytes() + paths + self.regional.bytes() + self.exchanges.bytes() + self.links.len() * 24 + (self.provinces.ports.len() + self.provinces.state_of.len()) * 24 + behind + self.worker_bytes
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Convoys {
    /// Records the rows `changes` touched in the columns `read`, to take again at the settle.
    fn mark(&mut self, changes: &Changes, read: &[usize]) {
        let deleted: Vec<u128> = changes.deleted().keys().unwrap_or_default().to_vec();
        let mut slots: Vec<u32> = changes.inserted().slots().to_vec();
        for written in changes.columns().iter().filter(|written| read.contains(&written.column())) {
            slots.extend_from_slice(written.slots());
        }
        if deleted.is_empty() && slots.is_empty() {
            return;
        }
        self.marked[0] += deleted.len() + slots.len();
        self.since.get_or_insert(self.frame);
        self.pending.push((changes.table(), slots, deleted));
    }
}

#[cfg(test)]
mod tests;
