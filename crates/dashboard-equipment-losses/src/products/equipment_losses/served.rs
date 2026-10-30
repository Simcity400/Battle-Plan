//! Equipment losses' served replies: every country's whole history by
//! destruction, and some countries' by both sources over each span and at
//! every hour read together.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

const SOURCES: [&str; 2] = ["destruction", "combatLog"];
/// Pixels a summary draws over.
const WIDTH: u32 = 64;

/// `tag`'s losses by `source` drawn over `from` to `to`, read exactly at
/// `reads`, with its head and detail hours.
fn losses(
    branch: &str,
    (tag, source): (&str, &str),
    (from, to, width): (i32, i32, u32),
    reads: &[i32],
    (head, detail): (i32, i32),
) -> Value {
    json!({
        "selection": { "branchId": branch, "tag": tag, "landSource": source },
        "from": from, "to": to, "width": width, "reads": reads,
        "headHour": head, "detailHour": detail,
    })
}

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        let countries = ask.countries(branch.id)?;
        for tag in &countries {
            ask.ask(
                "equipment-losses",
                losses(
                    &id,
                    (tag, "destruction"),
                    (first, last, WIDTH),
                    &[first, last],
                    (last, last),
                ),
            )?;
        }
        let hours = branch.hours();
        for tag in served::some(&countries) {
            for source in SOURCES {
                for (from, to) in branch.spans() {
                    let mut reads = vec![from, to];
                    reads.dedup();
                    ask.ask(
                        "equipment-losses",
                        losses(&id, (&tag, source), (from, to, WIDTH), &reads, (to, to)),
                    )?;
                }
                ask.ask(
                    "equipment-losses",
                    losses(
                        &id,
                        (&tag, source),
                        (first, last, 1),
                        &hours,
                        (last, branch.middle()),
                    ),
                )?;
            }
        }
    }
    Ok(())
}
