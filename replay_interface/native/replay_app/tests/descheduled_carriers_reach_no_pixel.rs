use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_source(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("{} is readable: {error}", path.display()))
        .replace("\r\n", "\n")
}

fn repo_root() -> PathBuf {
    manifest()
        .ancestors()
        .nth(3)
        .expect("replay_app is three levels below the repository root")
        .to_path_buf()
}

fn collect(dir: &Path, label: &str, into: &mut Vec<(String, String)>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("{} is readable: {error}", dir.display()));
    for entry in entries {
        let path = entry.expect("directory entry").path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("source entry name")
            .to_string();
        let nested = format!("{label}/{name}");
        if path.is_dir() {
            if name == "target" || name.starts_with(".") {
                continue;
            }
            collect(&path, &nested, into);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs")
            && nested.contains("/src/")
        {
            into.push((nested, read_source(&path)));
        }
    }
}

/// A module with every column-zero `#[cfg(test)]` item removed, declaration form
/// (`#[cfg(test)] #[path = "x_tests.rs"] mod x;`) and block form alike. Splitting
/// at the first attribute instead would cut `world.rs` at line 20, where it
/// declares its test files, and blind the census to the whole carrier layer.
/// Net bracket depth of one line, ignoring string literals (raw ones included),
/// char literals, line comments and block comments (which nest, and which the
/// state carries across lines). Counting raw bytes instead lets
/// `const MASK: &str = "{";` or `const MASK: () = /* { */ ();` hold the depth
/// open and swallow the production items after it.
#[derive(Default)]
struct ScanState {
    block_comments: u32,
    in_string: bool,
    raw_hashes: Option<usize>,
}

