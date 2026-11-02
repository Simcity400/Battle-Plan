use replay_columnar::manifest::Manifest;
use replay_data::{
    cadence::{self, Cadence},
    format::FormatVersion,
    rows::{TABLE_COUNT, TableId},
};

pub struct Reads {
    groups: Vec<(Cadence, Vec<TableId>)>,
}

impl Reads {
    pub fn new(format: FormatVersion) -> Self {
        let mut groups = Vec::<(Cadence, Vec<TableId>)>::new();
        for id in TableId::ALL {
            if id == TableId::CountryIntel {
                continue;
            }
            let cadence = cadence::cadence_in(id, format);
            if let Some((_, tables)) = groups.iter_mut().find(|(c, _)| *c == cadence) {
                tables.push(id);
            } else {
                groups.push((cadence, vec![id]));
            }
        }
        Self { groups }
    }

    pub fn supplied(&self, manifest: &Manifest<'_>) -> [bool; TABLE_COUNT] {
        let mut flags = [false; TABLE_COUNT];
        for (cadence, tables) in &self.groups {
            let read = cadence.is_refresh(manifest.hour)
                || tables.iter().any(|&id| acquired(manifest, id));
            for &id in tables {
                flags[id.index()] = read;
            }
        }
        flags[TableId::CountryIntel.index()] = acquired(manifest, TableId::CountryIntel);
        flags
    }
}

pub fn acquired(manifest: &Manifest<'_>, table: TableId) -> bool {
    manifest.read[table.index() / 64] >> (table.index() % 64) & 1 != 0
}
