//! BATCH 0 deliverable 1 + 4 + 6 — the VIEW REGISTRY: a stable per-view descriptor
//! (id, gui ROOT/TEMPLATE ref, required domains, factory fn, menu route), the
//! template (row/card) factory API, and the menubar inventory.
//!
//! Codex (b): register GUI SYMBOLS (ROOTS + repeated TEMPLATES), never whole files —
//! one `.gui` holds N window roots + M repeated row/card templates. A `ViewDescriptor`
//! names a single ROOT (or is fully custom); a `TemplateDescriptor` names a single
//! repeated TEMPLATE used by the row factory.
//!
//! Codex (d): registration is DISTRIBUTED via the `inventory` crate. Each view/cluster
//! module `inventory::submit!`s its descriptors IN ITS OWN FILE; the registry collects
//! them at startup with NO central list anyone edits. Two cluster agents can add views
//! concurrently and never touch a shared file — the conflict seam is gone.

use std::collections::HashMap;

use crate::host::context::BindContext;
use crate::host::view::{HostPolicy, ViewInstance};

/// A stable, interned view id (the descriptor key + the menubar route key).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ViewId(pub &'static str);

impl ViewId {
    pub const fn new(s: &'static str) -> ViewId {
        ViewId(s)
    }
    pub fn as_str(&self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for ViewId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

/// Whether a named `.gui` symbol is a top-level WINDOW ROOT or a repeated row/card
/// TEMPLATE (one file holds several of each — register the symbol, not the file).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiSymbolKind {
    Root,
    Template,
}

/// A reference to ONE named symbol inside a parsed `.gui` file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuiSymbolRef {
    /// The `.gui` file NAME (e.g. `"province_state.json"`), resolved via `Stage`.
    pub file: &'static str,
    /// The window-root or template `name` inside that file.
    pub name: &'static str,
    pub kind: GuiSymbolKind,
}

impl GuiSymbolRef {
    pub const fn root(file: &'static str, name: &'static str) -> GuiSymbolRef {
        GuiSymbolRef {
            file,
            name,
            kind: GuiSymbolKind::Root,
        }
    }
    pub const fn template(file: &'static str, name: &'static str) -> GuiSymbolRef {
        GuiSymbolRef {
            file,
            name,
            kind: GuiSymbolKind::Template,
        }
    }
}

/// Where a view appears in the menubar (the open surface, deliverable 6). A view with
/// `route: None` is reachable by drill/command only (e.g. the state panel opens on a
/// province pick, not from the menu).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuRoute {
    /// The top-level menu CLUSTER this view lives under (the 10 cluster groups + the
    /// migrated surfaces' own group).
    pub cluster: MenuCluster,
    /// The menu item label loc key OR literal (kept literal here to avoid a loc dep in
    /// the descriptor; the menubar resolves it).
    pub label: &'static str,
    /// Sort order within the cluster (lower = earlier).
    pub order: i32,
}

/// The menubar's top-level clusters — the 10 BATCH-2 view clusters (2a–2j) plus the
/// `Core` group for the 5 already-migrated surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MenuCluster {
    /// The 5 migrated surfaces (map/topbar/state/focus/combat).
    Core,
    PoliticsGovernment,
    ProductionConstructionDecisions,
    ResearchTechDoctrine,
    MilitaryManagement,
    Designers,
    LeadersOfficersTraits,
    IntelDiplomacyTrade,
    LogisticsCombatUnitsEvents,
    ChromeOutlinerMaps,
    ScriptedGuiOverlays,
}

