use super::{
    Identity, Scan, ScanState, Totals, aid,
    inventory::{Update, Updates},
};
use dashboard_base::prepared::frame_changes::FrameChanges;
use dashboard_text::metadata::slot_read;
use replay::{
    columnar_frames::{Views, metrics},
    columns,
    container::Replay,
    segments::{self, Operator, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{DivisionCensusRow, RowCodec, TABLE_COUNT, TableId, TableRows},
};
use replay_frame::frame::{DecodedFrame, TableView};
use std::{any::Any, collections::HashMap, io, sync::Arc};

type Parts = [Option<std::collections::BTreeMap<String, i64>>; 2];
type Version = (u64, u64, [bool; 2]);
type Shape = columns::Status;

pub(super) struct Census {
    pub(super) rows: Vec<DivisionCensusRow>,
    pub(super) deleted: Vec<u32>,
    pub(super) whole: bool,
}

pub(super) struct Country {
    pub(super) index: usize,
    pub(super) totals: Option<Totals>,
    pub(super) mix: Option<(Version, Arc<Parts>)>,
}

pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    pub(super) starts: bool,
    pub(super) identities: Option<Vec<(String, Identity)>>,
    pub(super) countries: Vec<Country>,
    pub(super) changed: bool,
    pub(super) refresh_live: bool,
    pub(super) live: bool,
    pub(super) divisions: bool,
    pub(super) strength: bool,
    pub(super) manpower: bool,
    pub(super) battalions: bool,
    pub(super) inventory_fallback: bool,
    pub(super) unreadable: usize,
    pub(super) composition_unreadable: usize,
    pub(super) invalid_table_reads: std::collections::BTreeMap<String, usize>,
    pub(super) aid: aid::Projection,
}

pub struct Projector {
    format: FormatVersion,
    gaps: Vec<(u64, i32, i32)>,
    naming: Views,
    frame: Option<DecodedFrame>,
    country_fields: [usize; 2],
    shapes: [Option<Shape>; TABLE_COUNT],
    scan: Scan,
    state: ScanState,
    aid: aid::Runtime,
    mixes: HashMap<String, Arc<Parts>>,
    hours: Vec<Hour>,
}

fn shape(state: &State, table: TableId) -> Shape {
    columns::status(state, table)
}

pub(super) fn rows<R: RowCodec>(state: &State, table: TableId, format: u16) -> io::Result<Vec<R>> {
    let held = columns::table(state, table)?;
    let mut slots: Vec<_> = held
        .live()
        .ones()
        .filter_map(|slot| held.key(slot).map(|key| (key, slot)))
        .collect();
    slots.sort_unstable();
    slots
        .into_iter()
        .map(|(_, slot)| columns::row(state, table, slot, format))
        .collect()
}

impl Projector {
    pub(super) fn new(replay: &Replay) -> io::Result<Self> {
        Ok(Self {
            format: replay.format,
            gaps: replay
                .gaps
                .iter()
                .map(|gap| (gap.branch, gap.first_hour, gap.last_hour))
                .collect(),
            naming: Views::new(
                &[TableId::Countries, TableId::CountryLinks],
                replay.format,
                replay.capture_mode,
            )?,
            frame: None,
            country_fields: [
                columns::field(TableId::Countries, "tag")?,
                columns::field(TableId::Countries, "canonical_slot")?,
            ],
            shapes: [None; TABLE_COUNT],
            scan: Scan::default(),
            state: ScanState::new(true),
            aid: aid::Runtime::default(),
            mixes: HashMap::new(),
            hours: Vec::new(),
        })
    }

