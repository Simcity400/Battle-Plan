//! Idea history prepared on its own, block by block. A pre-pass worker reads a
//! block alone with the idea tables, only at the hours the fields the idea
//! scan reads change ([`FAMILY`]'s fields): country keys and links, held
//! ideas and appointed advisors, the characters' naming and template fields,
//! the dictionary, the appearance fields a holder is named by, the civil-war,
//! enemy, war and existence operands that naming reads at a new holding, and
//! the modifier definitions packed with the dictionary. Nothing changes the
//! scan with no field changing, so the extract keeps no deadline. The worker
//! keeps the observed hours' frames (countries, characters and country
//! counters reduced to the fields the scan reads) and the spans of hours
//! between them.
//!
//! The family folds each block in order on the carried scan: each kept hour is
//! observed as hourly preparation observes it, and each span takes the scan's
//! step for all its hours at once ([`Scan::advance_quiet`]): no holder is
//! visited, the frontier advances and dated template refusals take the
//! span's last hour. When the carried scan does not continue into a span, the
//! rest of the block is observed hour by hour.

use super::{PreparedChanges, Scan};
use crate::{
    prepared::{
        frame_changes::FrameChanges,
        kept::{
            BlockFacts, Hour, Observed, Reduced, consecutive, kept_changes, observe_hours,
            push_quiet,
        },
        prepass::{Extract, Family, Folded},
    },
    products::politics_content::PoliticsContent,
};
use replay::{
    container::BlockIndex, container::QuietHours, container::Replay, container::TableFields,
};
use replay_data::{
    format::FormatVersion, rows::CharacterRow, rows::CountryCounterInputRow, rows::CountryRow,
    rows::TableId, rows::TableRows, rows::tables,
};
use replay_frame::frame::DecodedFrame;
use std::{any::Any, io, ops::Range};

/// The tables the idea scan reads ([`super::TABLES`]), and the modifier
/// definitions its change publisher packs with the dictionary.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::ActiveIdeas,
    TableId::HiredAdvisors,
    TableId::Characters,
    TableId::LexicalTokens,
    TableId::CountryAppearance,
    TableId::CountryCivilWars,
    TableId::CountryWars,
    TableId::WarRelations,
    TableId::CountryCounterInputs,
    TableId::ModifierDefinitions,
];

pub static FAMILY: Family = Family {
    name: "ideas",
    extract: "ideas.extract",
    sequential: "ideas.sequential",
    fallback: "ideas.fallback",
    tables: TABLES,
    fields: Some(fields),
    start,
};

/// The fields of the idea tables the scan and its change publisher read, so
/// that a block is read only at the hours they change. Campaign is checked on
/// every read and read by nothing else here. Row keys and availability are
/// watched with every table.
fn fields() -> io::Result<Vec<TableFields>> {
    Ok(vec![
        // Holders: the id (the row key), canonical slot and tag. The balances
        // move every hour and none is read.
        TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
        // Every link, compared whole.
        TableFields::all(TableId::CountryLinks),
        // Held ideas and appointed advisors, compared whole by holder.
        TableFields::all(TableId::ActiveIdeas),
        TableFields::all(TableId::HiredAdvisors),
        // An advisor's name, script token and template (the id is the row
        // key). Its country, roster, ordinal, saved name and gender are not
        // read.
        TableFields::named(
            TableId::Characters,
            &[
                "name_token_id",
                "token",
                "template_present",
                "template_tag",
                "template_file",
                "template_line",
                "template_generated",
            ],
        )?,
        // Every spelling: tokens, templates, and the definitions packet.
        TableFields::all(TableId::LexicalTokens),
        // What a holder is named by at a new holding: its name and cosmetic
        // tags and its original tag. Its colours, convoys and other naming
        // operands are not read; an hour at which only they change visits a
        // holder hourly preparation would find unchanged.
        TableFields::named(
            TableId::CountryAppearance,
            &[
                "cosmetic_tag",
                "name_tag",
                "original_tag",
                "original_tag_reference",
            ],
        )?,
        // Naming reads whether a holder is in a civil war at a new holding:
        // the civil-war lists, enemy vectors, war operands and whether each
        // country exists. Casualties, controlled states and the operand
        // lists move without changing them.
        TableFields::all(TableId::CountryCivilWars),
        TableFields::named(TableId::CountryWars, &["enemies_read", "enemies"])?,
        TableFields::named(TableId::WarRelations, &["start_hour", "end_hour"])?,
        TableFields::named(TableId::CountryCounterInputs, &["owned_state_count"])?,
        // The definitions packet stores the whole table whenever it changes.
        TableFields::all(TableId::ModifierDefinitions),
    ])
}

