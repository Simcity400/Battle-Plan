//! CORE SURFACE 2 (migrated) — the TOPBAR, a Chrome-band full-screen pure-frame
//! window. `bind` rebuilds the scene-graph fresh from the `.gui` ROOT, runs
//! `bind::apply_topbar` (the no-fabrication topbar spec), lays it out, and returns a
//! `Drawable` that draws the nodes + the player flag sprite (resolved by tag+ideology,
//! the Codex §4 cache-key concern). Byte-identical to the old topbar path: the topbar
//! does NOT block the map (its empty regions fall through), only its real controls
//! capture (`HitModel` over the laid-out roots).

use bind::{apply_topbar, BindReport, FlagResolver, TopbarSpec};
use gui_runtime::scene::Node;
use gui_runtime::{build_scene, layout, ManifestContent, Rect};

use crate::host::context::BindContext;
use crate::host::draw::FrameDraw;
use crate::host::registry::{ViewDescriptor, ViewId};
use crate::host::view::{
    BoundView, Drawable, HitModel, HostPolicy, InstanceId, ViewCommand, ViewInstance, ZBand,
};
use crate::host::FidelityReport;
use crate::register_view;

pub const TOPBAR_VIEW_ID: ViewId = ViewId("core.topbar");
const TOPBAR_FILE: &str = "topbar.json";

/// The topbar view instance — stateless across ticks (no scroll/camera). It carries
/// only the observed country tag, which the binary sets at open.
pub struct TopbarView {
    pub country: String,
}

impl TopbarView {
    pub fn new(country: impl Into<String>) -> TopbarView {
        TopbarView {
            country: country.into(),
        }
    }
}

/// The topbar's `Drawable`: the laid-out nodes + the player flag sprite.
pub struct TopbarDrawable {
    roots: Vec<Node>,
    /// (rect, tag, ideology) for the flag sprite (resolved per-frame by tag+ideology).
    flag: Option<(Rect, String, Option<String>)>,
}

impl Drawable for TopbarDrawable {
    /// The topbar's extra (non-node) pixel input is the FLAG sprite identity, which
    /// depends on TAG + ruling-party ideology (the Codex §4 cache-key trap). Digesting
    /// it here means a stale flag carried across a scrub would change the digest — the
    /// node digest alone (which has no flag) would miss it.
    fn draw_digest(&self) -> String {
        match &self.flag {
            Some((rect, tag, ideo)) => format!(
                "TOPBAR_FLAG {tag}/{ideo:?}@{:.1},{:.1},{:.1},{:.1}",
                rect.x, rect.y, rect.w, rect.h
            ),
            None => "TOPBAR_FLAG none".into(),
        }
    }

    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        if let Some((rect, tag, ideology)) = &self.flag {
            let flag_res = FlagResolver::new(&fd.stage.resolver);
            if let Some(rf) = flag_res.resolve(tag, ideology.as_deref()) {
                if let Ok(tex) = fd.renderer.load_image(&rf.path) {
                    fd.renderer
                        .draw_sprite(&tex, rect.x, rect.y, rect.w, rect.h);
                }
            }
        }
    }
}

impl ViewInstance for TopbarView {
    fn view_id(&self) -> ViewId {
        TOPBAR_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Chrome)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let stage = ctx.stage;
        let screen = ctx.screen;
        let cm = ManifestContent {
            manifest: &stage.manifest,
            measure: |_t, _f| None,
        };

        // fresh scene-graph each tick (no stale rows across a scrub).
        let mut roots: Vec<Node> = match stage.gui_file(TOPBAR_FILE) {
            Some(f) => build_scene(f, &stage.manifest, &stage.loc),
            None => Vec::new(),
        };
        let spec = TopbarSpec::topbar();
        let report: BindReport = apply_topbar(&mut roots, &spec, ctx.snapshot(), &self.country);
        for root in roots.iter_mut() {
            layout(root, screen, None, &cm);
        }

        let flag = flag_rect(&roots, &report, &self.country);
        let fidelity = topbar_fidelity(&report);

        BoundView {
            instance: InstanceId(0),
            view_id: TOPBAR_VIEW_ID,
            policy: self.policy(),
            // the topbar is a pure full-screen frame: NO block rect — its empty regions
            // fall through to the map; only its real controls capture (via the roots).
            hit: HitModel::from_roots(roots.clone()),
            drawable: Box::new(TopbarDrawable { roots, flag }),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// The player-flag rect + (tag, ideology) from the bound report (mirrors the old
/// `topbar_flag_rect`).
fn flag_rect(
    roots: &[Node],
    report: &BindReport,
    country: &str,
) -> Option<(Rect, String, Option<String>)> {
    let rect = find_rect(roots, "player_flag")?;
    Some((rect, country.to_string(), report.flag_ideology.clone()))
}

/// Tally the topbar's bound fields into a fidelity report (so the per-view accounting
/// is visible). The `apply_topbar` report's `not_captured` flag = an unavailable badge.
fn topbar_fidelity(report: &BindReport) -> FidelityReport {
    let mut f = FidelityReport::default();
    for field in &report.fields {
        if field.not_captured {
            f.unavailable += 1;
        } else {
            match field.fidelity {
                bind::Fidelity::Exact => f.exact += 1,
                bind::Fidelity::Derived => f.derived += 1,
                bind::Fidelity::Unavailable => f.unavailable += 1,
            }
        }
    }
    f
}

fn find_rect(roots: &[Node], name: &str) -> Option<Rect> {
    fn walk(n: &Node, name: &str) -> Option<Rect> {
        if n.name == name && n.rect.w > 0.0 && n.rect.h > 0.0 {
            return Some(n.rect);
        }
        for c in &n.children {
            if let Some(r) = walk(c, name) {
                return Some(r);
            }
        }
        None
    }
    roots.iter().find_map(|r| walk(r, name))
}

/// Seeded by the binary with the observed country; not menu-opened (it is always on).
fn open(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(TopbarView::new("GER"))
}

register_view!(ViewDescriptor {
    id: TOPBAR_VIEW_ID,
    gui_root: Some(crate::host::registry::GuiSymbolRef::root(
        TOPBAR_FILE,
        "topbar"
    )),
    required_domains: &["countries"],
    route: None, // always-on chrome, seeded at startup
    shortcut: None,
    open,
    policy: HostPolicy::window(ZBand::Chrome),
});

pub fn link() {}