impl MenuCluster {
    /// A stable display label for the cluster (the menubar top-level button text).
    pub fn label(&self) -> &'static str {
        match self {
            MenuCluster::Core => "Core",
            MenuCluster::PoliticsGovernment => "Politics",
            MenuCluster::ProductionConstructionDecisions => "Production",
            MenuCluster::ResearchTechDoctrine => "Research",
            MenuCluster::MilitaryManagement => "Military",
            MenuCluster::Designers => "Designers",
            MenuCluster::LeadersOfficersTraits => "Leaders",
            MenuCluster::IntelDiplomacyTrade => "Intel",
            MenuCluster::LogisticsCombatUnitsEvents => "Logistics",
            MenuCluster::ChromeOutlinerMaps => "Chrome",
            MenuCluster::ScriptedGuiOverlays => "Overlays",
        }
    }
    /// All clusters in menubar order (for building the bar).
    pub const ALL: [MenuCluster; 11] = [
        MenuCluster::Core,
        MenuCluster::PoliticsGovernment,
        MenuCluster::ProductionConstructionDecisions,
        MenuCluster::ResearchTechDoctrine,
        MenuCluster::MilitaryManagement,
        MenuCluster::Designers,
        MenuCluster::LeadersOfficersTraits,
        MenuCluster::IntelDiplomacyTrade,
        MenuCluster::LogisticsCombatUnitsEvents,
        MenuCluster::ChromeOutlinerMaps,
        MenuCluster::ScriptedGuiOverlays,
    ];
}

/// A factory fn that OPENS a view instance: given an optional integer argument (e.g. a
/// state id), it builds the persistent [`ViewInstance`]. The factory does NOT bind —
/// the host calls `instance.bind(ctx)` each tick. A custom overlay (focus/combat)
/// returns its own `ViewInstance` impl.
pub type OpenFn = fn(arg: Option<i64>) -> Box<dyn ViewInstance>;

/// THE per-view descriptor (deliverable 1). Immutable static data registered via
/// `inventory`. Note: no builder *closure* here — the descriptor is a `'static`
/// value, so the builder is a fn pointer (`OpenFn`); per-instance state is created by
/// the factory, satisfying "a builder must not mutate app globals".
pub struct ViewDescriptor {
    pub id: ViewId,
    /// The view's primary `.gui` ROOT (None = a fully custom overlay with no `.gui`).
    pub gui_root: Option<GuiSymbolRef>,
    /// The snapshot domains this view reads (so the host can warn if absent + so the
    /// per-view accounting test knows what to expect).
    pub required_domains: &'static [&'static str],
    /// The menubar route (None = drill/command-opened only).
    pub route: Option<MenuRoute>,
    /// An OPTIONAL global hotkey that toggles this view open/closed. A cluster view can
    /// bind a key here and the binary's generic shortcut dispatch toggles it WITHOUT a
    /// `main.rs` edit (Codex (b)/(d): keeps shortcuts off the shared file). `None` =
    /// no global shortcut.
    pub shortcut: Option<Shortcut>,
    /// The factory that opens an instance.
    pub open: OpenFn,
    /// The host policy (z-band / modal / rebind).
    pub policy: HostPolicy,
}

/// A global keyboard shortcut a view descriptor can bind (descriptor-driven, so a new
/// cluster view never edits `main.rs` to get a hotkey). The binary maps a winit key to
/// this and toggles the view generically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    /// A single letter A–Z (case-insensitive), e.g. `Shortcut::Letter('P')`.
    Letter(char),
}

/// THE template (row/card) descriptor (deliverable 4). A repeated `.gui` TEMPLATE +
/// the snapshot domains it reads. The actual row instantiation is the row-factory API
/// on [`ViewRegistry`]; this descriptor just makes templates first-class + discoverable
/// (so a cluster registers its row templates in its own file).
pub struct TemplateDescriptor {
    pub id: ViewId,
    pub gui_template: GuiSymbolRef,
    pub required_domains: &'static [&'static str],
}

// `inventory` distributed collection — each module submits into these slices in ITS
// OWN file; no central registration list exists to conflict on.
inventory::collect!(ViewDescriptor);
inventory::collect!(TemplateDescriptor);

/// Convenience macro for a cluster/view file to register a view in ONE place (its own
/// file). Expands to an `inventory::submit!`. Kept thin so descriptors stay plain data.
#[macro_export]
macro_rules! register_view {
    ($desc:expr) => {
        ::inventory::submit! { $desc }
    };
}

#[macro_export]
macro_rules! register_template {
    ($desc:expr) => {
        ::inventory::submit! { $desc }
    };
}

