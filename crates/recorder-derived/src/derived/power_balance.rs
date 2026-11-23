
use content_reader_layers::ResolvedLayers;
use content_reader_politics::static_modifiers::Descriptions;
use replay_data::{
    rows::ModifierBreakdownNode, rows::PowerBalanceModifierBreakdownRow, rows::tables,
};
use replay_derive::{modifier_registry::Registry, power_balance_derive as derive};
use replay_frame::frame::DecodedFrame;

use super::LISTED;

const DAILY: u32 = engine_executable::modifier_keys::POWER_BALANCE_DAILY;
const WEEKLY: u32 = engine_executable::modifier_keys::POWER_BALANCE_WEEKLY;

#[derive(Default)]
pub(super) struct Checker {
    statics: Descriptions,
    rows: u64,
    witnessed: u64,
    equal: u64,
    mismatches: u64,
    refused: u64,
    unavailable_hours: u64,
    listed: Vec<String>,
}

impl Checker {
    pub(super) fn observe(&mut self, frame: &DecodedFrame, layers: &ResolvedLayers) {
        let Ok(rows) = frame.complete::<tables::PowerBalanceModifierBreakdowns>() else {
            self.unavailable_hours += 1;
            return;
        };
        self.rows += rows.len() as u64;
        let mut witnessed = rows
            .iter()
            .filter(|row| !row.daily.is_empty() || !row.weekly.is_empty())
            .peekable();
        if witnessed.peek().is_none() {
            return;
        }
        let registry = Registry::from_frame(frame);
        for row in witnessed {
            self.witnessed += 1;
            let derived = registry
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|registry| self.derive(row, registry, layers));
            let line = match derived {
                Err(reason) => {
                    self.refused += 1;
                    format!(
                        "branch {} hour {} record {}: not derivable: {reason}",
                        frame.branch, frame.hour, row.record
                    )
                }
                Ok([daily, weekly])
                    if derive::witness_matches(&row.daily, &daily)
                        && derive::witness_matches(&row.weekly, &weekly) =>
                {
                    self.equal += 1;
                    continue;
                }
                Ok([daily, weekly]) => {
                    self.mismatches += 1;
                    format!(
                        "branch {} hour {} record {}: recorded daily {:?} weekly {:?}, derived daily {:?} weekly {:?}",
                        frame.branch,
                        frame.hour,
                        row.record,
                        summary(&row.daily),
                        summary(&row.weekly),
                        summary(&daily),
                        summary(&weekly)
                    )
                }
            };
            if self.listed.len() < LISTED {
                self.listed.push(line);
            }
        }
    }

    fn derive(
        &self,
        row: &PowerBalanceModifierBreakdownRow,
        registry: &Registry<'_>,
        layers: &ResolvedLayers,
    ) -> Result<[Vec<ModifierBreakdownNode>; 2], String> {
        let mut blocks = Vec::with_capacity(row.sources.len());
        for source in &row.sources {
            let key =
                std::str::from_utf8(content_reader_parser::parser::c_string(source)).map_err(|_| {
                    format!(
                        "static modifier {} is not UTF-8",
                        String::from_utf8_lossy(source)
                    )
                })?;
            blocks.push(self.statics.get(layers, key)?.1);
        }
        let inputs: Vec<_> = row
            .sources
            .iter()
            .zip(&blocks)
            .map(|(source, block)| derive::Source {
                name: source,
                auxiliary: !block.tooltip_table().is_empty() || !block.children.is_empty(),
                values: block
                    .values
                    .iter()
                    .map(|value| (value.name.as_str(), value.value))
                    .collect(),
            })
            .collect();
        Ok([
            derive::tree(DAILY, &inputs, registry)?,
            derive::tree(WEEKLY, &inputs, registry)?,
        ])
    }

    pub(super) fn failures(&self) -> [u64; 2] {
        [self.mismatches, self.refused + self.unavailable_hours]
    }

    pub(super) fn print(&self) {
        println!(
            "PowerBalanceModifierBreakdowns: {} rows ({} hours without a complete table), {} with native witness trees: {} derived equal, {} differing, {} not derivable on this content",
            self.rows,
            self.unavailable_hours,
            self.witnessed,
            self.equal,
            self.mismatches,
            self.refused
        );
        for line in &self.listed {
            println!("  power balance: {line}");
        }
    }
}

fn summary(nodes: &[ModifierBreakdownNode]) -> (u32, Vec<(String, i64)>) {
    (
        nodes.first().map_or(0, |root| root.children),
        nodes
            .iter()
            .skip(1)
            .map(|node| (String::from_utf8_lossy(&node.name).into_owned(), node.value))
            .collect(),
    )
}
