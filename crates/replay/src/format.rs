//! Readers accept proven unchanged contracts; writers emit only today's format.
use crate::{
    FORMAT_VERSION,
    codec::invalid,
    rows::{
        TableId,
        compatibility::{PROVEN_FROM, RETIRED_FORMAT, RecordKind},
    },
};
use std::io;

/// A recording version inside the proven compatibility horizon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatVersion(u16);

impl FormatVersion {
    pub const CURRENT: Self = Self(FORMAT_VERSION);

    pub fn read(version: u16) -> io::Result<Self> {
        if version < PROVEN_FROM {
            return Err(invalid(format!(
                "format {version} predates the proven compatibility history (from {PROVEN_FROM})"
            )));
        }
        if version > FORMAT_VERSION {
            return Err(invalid(format!(
                "recording format {version} is newer than the proven compatibility history (through {FORMAT_VERSION})"
            )));
        }
        if version == RETIRED_FORMAT {
            return Err(invalid(format!(
                "format {version} is retired and has no proven published recording contract"
            )));
        }
        let format = Self(version);
        for kind in [
            RecordKind::ReplayHeader,
            RecordKind::ReplayBlock,
            RecordKind::ReplayTrailer,
            RecordKind::ReplayFooter,
            RecordKind::Frame,
            RecordKind::Identity,
            RecordKind::SessionStart,
            RecordKind::BranchStart,
            RecordKind::Gap,
            RecordKind::RecoveryNote,
        ] {
            format.require(kind)?;
        }
        Ok(format)
    }

    pub const fn version(self) -> u16 {
        self.0
    }

    pub fn require(self, kind: RecordKind) -> io::Result<()> {
        let change = kind.last_change();
        if self.0 < change.format {
            Err(invalid(format!(
                "{kind:?} layout changed at format {}; this recording is format {}",
                change.format, self.0
            )))
        } else {
            Ok(())
        }
    }

    pub fn table_refusal(self, table: TableId) -> Option<String> {
        table.last_change().table_gap(table, self.0)
    }

    pub fn field_gaps(self, table: TableId) -> impl Iterator<Item = String> {
        table
            .last_change()
            .appended
            .iter()
            .filter(move |field| self.0 < field.introduced)
            .map(move |field| {
                format!(
                    "{table:?}.{} not recorded (introduced at format {}; this recording is format {}); other fields decode",
                    field.name, field.introduced, self.0
                )
            })
    }

    pub fn nested_field_gaps(self) -> impl Iterator<Item = String> {
        RecordKind::ALL.into_iter().flat_map(move |kind| {
            kind.last_change()
                .appended
                .iter()
                .filter(move |field| self.0 < field.introduced)
                .map(move |field| {
                    format!(
                        "{} not recorded (introduced at format {}; this recording is format {}); other fields decode",
                        field.name, field.introduced, self.0
                    )
                })
        })
    }

    /// Byte-boundary information for discarding an incompatible row, never
    /// decoding its contents or producing a row in a different contract.
    #[cfg(test)]
    pub(crate) fn fields(self, table: TableId) -> usize {
        table.last_change().fields(table, self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatibility_accepts_only_the_proven_horizon() {
        for version in 0..=u16::MAX {
            let result = FormatVersion::read(version);
            if (PROVEN_FROM..=FORMAT_VERSION).contains(&version) && version != RETIRED_FORMAT {
                assert_eq!(result.unwrap().version(), version);
            } else {
                assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
            }
        }
    }

    #[test]
    fn compatibility_meaning_change_refuses_same_layout() {
        let format = FormatVersion::read(93).unwrap();
        let table = TableId::RaidTooltipOperands;
        assert_eq!(format.fields(table), table.field_count());
        assert_eq!(
            format.table_refusal(table).unwrap(),
            "RaidTooltipOperands layout or meaning changed at format 94; this recording is format 93"
        );
        assert!(
            FormatVersion::read(94)
                .unwrap()
                .table_refusal(table)
                .is_none()
        );
    }

    #[test]
    fn compatibility_record_change_is_named() {
        let error = FormatVersion::read(91)
            .unwrap()
            .require(RecordKind::Partial)
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Partial layout changed at format 92; this recording is format 91"
        );
    }
}