/// The assembled, queryable registry: descriptors indexed by id + grouped by menu
/// cluster. Built once at startup from the `inventory` collection (plus any explicitly
/// registered descriptors, for tests).
pub struct ViewRegistry {
    views: HashMap<ViewId, &'static ViewDescriptor>,
    templates: HashMap<ViewId, &'static TemplateDescriptor>,
    /// view ids in stable registration order (for deterministic iteration).
    order: Vec<ViewId>,
}

impl ViewRegistry {
    /// Build the registry from the `inventory`-collected descriptors. This is the ONLY
    /// construction the binary uses; adding a view never touches this function.
    pub fn from_inventory() -> ViewRegistry {
        let mut reg = ViewRegistry {
            views: HashMap::new(),
            templates: HashMap::new(),
            order: Vec::new(),
        };
        // DETECT a silent duplicate ViewId/template-id across the (distributed,
        // conflict-free) `inventory` submissions BEFORE it is swallowed by the
        // last-wins HashMap insert below (Codex H4): two cluster files submitting the
        // same id would otherwise shadow one descriptor with NO error, and every
        // per-cluster `view(id).is_some()` test would still pass against the WRONG
        // descriptor. We surface it loudly here (the single place the whole inventory
        // is assembled) rather than fail the binary, so a real campaign still loads.
        for (id, dups) in Self::duplicate_ids_in_inventory() {
            eprintln!(
                "[registry] WARNING: ViewId {id:?} registered {dups} times across clusters \
                 (last-wins; one descriptor is shadowed) — give it a unique id."
            );
        }
        for d in inventory::iter::<ViewDescriptor> {
            reg.insert_view(d);
        }
        for t in inventory::iter::<TemplateDescriptor> {
            reg.templates.insert(t.id, t);
        }
        // stable, id-sorted order so the menubar + tests are deterministic regardless
        // of inventory link order.
        reg.order.sort_by(|a, b| a.0.cmp(b.0));
        reg
    }