/// One line, split into the code it contains and the net delimiter depth of
/// that code. Comments are dropped rather than counted: a token inside one is
/// not a route, and leaving them in let a comment mentioning a function be
/// traded for a call to it with no count moving. Strings (raw included) and
/// char literals keep their text but carry no delimiters.
fn scan_line(line: &str, state: &mut ScanState) -> (String, i32) {
    let bytes = line.as_bytes();
    let mut code = String::with_capacity(line.len());
    let mut depth = 0;
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        if state.block_comments > 0 {
            if byte == b'*' && bytes.get(i + 1) == Some(&b'/') {
                state.block_comments -= 1;
                i += 2;
            } else if byte == b'/' && bytes.get(i + 1) == Some(&b'*') {
                state.block_comments += 1;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if let Some(hashes) = state.raw_hashes {
            code.push(byte as char);
            if byte == b'"'
                && bytes[i + 1..]
                    .iter()
                    .take(hashes)
                    .filter(|b| **b == b'#')
                    .count()
                    == hashes
            {
                state.raw_hashes = None;
                for extra in &bytes[i + 1..(i + 1 + hashes).min(bytes.len())] {
                    code.push(*extra as char);
                }
                i += 1 + hashes;
            } else {
                i += 1;
            }
            continue;
        }
        if state.in_string {
            code.push(byte as char);
            if byte == b'\\' {
                if let Some(next) = bytes.get(i + 1) {
                    code.push(*next as char);
                }
                i += 2;
            } else {
                if byte == b'"' {
                    state.in_string = false;
                }
                i += 1;
            }
            continue;
        }
        if byte == b'/' && bytes.get(i + 1) == Some(&b'*') {
            state.block_comments = 1;
            i += 2;
            continue;
        }
        if byte == b'/' && bytes.get(i + 1) == Some(&b'/') {
            break;
        }
        if byte == b'r' {
            let hashes = bytes[i + 1..].iter().take_while(|b| **b == b'#').count();
            if bytes.get(i + 1 + hashes) == Some(&b'"') {
                state.raw_hashes = Some(hashes);
                for kept in &bytes[i..(i + 2 + hashes).min(bytes.len())] {
                    code.push(*kept as char);
                }
                i += 2 + hashes;
                continue;
            }
        }
        if byte == b'"' {
            state.in_string = true;
            code.push('"');
            i += 1;
            continue;
        }
        // A three-byte `'x'` is a char literal; anything else starting with `'`
        // is a lifetime and carries no delimiter.
        if byte == b'\'' && bytes.get(i + 2) == Some(&b'\'') {
            for kept in &bytes[i..i + 3] {
                code.push(*kept as char);
            }
            i += 3;
            continue;
        }
        match byte {
            b'{' | b'[' | b'(' => depth += 1,
            b'}' | b']' | b')' => depth -= 1,
            _ => {}
        }
        code.push(byte as char);
        i += 1;
    }
    (code, depth)
}

/// A module reduced to its shipping code: every column-zero `#[cfg(test)]` item
/// removed, declaration form and block form alike, and every comment dropped.
/// Splitting at the first attribute instead would cut `world.rs` at line 20,
/// where it declares its test files, and blind the census to the carrier layer.
fn production_text(module: &str) -> String {
    let lines: Vec<&str> = module.lines().collect();
    let mut out = String::with_capacity(module.len());
    let mut i = 0;
    let mut state = ScanState::default();
    let mut nesting: i32 = 0;
    while i < lines.len() {
        // Scanned first, then judged, and only at item level. Inside a block
        // comment a `#[cfg(test)]` line is text; inside a macro's token tree it
        // is an argument the macro may discard, so the item it precedes can be
        // compiled unconditionally. Either way, treating it as an attribute
        // would let the skip loop eat shipping code. Depth zero is the
        // conservative reading of "item context": anything nested stays in.
        let (code, delta) = scan_line(lines[i], &mut state);
        if nesting != 0 || code.trim() != "#[cfg(test)]" {
            nesting += delta;
            out.push_str(&code);
            out.push('\n');
            i += 1;
            continue;
        }
        i += 1;
        while i < lines.len() && lines[i].starts_with("#[") {
            i += 1;
        }
        if i >= lines.len() {
            break;
        }
        // The item ends where its own brackets close, or at a column-zero `}`,
        // whichever comes first. The terminator is judged on the code, so
        // `} // note` still ends the item.
        let mut depth: i32 = 0;
        let mut item = ScanState::default();
        loop {
            if i >= lines.len() {
                break;
            }
            let (code, delta) = scan_line(lines[i], &mut item);
            depth += delta;
            let trimmed = code.trim_end();
            i += 1;
            if trimmed == "}" {
                break;
            }
            if depth <= 0 && (trimmed.ends_with(';') || trimmed.ends_with('}')) {
                break;
            }
        }
    }
    out
}

/// Every shipping `.rs` file, at any depth, in every Replay crate -- not just
/// the two that render today, because which crate a route is written in is a
/// choice a later change makes. `tests/` trees are left out, and so is any
/// `*_tests.rs`, which is wholly test content included under a `#[cfg(test)]`
/// declaration; a test naming a consumer is how the consumer is exercised, and
/// this file would otherwise count its own census literals.
fn shipping_sources() -> Vec<(String, String)> {
    let root = repo_root();
    let mut sources = Vec::new();
    for tree in ["replay_interface/native", "portable_replay/native"] {
        collect(&root.join(tree), tree, &mut sources);
    }
    // `*_tests.rs` files are wholly test content, but that has to be shown from
    // the CODE rather than assumed from the name or read off raw text: a file is
    // skipped only when no module still declares it once comments are stripped
    // and `#[cfg(test)]` items removed. A commented-out gate above an
    // unconditional `mod hidden_tests;` leaves the declaration standing here.
    for source in sources.iter_mut() {
        source.1 = production_text(&source.1);
    }
    let declarations: String = sources
        .iter()
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>()
        .join("
");
    let mut skipped = 0;
    sources.retain(|(label, _)| {
        if !label.ends_with("_tests.rs") {
            return true;
        }
        let name = label.rsplit('/').next().expect("a file name");
        let stem = name.trim_end_matches(".rs");
        let still_declared = declarations.contains(name)
            || declarations.contains(&format!("mod {stem};"));
        if still_declared {
            return true;
        }
        skipped += 1;
        false
    });
    assert!(skipped > 0, "the test-only skip list is empty, so it is not being exercised");
    assert!(
        sources.len() > 200,
        "source walk collapsed to {} files",
        sources.len()
    );
    sources
}

/// The half of a module a shipping route can be written in. Split on the
/// column-zero form so `#[cfg(test)]` on an item inside production code does not
/// truncate the region.
fn production_region(module: &str) -> &str {
    module
        .split_once("\n#[cfg(test)]\n")
        .expect("module has a top-level test module")
        .0
}

fn product_execute(runtime: &str) -> &str {
    runtime
        .split_once("impl ProductObserverUiRuntime {")
        .expect("product runtime implementation")
        .1
        .split_once("pub fn invalidate(&mut self)")
        .expect("product execute boundary")
        .0
}

/// Occurrences of `identifier` that are not part of a longer identifier, so
/// `occupation_alert` does not count `occupation_alert_product_binding`. A
/// boundary is only required on an end the needle itself spells with an
/// identifier character, so a leading `.` still matches a field read.
fn identifier_occurrences(text: &str, identifier: &str) -> usize {
    let bytes = text.as_bytes();
    let needle = identifier.as_bytes();
    let ident_byte = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    let bounded_left = ident_byte(needle[0]);
    let bounded_right = ident_byte(needle[needle.len() - 1]);
    text.match_indices(identifier)
        .filter(|(start, _)| {
            let start = *start;
            let end = start + identifier.len();
            (!bounded_left || start == 0 || !ident_byte(bytes[start - 1]))
                && (!bounded_right || end >= bytes.len() || !ident_byte(bytes[end]))
        })
        .count()
}

/// Occurrences of `prefix` that start an identifier, whatever it goes on to
/// spell, so `selected_designer_identity_exact` also counts the `_status`
/// accessor beside it and any later sibling.
fn prefix_occurrences(text: &str, prefix: &str) -> usize {
    let bytes = text.as_bytes();
    let ident_byte = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    text.match_indices(prefix)
        .filter(|(start, _)| *start == 0 || !ident_byte(bytes[start - 1]))
        .count()
}

struct CarrierName {
    domain: &'static str,
    token: &'static str,
    /// The files allowed to name it, and how many times. Every other shipping
    /// source must be zero.
    recorded: &'static [(&'static str, usize)],
}

