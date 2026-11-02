//! The military air ledger's served replies: for each family, whether it read any hour, every
//! leaf over the branch and from its middle, a cause and region selection, and the top ranked
//! countries by each metric.

use dashboard_base::prepared::served::{Ask, Plan};
use serde_json::json;
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// The countries a ranked selection names.
const TOP: usize = 10;

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        for family in ["airLosses", "bombing"] {
            ask.ask(
                "military-air-ledger",
                json!({ "branchId": id, "family": family, "availability": true }),
            )?;
            for from in [None, Some(branch.middle())] {
                ask.ask(
                    "military-air-ledger",
                    json!({ "branchId": id, "family": family, "fromHour": from }),
                )?;
                for metric in ["dealt", "taken", "total"] {
                    ask.ask(
                        "military-air-ledger",
                        json!({
                            "branchId": id, "family": family, "fromHour": from,
                            "top": { "count": TOP, "metric": metric },
                        }),
                    )?;
                }
            }
        }
        ask.ask(
            "military-air-ledger",
            json!({ "branchId": id, "family": "airLosses", "causes": [1], "regions": [] }),
        )?;
        ask.ask(
            "military-air-ledger",
            json!({ "branchId": id, "family": "bombing", "causes": [1, 2] }),
        )?;
    }
    Ok(())
}