    /// The view ids OR template ids submitted to `inventory` MORE THAN ONCE WITHIN THEIR
    /// OWN namespace, each with its submission count (Codex H4 — the cross-cluster
    /// integrity guard). An empty result means every distributed registration is unique.
    /// This counts the RAW submissions (before the last-wins HashMap dedup), so it is the
    /// only way to observe a silent id collision between two cluster files. Views and
    /// templates are counted SEPARATELY (they live in distinct HashMaps, so a view and a
    /// template sharing an id do NOT shadow each other — counting them together would be
    /// a false positive, Codex follow-up).
    pub fn duplicate_ids_in_inventory() -> Vec<(&'static str, usize)> {
        fn dups_of<'a>(it: impl Iterator<Item = &'a str>) -> Vec<(&'a str, usize)> {
            let mut counts: std::collections::BTreeMap<&'a str, usize> =
                std::collections::BTreeMap::new();
            for id in it {
                *counts.entry(id).or_insert(0) += 1;
            }
            counts.into_iter().filter(|&(_, n)| n > 1).collect()
        }
        let mut out = dups_of(
            inventory::iter::<ViewDescriptor>
                .into_iter()
                .map(|d| d.id.0),
        );
        out.extend(dups_of(
            inventory::iter::<TemplateDescriptor>
                .into_iter()
                .map(|t| t.id.0),
        ));
        out
    }

    /// An EMPTY registry (for unit tests that register a handful of descriptors).
    pub fn empty() -> ViewRegistry {
        ViewRegistry {
            views: HashMap::new(),
            templates: HashMap::new(),
            order: Vec::new(),
        }
    }

    /// Register one descriptor explicitly (used by tests + by `from_inventory`). The
    /// iteration order is kept id-SORTED so `iter_views`/`menubar` are deterministic
    /// regardless of construction path (inventory link order is unspecified).
    pub fn insert_view(&mut self, d: &'static ViewDescriptor) {
        if self.views.insert(d.id, d).is_none() {
            self.order.push(d.id);
            self.order.sort_by(|a, b| a.0.cmp(b.0));
        }
    }

    pub fn insert_template(&mut self, t: &'static TemplateDescriptor) {
        self.templates.insert(t.id, t);
    }

    /// Look up a view descriptor by id.
    pub fn view(&self, id: ViewId) -> Option<&'static ViewDescriptor> {
        self.views.get(&id).copied()
    }

    /// Look up a template descriptor by id.
    pub fn template(&self, id: ViewId) -> Option<&'static TemplateDescriptor> {
        self.templates.get(&id).copied()
    }

    /// Iterate ALL view descriptors in stable id order.
    pub fn iter_views(&self) -> impl Iterator<Item = &'static ViewDescriptor> + '_ {
        self.order
            .iter()
            .filter_map(move |id| self.views.get(id).copied())
    }

    /// The view count (register/iterate test).
    pub fn len(&self) -> usize {
        self.views.len()
    }
    pub fn is_empty(&self) -> bool {
        self.views.is_empty()
    }
    pub fn template_count(&self) -> usize {
        self.templates.len()
    }

    /// Open an instance for `id` (the factory builds its persistent state). Returns
    /// `None` if the id is unregistered (never fabricates a view).
    pub fn open(&self, id: ViewId, arg: Option<i64>) -> Option<Box<dyn ViewInstance>> {
        let d = self.view(id)?;
        Some((d.open)(arg))
    }

    /// The view bound to a global letter shortcut (case-insensitive), if any. The binary
    /// uses this to toggle a view from a keypress WITHOUT a per-view branch — so a
    /// cluster view gets a hotkey purely from its descriptor (Codex (b)/(d)).
    pub fn shortcut_view(&self, letter: char) -> Option<ViewId> {
        let up = letter.to_ascii_uppercase();
        self.iter_views().find_map(|d| match d.shortcut {
            Some(Shortcut::Letter(c)) if c.to_ascii_uppercase() == up => Some(d.id),
            _ => None,
        })
    }

    /// The DISTINCT `.gui` FILE NAMES every registered view ROOT + template TEMPLATE
    /// references (deduped, sorted). The binary uses this to load EXACTLY the parsed
    /// `.gui` set the registered views need into `Stage.gui_files` — so a cluster fetches
    /// its real game layout by symbol (via `BindContext::view_root` / `gui_symbol`)
    /// instead of building synthetic chrome, and NO cluster ever loads a `.gui` itself
    /// (the conflict-free seam). Views with a custom (None) `gui_root` contribute nothing.
    pub fn required_gui_files(&self) -> Vec<&'static str> {
        let mut set: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
        for d in self.iter_views() {
            if let Some(sym) = &d.gui_root {
                set.insert(sym.file);
            }
        }
        for t in self.templates.values() {
            set.insert(t.gui_template.file);
        }
        set.into_iter().collect()
    }

    /// The menubar model: every routed view grouped by cluster, each cluster's items
    /// sorted by `order` then label. A view with `route: None` is excluded (it is not
    /// menu-reachable — it opens by drill/command).
    pub fn menubar(&self) -> Vec<MenuClusterItems> {
        let mut out: Vec<MenuClusterItems> = MenuCluster::ALL
            .iter()
            .map(|c| MenuClusterItems {
                cluster: *c,
                items: Vec::new(),
            })
            .collect();
        for d in self.iter_views() {
            if let Some(route) = d.route {
                if let Some(group) = out.iter_mut().find(|g| g.cluster == route.cluster) {
                    group.items.push(MenuItem {
                        view_id: d.id,
                        label: route.label,
                        order: route.order,
                    });
                }
            }
        }
        for g in &mut out {
            g.items
                .sort_by(|a, b| a.order.cmp(&b.order).then(a.label.cmp(b.label)));
        }
        out
    }
}

/// One cluster's menubar items.
pub struct MenuClusterItems {
    pub cluster: MenuCluster,
    pub items: Vec<MenuItem>,
}

/// One menubar item (a routed view).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuItem {
    pub view_id: ViewId,
    pub label: &'static str,
    pub order: i32,
}

// ===========================================================================
// The ROW / TEMPLATE FACTORY API (deliverable 4).
// ===========================================================================

