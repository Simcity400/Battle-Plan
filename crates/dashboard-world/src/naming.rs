//! What names are read with beside tags: each token's spelling, read in place from the walker,
//! and each country's appearance row, the last by raw id its links name.

use super::{Facts, Roster, countries::edited, invalid};
use replay::{columns, segments::State};
use replay_data::{
    format::FormatVersion,
    rows::{LexicalTokenRow, TableId},
};
use std::{borrow::Cow, collections::HashMap, io};

/// The naming operands of the hour.
pub struct Naming {
    format: FormatVersion,
    literal: usize,
    name_tag: usize,
    tokens_complete: bool,
    /// The tokens whose spelling the hour wrote, added or removed, ascending.
    tokens: Vec<u32>,
    tokens_rebuilt: bool,
    /// Each canonical slot's appearance row, by its raw id; None unless CountryAppearance and
    /// CountryLinks are complete.
    appearances: Option<HashMap<u32, u32>>,
    appearances_changed: bool,
}

impl Naming {
    pub(crate) fn new(format: FormatVersion) -> io::Result<Self> {
        Ok(Self {
            format,
            literal: columns::field(TableId::LexicalTokens, "literal")?,
            name_tag: columns::field(TableId::CountryAppearance, "name_tag")?,
            tokens_complete: false,
            tokens: Vec::new(),
            tokens_rebuilt: false,
            appearances: None,
            appearances_changed: false,
        })
    }

    /// Whether LexicalTokens holds every row this hour.
    pub fn tokens_complete(&self) -> bool {
        self.tokens_complete
    }

    /// Whether any spelling may have changed: the walk's first hour, or one LexicalTokens
    /// became complete or stopped being.
    pub fn tokens_rebuilt(&self) -> bool {
        self.tokens_rebuilt
    }

    /// The tokens whose spelling the hour wrote, added or removed, ascending; see
    /// [`Self::tokens_rebuilt`] for the hours any may have.
    pub fn changed_tokens(&self) -> &[u32] {
        &self.tokens
    }

    /// `token`'s spelling this hour; None when LexicalTokens is not complete or holds no row
    /// of it.
    pub fn spelling<'s>(&self, state: &'s State, token: u32) -> io::Result<Option<Cow<'s, [u8]>>> {
        if !self.tokens_complete {
            return Ok(None);
        }
        let held = columns::table(state, TableId::LexicalTokens)?;
        let Some(slot) = held.slot(u128::from(token)) else {
            return Ok(None);
        };
        if !held.recorded(self.literal, slot) {
            let row: LexicalTokenRow =
                columns::row(state, TableId::LexicalTokens, slot, self.format.version())?;
            return Ok(Some(Cow::Owned(row.literal)));
        }
        let id = *columns::text(held, self.literal)?
            .get(slot as usize)
            .ok_or_else(|| invalid(format!("LexicalTokens slot {slot} holds no cell")))?;
        let bytes = state
            .dictionary()
            .get(id)
            .ok_or_else(|| invalid("A token's spelling names no dictionary entry"))?;
        Ok(Some(Cow::Borrowed(bytes)))
    }

    /// The raw id of canonical slot `slot`'s appearance row; the outer None while
    /// CountryAppearance or CountryLinks is not complete.
    pub fn appearance(&self, slot: u32) -> Option<Option<u32>> {
        self.appearances
            .as_ref()
            .map(|appearances| appearances.get(&slot).copied())
    }

    /// Whether CountryAppearance and CountryLinks are complete, as [`Self::appearance`] reads.
    pub fn appearances_complete(&self) -> bool {
        self.appearances.is_some()
    }

    /// Whether the hour may have changed an appearance row's slot or name tag.
    pub fn appearances_changed(&self) -> bool {
        self.appearances_changed
    }

    pub(crate) fn hour(
        &mut self,
        state: &State,
        facts: &Facts<'_>,
        roster: &Roster,
    ) -> io::Result<()> {
        self.tokens.clear();
        let complete = columns::complete(state, TableId::LexicalTokens);
        self.tokens_rebuilt = facts.first() || complete != self.tokens_complete;
        self.tokens_complete = complete;
        let literal = 1_u64 << self.literal;
        if complete && !self.tokens_rebuilt && facts.touched(TableId::LexicalTokens, literal) {
            let held = columns::table(state, TableId::LexicalTokens)?;
            for slot in edited(state, TableId::LexicalTokens, literal)? {
                if let Some(key) = held.key(slot) {
                    engine_allocation::reserve(&mut self.tokens, 1)?;
                    self.tokens.push(key as u32);
                }
            }
            let deleted = &columns::changes(state, TableId::LexicalTokens)?.deleted_keys;
            engine_allocation::reserve(&mut self.tokens, deleted.len())?;
            self.tokens.extend(deleted.iter().map(|&key| key as u32));
            self.tokens.sort_unstable();
            self.tokens.dedup();
        }
        let name_tag = 1_u64 << self.name_tag;
        self.appearances_changed =
            roster.changed() || facts.touched(TableId::CountryAppearance, name_tag);
        let rows = facts.touched(TableId::CountryAppearance, 0);
        if roster.rebuilt() || !roster.changed_links().is_empty() || rows {
            self.index(state, roster)?;
        }
        Ok(())
    }

    /// Each canonical slot's appearance row anew: the last by raw id whose link names it.
    fn index(&mut self, state: &State, roster: &Roster) -> io::Result<()> {
        if !(roster.links_complete() && columns::complete(state, TableId::CountryAppearance)) {
            self.appearances = None;
            return Ok(());
        }
        let held = columns::table(state, TableId::CountryAppearance)?;
        let mut raws: Vec<u32> = held
            .live()
            .ones()
            .filter_map(|slot| held.key(slot).map(|key| key as u32))
            .collect();
        raws.sort_unstable();
        let mut appearances = HashMap::new();
        appearances
            .try_reserve(raws.len())
            .map_err(|_| engine_allocation::oom())?;
        for raw in raws {
            if let Some(slot) = roster.slot_of(raw) {
                appearances.insert(slot, raw);
            }
        }
        self.appearances = Some(appearances);
        Ok(())
    }
}
