
pub mod agencies;
pub mod agency_content;
pub mod decryption;
pub mod espionage;
pub mod operative_content;
pub mod operative_matrix;
pub mod placements;
pub mod roster;

use dashboard_base::Source;
use dashboard_core::config::LoadContext;
use dashboard_text::cryptology;
use content_reader_localisation::localisation::Localisation;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::Serialize;
use std::{collections::HashMap, io, sync::Arc};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const IDENTITY: [TableId; 2] = [TableId::Countries, TableId::CountryLinks];

pub struct Tags {
    slots: HashMap<u32, u32>,
    tags: HashMap<u32, String>,
}

impl Tags {
    pub fn read(frame: &DecodedFrame) -> io::Result<Self> {
        let countries = frame.complete::<tables::Countries>()?;
        let links = frame.complete::<tables::CountryLinks>()?;
        let slots = dashboard_text::metadata::link_slots(links);
        let mut tags = HashMap::with_capacity(countries.len());
        for row in countries {
            if let Ok(tag) = std::str::from_utf8(&row.tag)
                && !tag.is_empty()
                && !tag.contains('\0')
            {
                tags.entry(row.canonical_slot).or_insert_with(|| tag.to_owned());
            }
        }
        Ok(Self { slots, tags })
    }

    pub fn slot(&self, raw: u32) -> Option<u32> {
        self.slots.get(&raw).copied()
    }

    pub fn tag(&self, raw: u32) -> Option<String> {
        self.slot(raw).and_then(|slot| self.tags.get(&slot).cloned())
    }

    pub fn present(&self) -> Vec<Country> {
        let mut least: HashMap<u32, u32> = HashMap::new();
        for (&raw, &slot) in &self.slots {
            if slot != 0 && self.tags.contains_key(&slot) {
                least.entry(slot).and_modify(|held| *held = (*held).min(raw)).or_insert(raw);
            }
        }
        let mut out: Vec<Country> = least.into_iter().map(|(slot, raw)| Country { id: raw, tag: self.tags.get(&slot).cloned() }).collect();
        out.sort_unstable_by(|a, b| a.tag.cmp(&b.tag).then(a.id.cmp(&b.id)));
        out
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Country {
    pub id: u32,
    pub tag: Option<String>,
}

impl Country {
    pub fn of(tags: &Tags, raw: u32) -> Self {
        Self { id: raw, tag: tags.tag(raw) }
    }
}

pub struct Content {
    pub localisation: Result<Arc<Localisation>, String>,
    pub defines: Result<cryptology::Defines, String>,
}

impl Content {
    pub fn load(source: &Source, load: &LoadContext) -> Self {
        let Some(content) = load.content.as_ref() else {
            const UNCONFIGURED: &str = "Game content is not configured";
            return Self { localisation: Err(UNCONFIGURED.into()), defines: Err(UNCONFIGURED.into()) };
        };
        let localisation = dashboard_base::content::localisation(source, content).map_err(|error| error.to_string());
        let defines = source
            .content(&content.options)
            .map_err(|error| error.to_string())
            .and_then(|bound| content_reader_defines::defines::read(&bound.layers).map_err(|error| error.to_string()))
            .and_then(|defines| cryptology::Defines::read(&defines));
        Self { localisation, defines }
    }

    pub fn text(&self, key: &[u8]) -> Option<String> {
        if key.is_empty() {
            return None;
        }
        if let Ok(localisation) = &self.localisation
            && let Ok(text) = localisation.localise(key)
        {
            return Some(text);
        }
        Some(String::from_utf8_lossy(key).into_owned())
    }
}

pub fn missing(frame: &DecodedFrame, table: TableId) -> Option<String> {
    dashboard_text::scope::unread(table, &frame.tables[table.index()])
}

pub fn branch(branch_id: &str) -> Result<(), &'static str> {
    branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
}

#[cfg(test)]
mod tests;
