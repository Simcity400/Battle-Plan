//! BATCH 0 — the HOST SUBSYSTEM: the view registry, the generic window/modal host,
//! the bind/render contexts, the no-fabrication value types, and the row/template
//! factory. Every future view cluster plugs in HERE (its own module + an `inventory`
//! registration) without editing a shared file.
//!
//! Module map:
//!   * [`context`]      — `Stage`, `BindContext`, `RenderContext`, `Captured`/`DisplayAtom`.
//!   * [`view`]         — `ViewInstance` / `BoundView` / `Drawable` traits + `HostPolicy`.
//!   * [`registry`]     — `ViewRegistry` + `ViewDescriptor`/`TemplateDescriptor` + menubar + row factory.
//!   * [`window_host`]  — the generic open-view stack, z-order, modal, hit + open/close routing.
//!   * [`draw`]         — the per-frame `FrameDraw` immediate-mode bundle.
//!   * [`input`]        — backend-agnostic `ViewInput` events.

pub mod context;
pub mod draw;
pub mod input;
pub mod registry;
pub mod view;
pub mod window_host;

pub use context::{BindContext, Captured, DisplayAtom, FidelityReport, Stage, UnavailableReason};
pub use draw::{FrameDraw, RenderContext};
pub use input::{PointerButton, ViewInput, ViewKey};
pub use registry::{
    GuiSymbolKind, GuiSymbolRef, MenuCluster, MenuClusterItems, MenuItem, MenuRoute, RowFactory,
    Shortcut, TemplateDescriptor, TemplateRow, ViewDescriptor, ViewId, ViewRegistry,
};
pub use view::{
    BoundView, Drawable, HitModel, HostPolicy, InputOutcome, InstanceId, NodeDrawable,
    RebindContext, ViewCommand, ViewInstance, ZBand,
};
pub use window_host::{HostHit, RouteResult, WindowHost};

#[cfg(test)]
mod tests {
    use super::*;
    use bind::Fidelity;
    use gui_runtime::scene::{Anchor, ElementKind, Node};
    use gui_runtime::Rect;

    // ---- test doubles: a trivial view instance backed by a single node ----

    fn node(name: &str, kind: ElementKind, rect: Rect, transparent: bool) -> Node {
        Node {
            name: name.into(),
            raw_kind: "containerWindowType".into(),
            kind,
            pos: (0.0, 0.0),
            size_w: None,
            size_h: None,
            orientation: Anchor::UpperLeft,
            origo: Anchor::UpperLeft,
            clipping: false,
            alwaystransparent: transparent,
            rect,
            clip: None,
            children: vec![],
        }
    }

    struct TestView {
        id: ViewId,
        policy: HostPolicy,
        roots: Vec<Node>,
        block_rect: Option<Rect>,
    }

    impl ViewInstance for TestView {
        fn view_id(&self) -> ViewId {
            self.id
        }
        fn policy(&self) -> HostPolicy {
            self.policy
        }
        fn bind(&self, _ctx: &BindContext) -> BoundView {
            let mut hit = HitModel::from_roots(self.roots.clone());
            hit.block_rect = self.block_rect;
            BoundView {
                instance: InstanceId(0), // host overwrites via open; tests set per call
                view_id: self.id,
                policy: self.policy,
                hit,
                drawable: Box::new(NodeDrawable {
                    roots: self.roots.clone(),
                }),
                fidelity: FidelityReport::default(),
                commands: Vec::new(),
            }
        }
        crate::impl_view_any!();
    }

    // ---- a bind context backed by a tiny empty snapshot ----
    fn empty_stage() -> Stage {
        Stage {
            resolver: assets::resolver::GameResolver::new(None, None),
            manifest: assets::SpriteManifest {
                sprites: std::collections::HashMap::new(),
            },
            loc: gui_runtime::Loc::new(),
            gui_files: std::collections::HashMap::new(),
            map_dir: std::path::PathBuf::from("."),
            assets_dir: std::path::PathBuf::from("."),
        }
    }

