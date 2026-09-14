//! Which replay format versions this build reads, and what each one recorded.
//!
//! Format 14 records 148 tables, format 17 records 155, and format 18 adds
//! ProvinceVisibility at id155. These supported layouts retain the earlier
//! table encodings, container fields, keyframe rules and CRC checks.
//!
//! A frame body is a `table_ns` entry per recorded table followed by one update
//! per recorded table in `TableId` order. The timing and table arrays each use
//! the version's prefix of TableId::ALL; the entire frame is not a byte prefix.
//! Reading a supported old file needs no per-table decoding differences, only
//! its table count. Absent tables use [`FormatVersion::missing_reason`].
//!
//! Formats 15 and 16 were written during development but are not supported by
//! this compatibility reader. Compatibility applies to finalized replay files,
//! not old spool segments: H4SI spool records use [`FormatVersion::CURRENT`].

use crate::{FORMAT_VERSION, codec::invalid, frame::TABLE_COUNT, rows::TableId};
use std::io;

/// A replay format version this build can decode.
///
/// Construct one only through [`FormatVersion::read`], so a table count can
/// never be invented for an unsupported version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatVersion {
    version: u16,
    recorded_tables: usize,
}

impl FormatVersion {
    /// The only version the writer emits.
    pub const CURRENT: Self = Self {
        version: FORMAT_VERSION,
        recorded_tables: TABLE_COUNT,
    };

    /// The last version before the map tables were appended. It recorded
    /// `Campaign`..`CountryIntel`, which is every table up to and including id
    /// 147.
    pub const LEGACY_14: Self = Self {
        version: 14,
        recorded_tables: TableId::CountryIntel.index() + 1,
    };

    /// The map-addition format before ProvinceVisibility.
    pub const LEGACY_17: Self = Self {
        version: 17,
        recorded_tables: TableId::DivisionEta.index() + 1,
    };

    /// Every readable version, oldest first.
    pub const READABLE: [Self; 3] = [Self::LEGACY_14, Self::LEGACY_17, Self::CURRENT];

    /// Accepts a header's version word, or names every version that would work.
    pub fn read(version: u16) -> io::Result<Self> {
        Self::READABLE
            .into_iter()
            .find(|format| format.version == version)
            .ok_or_else(|| {
                let readable: Vec<_> = Self::READABLE
                    .iter()
                    .map(|format| format.version.to_string())
                    .collect();
                invalid(format!(
                    "unsupported replay format version {version}; this build reads {} and writes \
                     {FORMAT_VERSION}",
                    readable.join(", ")
                ))
            })
    }

    pub const fn version(self) -> u16 {
        self.version
    }

    /// How many leading `TableId::ALL` entries this version's frames carry.
    pub const fn recorded_tables(self) -> usize {
        self.recorded_tables
    }

    pub const fn records(self, table: TableId) -> bool {
        table.index() < self.recorded_tables
    }

    /// The unavailable reason a table this version never had reads back with.
    pub fn missing_reason(self) -> String {
        format!("not recorded by format {}", self.version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_readable_version_stops_at_a_real_table_boundary() {
        for format in FormatVersion::READABLE {
            assert!(format.recorded_tables > 0);
            assert!(format.recorded_tables <= TABLE_COUNT);
        }
        assert_eq!(FormatVersion::LEGACY_14.recorded_tables, 148);
        assert_eq!(FormatVersion::LEGACY_17.recorded_tables, 155);
        assert_eq!(FormatVersion::CURRENT.recorded_tables, 156);
        // The writer's version must stay readable, or a file this build wrote
        // could not be reopened.
        assert_eq!(
            FormatVersion::read(FORMAT_VERSION).unwrap(),
            FormatVersion::CURRENT
        );
    }

    #[test]
    fn format_14_records_the_old_tables_and_none_of_the_map_tables() {
        let old = FormatVersion::LEGACY_14;
        assert!(old.records(TableId::Provinces));
        assert!(old.records(TableId::CountryIntel));
        for table in [
            TableId::ProvinceNames,
            TableId::LandCombatProgress,
            TableId::DivisionTemplates,
            TableId::DivisionMovement,
            TableId::DivisionStatus,
            TableId::StateFlags,
            TableId::DivisionEta,
            TableId::ProvinceVisibility,
        ] {
            assert!(!old.records(table), "{table:?} postdates format 14");
            assert!(FormatVersion::CURRENT.records(table));
        }
        assert_eq!(old.missing_reason(), "not recorded by format 14");
    }

    #[test]
    fn format_17_records_all_prior_map_tables_but_not_visibility() {
        let old = FormatVersion::LEGACY_17;
        for table in TableId::ALL {
            assert_eq!(old.records(table), table != TableId::ProvinceVisibility);
            assert!(FormatVersion::CURRENT.records(table));
        }
        assert_eq!(old.missing_reason(), "not recorded by format 17");
    }

    #[test]
    fn unsupported_versions_are_refused() {
        // Development formats15/16 existed but have no supported decoder here.
        for version in [0, 13, 15, 16, 19, u16::MAX] {
            let error = FormatVersion::read(version).unwrap_err().to_string();
            assert_eq!(
                error,
                format!(
                    "unsupported replay format version {version}; this build reads 14, 17, 18 and \
                     writes 18"
                )
            );
        }
    }
}
