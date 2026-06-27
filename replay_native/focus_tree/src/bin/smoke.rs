//! Quick smoke check: load the real focus catalog (mod over vanilla), place the
//! GER `german_focus` tree, bind it against a real snapshot, and print a summary.
//! Read-only. Paths via CLI/env/documented defaults.

use std::path::PathBuf;

use assets::resolver::GameResolver;
use focus_tree::{
    bind_node, eval_context, load_catalog, place, FocusDomain, NodeState, PosFidelity,
};
use snapshot::Store;

fn arg(flag: &str) -> Option<String> {
    let mut it = std::env::args();
    while let Some(a) = it.next() {
        if a == flag {
            return it.next();
        }
    }
    None
}

fn repo_root() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn main() {
    let vanilla = arg("--vanilla");
    let mod_root = arg("--mod").or_else(|| {
        // default to the AMSB test mod next to the repo if present
        let p = repo_root().parent()?.join("test");
        if p.join("common").join("national_focus").is_dir() {
            Some(p.to_string_lossy().into_owned())
        } else {
            None
        }
    });
    let resolver = GameResolver::new(vanilla.as_deref(), mod_root.as_deref());
    eprintln!("[smoke] vanilla = {}", resolver.vanilla.display());
    if let Some(m) = &resolver.mod_root {
        eprintln!("[smoke] mod = {}", m.display());
    }

    let cat = load_catalog(&resolver).expect("load catalog");
    eprintln!(
        "[smoke] trees: {}  shared: {}",
        cat.trees.len(),
        cat.shared.len()
    );

    let tree_id = arg("--tree").unwrap_or_else(|| "german_focus".to_string());
    let tree = cat.tree(&tree_id).unwrap_or_else(|| {
        eprintln!("[smoke] tree `{tree_id}` not found. Available:");
        for t in &cat.trees {
            eprintln!("    {} ({} nodes)", t.id, t.nodes.len());
        }
        std::process::exit(1);
    });
    eprintln!("[smoke] tree `{}` has {} nodes", tree.id, tree.nodes.len());

    // Bind against a real snapshot if available.
    let store_root = arg("--store")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("replay").join("snapshots"));
    let game = arg("--game").unwrap_or_else(|| "campaign".to_string());
    let tag = arg("--country").unwrap_or_else(|| "GER".to_string());

    let store = Store::open(&store_root);
    let mut domain = FocusDomain::default();
    if let Ok(idx) = store.index(&game) {
        if let Some(first) = idx.snapshots.iter().find_map(|e| e.hour_index) {
            if let Ok(snap) = store.load(&game, first) {
                if let Some(c) = snap.country(&tag) {
                    domain = FocusDomain::from_country(c);
                    eprintln!(
                        "[smoke] snapshot {} {}: completed={} current={:?} progress={:?}",
                        game,
                        snap.date.clone().unwrap_or_default(),
                        domain.completed.len(),
                        domain.current,
                        domain.progress
                    );
                }
            }
        }
    }

    let ctx = eval_context(&tag, &domain, None);
    let placed = place(&tree, &ctx);
    let (minx, miny, maxx, maxy) = placed.bounds();
    eprintln!(
        "[smoke] placed bounds px: ({minx:.0},{miny:.0})..({maxx:.0},{maxy:.0})  approximate={}",
        placed.approximate_count
    );

    let (mut done, mut cur, mut avail, mut locked) = (0, 0, 0, 0);
    let mut approx = 0;
    for n in &placed.nodes {
        let b = bind_node(&n.id, &n.prerequisites, &domain);
        match b.state {
            NodeState::Completed => done += 1,
            NodeState::Current => cur += 1,
            NodeState::PrereqsMet => avail += 1,
            NodeState::PrereqsUnmet => locked += 1,
        }
        if matches!(n.fidelity, PosFidelity::Approximate { .. }) {
            approx += 1;
        }
    }
    eprintln!(
        "[smoke] node states: completed={done} current={cur} prereqs_met={avail} prereqs_unmet={locked} | approx-pos={approx}"
    );
}
