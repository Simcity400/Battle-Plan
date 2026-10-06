//! Proven field, wire and meaning history; no translation or old decoders.
use super::TableId;

/// Earlier history has not been audited for today's readers.
pub const PROVEN_FROM: u16 = 91;
/// Private format 95 was retired when #343 merged main's format 96; its
/// divergent table contracts were never published (PLAN format ledger).
pub const RETIRED_FORMAT: u16 = 95;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordKind {
    ReplayHeader,
    ReplayBlock,
    ReplayTrailer,
    ReplayFooter,
    SpoolEnvelope,
    Frame,
    Identity,
    SessionStart,
    BranchStart,
    Gap,
    RecoveryNote,
    Detach,
    ClockWitness,
    GapSummary,
    Partial,
}

impl RecordKind {
    pub const ALL: [Self; 15] = [
        Self::ReplayHeader,
        Self::ReplayBlock,
        Self::ReplayTrailer,
        Self::ReplayFooter,
        Self::SpoolEnvelope,
        Self::Frame,
        Self::Identity,
        Self::SessionStart,
        Self::BranchStart,
        Self::Gap,
        Self::RecoveryNote,
        Self::Detach,
        Self::ClockWitness,
        Self::GapSummary,
        Self::Partial,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subject {
    Table(TableId),
    Record(RecordKind),
}

#[derive(Clone, Copy, Debug)]
pub struct LastChange {
    pub subject: Subject,
    /// Table lifetime in the timing/update stream. Baseline entries were
    /// already present at PROVEN_FROM; their earlier introduction is unaudited.
    pub introduced: u16,
    /// Retain retired TableIds and their field framing as wire tombstones.
    /// Never reuse an id or shift the remaining timing/update positions.
    pub retired: Option<u16>,
    /// For baseline entries: unchanged since this proven horizon, possibly older.
    pub format: u16,
    pub commit: &'static str,
    pub kind: ChangeKind,
    pub appended: &'static [AppendedField],
}

/// All audited row diffs preserve keys, field kinds and prefix order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Unchanged,
    AppendedOnly,
    Layout,
    Meaning,
}

#[derive(Clone, Copy, Debug)]
pub struct AppendedField {
    pub index: usize,
    pub name: &'static str,
    pub introduced: u16,
}

/// One code-owned catalog for tables and every framing/metadata/control kind.
/// Evidence: row declarations and typed_codec diffs at each listed commit,
/// including the wider DivisionStatus mask (the recorded field count owns it).
/// The baseline is #302; #323/#325/#328/#341/#343 were audited against it. New
/// fields were appended; nested Opaque values keep their byte-length framing.
/// Record-kind dates 1/24/33 are inert below the proven horizon of 91.
/// Adding/retiring a table changes its lifetime, not the Frame framing date.
pub static HISTORY: [LastChange; TableId::ALL.len() + RecordKind::ALL.len()] = {
    let baseline = LastChange {
        subject: Subject::Table(TableId::Clock),
        introduced: PROVEN_FROM,
        retired: None,
        format: PROVEN_FROM,
        commit: "0e632a9b16",
        kind: ChangeKind::Unchanged,
        appended: &[],
    };
    let mut history = [baseline; TableId::ALL.len() + RecordKind::ALL.len()];
    let mut i = 0;
    while i < TableId::ALL.len() {
        history[i].subject = Subject::Table(TableId::ALL[i]);
        i += 1;
    }
    let mut r = 0;
    while r < RecordKind::ALL.len() {
        let kind = RecordKind::ALL[r];
        let (format, commit) = match kind {
            RecordKind::SessionStart => (24, "9ae9a5498c"),
            RecordKind::SpoolEnvelope => (33, "1b7474a555"),
            RecordKind::Frame => (91, "0e632a9b16"),
            RecordKind::GapSummary => (1, "7185d6e386"),
            RecordKind::Partial => (92, "11b905d2b2"),
            _ => (1, "5d3b4dd945"),
        };
        history[i + r] = LastChange {
            subject: Subject::Record(kind),
            introduced: format,
            retired: None,
            format,
            commit,
            kind: if matches!(kind, RecordKind::Partial) {
                ChangeKind::Layout
            } else {
                ChangeKind::Unchanged
            },
            appended: &[],
        };
        r += 1;
    }
    history[TableId::Countries.index()] = table(TableId::Countries, 93, "9fb09a565f");
    history[TableId::LandCombatDetails.index()] =
        table(TableId::LandCombatDetails, 93, "9fb09a565f");
    history[TableId::UnitLeaders.index()] = table(TableId::UnitLeaders, 93, "9fb09a565f");
    history[TableId::SimulationFeatures.index()] =
        table(TableId::SimulationFeatures, 93, "9fb09a565f");
    history[TableId::WeatherProvinces.index()] =
        table(TableId::WeatherProvinces, 93, "9fb09a565f");
    history[TableId::StateFlags.index()] = table(TableId::StateFlags, 93, "9fb09a565f");
    history[TableId::DivisionStatus.index()] =
        table(TableId::DivisionStatus, 93, "9fb09a565f");
    // WING_NAME extended the meaning of the existing packed entry family.
    history[TableId::RaidTooltipOperands.index()] =
        table(TableId::RaidTooltipOperands, 94, "ccf740b81d");
    history[TableId::DivisionMovement.index()] =
        table(TableId::DivisionMovement, 96, "3cb8d1c09b");
    history[TableId::CountryAppearance.index()] =
        table(TableId::CountryAppearance, 97, "d313f6a6e3");
    history[TableId::DivisionTemplates.index()] =
        table(TableId::DivisionTemplates, 97, "d313f6a6e3");
    history[TableId::Decisions.index()] = table(TableId::Decisions, 97, "d313f6a6e3");
    history[TableId::Countries.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::Countries.index()].appended = &[AppendedField {
        index: 9,
        name: "border_land_doctrine_level",
        introduced: 93,
    }];
    history[TableId::LandCombatDetails.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::LandCombatDetails.index()].appended = &[AppendedField {
        index: 9,
        name: "border_war",
        introduced: 93,
    }];
    history[TableId::UnitLeaders.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::UnitLeaders.index()].appended = &[AppendedField {
        index: 20,
        name: "exile_country",
        introduced: 93,
    }];
    history[TableId::SimulationFeatures.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::SimulationFeatures.index()].appended = &[AppendedField {
        index: 2,
        name: "weather_suspend",
        introduced: 93,
    }];
    history[TableId::WeatherProvinces.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::WeatherProvinces.index()].appended = &[
        AppendedField {
            index: 4,
            name: "temperature",
            introduced: 93,
        },
        AppendedField {
            index: 5,
            name: "raw_mud",
            introduced: 93,
        },
    ];
    history[TableId::StateFlags.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::StateFlags.index()].appended = &[
        AppendedField {
            index: 1,
            name: "border_conflict_active",
            introduced: 93,
        },
        AppendedField {
            index: 2,
            name: "border_conflict_countries",
            introduced: 93,
        },
    ];
    history[TableId::DivisionStatus.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::DivisionStatus.index()].appended = &[AppendedField {
        index: 16,
        name: "border_combat_stats",
        introduced: 93,
    }];
    history[TableId::DivisionMovement.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::DivisionMovement.index()].appended = &[AppendedField {
        index: 14,
        name: "strategic_action",
        introduced: 96,
    }];
    history[TableId::CountryAppearance.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::CountryAppearance.index()].appended = &[AppendedField {
        index: 18,
        name: "subjects",
        introduced: 97,
    }];
    history[TableId::DivisionTemplates.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::DivisionTemplates.index()].appended = &[AppendedField {
        index: 3,
        name: "name_key",
        introduced: 97,
    }];
    history[TableId::Decisions.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::Decisions.index()].appended = &[AppendedField {
        index: 7,
        name: "description_seed",
        introduced: 97,
    }];
    history[TableId::RaidTooltipOperands.index()].kind = ChangeKind::Meaning;
    history
};

const fn table(id: TableId, format: u16, commit: &'static str) -> LastChange {
    LastChange {
        subject: Subject::Table(id),
        introduced: PROVEN_FROM,
        retired: None,
        format,
        commit,
        kind: ChangeKind::Layout,
        appended: &[],
    }
}

impl LastChange {
    pub const fn present(&self, format: u16) -> bool {
        format >= self.introduced
            && match self.retired {
                Some(retired) => format < retired,
                None => true,
            }
    }

