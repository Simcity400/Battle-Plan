
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Path {
    #[default]
    Changes,
    AllScopes(AllScopes),
    Complete(Complete),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AllScopes {
    NoKeptHour,
    NoCaptureDelta,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Complete {
    NoBaseline,
    NotComplete,
    Refused,
}

const PATHS: [(&str, Path); 6] = [
    ("changes", Path::Changes),
    (
        "all_scopes_no_kept_hour",
        Path::AllScopes(AllScopes::NoKeptHour),
    ),
    (
        "all_scopes_no_capture_delta",
        Path::AllScopes(AllScopes::NoCaptureDelta),
    ),
    ("complete_no_baseline", Path::Complete(Complete::NoBaseline)),
    (
        "complete_not_complete",
        Path::Complete(Complete::NotComplete),
    ),
    ("complete_refused", Path::Complete(Complete::Refused)),
];

impl Path {
    fn slot(self) -> usize {
        PATHS
            .iter()
            .position(|&(_, path)| path == self)
            .expect("every path is listed")
    }

    fn name(self) -> &'static str {
        PATHS[self.slot()].0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VariableWork {
    pub path: Path,
    pub scopes: u64,
    pub reordered: bool,
    pub scopes_compared: u64,
    pub entries_compared: u64,
    pub bytes_compared: u64,
    pub rows_built: u64,
    pub rows_changed: u64,
    pub entry_sets_changed: u64,
    pub scopes_added: u64,
    pub scopes_removed: u64,
    pub compare_ns: u64,
    pub build_ns: u64,
    pub ns: u64,
}

impl VariableWork {
    fn select_ns(&self) -> u64 {
        self.ns
            .saturating_sub(self.compare_ns)
            .saturating_sub(self.build_ns)
    }

    pub fn status(&self) -> impl serde::Serialize + '_ {
        crate::status::status!({
            "path": self.path.name(),
            "scopes": self.scopes,
            "reordered": self.reordered,
            "scopes_compared": self.scopes_compared,
            "entries_compared": self.entries_compared,
            "bytes_compared": self.bytes_compared,
            "rows_built": self.rows_built,
            "rows_changed": self.rows_changed,
            "entry_sets_changed": self.entry_sets_changed,
            "scopes_added": self.scopes_added,
            "scopes_removed": self.scopes_removed,
            "select_ns": self.select_ns(),
            "compare_ns": self.compare_ns,
            "build_ns": self.build_ns,
            "ns": self.ns,
        })
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct VariableTotals {
    hours: u64,
    paths: [u64; PATHS.len()],
    reordered: u64,
    sum: VariableWork,
    max: VariableWork,
    select_ns: [u64; 2],
    latest: Option<VariableWork>,
}

impl VariableTotals {
    pub fn add(&mut self, hour: &VariableWork) {
        self.hours += 1;
        self.paths[hour.path.slot()] += 1;
        self.reordered += u64::from(hour.reordered);
        for (sum, max, value) in [
            (
                &mut self.sum.scopes_compared,
                &mut self.max.scopes_compared,
                hour.scopes_compared,
            ),
            (
                &mut self.sum.entries_compared,
                &mut self.max.entries_compared,
                hour.entries_compared,
            ),
            (
                &mut self.sum.bytes_compared,
                &mut self.max.bytes_compared,
                hour.bytes_compared,
            ),
            (
                &mut self.sum.rows_built,
                &mut self.max.rows_built,
                hour.rows_built,
            ),
            (
                &mut self.sum.rows_changed,
                &mut self.max.rows_changed,
                hour.rows_changed,
            ),
            (
                &mut self.sum.entry_sets_changed,
                &mut self.max.entry_sets_changed,
                hour.entry_sets_changed,
            ),
            (
                &mut self.sum.scopes_added,
                &mut self.max.scopes_added,
                hour.scopes_added,
            ),
            (
                &mut self.sum.scopes_removed,
                &mut self.max.scopes_removed,
                hour.scopes_removed,
            ),
            (
                &mut self.sum.compare_ns,
                &mut self.max.compare_ns,
                hour.compare_ns,
            ),
            (
                &mut self.sum.build_ns,
                &mut self.max.build_ns,
                hour.build_ns,
            ),
            (&mut self.sum.ns, &mut self.max.ns, hour.ns),
        ] {
            *sum = sum.saturating_add(value);
            *max = (*max).max(value);
        }
        self.max.scopes = self.max.scopes.max(hour.scopes);
        let select = hour.select_ns();
        self.select_ns = [
            self.select_ns[0].saturating_add(select),
            self.select_ns[1].max(select),
        ];
        self.latest = Some(*hour);
    }

    pub fn status(&self) -> impl serde::Serialize + '_ {
        let counts = |work: &VariableWork, select_ns: u64| {
            crate::status::status!({
                "scopes_compared": work.scopes_compared,
                "entries_compared": work.entries_compared,
                "bytes_compared": work.bytes_compared,
                "rows_built": work.rows_built,
                "rows_changed": work.rows_changed,
                "entry_sets_changed": work.entry_sets_changed,
                "scopes_added": work.scopes_added,
                "scopes_removed": work.scopes_removed,
                "select_ns": select_ns,
                "compare_ns": work.compare_ns,
                "build_ns": work.build_ns,
                "ns": work.ns,
            })
        };
        crate::status::status!({
            "hours": self.hours,
            "paths": crate::status::map(
                PATHS.iter().zip(self.paths).map(|(&(name, _), hours)| (name, hours)),
            ),
            "reordered_hours": self.reordered,
            "max_scopes": self.max.scopes,
            "sum": counts(&self.sum, self.select_ns[0]),
            "max": counts(&self.max, self.select_ns[1]),
            "latest": self.latest.as_ref().map(VariableWork::status),
            "scope": "ScriptVariables conversion on the spool's first helper: per hour, the path that decided the input, the scopes, entries and bytes compared with their baseline rows, the rows built and changed, scopes added and removed, and the time selecting, comparing and building; summed over the session, each hour's largest, and the latest hour",
        })
    }
}
