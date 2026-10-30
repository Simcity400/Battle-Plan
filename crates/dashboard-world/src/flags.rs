//! What the game picks each country's flag by, beside its own tag: its cosmetic tag, its
//! original-or-own tag and its ruling party's ideology group. Kept per country object (its
//! raw link id, `Countries.id`) from the hour's changed rows of CountryAppearance, Politics
//! and PoliticalParties, each held only while complete. Choosing the flag is the reader's.

use super::{Facts, Tag, countries::edited, invalid};
use replay::{columns, segments::State};
use replay_data::{
    format::FormatVersion,
    rows::{CountryAppearanceRow, PoliticalPartyRow, PoliticsRow, Recorded, TableId},
};
use std::{collections::HashMap, io};

/// The tables flag inputs are read from.
pub const FLAG_TABLES: [TableId; 3] = [
    TableId::CountryAppearance,
    TableId::Politics,
    TableId::PoliticalParties,
];

/// A country's CountryAppearance operands of its flag, as recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Appearance {
    /// The empty tag when it has none.
    pub cosmetic: Tag,
    /// Positive: `original` is the original tag; otherwise its own tag is.
    pub reference: Recorded<Option<i32>>,
    pub original: Recorded<Option<Tag>>,
}

/// A country's flag inputs this hour. Each part is None without its row, or while its table
/// is not complete ([`Flags::complete`]): never a default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FlagInputs {
    pub appearance: Option<Appearance>,
    /// `Politics.ruling_party`: the inner None is a null ruling-party pointer.
    pub ruling: Option<Option<u32>>,
    /// The ruling party's `PoliticalParties.token`, its ideology group.
    pub group: Option<Tag>,
}

/// The columns flag inputs read.
struct Fields {
    appearance: u64,
    ruling: usize,
    token: usize,
}

/// Every country object's flag inputs, by raw id.
pub struct Flags {
    fields: Fields,
    complete: [bool; 3],
    appearances: HashMap<u32, Appearance>,
    ruling: HashMap<u32, Option<u32>>,
    tokens: HashMap<(u32, u32), Tag>,
    rebuilt: bool,
    changed: Vec<u32>,
}

impl Flags {
    pub(crate) fn new() -> io::Result<Self> {
        let appearance = |name| columns::field(TableId::CountryAppearance, name);
        Ok(Self {
            fields: Fields {
                appearance: 1 << appearance("cosmetic_tag")?
                    | 1 << appearance("original_tag")?
                    | 1 << appearance("original_tag_reference")?,
                ruling: columns::field(TableId::Politics, "ruling_party")?,
                token: columns::field(TableId::PoliticalParties, "token")?,
            },
            complete: [false; 3],
            appearances: HashMap::new(),
            ruling: HashMap::new(),
            tokens: HashMap::new(),
            rebuilt: false,
            changed: Vec::new(),
        })
    }

    /// Whether CountryAppearance, Politics and PoliticalParties each hold every row.
    pub fn complete(&self) -> [bool; 3] {
        self.complete
    }

    /// Country `id`'s flag inputs.
    pub fn inputs(&self, id: u32) -> FlagInputs {
        let ruling = self.ruling.get(&id).copied();
        FlagInputs {
            appearance: self.appearances.get(&id).copied(),
            ruling,
            group: ruling
                .flatten()
                .and_then(|party| self.tokens.get(&(id, party)).copied()),
        }
    }

    /// Whether the hour read every flag input anew: the walk's first hour, or one a flag
    /// table became complete or stopped being.
    pub fn rebuilt(&self) -> bool {
        self.rebuilt
    }

    /// The raw ids whose flag inputs the hour may have changed, ascending; see
    /// [`Self::rebuilt`] for the hours every one may have.
    pub fn changed(&self) -> &[u32] {
        &self.changed
    }

    /// Keeps of the changed ids those `country` holds a country of.
    pub(crate) fn keep(&mut self, country: impl Fn(u32) -> bool) {
        self.changed.retain(|&id| country(id));
    }

    fn note(&mut self, id: u32) -> io::Result<()> {
        engine_allocation::reserve(&mut self.changed, 1)?;
        self.changed.push(id);
        Ok(())
    }

    fn read_appearance(
        state: &State,
        slot: u32,
        format: FormatVersion,
        number: &mut dyn FnMut(&[u8]) -> io::Result<Tag>,
    ) -> io::Result<(u32, Appearance)> {
        // The original tag is an opaque cell: the row decoder reads it.
        let row: CountryAppearanceRow =
            columns::row(state, TableId::CountryAppearance, slot, format.version())?;
        let original = match &row.original_tag {
            Recorded::NotRecorded => Recorded::NotRecorded,
            Recorded::Value(None) => Recorded::Value(None),
            Recorded::Value(Some(bytes)) => Recorded::Value(Some(number(bytes)?)),
        };
        Ok((
            row.country,
            Appearance {
                cosmetic: number(&row.cosmetic_tag)?,
                reference: row.original_tag_reference,
                original,
            },
        ))
    }

