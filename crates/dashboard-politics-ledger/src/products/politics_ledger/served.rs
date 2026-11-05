//! The political and command power ledger's served replies, as its widgets ask them: every
//! country's spending totals and last day's balance, and some countries' spending, balance days,
//! income and spending parts, governments and navigation through some hours.

use dashboard_base::prepared::served::{self, Ask, Plan};
use serde_json::{Value, json};
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

/// Pixels a summary or marks view draws over.
const WIDTH: u32 = 64;
/// Governments a page asks for, as the change list does.
const PAGE: u32 = 32;

fn day(hour: i32) -> i32 {
    hour.div_euclid(24) * 24
}

fn ledger(ask: &mut dyn Ask, params: Value) -> io::Result<()> {
    ask.ask("politics-ledger", params).map(drop)
}

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let id = branch.name();
        let (first, last) = (branch.first_hour, branch.last_hour);
        let countries = ask.countries(branch.id)?;
        for tag in &countries {
            ledger(
                ask,
                json!({ "branchId": id, "tag": tag, "view": "spending", "hour": last, "totals": true }),
            )?;
            ledger(
                ask,
                json!({ "branchId": id, "tag": tag, "view": "balance", "from": day(last), "hour": last }),
            )?;
        }
        for tag in served::some(&countries) {
            for hour in [first, branch.middle(), last] {
                let within = json!([{ "firstHour": first, "lastHour": hour }]);
                let navigation = |key: &str| {
                    json!({ "key": key, "from": hour, "firstHour": first, "lastHour": hour })
                };
                for totals in [false, true] {
                    ledger(
                        ask,
                        json!({ "branchId": id, "tag": tag, "view": "spending", "hour": hour, "totals": totals }),
                    )?;
                    ledger(
                        ask,
                        json!({
                            "branchId": id, "tag": tag, "view": "spending", "from": day(hour),
                            "hour": hour, "totals": totals, "within": within,
                        }),
                    )?;
                }
                ledger(
                    ask,
                    json!({ "branchId": id, "tag": tag, "view": "balance", "from": day(hour), "hour": hour }),
                )?;
                let bindings = Value::Object([(tag.clone(), within.clone())].into_iter().collect());
                for currency in ["Political", "Command"] {
                    for mode in ["income", "spending"] {
                        let summary = json!({
                            "from": first, "to": hour, "width": WIDTH, "currency": currency, "mode": mode,
                        });
                        ledger(
                            ask,
                            json!({
                                "branchId": id, "tag": tag, "view": "balance", "hour": hour,
                                "summary": summary, "bindings": bindings,
                            }),
                        )?;
                        for key in ["ArrowLeft", "ArrowRight"] {
                            ledger(
                                ask,
                                json!({
                                    "branchId": id, "tag": tag, "view": "balance", "hour": hour,
                                    "summary": summary, "bindings": bindings, "navigation": navigation(key),
                                }),
                            )?;
                        }
                    }
                }
                ledger(
                    ask,
                    json!({
                        "branchId": id, "tag": tag, "view": "governments", "from": first, "hour": hour,
                        "marks": { "from": first, "to": hour, "width": WIDTH },
                    }),
                )?;
                ledger(
                    ask,
                    json!({
                        "branchId": id, "tag": tag, "view": "governments", "from": first, "hour": hour,
                        "matches": true, "within": within,
                    }),
                )?;
                ledger(
                    ask,
                    json!({ "branchId": id, "tag": tag, "view": "governments", "from": day(hour), "hour": hour }),
                )?;
                ledger(
                    ask,
                    json!({
                        "branchId": id, "tag": tag, "view": "governments", "hour": hour,
                        "page": { "from": first, "to": hour, "cursor": "0", "pageSize": PAGE },
                    }),
                )?;
                for key in ["ArrowLeft", "ArrowRight"] {
                    ledger(
                        ask,
                        json!({
                            "branchId": id, "tag": tag, "view": "governments", "hour": hour,
                            "navigation": navigation(key),
                        }),
                    )?;
                }
            }
        }
    }
    Ok(())
}
