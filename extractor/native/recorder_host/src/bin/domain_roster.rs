use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use compact_contract::{
    assigned_descriptor_by_id, domain_status_registry_key, is_descheduled_from_capture,
    passive_capture_roster, CompactDomainId,
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
            "--native-root" => {
                native_root = Some(PathBuf::from(args.next().unwrap_or_else(|| usage())))
            }
            "--json" => as_json = true,
            _ => usage(),
        }
    }
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

    let membership: BTreeMap<CompactDomainId, &'static str> = passive_capture_roster()
        .into_iter()
        .map(|descriptor| (descriptor.id, descriptor.wire_name))
        .collect();
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
            if schedulable && status_key_in_production(status) {
                problems.push(format!(
                    "domain {id} ({name}) has no compact_contract descriptor"
                ));
            }
            continue;
        };

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

        let registry_capture_scheduled = entry["capture_scheduled"].as_bool().unwrap_or(true);
        let descriptor_descheduled = is_descheduled_from_capture(id);
        if registry_capture_scheduled == descriptor_descheduled {
            problems.push(format!(
                "domain {id} ({name}): registry capture_scheduled={registry_capture_scheduled}, \
                 compact_contract descheduled={descriptor_descheduled}"
            ));
        }

        if let Some(wire_name) = membership.get(&id) {
            roster.insert(id, (name, *wire_name));
        }
    }
    for (id, wire_name) in &membership {
        if !roster.contains_key(id) {
            problems.push(format!(
                "domain {id} ({wire_name}) is in the passive production roster but \
                 domain_registry.json has no row for it"
            ));
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

    if as_json {
        let rows: Vec<serde_json::Value> = roster
            .iter()
            .map(|(id, (name, wire))| {
                serde_json::json!({ "id": id, "name": name, "wire_name": wire })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&rows).expect("roster serializes")
        );
    } else {
        for (_, (_, wire)) in &roster {
            println!("{wire}");
        }
    }
}

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
