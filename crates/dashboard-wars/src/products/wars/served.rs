//! Wars' served replies: the recorded viewpoint's and some countries' wars, and at every served
//! hour what the widgets read there: the wars' casualties, and the groups and contributions.

use dashboard_base::prepared::served::{self, Ask, Plan};
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
        let hours = branch.hours();
        let views = std::iter::once(None).chain(served::some(&countries).into_iter().map(Some));
        for view in views {
            ask.ask("wars", json!({ "branchId": id, "viewpointTag": view }))?;
            // Wars reads its playhead's casualties, and the contributions widget an hour's groups.
            for &hour in &hours {
                for read in ["reads", "current"] {
                    let mut params = json!({ "branchId": id, "viewpointTag": view });
                    params[read] = json!([{ "hour": hour }]);
                    ask.ask("wars", params)?;
                }
            }
        }
    }
    Ok(())
}