    pub(crate) fn table_gap(&self, table: TableId, format: u16) -> Option<String> {
        if format < self.introduced {
            Some(format!(
                "{table:?} not recorded at format {format} (introduced at format {})",
                self.introduced
            ))
        } else if let Some(retired) = self
            .retired
            .filter(|retired| *retired <= crate::FORMAT_VERSION)
        {
            Some(format!(
                "{table:?} retired at format {retired}; this recording is format {format}"
            ))
        } else if format < self.format && self.kind != ChangeKind::AppendedOnly {
            Some(format!(
                "{table:?} layout or meaning changed at format {}; this recording is format {format}",
                self.format
            ))
        } else {
            None
        }
    }

    pub(crate) fn fields(&self, table: TableId, format: u16) -> usize {
        self.appended
            .iter()
            .find(|field| format < field.introduced)
            .map_or(table.field_count(), |field| field.index)
    }
}

impl TableId {
    pub const fn last_change(self) -> &'static LastChange {
        &HISTORY[self.index()]
    }
}

impl RecordKind {
    pub const fn last_change(self) -> &'static LastChange {
        &HISTORY[TableId::ALL.len() + self as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatibility_catalog_covers_every_table_and_record_once() {
        for table in TableId::ALL {
            let entry = table.last_change();
            assert_eq!(entry.subject, Subject::Table(table));
            assert!(!entry.commit.is_empty());
            assert!(entry.introduced <= entry.format);
            assert!(
                entry
                    .retired
                    .is_none_or(|retired| retired > entry.introduced)
            );
            if entry.kind == ChangeKind::AppendedOnly {
                assert!(!entry.appended.is_empty());
                let first = entry.appended[0].index;
                for (offset, field) in entry.appended.iter().enumerate() {
                    assert_eq!(field.index, first + offset);
                    assert!(field.introduced > PROVEN_FROM && field.introduced <= entry.format);
                }
                assert_eq!(first + entry.appended.len(), table.field_count());
            }
        }
        for kind in RecordKind::ALL {
            assert_eq!(kind.last_change().subject, Subject::Record(kind));
        }
        let distinct: std::collections::BTreeSet<_> = HISTORY
            .iter()
            .map(|entry| format!("{:?}", entry.subject))
            .collect();
        assert_eq!(distinct.len(), HISTORY.len());
    }
}