    fn read_ruling(
        &self,
        state: &State,
        slot: u32,
        format: FormatVersion,
    ) -> io::Result<(u32, Option<u32>)> {
        let held = columns::table(state, TableId::Politics)?;
        let country = held
            .key(slot)
            .ok_or_else(|| invalid(format!("Politics slot {slot} holds no row")))?
            as u32;
        if !held.recorded(self.fields.ruling, slot) {
            let row: PoliticsRow = columns::row(state, TableId::Politics, slot, format.version())?;
            return Ok((country, row.ruling_party));
        }
        let (values, present) = columns::optional::<u32>(held, self.fields.ruling)?;
        let value = values
            .get(slot as usize)
            .ok_or_else(|| invalid(format!("Politics slot {slot} holds no cell")))?;
        Ok((country, present.get(slot).then_some(*value)))
    }

    fn read_token(
        &self,
        state: &State,
        slot: u32,
        format: FormatVersion,
        number: &mut dyn FnMut(&[u8]) -> io::Result<Tag>,
    ) -> io::Result<((u32, u32), Tag)> {
        let held = columns::table(state, TableId::PoliticalParties)?;
        let key = held
            .key(slot)
            .ok_or_else(|| invalid(format!("PoliticalParties slot {slot} holds no row")))?;
        let party = ((key >> 32) as u32, key as u32);
        if !held.recorded(self.fields.token, slot) {
            let row: PoliticalPartyRow =
                columns::row(state, TableId::PoliticalParties, slot, format.version())?;
            return Ok((party, number(&row.token)?));
        }
        let id = *columns::text(held, self.fields.token)?
            .get(slot as usize)
            .ok_or_else(|| invalid(format!("PoliticalParties slot {slot} holds no cell")))?;
        let bytes = state
            .dictionary()
            .get(id)
            .ok_or_else(|| invalid("A party's token names no dictionary entry"))?;
        Ok((party, number(bytes)?))
    }

    /// The hour's flag inputs; `number` numbers text as the countries' tags are.
    pub(crate) fn hour(
        &mut self,
        state: &State,
        facts: &Facts<'_>,
        format: FormatVersion,
        number: &mut dyn FnMut(&[u8]) -> io::Result<Tag>,
    ) -> io::Result<()> {
        self.rebuilt = false;
        self.changed.clear();
        let complete = FLAG_TABLES.map(|table| columns::complete(state, table));
        let anew = facts.first() || complete != self.complete;
        self.complete = complete;
        if anew {
            self.rebuilt = true;
            self.appearances.clear();
            self.ruling.clear();
            self.tokens.clear();
        }
        let [appearances, politics, parties] = complete;
        if appearances && (anew || facts.touched(FLAG_TABLES[0], self.fields.appearance)) {
            let table = FLAG_TABLES[0];
            if !anew {
                for &key in &columns::changes(state, table)?.deleted_keys {
                    if self.appearances.remove(&(key as u32)).is_some() {
                        self.note(key as u32)?;
                    }
                }
            }
            let slots: Vec<u32> = if anew {
                columns::table(state, table)?.live().ones().collect()
            } else {
                edited(state, table, self.fields.appearance)?
            };
            for slot in slots {
                let (id, appearance) = Self::read_appearance(state, slot, format, number)?;
                if self.appearances.insert(id, appearance) != Some(appearance) {
                    self.note(id)?;
                }
            }
        }
        if politics && (anew || facts.touched(FLAG_TABLES[1], 1 << self.fields.ruling)) {
            let table = FLAG_TABLES[1];
            if !anew {
                for &key in &columns::changes(state, table)?.deleted_keys {
                    if self.ruling.remove(&(key as u32)).is_some() {
                        self.note(key as u32)?;
                    }
                }
            }
            let slots: Vec<u32> = if anew {
                columns::table(state, table)?.live().ones().collect()
            } else {
                edited(state, table, 1 << self.fields.ruling)?
            };
            for slot in slots {
                let (id, ruling) = self.read_ruling(state, slot, format)?;
                if self.ruling.insert(id, ruling) != Some(ruling) {
                    self.note(id)?;
                }
            }
        }
        if parties && (anew || facts.touched(FLAG_TABLES[2], 1 << self.fields.token)) {
            let table = FLAG_TABLES[2];
            // A party reaches its country's flag only while it rules it.
            let rules = |ruling: &HashMap<u32, Option<u32>>, (country, party): (u32, u32)| {
                ruling.get(&country) == Some(&Some(party))
            };
            if !anew {
                for &key in &columns::changes(state, table)?.deleted_keys {
                    let party = ((key >> 32) as u32, key as u32);
                    if self.tokens.remove(&party).is_some() && rules(&self.ruling, party) {
                        self.note(party.0)?;
                    }
                }
            }
            let slots: Vec<u32> = if anew {
                columns::table(state, table)?.live().ones().collect()
            } else {
                edited(state, table, 1 << self.fields.token)?
            };
            for slot in slots {
                let (party, token) = self.read_token(state, slot, format, number)?;
                if self.tokens.insert(party, token) != Some(token) && rules(&self.ruling, party) {
                    self.note(party.0)?;
                }
            }
        }
        self.changed.sort_unstable();
        self.changed.dedup();
        Ok(())
    }
}
