
use replay_data::{Hour, rows::ScriptVariablesRow, rows::tables};
use replay_frame::frame::{BaselineToken, TableInput, TableView};

const NAMED: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Difference {
    pub hour: Option<Hour>,
    pub scope: Option<(u8, u32)>,
    pub column: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct Check {
    pub hours: u64,
    pub hours_differing: u64,
    pub differences: u64,
    pub first: [Option<Difference>; NAMED],
}

impl Default for Check {
    fn default() -> Self {
        Self::new()
    }
}

impl Check {
    pub const fn new() -> Self {
        Self {
            hours: 0,
            hours_differing: 0,
            differences: 0,
            first: [None; NAMED],
        }
    }

    pub fn record(
        &mut self,
        baseline: Option<(BaselineToken, &[ScriptVariablesRow])>,
        reference: &TableInput,
        input: &TableInput,
    ) {
        self.hours += 1;
        let hour = baseline.map(|(token, _)| token.destination().2);
        let previous = baseline.map(|(_, rows)| rows);
        let before = self.differences;
        match (Held::of(reference, previous), Held::of(input, previous)) {
            (Some(expected), Some(actual)) => self.compare(hour, expected, actual),
            (None, None) if reference == input => {}
            _ => self.differ(hour, None, "outcome"),
        }
        if self.differences != before {
            self.hours_differing += 1;
        }
    }

    fn compare(&mut self, hour: Option<Hour>, mut expected: Held<'_>, mut actual: Held<'_>) {
        loop {
            match (expected.next(), actual.next()) {
                (None, None) => return,
                (Some(expected), Some(actual)) if key(expected) == key(actual) => {
                    if expected.entries != actual.entries {
                        self.differ(hour, Some(key(expected)), "entries");
                    }
                    if expected.seed != actual.seed {
                        self.differ(hour, Some(key(expected)), "seed");
                    }
                }
                (expected, actual) => {
                    self.differ(hour, expected.or(actual).map(key), "scope");
                    return;
                }
            }
        }
    }

    fn differ(&mut self, hour: Option<Hour>, scope: Option<(u8, u32)>, column: &'static str) {
        if let Some(slot) = self.first.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(Difference {
                hour,
                scope,
                column,
            });
        }
        self.differences += 1;
    }
}

fn key(row: &ScriptVariablesRow) -> (u8, u32) {
    (row.scope_kind, row.scope_id)
}

struct Held<'a> {
    rows: &'a [ScriptVariablesRow],
    next: usize,
    indices: &'a [usize],
    replacements: &'a [ScriptVariablesRow],
    keys: &'a [u128],
    fresh: &'a [ScriptVariablesRow],
    next_fresh: usize,
}

impl<'a> Held<'a> {
    fn of(input: &'a TableInput, previous: Option<&'a [ScriptVariablesRow]>) -> Option<Self> {
        let rows = |rows| Self {
            rows,
            next: 0,
            indices: &[],
            replacements: &[],
            keys: &[],
            fresh: &[],
            next_fresh: 0,
        };
        match input {
            TableInput::ComparedEqual(_) => Some(rows(previous?)),
            TableInput::ComparedChanges {
                indices,
                rows: replacements,
                ..
            } => {
                let replacements = replacements.rows::<tables::ScriptVariables>()?;
                (replacements.len() == indices.len()).then_some(Self {
                    indices,
                    replacements,
                    ..rows(previous?)
                })
            }
            TableInput::SliceUpdate { update, .. }
                if update.ranges.is_empty() && update.owners().is_empty() =>
            {
                Some(Self {
                    keys: &update.keys,
                    fresh: update.rows().rows::<tables::ScriptVariables>()?,
                    ..rows(previous?)
                })
            }
            TableInput::Sampled(TableView::Complete(complete)) => {
                Some(rows(complete.rows::<tables::ScriptVariables>()?))
            }
            TableInput::SharedComplete(complete) => {
                Some(rows(complete.rows::<tables::ScriptVariables>()?))
            }
            _ => None,
        }
    }
}

impl<'a> Iterator for Held<'a> {
    type Item = &'a ScriptVariablesRow;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let fresh = self.fresh.get(self.next_fresh);
            let Some(row) = self.rows.get(self.next) else {
                self.next_fresh += 1;
                return fresh;
            };
            if let Some(fresh) = fresh.filter(|fresh| fresh.key() < row.key()) {
                self.next_fresh += 1;
                return Some(fresh);
            }
            let at = self.next;
            self.next += 1;
            if self.keys.binary_search(&row.key()).is_ok() {
                continue;
            }
            return match self.indices.binary_search(&at) {
                Ok(replacement) => self.replacements.get(replacement),
                Err(_) => Some(row),
            };
        }
    }
}

#[cfg(feature = "conversion_check")]
static CHECK: std::sync::Mutex<Check> = std::sync::Mutex::new(Check::new());

#[cfg(feature = "conversion_check")]
pub fn record(
    baseline: Option<(BaselineToken, &[ScriptVariablesRow])>,
    reference: &TableInput,
    input: &TableInput,
) {
    CHECK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .record(baseline, reference, input);
}

#[cfg(feature = "conversion_check")]
pub fn status() -> impl serde::Serialize {
    let check = *CHECK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::status::status!({
        "table": "ScriptVariables",
        "hours": check.hours,
        "hours_differing": check.hours_differing,
        "differences": check.differences,
        "first": crate::status::sequence(check.first.into_iter().flatten().map(|difference| {
            crate::status::status!({
                "hour": difference.hour,
                "scope_kind": difference.scope.map(|scope| scope.0),
                "scope_id": difference.scope.map(|scope| scope.1),
                "column": difference.column,
            })
        })),
    })
}