fn start(_: FormatVersion) -> Box<dyn Extract> {
    Box::new(BlockExtract {
        hours: Vec::new(),
        countries: Reduced::new(TableId::Countries),
        characters: Reduced::new(TableId::Characters),
        counters: Reduced::new(TableId::CountryCounterInputs),
        frames: 0,
    })
}

struct BlockExtract {
    hours: Vec<Hour>,
    countries: Reduced<CountryRow>,
    characters: Reduced<CharacterRow>,
    counters: Reduced<CountryCounterInputRow>,
    frames: usize,
}

impl BlockExtract {
    /// The frame as the scan reads it: the countries, characters and country
    /// counters reduced to the fields it reads.
    fn kept(&mut self, frame: &DecodedFrame, reported: Option<&[TableId]>) -> DecodedFrame {
        let mut kept = frame.clone();
        let unchanged = |table| reported.is_some_and(|reported| !reported.contains(&table));
        if !unchanged(TableId::Countries) || !self.countries.reuse(&mut kept) {
            self.countries.countries(&mut kept);
        }
        if !unchanged(TableId::Characters) || !self.characters.reuse(&mut kept) {
            self.characters.reduce(
                &mut kept,
                |frame| {
                    frame.complete::<tables::Characters>().ok().map(|rows| {
                        rows.iter()
                            .map(|row| CharacterRow {
                                id: row.id,
                                name_token_id: row.name_token_id,
                                token: row.token.clone(),
                                template_present: row.template_present,
                                template_tag: row.template_tag,
                                template_file: row.template_file.clone(),
                                template_line: row.template_line,
                                template_generated: row.template_generated,
                                ..Default::default()
                            })
                            .collect()
                    })
                },
                TableRows::Characters,
            );
        }
        if !unchanged(TableId::CountryCounterInputs) || !self.counters.reuse(&mut kept) {
            self.counters.reduce(
                &mut kept,
                |frame| {
                    frame
                        .complete::<tables::CountryCounterInputs>()
                        .ok()
                        .map(|rows| {
                            rows.iter()
                                .map(|row| CountryCounterInputRow {
                                    country: row.country,
                                    owned_state_count: row.owned_state_count,
                                    ..Default::default()
                                })
                                .collect()
                        })
                },
                TableRows::CountryCounterInputs,
            );
        }
        kept
    }

    fn hour(&mut self, frame: &DecodedFrame, changes: &FrameChanges, reported: Option<&[TableId]>) {
        self.frames += 1;
        let frame = self.kept(frame, reported);
        self.hours.push(Hour::Observed {
            frame,
            changes: kept_changes(changes),
        });
    }
}

impl Extract for BlockExtract {
    /// What the block's kept hours hold.
    fn bytes(&self) -> u64 {
        crate::prepared::kept::hours_bytes(&self.hours)
    }

    fn observe(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        self.hour(frame, changes, None);
        Ok(())
    }

    fn observe_reported(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        reported: &[TableId],
    ) -> io::Result<()> {
        self.hour(frame, changes, Some(reported));
        Ok(())
    }

    /// Hours at which no field the scan reads changed: the fold takes the
    /// scan's step over them at once.
    fn quiet(&mut self, _: &Replay, hours: &QuietHours, span: Range<usize>) -> io::Result<()> {
        if let Some((first, last)) = consecutive(hours, span)? {
            push_quiet(&mut self.hours, first, last);
        }
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        if self.hours.is_empty() {
            return Err(io::Error::other("Idea block has no hours"));
        }
        Ok(Box::new(BlockFacts {
            hours: self.hours,
            frames: self.frames,
        }))
    }
}

