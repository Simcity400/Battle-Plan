//! The catalogue's served reply: campaign-wide, asked once.

use dashboard_base::prepared::served::{Ask, Plan};
use serde_json::json;
use std::io;

pub static PLAN: Plan = Plan {
    queries,
    unordered: &[],
};

fn queries(ask: &mut dyn Ask) -> io::Result<()> {
    ask.ask("land-owners", json!({}))?;
    Ok(())
}