/// Every naming of a descheduled carrier, its accessors and its typed consumers
/// in Replay's shipping source. A count that moves is a module reaching a
/// carrier the ledger records as reaching no pixel -- whether it is a new crate,
/// a new module, a new call in an existing compositor, or an accessor spelled
/// with a suffix. The `replay_core` entries are the carriers' own declarations
/// and accessors; every other file names a route into them.
const CARRIER_CENSUS: &[CarrierName] = &[
    CarrierName {
        domain: "d72",
        token: "resolved_tooltip_exact",
        recorded: &[
            ("replay_interface/native/game_ui/src/production_renderer.rs", 1),
            ("replay_interface/native/replay_core/src/world.rs", 7),
        ],
    },
    CarrierName {
        domain: "d72",
        token: "_r2_resolved_tooltip_instances_live",
        recorded: &[
            ("replay_interface/native/replay_core/src/world.rs", 1),
        ],
    },
    CarrierName {
        domain: "d72",
        token: "tooltip_binding_from_frame",
        recorded: &[
            ("replay_interface/native/game_ui/src/production_renderer.rs", 2),
        ],
    },
    CarrierName {
        domain: "d72",
        token: "tooltip_binding_from_exact_view",
        recorded: &[
            ("replay_interface/native/game_ui/src/production_renderer.rs", 2),
        ],
    },
    CarrierName {
        domain: "d72",
        token: "admit_xui05_tooltip_route",
        recorded: &[
            ("replay_interface/native/game_ui/src/production_renderer.rs", 3),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "occupation_alert_product_binding",
        recorded: &[
            ("replay_interface/native/game_ui/src/occupation_binding.rs", 1),
            ("replay_interface/native/replay_app/src/ui_runtime.rs", 1),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "alert_feed_from_frame",
        recorded: &[
            ("replay_interface/native/game_ui/src/alerts.rs", 1),
            ("replay_interface/native/game_ui/src/occupation_binding.rs", 1),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "hud_surface_activation",
        recorded: &[
            ("replay_interface/native/replay_core/src/world.rs", 7),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "admit_authenticated_hud_surface_activation",
        recorded: &[
            ("replay_interface/native/game_ui/src/production_renderer.rs", 1),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "topbar_alerts",
        recorded: &[
            ("replay_interface/native/game_ui/src/alerts.rs", 4),
            ("replay_interface/native/game_ui/src/binding.rs", 1),
            ("replay_interface/native/replay_core/src/world.rs", 20),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "topbar_binding_from_frame",
        recorded: &[
            ("replay_interface/native/game_ui/src/binding.rs", 1),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "_r2_hud_surface_activation_live",
        recorded: &[
            ("replay_interface/native/replay_core/src/world.rs", 1),
        ],
    },
    CarrierName {
        domain: "d85",
        token: "selected_designer_identity_exact",
        recorded: &[
            ("replay_interface/native/replay_core/src/world.rs", 7),
        ],
    },
    CarrierName {
        domain: "d85",
        token: "admit_exact_selected_designer_identity",
        recorded: &[
            ("replay_interface/native/game_ui/src/readonly_design_peace.rs", 1),
        ],
    },
    CarrierName {
        domain: "d85",
        token: "_r2_selected_designer_identity_live",
        recorded: &[
            ("replay_interface/native/replay_core/src/world.rs", 1),
        ],
    },
    CarrierName {
        domain: "d72",
        token: "ResolvedTooltipExactView",
        recorded: &[
            ("replay_interface/native/game_ui/src/production_renderer.rs", 1),
            ("replay_interface/native/replay_core/src/lib.rs", 1),
            ("replay_interface/native/replay_core/src/world.rs", 7),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "HudSurfaceActivationView",
        recorded: &[
            ("replay_interface/native/game_ui/src/production_renderer.rs", 1),
            ("replay_interface/native/replay_core/src/lib.rs", 1),
            ("replay_interface/native/replay_core/src/world.rs", 9),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "AdmittedHudSurfaceActivation",
        recorded: &[
            ("replay_interface/native/game_ui/src/production_renderer.rs", 4),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "TopbarAlerts",
        recorded: &[
            ("replay_interface/native/game_ui/src/alerts.rs", 4),
            ("replay_interface/native/game_ui/src/binding.rs", 1),
            ("replay_interface/native/replay_core/src/lib.rs", 1),
            ("replay_interface/native/replay_core/src/world.rs", 19),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "AlertFeedBinding",
        recorded: &[
            ("replay_interface/native/game_ui/src/alerts.rs", 9),
        ],
    },
    CarrierName {
        domain: "d85",
        token: "SelectedDesignerIdentityExactView",
        recorded: &[
            ("replay_interface/native/game_ui/src/readonly_design_peace.rs", 2),
            ("replay_interface/native/replay_core/src/lib.rs", 1),
            ("replay_interface/native/replay_core/src/world.rs", 7),
        ],
    },
    CarrierName {
        domain: "d85",
        token: "AdmittedSelectedDesignerIdentity",
        recorded: &[
            ("replay_interface/native/game_ui/src/readonly_design_peace.rs", 4),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "alerts_strip_from_frame",
        recorded: &[
            ("replay_interface/native/game_ui/src/alerts.rs", 1),
        ],
    },
    CarrierName {
        domain: "d72",
        token: "exact_a729_tooltip_controller",
        recorded: &[
            ("replay_interface/native/replay_core/src/world.rs", 1),
        ],
    },
    CarrierName {
        domain: "d85",
        token: "exact_a729_selected_designer_identity",
        recorded: &[
            ("replay_interface/native/replay_core/src/world.rs", 1),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "intel_available",
        recorded: &[
            ("replay_interface/native/game_ui/src/binding.rs", 4),
            ("replay_interface/native/game_ui/src/topbar.rs", 5),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "intel_prepared_or_completed",
        recorded: &[
            ("replay_interface/native/game_ui/src/alerts.rs", 1),
            ("replay_interface/native/game_ui/src/binding.rs", 5),
            ("replay_interface/native/game_ui/src/topbar.rs", 5),
            ("replay_interface/native/replay_core/src/world.rs", 5),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "non_delivering_contracts",
        recorded: &[
            ("replay_interface/native/game_ui/src/binding.rs", 4),
            ("replay_interface/native/game_ui/src/topbar.rs", 6),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "ongoing_contracts",
        recorded: &[
            ("replay_interface/native/game_ui/src/binding.rs", 4),
            ("replay_interface/native/game_ui/src/topbar.rs", 5),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "decision_available_items",
        recorded: &[
            ("replay_interface/native/game_ui/src/binding.rs", 5),
            ("replay_interface/native/game_ui/src/topbar.rs", 5),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "decision_timeout_items",
        recorded: &[
            ("replay_interface/native/game_ui/src/binding.rs", 5),
            ("replay_interface/native/game_ui/src/topbar.rs", 5),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "prototype_rewards",
        recorded: &[
            ("replay_interface/native/game_ui/src/alerts.rs", 2),
            ("replay_interface/native/game_ui/src/binding.rs", 8),
            ("replay_interface/native/game_ui/src/topbar.rs", 8),
            ("replay_interface/native/replay_core/src/world.rs", 4),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "initiative",
        recorded: &[
            ("replay_interface/native/game_ui/src/alerts.rs", 2),
            ("replay_interface/native/game_ui/src/authenticated_semantic_provider.rs", 2),
            ("replay_interface/native/game_ui/src/binding.rs", 9),
            ("replay_interface/native/game_ui/src/topbar.rs", 6),
            ("replay_interface/native/replay_core/src/world.rs", 5),
            ("replay_interface/native/replay_fixture_gen/src/main.rs", 1),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "exact_a729_hud_controller_roots",
        recorded: &[
            ("replay_interface/native/game_ui/src/production_renderer.rs", 1),
            ("replay_interface/native/replay_core/src/world.rs", 1),
        ],
    },
    CarrierName {
        domain: "d72",
        token: "resolved_tooltip_instances_live",
        recorded: &[],
    },
    CarrierName {
        domain: "d85",
        token: "selected_designer_identity_live",
        recorded: &[],
    },
    CarrierName {
        domain: "d72",
        token: "final_resolved_text",
        recorded: &[
            ("replay_interface/native/game_ui/src/production_renderer.rs", 1),
            ("replay_interface/native/replay_core/src/world.rs", 2),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "active_feed",
        recorded: &[
            ("replay_interface/native/game_ui/src/alerts.rs", 1),
            ("replay_interface/native/replay_core/src/world.rs", 5),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "market_contracts",
        recorded: &[
            ("replay_interface/native/game_ui/src/alerts.rs", 2),
            ("replay_interface/native/game_ui/src/binding.rs", 2),
            ("replay_interface/native/replay_core/src/world.rs", 10),
            ("replay_interface/native/replay_fixture_gen/src/main.rs", 2),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "decisions_available",
        recorded: &[
            ("replay_interface/native/game_ui/src/alerts.rs", 1),
            ("replay_interface/native/game_ui/src/binding.rs", 1),
            ("replay_interface/native/replay_core/src/world.rs", 5),
            ("replay_interface/native/replay_fixture_gen/src/main.rs", 1),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "decisions_timeout",
        recorded: &[
            ("replay_interface/native/game_ui/src/alerts.rs", 1),
            ("replay_interface/native/game_ui/src/binding.rs", 1),
            ("replay_interface/native/replay_core/src/world.rs", 5),
        ],
    },
    CarrierName {
        domain: "d85",
        token: "ordered_designs",
        recorded: &[
            ("replay_interface/native/game_ui/src/readonly_design_peace.rs", 2),
            ("replay_interface/native/replay_core/src/world.rs", 5),
        ],
    },
    CarrierName {
        domain: "d72",
        token: "pub fn resolved_tooltip_exact",
        recorded: &[
            ("replay_interface/native/replay_core/src/world.rs", 2),
        ],
    },
    CarrierName {
        domain: "d72",
        token: "self.resolved_tooltip_exact",
        recorded: &[
            ("replay_interface/native/replay_core/src/world.rs", 2),
        ],
    },
    CarrierName {
        domain: "d85",
        token: "pub fn selected_designer_identity_exact",
        recorded: &[
            ("replay_interface/native/replay_core/src/world.rs", 2),
        ],
    },
    CarrierName {
        domain: "d85",
        token: "self.selected_designer_identity_exact",
        recorded: &[
            ("replay_interface/native/replay_core/src/world.rs", 2),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "pub fn topbar_alerts",
        recorded: &[
            ("replay_interface/native/replay_core/src/world.rs", 3),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "self.topbar_alerts",
        recorded: &[
            ("replay_interface/native/replay_core/src/world.rs", 3),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "pub fn hud_surface_activation",
        recorded: &[
            ("replay_interface/native/replay_core/src/world.rs", 2),
        ],
    },
    CarrierName {
        domain: "d79",
        token: "self.hud_surface_activation",
        recorded: &[
            ("replay_interface/native/replay_core/src/world.rs", 2),
        ],
    },
];

/// None of the three carrier root keys is a registered base-frame sidecar, so an
/// hour that carried one would enter the XUI12/XUI15/XUI23 identity preimage --
/// which is why each row states that scheduling its domain has to register the
/// key first. If the registry gains one, that requirement is stale.
fn assert_carrier_key_unregistered(key: &str) {
    let registry = read_source(
        &repo_root().join("portable_replay/native/snapshot/src/replay_frame_identity.rs"),
    );
    let listed = registry
        .split_once("pub const REGISTERED_ROOT_SIDECAR_KEYS")
        .expect("the base-frame sidecar registry")
        .1
        .split_once("];")
        .expect("the registry list ends")
        .0;
    assert!(
        !listed.contains(key),
        "{key} is now a registered base-frame sidecar, so its row's identity requirement is stale"
    );
}

fn assert_census(domain: &str) {
    let sources = shipping_sources();
    let mut checked = 0;
    for name in CARRIER_CENSUS.iter().filter(|name| name.domain == domain) {
        checked += 1;
        for (label, text) in &sources {
            let recorded = name
                .recorded
                .iter()
                .find(|(file, _)| file == label)
                .map_or(0, |(_, count)| *count);
            let found = prefix_occurrences(text, name.token);
            assert_eq!(
                found, recorded,
                "{label} names {:?} {found} times, and the census records {recorded}",
                name.token
            );
        }
        for (file, _) in name.recorded {
            assert!(
                sources.iter().any(|(label, _)| label == file),
                "the census records {file} for {:?}, and the source walk did not reach it",
                name.token
            );
        }
    }
    assert!(checked > 0, "the census has no {domain} names");
}

fn identifiers(text: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut current = String::new();
    for character in text.chars() {
        if character.is_ascii_alphanumeric() || character == '_' {
            current.push(character);
        } else if !current.is_empty() {
            names.insert(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        names.insert(current);
    }
    names
}

/// Names the d79 join statement and the pixel-composition region may share.
/// Every one is a language item, a shared input, or a receipt sink. A name added
/// to both regions is the channel a derived overlay would travel down, whatever
/// it was called. The two receipt sinks DO carry d79-derived text out of the
/// join, so `receipt_sinks_are_write_only_where_pixels_are_composed` below
/// requires the composition region to write to them and never read them.
const D79_SHARED_WITH_COMPOSITION: &[&str] = &[
    "Err",
    "None",
    "Ok",
    "Some",
    "_",
    "and_then",
    "as_deref",
    "as_ref",
    "binding",
    "country",
    "domain",
    "else",
    "error",
    "feature_local_gaps",
    "field",
    "format",
    "game_ui",
    "if",
    "let",
    "match",
    "measured_empty",
    "occupation_outcome",
    "outcome",
    "push",
    "selected_country",
    "semantic_receipts",
    "slot",
    "surface",
    "unavailable",
    "viewer",
    "world_frame",
];

/// EIR-NE-017's `typed_consumer_not_shipping` classification. The d79 join is
/// the one nonshipping consumer the product runtime does call, so the pin is
/// that nothing it produces reaches a pixel, rather than that it is never
/// called.
#[test]
fn product_runtime_stores_the_d79_alert_join_without_drawing_from_it() {
    let module = read_source(&manifest().join("src/ui_runtime.rs"));
    let production = production_region(&module);

    let join_calls = production
        .matches("occupation_alert_product_binding(")
        .count();
    assert_eq!(join_calls, 1, "the d79 alert join is called {join_calls} times");

    // Zero, since round 17: the join's result is discarded where it is produced
    // and the outcome no longer carries it, so the name appears nowhere in the
    // shipping module and there is nothing left to read or to destructure.
    let reads = identifier_occurrences(production, "occupation_alert");
    assert_eq!(
        reads, 0,
        "the d79 alert binding is named {reads} times in the shipping module"
    );
    for (label, text) in shipping_sources() {
        let named = identifier_occurrences(&text, "occupation_alert");
        assert_eq!(named, 0, "{label} names the retired d79 alert binding");
    }

    let execute = product_execute(production);
    let join = execute
        .find("occupation_alert_product_binding(")
        .and_then(|at| execute[..at].rfind("let _ = match ("))
        .expect("d79 alert join statement");
    let statement_end = execute[join..]
        .find("\n        let mut production_lines_composited")
        .map(|offset| join + offset)
        .expect("statement following the d79 join");
    let framebuffer = execute
        .find("let mut framebuffer =")
        .expect("framebuffer construction");
    // One upload, asserted rather than assumed: the receipt scan below covers
    // every pixel operation, and a second upload after the first would leave
    // half of them outside any scanned region.
    let uploads = execute
        .matches(".image\n            .set_rgba(gfx, &framebuffer)")
        .count();
    assert_eq!(
        uploads, 1,
        "the product runtime uploads the framebuffer {uploads} times"
    );
    let upload = execute
        .find(".image\n            .set_rgba(gfx, &framebuffer)")
        .expect("single framebuffer upload");
    assert!(
        statement_end < framebuffer,
        "the d79 join must resolve before any pixel surface exists"
    );

    // A new name is one way out of the join; writing through a name that is
    // already shared is the other, so the statement introduces every binding it
    // makes and hands out no mutable borrow.
    let statement = &execute[join..statement_end];
    assert!(
        !statement.contains("&mut "),
        "the d79 join now lends something out mutably"
    );
    for line in statement.lines().filter(|line| line.contains(" = ")) {
        assert!(
            line.contains("let "),
            "the d79 join assigns to a binding declared outside it: {line:?}"
        );
    }

    let allowed: BTreeSet<String> = D79_SHARED_WITH_COMPOSITION
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    let shared: Vec<String> = identifiers(&execute[join..statement_end])
        .intersection(&identifiers(&execute[framebuffer..upload]))
        .filter(|name| !allowed.contains(*name))
        .cloned()
        .collect();
    assert!(
        shared.is_empty(),
        "the d79 join and the pixel-composition region now share {shared:?}"
    );

    for (label, text) in shipping_sources() {
        let field_reads = identifier_occurrences(&text, ".occupation_alert");
        assert_eq!(
            field_reads, 0,
            "{label} reads the runtime's d79 alert output field"
        );
    }

    // The checks above are about this one join. The census is about every other
    // way into the trailer: the alert feed, the `topbar_alerts` accessor every
    // consumer goes through, and `topbar_binding_from_frame`, which fills eight
    // TopbarBinding alert fields from it and has had no shipping caller since
    // the flag sites moved to the flag-only binder.
    assert_census("d79");
    assert_carrier_key_unregistered("_r2_hud_surface_activation_live");

    // A helper that is handed the framebuffer may not take a receipt vector under
    // another name: the call site's `&mut semantic_receipts` is allowed as a
    // write, and a renamed parameter would then be read in the body with no
    // protected token in sight.
    for (at, _) in production.match_indices("&mut Vec<String>") {
        let signature_start = production[..at]
            .rfind("
fn ")
            .or_else(|| production[..at].rfind("
    fn "))
            .unwrap_or(0);
        let signature = &production[signature_start..at];
        if !signature.contains("framebuffer: &mut") {
            continue;
        }
        let parameter = signature
            .rsplit_once('\n')
            .map_or(signature, |(_, last)| last)
            .trim()
            .trim_end()
            .trim_end_matches(":");
        assert!(
            parameter == "semantic_receipts" || parameter == "feature_local_gaps",
            "a helper holding the framebuffer takes a receipt vector named {parameter:?}"
        );
    }

    // The runtime hands its outcome to the presenter, which holds it across
    // `gfx.present()`. `ObserverUiOutcome` deliberately derives Clone alone, so the
    // whole value cannot be formatted into a decision; these are the only reads
    // of the two d79-derived sinks the presenter is allowed to make.
    let presenter = production_text(&read_source(
        &repo_root().join("replay_interface/native/replay_app/src/app.rs"),
    ));
    // Position, not just count: moving a read from post-Present logging to a
    // pre-Present gate keeps the total where it was, so every read has to sit
    // after the frame is on screen, where it can no longer change one.
    let presents = presenter.matches("gfx.present()").count();
    assert_eq!(
        presents, 1,
        "the presenter calls gfx.present() {presents} times, so 'after Present' is ambiguous"
    );
    let present = presenter
        .find("gfx.present()")
        .expect("the presenter presents");
    for (sink, recorded) in [("semantic_receipts", 2usize), ("feature_local_gaps", 2)] {
        let found = identifier_occurrences(&presenter, sink);
        assert_eq!(
            found, recorded,
            "the presenter names the d79 sink {sink:?} {found} times, and {recorded} are recorded"
        );
        for (at, _) in presenter.match_indices(sink) {
            assert!(
                at > present,
                "the presenter reads the d79 sink {sink:?} before the frame is presented"
            );
        }
    }
    let outcome_derive = production
        .split_once("pub struct ObserverUiOutcome {")
        .expect("the outcome struct")
        .0;
    let outcome_derive = outcome_derive
        .rsplit_once("#[derive(")
        .expect("the outcome derives something")
        .1;
    assert_eq!(
        outcome_derive
            .trim_end()
            .trim_end_matches(']')
            .trim_end_matches(')'),
        "Clone",
        "the outcome derives more than Clone: Debug formats its d79 receipts into one          string and PartialEq turns them into a boolean"
    );

    // Which is only safe while the rendered topbar comes from the observer
    // binding, whose alert values are declared gaps, rather than from the
    // trailer-filled one.
    let renderer = read_source(
        &repo_root().join("replay_interface/native/game_ui/src/production_renderer.rs"),
    );
    // Every shipping caller of the trailer-filled binding destructures it to the
    // three flag fields instead of holding it, so the eight alert fields are not
    // in scope at a pixel site and cannot be read, formatted or hashed there.
    // Round 9: restricting which fields the flag sites take is not enough on its
    // own -- an approved field could be assigned from the trailer inside the
    // producer. `apply_alerts` is the only function there that sees the trailer,
    // so it must write the alert fields and nothing else.
    let binding_rs = read_source(
        &repo_root().join("replay_interface/native/game_ui/src/binding.rs"),
    );
    let apply_alerts = production_region(&binding_rs)
        .split_once("fn apply_alerts(")
        .expect("the trailer producer")
        .1;
    let apply_alerts = apply_alerts
        .split_once("
}
")
        .expect("the trailer producer ends")
        .0;
    // Targets, not `binding.` occurrences: an alias (`let f = &mut
    // binding.country_tag; *f = ...`) writes a field this scan would never see
    // under that name, so every assignment in the body has to spell its target.
    assert!(
        !apply_alerts.contains("&mut binding"),
        "the trailer producer lends out a field it could then write through"
    );
    let mut written: BTreeSet<String> = BTreeSet::new();
    for line in apply_alerts.lines() {
        let Some((target, _)) = line.split_once(" = ") else {
            continue;
        };
        let target = target.trim();
        let field = target.strip_prefix("binding.").unwrap_or_else(|| {
            panic!("the trailer producer assigns to {target:?}, which is not a binding field")
        });
        written.insert(field.to_string());
    }
    let alert_fields: BTreeSet<String> = [
        "initiative",
        "decision_available_items",
        "decision_timeout_items",
        "intel_available",
        "intel_prepared_or_completed",
        "prototype_rewards",
        "non_delivering_contracts",
        "ongoing_contracts",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    assert_eq!(
        written, alert_fields,
        "the trailer producer writes {written:?}, which is not the eight alert fields"
    );

    let renderer_production = production_region(&renderer);

    // The alert-bearing binder is not named in the renderer at all any more:
    // round 14 showed that holding its result anywhere lets code between the
    // trailer write and the flag read taint an approved field. The flag sites
    // take a binder that never sees the trailer, and bind exactly its three
    // fields.
    let held = renderer_production
        .matches("topbar_binding_from_frame(")
        .count();
    assert_eq!(
        held, 0,
        "the renderer builds the trailer-filled topbar binding {held} times"
    );
    let flag_sites = renderer_production
        .matches("} = topbar_flag_identity_from_frame(")
        .count();
    assert!(flag_sites > 0, "no flag site uses the flag-only binder");
    let expected: BTreeSet<String> = ["country_tag", "country_ideology", "flag_texture"]
        .into_iter()
        .map(str::to_string)
        .collect();
    for (site, _) in renderer_production.match_indices("} = topbar_flag_identity_from_frame(") {
        let pattern = renderer_production[..site]
            .rsplit_once("TopbarFlagIdentity {")
            .expect("each flag site destructures the flag identity")
            .1;
        let bound: BTreeSet<String> = identifiers(pattern);
        assert_eq!(
            bound, expected,
            "a flag site binds {bound:?}, not the flag three"
        );
    }

    // And the flag-only binder never reaches the trailer itself.
    let flag_binder = production_region(&binding_rs)
        .split_once("pub fn topbar_flag_identity_from_frame(")
        .expect("the flag-only binder")
        .1
        .split_once("
}
")
        .expect("the flag-only binder ends")
        .0;
    for forbidden in ["alerts", "topbar_alerts", "apply_alerts"] {
        assert_eq!(
            identifier_occurrences(flag_binder, forbidden),
            0,
            "the flag-only binder names {forbidden:?}"
        );
    }

    // Which is only safe while the rendered topbar comes from the observer
    // binding, whose alert values are declared gaps, rather than from the
    // trailer-filled one.
    let renderer = read_source(
        &repo_root().join("replay_interface/native/game_ui/src/production_renderer.rs"),
    );

    // The join pushes d79 alert identity into these two, so any READ of either
    // is d79 leaving the join. Scanned over the whole shipping module, not the
    // composition slice: the feature-local helpers take `&mut` to both a sink
    // and the framebuffer, so a read inside one of them is a read at a pixel.
    // Writes are `.push(`, `.extend(`, a `&mut` argument, a binding or a move.
    let outcome_at = production.find("Ok(ObserverUiOutcome {");
    for sink in ["semantic_receipts", "feature_local_gaps"] {
        let mut reads = Vec::new();
        for (at, _) in production.match_indices(sink) {
            let after = &production[at + sink.len()..];
            let before = production[..at].trim_end();
            // Safe shapes, enumerated: anything else -- a move into another
            // name included -- is a read waiting to be spelled through the new
            // one.
            let next = after.trim_start();
            let written = next.starts_with(".push(") || next.starts_with(".extend(");
            let declared = next.starts_with(':');
            let lent = before.ends_with("&mut") && (next.starts_with(',') || next.starts_with(')'));
            let bound = (before.ends_with("let mut") || before.ends_with("let"))
                && next.starts_with('=');
            let moved = (before.ends_with('(') || before.ends_with(',') || before.ends_with('{'))
                && (next.starts_with(',') || next.starts_with(')'));
            // A move is only safe once the frame is finished: before that, a
            // by-value hand-off to a renamed parameter reads the receipts under
            // the new name while the old one stays on its allow-list.
            let after_frame = outcome_at.is_some_and(|outcome| at > outcome);
            if !(written || declared || lent || bound || (moved && after_frame)) {
                reads.push(after[..24.min(after.len())].to_string());
            }
        }
        assert!(
            reads.is_empty(),
            "the runtime reads the d79 receipt sink {sink:?}: {reads:?}"
        );
    }

    let assignments: Vec<&str> = production_region(&renderer)
        .lines()
        .filter(|line| line.contains("hud_binding.topbar ="))
        .collect();
    assert_eq!(
        assignments.len(),
        1,
        "the rendered topbar is assigned {} times: {assignments:?}",
        assignments.len()
    );
    assert!(
        assignments[0].contains("observer_topbar.binding"),
        "the rendered topbar is filled from {:?}",
        assignments[0].trim()
    );
}

/// EIR-NE-018's `typed_consumer_not_shipping` classification: the d85 consumer
/// is typed and exercised by its own integration test, and nothing else.
#[test]
fn no_shipping_module_calls_the_d85_selected_designer_consumer() {
    assert_census("d85");
    assert_carrier_key_unregistered("_r2_selected_designer_identity_live");
}

/// EIR-NE-016's `typed_consumer_not_shipping` classification, over every
/// shipping module rather than the product runtime alone: the tooltip binders
/// exist in one file and no route outside it reaches them.
#[test]
fn no_shipping_module_outside_the_binders_reaches_the_d72_tooltip_carrier() {
    assert_census("d72");
    assert_carrier_key_unregistered("_r2_resolved_tooltip_instances_live");
}