    fn census(
        &self,
        hour: &segments::Hour<'_>,
        changed_links: bool,
        changed_census: bool,
    ) -> io::Result<Census> {
        let read = columns::complete(hour.state, TableId::DivisionCensus)
            && slot_read(
                self.format,
                &metrics(hour.manifest),
                TableId::DivisionCensus,
                hour.hour,
            );
        let changes = columns::changes(hour.state, TableId::DivisionCensus)?;
        if self.state.census_ready == Some(read) && !changed_links && !changed_census {
            return Ok(Census {
                rows: Vec::new(),
                deleted: Vec::new(),
                whole: false,
            });
        }
        let whole = !read
            || hour.starts
            || changed_links
            || self.state.census_ready != Some(true)
            || self
                .state
                .census_hour
                .is_none_or(|at| at.checked_add(1) != Some(hour.hour))
            || changes.deleted_keys.iter().any(|&key| {
                u32::try_from(key)
                    .ok()
                    .is_none_or(|key| !self.state.census_rows.contains_key(&key))
            });
        if whole {
            return Ok(Census {
                rows: if read {
                    rows(hour.state, TableId::DivisionCensus, self.format.version())?
                } else {
                    Vec::new()
                },
                deleted: Vec::new(),
                whole: true,
            });
        }
        let slots: std::collections::BTreeSet<_> = changes
            .changed
            .iter()
            .chain(&changes.fill_slots)
            .chain(&changes.inserted)
            .copied()
            .collect();
        Ok(Census {
            rows: slots
                .into_iter()
                .map(|slot| {
                    columns::row(
                        hour.state,
                        TableId::DivisionCensus,
                        slot,
                        self.format.version(),
                    )
                })
                .collect::<io::Result<_>>()?,
            deleted: changes.deleted_keys.iter().map(|&key| key as u32).collect(),
            whole: false,
        })
    }
}

