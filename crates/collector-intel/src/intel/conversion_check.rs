use super::VariableBaselines;
use replay_data::{
    Hour,
    rows::{
        Recorded, RowCodec, ScriptVariableNameRow, ScriptVariableValueRow, ScriptVariablesRow,
        TableSchema, tables,
    },
};
use replay_frame::frame::{TableInput, TableView};
use std::collections::BTreeMap;

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

type LogicalScope<'a> = (Recorded<Option<u32>>, BTreeMap<&'a [u8], i64>);
type LogicalState<'a> = BTreeMap<(u8, u32), LogicalScope<'a>>;

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
        baselines: VariableBaselines<'_>,
        reference: &[TableInput; 3],
        inputs: &[TableInput; 3],
    ) {
        self.hours += 1;
        let hour = baselines.scopes.map(|(token, _)| token.destination().2);
        let before = self.differences;
        match (
            reconstruct(baselines, reference),
            reconstruct(baselines, inputs),
        ) {
            (Some(expected), Some(actual)) => self.compare(hour, &expected, &actual),
            (None, None) if reference == inputs => {}
            _ => self.differ(hour, None, "outcome"),
        }
        if self.differences != before {
            self.hours_differing += 1;
        }
    }

    fn compare(
        &mut self,
        hour: Option<Hour>,
        expected: &LogicalState<'_>,
        actual: &LogicalState<'_>,
    ) {
        let mut expected_iter = expected.iter();
        let mut actual_iter = actual.iter();
        loop {
            match (expected_iter.next(), actual_iter.next()) {
                (None, None) => return,
                (Some((ek, (eseed, eentries))), Some((ak, (aseed, aentries)))) if ek == ak => {
                    if eentries != aentries {
                        self.differ(hour, Some(*ek), "entries");
                    }
                    if eseed != aseed {
                        self.differ(hour, Some(*ek), "seed");
                    }
                }
                (expected, actual) => {
                    self.differ(hour, expected.or(actual).map(|(k, _)| *k), "scope");
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

fn reconstruct<'a>(
    baselines: VariableBaselines<'a>,
    inputs: &'a [TableInput; 3],
) -> Option<LogicalState<'a>> {
    let scopes = Held::<tables::ScriptVariables>::of(
        &inputs[0],
        baselines.scopes.map(|(_, rows)| rows),
    )?;
    let names = Held::<tables::ScriptVariableNames>::of(
        &inputs[1],
        baselines.names.map(|(_, rows)| rows),
    )?;
    let values = Held::<tables::ScriptVariableValues>::of(
        &inputs[2],
        baselines.values.map(|(_, rows)| rows),
    )?;
    let name_map: BTreeMap<u32, &'a [u8]> = names.map(|row| (row.id, row.name.as_slice())).collect();
    let mut state: LogicalState<'a> = BTreeMap::new();
    for scope in scopes {
        let mut entries = BTreeMap::new();
        for entry in &scope.entries {
            entries.insert(entry.name.as_slice(), entry.value_raw);
        }
        state.insert((scope.scope_kind, scope.scope_id), (scope.seed, entries));
    }
    for value in values {
        let &name = name_map.get(&value.var_id)?;
        let (_, entries) = state.get_mut(&(value.scope_kind, value.scope_id))?;
        entries.insert(name, value.value_raw);
    }
    Some(state)
}

struct Held<'a, T: TableSchema> {
    rows: &'a [T::Row],
    next: usize,
    indices: &'a [usize],
    replacements: &'a [T::Row],
    keys: &'a [u128],
    fresh: &'a [T::Row],
    next_fresh: usize,
}

impl<'a, T: TableSchema> Held<'a, T> {
    fn of(input: &'a TableInput, previous: Option<&'a [T::Row]>) -> Option<Self> {
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
                let replacements = replacements.rows::<T>()?;
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
                    fresh: update.rows().rows::<T>()?,
                    ..rows(previous?)
                })
            }
            TableInput::Sampled(TableView::Complete(complete)) => Some(rows(complete.rows::<T>()?)),
            TableInput::SharedComplete(complete) => Some(rows(complete.rows::<T>()?)),
            _ => None,
        }
    }
}

impl<'a, T: TableSchema> Iterator for Held<'a, T>
where
    T::Row: RowCodec,
{
    type Item = &'a T::Row;

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
    baselines: VariableBaselines<'_>,
    reference: &[TableInput; 3],
    inputs: &[TableInput; 3],
) {
    CHECK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .record(baselines, reference, inputs);
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
