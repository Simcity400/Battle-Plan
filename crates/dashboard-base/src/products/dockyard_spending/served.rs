//! Dockyard spending's served replies: some countries' new ships, refits and repairs through the
//! whole branch and each of its spans, in both views.

use crate::prepared::served::{self, Ask, Plan};
use serde_json::json;
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let countries = ask.countries(branch.id)?;
        let mut windows = vec![(None, None, None)];
        windows.extend(
            branch
                .spans()
                .into_iter()
                .map(|(from, to)| (Some(from), Some(to), Some(to))),
        );
        for tag in served::some(&countries) {
            for &(from, hour, end) in &windows {
                for view in ["circle", "months"] {
                    ask.ask(
                        "dockyard-spending",
                        json!({
                            "branchId": id, "tag": tag, "view": view, "from": from,
                            "hour": hour, "end": end,
                        }),
                    )?;
                }
            }
        }
    }
    Ok(())
}
