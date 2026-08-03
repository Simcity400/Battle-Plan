//! Print the production-profile compact domain roster as wire names.
//!
//! `--scheduled-domain` takes wire names, and `domain_registry.json` carries
//! the descriptor `name`, which is not always the wire name (`divisions_live`
//! vs `divisions`). This joins the two authorities by domain id:
//!
//!   * `domain_registry.json` — which ids the registry declares schedulable
//!     and production-status
//!   * `compact_contract`'s descriptor table — the wire name, and its own
//!     copy of schedulable/status
//!
//! It replaces a Python tool that regex-scraped the `domain(...)` table out of
//! `compact_contract/src/lib.rs`. Calling the crate is not merely tidier: the
//! scraper re-derived a mapping the crate already exports, so the recorder's
//! roster could disagree with the recorder's own validation, and it went
//! through a text pipeline that handed the recorder `"fuel_live\r"` on
//! 2026-07-26 — every trial in that run silently failed to inject.
//!
//! Because the descriptor carries `schedulable` and `status` too, this checks
//! all three facts rather than only the wire name, so registry-vs-table drift
//! fails closed here instead of surfacing as a mysteriously short roster.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use compact_contract::{
    assigned_descriptor_by_id, domain_status_registry_key, is_descheduled_from_capture,
    passive_production_admits, status_in_production_profile, CompactDomainId,
};

fn usage() -> ! {
    eprintln!("usage: domain_roster [--native-root <dir>] [--json]");
    std::process::exit(2);
}

fn main() {
    let mut native_root: Option<PathBuf> = None;
    let mut as_json = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--native-root" => native_root = Some(PathBuf::from(args.next().unwrap_or_else(|| usage()))),
            "--json" => as_json = true,
            _ => usage(),
        }
    }
    // Default to the workspace this binary was built in: <native>/recorder_host.
    let native_root = native_root.unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("recorder_host sits under the native root")
            .to_path_buf()
    });

    let registry_path = native_root.join("domain_registry.json");
    let text = std::fs::read_to_string(&registry_path).unwrap_or_else(|err| {
        eprintln!("cannot read {}: {err}", registry_path.display());
        std::process::exit(1);
    });
    let registry: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|err| {
        eprintln!("{} is not valid JSON: {err}", registry_path.display());
        std::process::exit(1);
    });
    let domains = registry["domains"].as_array().unwrap_or_else(|| {
        eprintln!("{} has no `domains` array", registry_path.display());
        std::process::exit(1);
    });

    let mut roster: BTreeMap<CompactDomainId, (String, &'static str)> = BTreeMap::new();
    let mut problems: Vec<String> = Vec::new();

    for entry in domains {
        let Some(id) = entry["id"].as_u64() else {
            problems.push(format!("registry entry without a numeric id: {entry}"));
            continue;
        };
        let id = id as CompactDomainId;
        let name = entry["name"].as_str().unwrap_or("<unnamed>").to_string();
        let schedulable = entry["schedulable"].as_bool().unwrap_or(false);
        let status = entry["status"].as_str().unwrap_or("<none>");

        let Some(descriptor) = assigned_descriptor_by_id(id) else {
            // Only a domain the registry actually wants scheduled has to
            // resolve; the registry legitimately carries reserved ids the
            // descriptor table does not assign.
            if schedulable && status_key_in_production(status) {
                problems.push(format!("domain {id} ({name}) has no compact_contract descriptor"));
            }
            continue;
        };

        // Drift between the two authorities is an error wherever it appears,
        // not only on the rows that would have been scheduled.
        if descriptor.schedulable != schedulable {
            problems.push(format!(
                "domain {id} ({name}): registry schedulable={schedulable}, descriptor={}",
                descriptor.schedulable
            ));
        }
        let descriptor_status = domain_status_registry_key(descriptor.status);
        if descriptor_status != status {
            problems.push(format!(
                "domain {id} ({name}): registry status={status:?}, descriptor={descriptor_status:?}"
            ));
        }

        // The deschedule is a third fact the two authorities must agree on, and
        // it is the one a reader is most likely to change in only one place:
        // the registry row is the human-facing surface, the id list is what
        // every roster actually filters on. Absent key means scheduled.
        let registry_capture_scheduled = entry["capture_scheduled"].as_bool().unwrap_or(true);
        let descriptor_descheduled = is_descheduled_from_capture(id);
        if registry_capture_scheduled == descriptor_descheduled {
            problems.push(format!(
                "domain {id} ({name}): registry capture_scheduled={registry_capture_scheduled}, \
                 compact_contract descheduled={descriptor_descheduled}"
            ));
        }

        // Production-profile status is not admissibility. The collector refuses
        // a few production-profile domains outright in the shipped passive
        // product, and emitting them here produced a roster that every live
        // passive capture was guaranteed to reject at attach time. The refusal
        // set is owned by compact_contract so this cannot drift again.
        //
        // A descheduled domain is excluded here for a different reason — the
        // owner ruled it out of capture scope, not the transport. This tool's
        // stdout cannot say so (see the output note below), so the record that
        // keeps the shorter roster explicable is
        // `compact_stream.descheduled_domains` in the session manifest.
        if schedulable
            && status_in_production_profile(descriptor.status)
            && passive_production_admits(id)
            && !descriptor_descheduled
        {
            roster.insert(id, (name, descriptor.wire_name));
        }
    }

    if !problems.is_empty() {
        for problem in &problems {
            eprintln!("{problem}");
        }
        eprintln!("domain registry and compact_contract descriptor table disagree");
        std::process::exit(1);
    }
    if roster.is_empty() {
        eprintln!("derived production domain roster is empty");
        std::process::exit(1);
    }

    // BOTH output streams are byte-contracts and neither may carry the
    // deschedule note.
    //
    // Plain stdout is parsed as a bare newline-separated wire-name list; this
    // file's own header records the day a text pipeline handed the recorder
    // "fuel_live\r" and every trial silently failed to inject.
    //
    // `--json` stdout is written verbatim to
    // `production_domain_roster_authority.json` by
    // `Start-Hoi4ProductionCaptureSafe.ps1`, which invokes this tool with
    // `2>&1` and then `ConvertFrom-Json`s the result. Anything on stderr lands
    // in that file and makes it invalid JSON; anything but a flat array of
    // {id, name, wire_name} rows fails the promotion gate's shape and count
    // checks in `ProductionCapture.Promotion.ps1`.
    //
    // The audit record for the deschedule therefore lives where a reader of a
    // sealed capture actually looks: `compact_stream.descheduled_domains` in
    // the session manifest, alongside this file in the same run directory.
    if as_json {
        let rows: Vec<serde_json::Value> = roster
            .iter()
            .map(|(id, (name, wire))| {
                serde_json::json!({ "id": id, "name": name, "wire_name": wire })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows).expect("roster serializes"));
    } else {
        for (_, (_, wire)) in &roster {
            println!("{wire}");
        }
    }
}

/// The registry's own spelling of a production-profile status, for the one
/// check that runs before a descriptor is in hand.
fn status_key_in_production(status: &str) -> bool {
    matches!(
        status,
        "existing"
            | "production_complete"
            | "production_complete_base"
            | "validated_slice"
            | "validated_partial_slice"
            | "validated_slice_open_gaps"
    )
}