// Every segment starts cold: a row block's first hour reads everything again, from the
// runtime state its fold forgot at the block before's end.
impl Operator for Projector {
    fn hour(&mut self, hour: &segments::Hour<'_>) -> io::Result<()> {
        let state = hour.state;
        let continuous = self
            .scan
            .frontier
            .is_some_and(|at| at.checked_add(1) == Some(hour.hour));
        let mut changed = [false; TABLE_COUNT];
        let mut reshaped = [false; TABLE_COUNT];
        for &table in super::prepared::TABLES {
            let index = table.index();
            let now = shape(state, table);
            reshaped[index] = self.shapes[index] != Some(now);
            let edits = columns::changes(state, table)?;
            changed[index] = hour.starts
                || hour.opens
                || !continuous
                || reshaped[index]
                || !edits.changed.is_empty()
                || !edits.fill_slots.is_empty()
                || !edits.inserted.is_empty()
                || !edits.deleted.is_empty()
                || edits.truncated;
            self.shapes[index] = Some(now);
        }
        let country_changes = columns::changes(state, TableId::Countries)?;
        let mask = self
            .country_fields
            .iter()
            .fold(0_u64, |mask, &field| mask | 1_u64 << field);
        let naming_changed = self.frame.is_none()
            || hour.starts
            || reshaped[TableId::Countries.index()]
            || changed[TableId::CountryLinks.index()]
            || !country_changes.inserted.is_empty()
            || !country_changes.deleted.is_empty()
            || country_changes
                .masks
                .iter()
                .chain(&country_changes.fill_masks)
                .any(|value| value & mask != 0);
        if naming_changed {
            self.frame = Some(self.naming.advance(state, hour.manifest, true)?);
        }
        let mut frame = self.frame.as_ref().expect("naming frame").clone();
        frame.branch = hour.branch;
        frame.hour = hour.hour;
        frame.metrics = metrics(hour.manifest);
        // Inventory and census rows come from slots; these views carry only read status.
        for (table, rows) in [
            (TableId::Divisions, TableRows::Divisions(Vec::new())),
            (
                TableId::DivisionTemplateIcons,
                TableRows::DivisionTemplateIcons(Vec::new()),
            ),
            (
                TableId::DivisionLandInputs,
                TableRows::DivisionLandInputs(Vec::new()),
            ),
            (
                TableId::DivisionCensus,
                TableRows::DivisionCensus(Vec::new()),
            ),
        ] {
            frame.tables[table.index()] = if columns::complete(state, table) {
                TableView::Complete(rows)
            } else {
                TableView::Unavailable {
                    reason: String::new(),
                }
            }
            .into();
        }
        let mut changes = FrameChanges::alone(&frame, self.format);
        changes.rows_changed = changed;
        let gap = self.gaps.iter().any(|&(branch, first, last)| {
            branch == hour.branch && first <= hour.hour && hour.hour <= last
        });
        let valid = frame
            .complete::<replay_data::rows::tables::Countries>()
            .is_ok()
            && frame
                .complete::<replay_data::rows::tables::CountryLinks>()
                .is_ok();
        self.state.columnar_census = if valid && !gap {
            Some(self.census(
                hour,
                changed[TableId::CountryLinks.index()],
                changed[TableId::DivisionCensus.index()],
            )?)
        } else {
            None
        };
        self.state
            .inventory
            .as_mut()
            .expect("army inventory")
            .columnar_names(
                state,
                self.format.version(),
                hour.starts || reshaped[TableId::LexicalTokens.index()],
            )?;
        let army_changed = self.state.step_input(
            &mut self.scan,
            Updates::columnar(state, self.format.version(), hour.starts),
            &frame,
            self.format,
            gap,
            Some(&changes),
        );
        let aid = self.aid.step_input(
            &frame,
            &changes,
            self.state.inventory.as_ref().expect("army inventory"),
            &self.state.identities,
            self.state.identity_changed,
            gap,
            Some(Update::Columnar(state, self.format.version(), hour.starts)),
        );
        let identities = (hour.starts || self.state.identity_changed || !continuous)
            .then(|| self.state.identities.clone());
        if identities.is_some() {
            self.mixes.retain(|tag, _| self.scan.live.contains(tag));
        }
        let mut countries = Vec::with_capacity(self.scan.observed.len());
        for &index in &self.scan.observed {
            let (tag, _) = &self.state.identities[index];
            let track = self.scan.tracks.get_mut(tag).expect("observed army holder");
            let mix = track
                .mix
                .last()
                .filter(|mix| mix.last == hour.hour)
                .map(|mix| {
                    let parts = if mix.first == hour.hour || !self.mixes.contains_key(tag) {
                        let parts = Arc::new(mix.parts.clone());
                        self.mixes.insert(tag.clone(), Arc::clone(&parts));
                        parts
                    } else {
                        Arc::clone(&self.mixes[tag])
                    };
                    (track.mix_version.expect("composition version"), parts)
                });
            countries.push(Country {
                index,
                totals: track
                    .runs
                    .last()
                    .filter(|run| run.last == hour.hour)
                    .map(|run| run.totals),
                mix,
            });
            if track.runs.len() > 1 {
                track.runs.drain(..track.runs.len() - 1);
            }
            if track.mix.len() > 1 {
                track.mix.drain(..track.mix.len() - 1);
            }
            if track.segments.len() > 1 {
                track.segments.drain(..track.segments.len() - 1);
            }
        }
        self.hours.push(Hour {
            branch: hour.branch,
            hour: hour.hour,
            starts: hour.starts,
            identities,
            countries,
            changed: army_changed,
            refresh_live: gap || !valid || self.state.identity_changed || !continuous,
            live: !gap && valid,
            divisions: self.scan.divisions,
            strength: self.scan.strength,
            manpower: self.scan.manpower,
            battalions: self.scan.battalions,
            inventory_fallback: self.scan.inventory_fallback,
            unreadable: std::mem::take(&mut self.scan.unreadable),
            composition_unreadable: std::mem::take(&mut self.scan.composition_unreadable),
            invalid_table_reads: std::mem::take(&mut self.scan.invalid_table_reads),
            aid,
        });
        Ok(())
    }

    fn take(&mut self) -> io::Result<Box<dyn Any + Send>> {
        Ok(Box::new(std::mem::take(&mut self.hours)))
    }
}