/// One instantiated row from a template: a laid-out scene-graph node (already
/// positioned by the row factory) + the fidelity tally for the row's bound fields.
/// The caller draws the node via the normal node path (no per-view branch).
pub struct TemplateRow {
    pub node: gui_runtime::scene::Node,
}

/// The row-factory: given a registered TEMPLATE id, a viewport rect, and a per-row
/// binder closure, instantiate N laid-out rows. This surfaces `gui_runtime::gridbox`
/// through the registry so a template-instantiated view (deliverable 4) never re-wires
/// the gridbox plumbing. The binder is the no-fabrication seam: it receives the row
/// node + the row index and binds via `bind::apply` / `DisplayAtom`.
pub struct RowFactory<'a> {
    template: gui_runtime::scene::Node,
    viewport: gui_runtime::Rect,
    spec: gui_runtime::gridbox::GridSpec,
    /// the manifest used to build the per-call `ManifestContent` (it is generic over the
    /// measure closure, so we cannot store the content directly).
    manifest: &'a SpriteManifest,
}

use assets::SpriteManifest;

impl<'a> RowFactory<'a> {
    /// Build a row factory for a registered template against a viewport (a gridbox
    /// region in a laid-out scene). `ctx` resolves the template's `.gui` file.
    pub fn new(
        ctx: &'a BindContext<'a>,
        template_id: ViewId,
        registry: &ViewRegistry,
        viewport: gui_runtime::Rect,
        gutter: f32,
    ) -> Option<RowFactory<'a>> {
        let t = registry.template(template_id)?;
        let file = ctx.stage.gui_file(t.gui_template.file)?;
        let raw = file
            .windows
            .iter()
            .find(|w| w.name.as_deref() == Some(t.gui_template.name))?;
        let loc = gui_runtime::Loc::new();
        let template = gui_runtime::scene::Node::from_raw(raw, &ctx.stage.manifest, &loc);
        let viewport_stub = stub_gridbox(t.gui_template.name, viewport);
        let spec = gui_runtime::gridbox::GridSpec::from_gridbox(&viewport_stub, &template, gutter);
        Some(RowFactory {
            template,
            viewport,
            spec,
            manifest: &ctx.stage.manifest,
        })
    }

    /// Instantiate `n` rows, calling `bind_row(index, &mut node)` for each so the
    /// caller fills the row's text/icons (via the no-fabrication binder). Returns the
    /// laid-out rows + the optional scrollbar geometry.
    pub fn instantiate(
        &self,
        n: usize,
        mut bind_row: impl FnMut(usize, &mut gui_runtime::scene::Node),
    ) -> (Vec<TemplateRow>, Option<gui_runtime::gridbox::ScrollBar>) {
        let content = gui_runtime::ManifestContent {
            manifest: self.manifest,
            measure: |_t, _f| None,
        };
        let mut inst = gui_runtime::gridbox::instantiate(
            &self.template,
            n,
            self.viewport,
            &self.spec,
            0.0,
            &content,
        );
        let mut rows = Vec::with_capacity(inst.rows.len());
        for (i, row) in inst.rows.iter_mut().enumerate() {
            bind_row(i, row);
        }
        for row in inst.rows.into_iter() {
            rows.push(TemplateRow { node: row });
        }
        (rows, inst.scrollbar)
    }
}

/// A stub gridbox container node carrying the viewport rect (the row factory needs a
/// `gridBoxType` node to derive the grid spec; we don't have the real one in scope).
fn stub_gridbox(name: &str, rect: gui_runtime::Rect) -> gui_runtime::scene::Node {
    gui_runtime::scene::Node {
        name: name.to_string(),
        raw_kind: "gridBoxType".into(),
        tooltip: None,
        kind: gui_runtime::scene::ElementKind::Container { background: None },
        pos: (0.0, 0.0),
        size_w: None,
        size_h: None,
        orientation: gui_runtime::Anchor::UpperLeft,
        origo: gui_runtime::Anchor::UpperLeft,
        clipping: false,
        alwaystransparent: false,
        rect,
        clip: None,
        children: vec![],
    }
}