/// Fold one block's facts into the carried scan and its change publisher,
/// classifying additions with `content`. Returns how it folded and the last
/// hour it reached.
#[allow(clippy::too_many_arguments)]
pub fn fold(
    scan: &mut Scan,
    changes: &mut PreparedChanges,
    content: Option<&PoliticsContent>,
    facts: Box<dyn Any + Send>,
    replay: &Replay,
    index: &BlockIndex,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<(Folded, Option<i32>)> {
    let _cpu = ::replay_frame::cpu::scope("fold driver", None);
    let facts = {
        let _cpu = ::replay_frame::cpu::scope("fold facts", None);
        facts
            .downcast::<BlockFacts>()
            .map_err(|_| io::Error::other("Pre-pass facts of another family"))?
    };
    #[cfg(any(test, feature = "test-support"))]
    crate::prepared::kept::note_frames(FAMILY.name, facts.frames);
    let mut reached = None;
    for hour in facts.hours {
        let _cpu = ::replay_frame::cpu::scope("fold driver", None);
        check()?;
        match hour {
            Hour::Observed {
                frame,
                changes: facts,
            } => {
                reached = Some(frame.hour);
                #[cfg(any(test, feature = "test-support"))]
                crate::prepared::kept::note_observed(FAMILY.name);
                let result = {
                    let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                    scan.prepare_changes_in(&frame, &facts, changes, content)
                };
                if !observed(frame.hour, result)? {
                    return Ok((Folded::Fast, reached));
                }
            }
            Hour::Quiet { first, last } => {
                let advanced = {
                    let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                    scan.advance_quiet(first, last, changes)
                };
                if !advanced {
                    let reached = observe_block(
                        scan,
                        changes,
                        content,
                        replay,
                        index,
                        Some(first),
                        observed,
                        check,
                    )?;
                    return Ok((Folded::Fallback, reached));
                }
                reached = Some(last);
                if !observed(last, Ok(()))? {
                    return Ok((Folded::Fast, reached));
                }
            }
        }
    }
    Ok((Folded::Fast, reached))
}

/// Observe the block hour by hour from `from` (every hour without it), with
/// the change facts of the block alone, as hourly preparation does. Returns
/// the last hour observed.
#[allow(clippy::too_many_arguments)]
pub fn observe_block(
    scan: &mut Scan,
    changes: &mut PreparedChanges,
    content: Option<&PoliticsContent>,
    replay: &Replay,
    index: &BlockIndex,
    from: Option<i32>,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<Option<i32>> {
    let _cpu = ::replay_frame::cpu::scope("fold driver", None);
    observe_hours(
        replay,
        index,
        TABLES,
        from,
        observed,
        check,
        |frame, facts| {
            #[cfg(any(test, feature = "test-support"))]
            crate::prepared::kept::note_observed(FAMILY.name);
            scan.prepare_changes_in(frame, facts, changes, content)
        },
    )
}

#[cfg(test)]
mod reduction_tests {
    use super::*;
    use replay_frame::{frame::SharedView, frame::TableView};

    #[test]
    fn reported_unchanged_fields_reuse_reductions_even_when_sources_are_rebuilt() {
        let new = || BlockExtract {
            hours: Vec::new(),
            countries: Reduced::new(TableId::Countries),
            characters: Reduced::new(TableId::Characters),
            counters: Reduced::new(TableId::CountryCounterInputs),
            frames: 0,
        };
        let mut frame = crate::test_fixtures::source(1, 100, None);
        frame.tables[TableId::Characters.index()] =
            TableView::Complete(TableRows::Characters(vec![CharacterRow {
                id: 1,
                token: b"synthetic".to_vec(),
                ..Default::default()
            }]))
            .into();
        frame.tables[TableId::CountryCounterInputs.index()] =
            TableView::Complete(TableRows::CountryCounterInputs(vec![
                CountryCounterInputRow {
                    country: 1,
                    ..Default::default()
                },
            ]))
            .into();
        let mut actual = new();
        actual.kept(&frame, Some(TABLES));
        let before = crate::prepared::prepass::noted("kept.reduce");
        let mut next = frame.clone();
        next.hour += 1;
        for table in [TableId::Characters, TableId::CountryCounterInputs] {
            next.tables[table.index()] = SharedView::from((*frame.tables[table.index()]).clone());
        }
        let actual = actual.kept(&next, Some(&[TableId::ActiveIdeas]));
        assert_eq!(crate::prepared::prepass::noted("kept.reduce"), before);
        let mut reference = new();
        reference.kept(&frame, None);
        assert_eq!(actual, reference.kept(&next, None));
    }
}