    fn ctx<'a>(stage: &'a Stage, snap: &'a snapshot::Snapshot) -> BindContext<'a> {
        BindContext {
            stage,
            snapshot: snap,
            country: "GER",
            screen: Rect {
                x: 0.0,
                y: 0.0,
                w: 1600.0,
                h: 900.0,
            },
            date: "",
            cursor: (0.0, 0.0),
            map_camera: None,
        }
    }

    fn bound_for(host_open: &[(InstanceId, &TestView)]) -> Vec<BoundView> {
        host_open
            .iter()
            .map(|(iid, v)| {
                let mut hit = HitModel::from_roots(v.roots.clone());
                hit.block_rect = v.block_rect;
                BoundView {
                    instance: *iid,
                    view_id: v.id,
                    policy: v.policy,
                    hit,
                    drawable: Box::new(NodeDrawable {
                        roots: v.roots.clone(),
                    }),
                    fidelity: FidelityReport::default(),
                    commands: Vec::new(),
                }
            })
            .collect()
    }

    // =====================================================================
    // No-fabrication: a blocked field badges `unavailable`, never a 0.
    // =====================================================================
    #[test]
    fn blocked_field_badges_unavailable_not_zero() {
        // an ABSENT capture must render "—", not "0".
        let absent: Captured<i64> = Captured::Absent;
        let atom = absent.display_with(|v| v.to_string());
        assert!(atom.is_unavailable());
        assert_eq!(atom.as_str(), "—");
        assert_eq!(atom.fidelity(), Fidelity::Unavailable);

        // a NULL capture -> also "—", but with the NULL reason (distinct from Absent).
        let null: Captured<i64> = Captured::Null;
        let null_atom = null.display_with(|v| v.to_string());
        assert_eq!(null_atom.as_str(), "—");
        assert_eq!(null_atom.reason(), Some(UnavailableReason::Null));
        assert_eq!(atom.reason(), Some(UnavailableReason::Absent));

        // a MEASURED 0 is a REAL value -> "0" with its badge (never confused with the
        // not-captured glyph).
        let measured: Captured<i64> = Captured::Measured(0);
        let atom0 = measured.display_with(|v| v.to_string());
        assert!(!atom0.is_unavailable());
        assert_eq!(atom0.as_str(), "0");
        assert_eq!(atom0.fidelity(), Fidelity::Exact);
    }

    #[test]
    fn classify_value_trichotomy() {
        use serde_json::json;
        let obj = json!({"a": 5, "b": null});
        assert!(matches!(
            context::classify_value(obj.get("a")),
            Captured::Measured(_)
        ));
        assert!(matches!(
            context::classify_value(obj.get("b")),
            Captured::Null
        ));
        assert!(matches!(
            context::classify_value(obj.get("missing")),
            Captured::Absent
        ));
    }

    #[test]
    fn fidelity_report_worst_badge() {
        let mut r = FidelityReport::default();
        r.record(&DisplayAtom::text("x", Fidelity::Exact));
        assert_eq!(r.worst_badge(), "exact");
        r.record(&DisplayAtom::text("y", Fidelity::Derived));
        assert_eq!(r.worst_badge(), "derived");
        r.record(&DisplayAtom::unavailable());
        assert_eq!(r.worst_badge(), "unavailable");
        assert_eq!(r.total(), 3);
    }

    // =====================================================================
    // Registry: register / lookup / iterate.
    // =====================================================================
    fn test_open(_arg: Option<i64>) -> Box<dyn ViewInstance> {
        Box::new(TestView {
            id: ViewId("test.dummy"),
            policy: HostPolicy::window(ZBand::Panel),
            roots: vec![],
            block_rect: None,
        })
    }

    static D1: ViewDescriptor = ViewDescriptor {
        id: ViewId("z.alpha"),
        gui_root: None,
        required_domains: &["countries"],
        route: Some(MenuRoute {
            cluster: MenuCluster::PoliticsGovernment,
            label: "Alpha",
            order: 1,
        }),
        shortcut: Some(registry::Shortcut::Letter('P')),
        open: test_open,
        policy: HostPolicy::window(ZBand::Panel),
    };
    static D2: ViewDescriptor = ViewDescriptor {
        id: ViewId("a.beta"),
        gui_root: Some(GuiSymbolRef::root("x.json", "root_b")),
        required_domains: &[],
        route: None,
        shortcut: None,
        open: test_open,
        policy: HostPolicy::modal_overlay(ZBand::Overlay),
    };

    #[test]
    fn registry_register_lookup_iterate() {
        let mut reg = ViewRegistry::empty();
        reg.insert_view(&D1);
        reg.insert_view(&D2);
        assert_eq!(reg.len(), 2);
        assert_eq!(reg.view(ViewId("z.alpha")).unwrap().id, ViewId("z.alpha"));
        assert!(reg.view(ViewId("nope")).is_none());
        // iteration is id-sorted -> "a.beta" before "z.alpha".
        let ids: Vec<&str> = reg.iter_views().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, vec!["a.beta", "z.alpha"]);
        // menubar: only the routed view (D1) appears, in its cluster.
        let bar = reg.menubar();
        let politics = bar
            .iter()
            .find(|g| g.cluster == MenuCluster::PoliticsGovernment)
            .unwrap();
        assert_eq!(politics.items.len(), 1);
        assert_eq!(politics.items[0].view_id, ViewId("z.alpha"));
        // D2 has route None -> not menu-reachable anywhere.
        assert!(bar
            .iter()
            .all(|g| g.items.iter().all(|i| i.view_id != ViewId("a.beta"))));
    }

    #[test]
    fn registry_shortcut_lookup_is_descriptor_driven() {
        // a view binds a hotkey purely in its descriptor; the registry resolves the key
        // with NO per-view branch in the binary (Codex (b)/(d) — cluster hotkeys add no
        // main.rs edit).
        let mut reg = ViewRegistry::empty();
        reg.insert_view(&D1);
        reg.insert_view(&D2);
        assert_eq!(reg.shortcut_view('p'), Some(ViewId("z.alpha"))); // case-insensitive
        assert_eq!(reg.shortcut_view('P'), Some(ViewId("z.alpha")));
        assert_eq!(reg.shortcut_view('Z'), None); // no view bound 'Z'
    }

    #[test]
    fn registry_insert_dedups_order_on_duplicate_id() {
        // inserting the SAME id twice must keep `order` to a single entry (no dup
        // iteration) while last-wins on the descriptor — the silent-shadow shape the
        // cross-cluster integrity guard watches for (Codex H4). The registry-level
        // `duplicate_ids_in_inventory` surfaces it loudly against the real inventory.
        let mut reg = ViewRegistry::empty();
        reg.insert_view(&D1);
        reg.insert_view(&D1); // same id again
        assert_eq!(reg.len(), 1, "duplicate id collapses to one view");
        let ids: Vec<&str> = reg.iter_views().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, vec!["z.alpha"], "order has no duplicate entry");
    }

    #[test]
    fn real_inventory_has_no_duplicate_view_or_template_ids() {
        // the cross-cluster integrity assertion at the REGISTRY layer: every
        // distributed `inventory` registration across all clusters is unique. A
        // collision would silently shadow a descriptor (last-wins HashMap) yet pass
        // per-cluster `view(id).is_some()` tests, so this is the guard that catches it.
        let dups = ViewRegistry::duplicate_ids_in_inventory();
        assert!(
            dups.is_empty(),
            "duplicate view/template ids across clusters: {dups:?}"
        );
    }

    #[test]
    fn registry_open_builds_instance() {
        let mut reg = ViewRegistry::empty();
        reg.insert_view(&D1);
        let inst = reg.open(ViewId("z.alpha"), Some(7)).unwrap();
        assert_eq!(inst.view_id(), ViewId("test.dummy"));
        assert!(reg.open(ViewId("missing"), None).is_none());
    }

    // =====================================================================
    // Window host: open / close / z-order / modal.
    // =====================================================================
    fn panel_view(id: &'static str, rect: Rect, block: bool) -> TestView {
        let root = node(
            "bg",
            ElementKind::Container {
                background: Some("p".into()),
            },
            rect,
            false,
        );
        TestView {
            id: ViewId(id),
            policy: HostPolicy::window(ZBand::Panel),
            roots: vec![root],
            block_rect: block.then_some(rect),
        }
    }

    #[test]
    fn host_open_close_singleton() {
        let mut host = WindowHost::new();
        let v = panel_view(
            "state_panel",
            Rect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 100.0,
            },
            false,
        );
        host.open_instance(Box::new(TestView { ..clone_view(&v) }));
        assert_eq!(host.open_count(), 1);
        // re-open same id -> still ONE (singleton replace).
        host.open_instance(Box::new(TestView { ..clone_view(&v) }));
        assert_eq!(host.open_count(), 1);
        host.close_view(ViewId("state_panel"));
        assert_eq!(host.open_count(), 0);
    }

    fn clone_view(v: &TestView) -> TestView {
        TestView {
            id: v.id,
            policy: v.policy,
            roots: v.roots.clone(),
            block_rect: v.block_rect,
        }
    }

    #[test]
    fn host_z_order_overlay_above_panel() {
        let mut host = WindowHost::new();
        // open a panel, then a higher-band overlay.
        host.open_instance(Box::new(panel_view(
            "panel",
            Rect {
                x: 0.0,
                y: 0.0,
                w: 800.0,
                h: 800.0,
            },
            true,
        )));
        let overlay = TestView {
            id: ViewId("overlay"),
            policy: HostPolicy::modal_overlay(ZBand::Overlay),
            roots: vec![node(
                "ov",
                ElementKind::Container {
                    background: Some("o".into()),
                },
                Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 1600.0,
                    h: 900.0,
                },
                false,
            )],
            block_rect: None,
        };
        host.open_instance(Box::new(overlay));
        // front-most closeable should be the overlay (higher z).
        assert_eq!(host.front_view(), Some(ViewId("overlay")));
        assert_eq!(host.front_closeable(), Some(ViewId("overlay")));
    }

    #[test]
    fn host_modal_captures_everywhere() {
        let mut host = WindowHost::new();
        let panel = panel_view(
            "panel",
            Rect {
                x: 0.0,
                y: 0.0,
                w: 200.0,
                h: 200.0,
            },
            true,
        );
        let panel_iid = host.open_instance(Box::new(clone_view(&panel)));
        let modal = TestView {
            id: ViewId("modal"),
            policy: HostPolicy::modal_overlay(ZBand::Overlay),
            roots: vec![], // no leaves: a modal with no control still consumes everywhere
            block_rect: None,
        };
        let modal_iid = host.open_instance(Box::new(modal));

        let bound = bound_for(&[
            (panel_iid, &panel),
            (
                modal_iid,
                &TestView {
                    id: ViewId("modal"),
                    policy: HostPolicy::modal_overlay(ZBand::Overlay),
                    roots: vec![],
                    block_rect: None,
                },
            ),
        ]);
        // a click anywhere is consumed by the modal (Blocked, since it has no leaf),
        // NOT routed to the panel, NOT a map miss.
        match host.resolve_click(&bound, 50.0, 50.0) {
            HostHit::Blocked { view_id, .. } => assert_eq!(view_id, ViewId("modal")),
            other => panic!("modal must capture everywhere, got {other:?}"),
        }
    }

    #[test]
    fn host_block_rect_consumes_interior_no_map_pick() {
        let mut host = WindowHost::new();
        let panel = panel_view(
            "panel",
            Rect {
                x: 40.0,
                y: 40.0,
                w: 100.0,
                h: 100.0,
            },
            true,
        );
        let pure = TestView {
            // a pure container (no capturing leaf) but WITH a block rect.
            id: ViewId("pure"),
            policy: HostPolicy::window(ZBand::Panel),
            roots: vec![node(
                "empty",
                ElementKind::Container { background: None },
                Rect {
                    x: 200.0,
                    y: 200.0,
                    w: 100.0,
                    h: 100.0,
                },
                false,
            )],
            block_rect: Some(Rect {
                x: 200.0,
                y: 200.0,
                w: 100.0,
                h: 100.0,
            }),
        };
        let p_iid = host.open_instance(Box::new(clone_view(&panel)));
        let pure_iid = host.open_instance(Box::new(clone_view(&pure)));
        let bound = bound_for(&[(p_iid, &panel), (pure_iid, &pure)]);

        // click the pure panel's interior (no leaf) -> Blocked (consumed), not a Miss.
        match host.resolve_click(&bound, 250.0, 250.0) {
            HostHit::Blocked { view_id, .. } => assert_eq!(view_id, ViewId("pure")),
            other => panic!("pure-container interior must be Blocked, got {other:?}"),
        }
        // click the opaque panel -> Captured by it.
        match host.resolve_click(&bound, 60.0, 60.0) {
            HostHit::Captured { view_id, .. } => assert_eq!(view_id, ViewId("panel")),
            other => panic!("opaque panel must capture, got {other:?}"),
        }
        // click empty space outside both -> Miss (the app then does a map pick).
        assert_eq!(host.resolve_click(&bound, 900.0, 700.0), HostHit::Miss);
    }

    #[test]
    fn host_execute_open_close_commands() {
        let mut reg = ViewRegistry::empty();
        reg.insert_view(&D1);
        let mut host = WindowHost::new();
        // OpenView command opens the registered view.
        host.execute_commands(
            &reg,
            None,
            [ViewCommand::OpenView {
                view_id: ViewId("z.alpha"),
                arg: None,
            }],
        );
        assert!(host.is_open(ViewId("test.dummy")) || host.open_count() == 1);
        // CloseView closes by id.
        let open_id = host.front_view().unwrap();
        host.execute_commands(&reg, None, [ViewCommand::CloseView { view_id: open_id }]);
        assert_eq!(host.open_count(), 0);
    }

    // =====================================================================
    // route_input: pointer-capture drag stream — no double dispatch (Codex H3),
    // and capture is NOT honoured for non-drag events like Escape (Codex H2-adj).
    // =====================================================================

    use std::cell::Cell;
    use std::rc::Rc;

    /// A view that records how many times `on_input` was called, consumes a
    /// `PointerDown` (to grab the drag capture) but IGNORES `PointerMove`/`PointerUp`
    /// (the realistic focus/tech-tree case the double-dispatch bug fired on), and
    /// records whether it ever saw an Escape `Key`.
    struct DragView {
        id: ViewId,
        policy: HostPolicy,
        calls: Rc<Cell<u32>>,
        saw_escape: Rc<Cell<bool>>,
        consume_down: bool,
    }
    impl ViewInstance for DragView {
        fn view_id(&self) -> ViewId {
            self.id
        }
        fn policy(&self) -> HostPolicy {
            self.policy
        }
        fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
            self.calls.set(self.calls.get() + 1);
            match ev {
                ViewInput::PointerDown { .. } if self.consume_down => InputOutcome::consumed(),
                ViewInput::Key {
                    key: ViewKey::Escape,
                } => {
                    self.saw_escape.set(true);
                    InputOutcome::ignored()
                }
                // moves/ups are NOT consumed (the real overlays often ignore them).
                _ => InputOutcome::ignored(),
            }
        }
        fn bind(&self, _ctx: &BindContext) -> BoundView {
            BoundView {
                instance: InstanceId(0),
                view_id: self.id,
                policy: self.policy,
                hit: HitModel::none(),
                drawable: Box::new(NodeDrawable { roots: vec![] }),
                fidelity: FidelityReport::default(),
                commands: Vec::new(),
            }
        }
        crate::impl_view_any!();
    }

    #[test]
    fn route_input_captured_drag_is_not_double_dispatched() {
        let mut host = WindowHost::new();
        let calls = Rc::new(Cell::new(0));
        let v = DragView {
            id: ViewId("drag"),
            policy: HostPolicy::window(ZBand::Panel),
            calls: calls.clone(),
            saw_escape: Rc::new(Cell::new(false)),
            consume_down: true,
        };
        host.open_instance(Box::new(v));
        // PointerDown -> consumed once, grabs capture.
        host.route_input(&ViewInput::PointerDown {
            x: 5.0,
            y: 5.0,
            button: PointerButton::Left,
        });
        assert_eq!(calls.get(), 1, "down dispatched once");
        // A PointerMove the captured view does NOT consume must be dispatched EXACTLY
        // ONCE (to the capture owner), not twice (capture path + top-down loop).
        calls.set(0);
        host.route_input(&ViewInput::PointerMove {
            x: 6.0,
            y: 6.0,
            drag: true,
        });
        assert_eq!(
            calls.get(),
            1,
            "captured move dispatched exactly once (no double-dispatch)"
        );
        // PointerUp also exactly once, and it releases the capture.
        calls.set(0);
        host.route_input(&ViewInput::PointerUp {
            x: 7.0,
            y: 7.0,
            button: PointerButton::Left,
        });
        assert_eq!(calls.get(), 1, "captured up dispatched exactly once");
    }

    #[test]
    fn pointer_capture_does_not_divert_escape_key() {
        // a non-modal view grabs a drag capture; a MODAL then opens; a subsequent
        // Escape Key must NOT be diverted to the captured non-modal view (the modal
        // sees it first / the captured view never sees the Escape).
        let mut host = WindowHost::new();
        let saw_escape = Rc::new(Cell::new(false));
        let lower = DragView {
            id: ViewId("lower"),
            policy: HostPolicy::window(ZBand::Panel),
            calls: Rc::new(Cell::new(0)),
            saw_escape: saw_escape.clone(),
            consume_down: true,
        };
        host.open_instance(Box::new(lower));
        // lower grabs the capture on a consumed PointerDown.
        host.route_input(&ViewInput::PointerDown {
            x: 1.0,
            y: 1.0,
            button: PointerButton::Left,
        });
        // a modal overlay opens on top (e.g. focus tree) — it consumes everything.
        let modal = DragView {
            id: ViewId("modal"),
            policy: HostPolicy::modal_overlay(ZBand::Overlay),
            calls: Rc::new(Cell::new(0)),
            saw_escape: Rc::new(Cell::new(false)),
            consume_down: false,
        };
        host.open_instance(Box::new(modal));
        // an Escape arrives: the captured LOWER view must not receive it.
        host.route_input(&ViewInput::Key {
            key: ViewKey::Escape,
        });
        assert!(
            !saw_escape.get(),
            "captured non-modal view must NOT be diverted the Escape key"
        );
    }

    // a stub snapshot for the ctx (the contexts compile against a real Snapshot).
    #[test]
    fn bind_context_reads_trichotomy() {
        let stage = empty_stage();
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1936.1.1.12",
            "countries": { "GER": { "politics": { "stability": 0.5, "war_support": null } } }
        }))
        .unwrap();
        let c = ctx(&stage, &snap);
        assert!(c.country_field("GER", "politics.stability").is_measured());
        assert!(matches!(
            c.country_field("GER", "politics.war_support"),
            Captured::Null
        ));
        assert!(matches!(
            c.country_field("GER", "politics.missing"),
            Captured::Absent
        ));
    }

    // =====================================================================
    // HOST-DRIVEN scrub integration (Criterion-4): MULTIPLE registered windows
    // opened in a real `WindowHost`, the full host loop (`rebind_all` then
    // `bind_all`) driving them across a date scrub A→B→A — NOT `view.bind`
    // directly — must produce byte-identical A==A output for EVERY open window.
    // This proves the host loop drives all open windows deterministically.
    // =====================================================================

    use crate::host::view::RebindContext;
    use crate::views::digest::bound_digest;

    /// The real `game_assets/gui` dir (sibling of `replay_native`), or None if the
    /// (gitignored) extracted assets are absent — the test then skips cleanly.
    fn host_gui_dir() -> Option<std::path::PathBuf> {
        let d = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("game_assets")
            .join("gui");
        d.join("diplomacy.json").exists().then_some(d)
    }

    /// A Stage that pre-loads EXACTLY the `.gui` files the registered views need
    /// (via `ViewRegistry::required_gui_files`) + carries the real `assets_dir`,
    /// so each opened real view resolves its actual game layout (no synthetic chrome).
    fn host_stage(reg: &ViewRegistry, gui_dir: &std::path::Path) -> Stage {
        let needed = reg.required_gui_files();
        let (gui_files, _failed) = Stage::load_gui_set(gui_dir, &needed);
        // The `.gui` set this test's 5 windows actually render MUST have loaded (the registry
        // also lists scripted-gui `.gui` SOURCE files that are not extracted as files — those
        // legitimately land in `_failed` and are not asserted; the views under test are).
        for f in [
            "diplomacy.json",
            "naval_combat.json",
            "combat_log.json",
            "logistics.json",
            "war_overview.json",
        ] {
            assert!(
                gui_files.contains_key(f),
                "the host scrub needs a real `{f}` layout to prove anything"
            );
        }
        Stage {
            resolver: assets::resolver::GameResolver::new(None, None),
            manifest: assets::SpriteManifest {
                sprites: std::collections::HashMap::new(),
            },
            loc: gui_runtime::Loc::new(),
            gui_files,
            map_dir: std::path::PathBuf::from("."),
            assets_dir: gui_dir.parent().unwrap().to_path_buf(),
        }
    }

    fn host_ctx<'a>(
        stage: &'a Stage,
        snap: &'a snapshot::Snapshot,
        date: &'a str,
    ) -> BindContext<'a> {
        BindContext {
            stage,
            snapshot: snap,
            country: "GER",
            screen: Rect {
                x: 0.0,
                y: 0.0,
                w: 1600.0,
                h: 900.0,
            },
            date,
            cursor: (0.0, 0.0),
            map_camera: None,
        }
    }

    /// Bind EVERY open window THROUGH the host loop (rebind_all → bind_all) and return a
    /// per-window digest keyed by the STABLE (instance_id, view_id) the host stamped on
    /// open — so an A→A comparison is window-for-window, not order-dependent.
    fn host_render(
        host: &mut WindowHost,
        stage: &Stage,
        snap: &snapshot::Snapshot,
        date: &str,
    ) -> Vec<((InstanceId, ViewId), String)> {
        // 1) the per-tick hour refresh on every open window (no-op for plain windows).
        host.rebind_all(&RebindContext {
            snapshot: snap,
            country: "GER",
            screen: (1600, 900),
            refit: false,
        });
        // 2) the pure per-frame bind of every open window — through the HOST, which stamps
        //    each BoundView with the instance id minted at open (NOT a per-bind id).
        let ctx = host_ctx(stage, snap, date);
        let mut out: Vec<((InstanceId, ViewId), String)> = host
            .bind_all(&ctx)
            .iter()
            .map(|bv| ((bv.instance, bv.view_id), bound_digest(bv)))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    #[test]
    fn host_drives_multiple_windows_scrub_a_b_a_byte_identical() {
        let gui_dir = match host_gui_dir() {
            Some(d) => d,
            None => {
                eprintln!("[host] skipped scrub-integration: game_assets/gui not extracted");
                return;
            }
        };
        let reg = ViewRegistry::from_inventory();
        let stage = host_stage(&reg, &gui_dir);

        // OPEN multiple registered windows THROUGH the registry (the menubar/command path),
        // incl. the 3 newly-registered views — so the proof drives real, registered windows.
        // CAPTURE the instance id the host MINTS at open, so we can prove the host stamps THAT
        // id on the bound view (not a per-bind id) — Codex: the (instance,view) key must be the
        // open-minted one for the A==A comparison to be a real host-loop proof.
        let mut host = WindowHost::new();
        let opened: Vec<(ViewId, InstanceId)> = [
            ViewId("intel.diplomacy"),
            ViewId("logistics.naval_combat"),
            ViewId("logistics.combat_log"),
            ViewId("logistics.logistics"),
            ViewId("intel.war_overview"),
        ]
        .into_iter()
        .map(|id| {
            let iid = host
                .open_view(&reg, id, None)
                .expect("registered view must open through the host");
            (id, iid)
        })
        .collect();
        assert_eq!(host.open_count(), 5, "five registered windows are open");

        // Two distinct hours so the host loop demonstrably MOVES between A and B.
        let snap_a: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1941.4.13.11", "hour_index": 100,
            "countries": { "GER": {
                "politics": { "name": "German Reich", "ruling_party": "fascism", "stability": 0.72, "war_support": 0.6, "faction": {"name": "axis"} },
                "focus": { "current": "GER_rhineland" },
                "diplomacy": { "opinion": { "GER": {"opinion_sum": 0} }, "guarantees": ["POL"] },
                "economy": { "resources": {"produced": {"steel": 1196.0}}, "fuel": {"current": 5000.0, "max": 10000.0}, "factories": {"mil": 131, "dockyard": 16} }
            } },
            "divisions": [],
            "combats": [ { "combat_id": 1, "kind": "land", "location_province": 5 } ],
            "diplomacy_world": { "wars": [], "factions": [], "subjects": [] }
        })).unwrap();
        let snap_b: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1941.5.1.11", "hour_index": 200,
            "countries": { "GER": {
                "politics": { "name": "German Reich", "ruling_party": "fascism", "stability": 0.9, "war_support": 0.8, "faction": {"name": "axis"} },
                "focus": { "current": "GER_four_year_plan" },
                "diplomacy": { "opinion": { "GER": {"opinion_sum": 0} } },
                "economy": { "resources": {"produced": {"steel": 900.0}}, "fuel": {"current": 9000.0, "max": 10000.0}, "factories": {"mil": 140, "dockyard": 18} }
            } },
            "divisions": [],
            "combats": [ { "combat_id": 9, "kind": "naval", "location_province": 4,
                           "attackers": [ { "owner": "GER" } ], "defenders": [ { "owner": "ENG" } ] } ],
            "diplomacy_world": { "wars": [], "factions": [], "subjects": [] }
        })).unwrap();

        // SCRUB A → B → A, ALWAYS through the host loop (same host, same open instances).
        let a1 = host_render(&mut host, &stage, &snap_a, "1941.4.13.11");
        let b = host_render(&mut host, &stage, &snap_b, "1941.5.1.11");
        let a2 = host_render(&mut host, &stage, &snap_a, "1941.4.13.11");

        // The host stamped each bound view with the OPEN-MINTED instance id (not a per-bind
        // id): the bound keys equal exactly the (view_id, instance_id) pairs `open_view`
        // returned — so the A==A comparison is keyed on real host-tracked instances.
        let mut expected: Vec<(InstanceId, ViewId)> =
            opened.iter().map(|(v, i)| (*i, *v)).collect();
        expected.sort();
        let bound_keys: Vec<(InstanceId, ViewId)> = a1.iter().map(|(k, _)| *k).collect();
        assert_eq!(
            bound_keys, expected,
            "the host did not stamp the open-minted instance ids on bind_all"
        );
        // the open-minted ids are DISTINCT (the host hands out unique instance ids), so the
        // (instance,view) key cannot be a regenerated-from-order artifact (Codex provenance
        // hardening): a bind path that re-derived a colliding id would fail the equality above.
        let mut iids: Vec<InstanceId> = opened.iter().map(|(_, i)| *i).collect();
        iids.sort();
        iids.dedup();
        assert_eq!(
            iids.len(),
            opened.len(),
            "the host must mint DISTINCT instance ids per open window"
        );

        // the host kept the SAME open instances across the whole scrub (no churn).
        let keys_a2: Vec<_> = a2.iter().map(|(k, _)| *k).collect();
        assert_eq!(
            bound_keys, keys_a2,
            "the host's open instances changed across the scrub"
        );
        assert_eq!(a1.len(), 5, "all 5 windows were host-bound at A");

        // Per-VIEW the 3 NEWLY-REGISTERED windows must each MOVE between A and B (not rely on
        // an aggregate `a1 != b`, which one unrelated window could satisfy) — proving the host
        // loop actually re-derives each new view at the B hour.
        let by_view = |rendered: &[((InstanceId, ViewId), String)], v: &str| -> String {
            rendered
                .iter()
                .find(|((_, vid), _)| vid.as_str() == v)
                .map(|(_, d)| d.clone())
                .unwrap()
        };
        for v in [
            "intel.diplomacy",
            "logistics.naval_combat",
            "logistics.combat_log",
        ] {
            assert_ne!(
                by_view(&a1, v),
                by_view(&b, v),
                "the new view {v} must move A→B through the host loop"
            );
        }

        // THE PROOF: every open window's host-driven render is byte-identical A==A —
        // window-for-window by the host's stable (instance_id, view_id) key.
        for ((k1, d1), (k2, d2)) in a1.iter().zip(a2.iter()) {
            assert_eq!(k1, k2, "window identity drifted across the host scrub");
            assert_eq!(
                d1, d2,
                "host-driven window {:?} was NOT byte-identical A→B→A",
                k1.1
            );
        }
        assert_eq!(
            a1, a2,
            "the host loop did not drive every open window deterministically A→B→A"
        );
    }
}
