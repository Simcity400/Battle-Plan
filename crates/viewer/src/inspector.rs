//! Inspect the decoded hour without borrowing data from an earlier frame.

use std::collections::BTreeMap;

use eframe::egui;
use replay::{
    container::SeekResult,
    frame::{DecodedFrame, TableView},
    rows::{Row, TableId},
};

pub struct Inspector {
    pub open: bool,
    table: TableId,
    country: Option<u32>,
    at: Option<(u64, i32)>,
    countries: BTreeMap<u32, String>,
    choices: Vec<(u32, String)>,
    links: BTreeMap<u32, u32>,
    filtered_at: Option<(TableId, Option<u32>)>,
    indices: Vec<usize>,
    unresolved: usize,
}

impl Default for Inspector {
    fn default() -> Self {
        Self {
            open: false,
            table: TableId::Countries,
            country: None,
            at: None,
            countries: BTreeMap::new(),
            choices: Vec::new(),
            links: BTreeMap::new(),
            filtered_at: None,
            indices: Vec::new(),
            unresolved: 0,
        }
    }
}

impl Inspector {
    pub fn reset(&mut self) {
        *self = Self {
            open: self.open,
            ..Self::default()
        };
    }

    pub fn show(&mut self, ctx: &egui::Context, current: &SeekResult) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        egui::Window::new("Table inspector")
            .open(&mut open)
            .default_size(egui::vec2(1050.0, 460.0))
            .min_size(egui::vec2(420.0, 240.0))
            .show(ctx, |ui| match current {
                SeekResult::Gap {
                    branch,
                    hour,
                    reason,
                } => {
                    ui.label(format!("Branch {branch} · Hour {hour}"));
                    ui.colored_label(egui::Color32::LIGHT_RED, format!("Gap: {reason}"));
                    ui.label("No rows are available for this hour.");
                }
                SeekResult::Frame(frame) => self.frame(ui, frame),
            });
        self.open = open;
    }

    fn refresh(&mut self, frame: &DecodedFrame) {
        let at = (frame.branch, frame.hour);
        if self.at == Some(at) {
            return;
        }
        self.at = Some(at);
        self.filtered_at = None;
        self.countries.clear();
        for row in rows(&frame.tables[TableId::Countries.index()]) {
            if let Row::Country(country) = row {
                let tag = String::from_utf8_lossy(&country.tag);
                self.countries.insert(
                    country.canonical_slot,
                    format!("{tag} · {}", country.canonical_slot),
                );
            }
        }
        self.choices = self
            .countries
            .iter()
            .map(|(&slot, label)| (slot, label.clone()))
            .collect();
        self.choices.sort_by(|a, b| a.1.cmp(&b.1));
        self.links = rows(&frame.tables[TableId::CountryLinks.index()])
            .iter()
            .filter_map(|row| match row {
                Row::CountryLink(link) => Some((link.id, link.canonical_slot)),
                _ => None,
            })
            .collect();
    }

    fn frame(&mut self, ui: &mut egui::Ui, frame: &DecodedFrame) {
        self.refresh(frame);
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("inspector_table")
                .selected_text(table_name(self.table))
                .show_ui(ui, |ui| {
                    for table in TableId::ALL {
                        ui.selectable_value(&mut self.table, table, table_name(table));
                    }
                });
            let country_label = self.country.map_or_else(
                || "All countries".into(),
                |slot| {
                    self.countries
                        .get(&slot)
                        .cloned()
                        .unwrap_or_else(|| format!("Country slot {slot}"))
                },
            );
            egui::ComboBox::from_id_salt("inspector_country")
                .selected_text(country_label)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.country, None, "All countries");
                    for (slot, label) in &self.choices {
                        ui.selectable_value(&mut self.country, Some(*slot), label);
                    }
                });
            ui.weak(format!("Branch {} · Hour {}", frame.branch, frame.hour));
        });
        match self.table {
            TableId::Clock | TableId::Campaign => {
                ui.weak("Campaign-wide data; country selection does not filter this table.");
            }
            TableId::Provinces | TableId::States => {
                ui.weak("Country selection includes rows owned or controlled by that country.");
            }
            TableId::Divisions => {
                ui.weak("Country selection includes roster, logical and expeditionary owners.");
            }
            _ => {}
        }
        if let Some(message) = limitation(&frame.tables[TableId::Countries.index()]) {
            ui.colored_label(egui::Color32::YELLOW, format!("Country list: {message}"));
        }
        if matches!(
            self.table,
            TableId::Provinces | TableId::States | TableId::Divisions
        ) && let Some(message) = limitation(&frame.tables[TableId::CountryLinks.index()])
        {
            ui.colored_label(
                egui::Color32::YELLOW,
                format!("Country references cannot all be resolved: {message}"),
            );
        }
        let view = &frame.tables[self.table.index()];
        match view {
            TableView::Complete(values) => {
                ui.label(format!("Complete · {} recorded rows", values.len()));
            }
            TableView::Truncated { rows, missing } => {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    format!(
                        "Truncated · {} recorded rows · {missing} missing",
                        rows.len()
                    ),
                );
            }
            TableView::Unavailable { reason } => {
                ui.colored_label(egui::Color32::LIGHT_RED, format!("Unavailable: {reason}"));
                return;
            }
        }
        let values = rows(view);
        let key = (self.table, self.country);
        if self.filtered_at != Some(key) {
            self.indices.clear();
            self.unresolved = 0;
            for (index, row) in values.iter().enumerate() {
                match country_match(row, self.country, &self.links) {
                    CountryMatch::Included => self.indices.push(index),
                    CountryMatch::Excluded => {}
                    CountryMatch::Unresolved => self.unresolved += 1,
                }
            }
            self.filtered_at = Some(key);
        }
        if self.country.is_some() && !matches!(self.table, TableId::Clock | TableId::Campaign) {
            ui.label(format!(
                "{} rows match the selected country",
                self.indices.len()
            ));
        }
        if self.unresolved > 0 {
            ui.colored_label(
                egui::Color32::YELLOW,
                format!(
                    "{} rows could not be matched because their country references are unresolved.",
                    self.unresolved
                ),
            );
        }
        ui.weak("Hover a value to see its exact recorded representation.");
        ui.separator();
        let columns = columns(self.table);
        let height = ui.text_style_height(&egui::TextStyle::Body);
        egui::ScrollArea::horizontal()
            .id_salt(("inspector_columns", self.table.index()))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let width = columns.iter().map(|(_, width)| width).sum::<f32>()
                    + ui.spacing().item_spacing.x * (columns.len() - 1) as f32;
                ui.set_min_width(width);
                egui::Grid::new(("inspector_headings", self.table.index()))
                    .min_col_width(0.0)
                    .min_row_height(height)
                    .show(ui, |ui| {
                        for (label, width) in columns {
                            ui.add_sized(
                                [*width, height],
                                egui::Label::new(egui::RichText::new(*label).strong()).truncate(),
                            )
                            .on_hover_text(*label);
                        }
                        ui.end_row();
                    });
                egui::ScrollArea::vertical()
                    .id_salt(("inspector_rows", self.table.index(), self.country))
                    .auto_shrink([false, false])
                    .show_rows(ui, height, self.indices.len(), |ui, range| {
                        egui::Grid::new(("inspector_grid", self.table.index()))
                            .striped(true)
                            .min_col_width(0.0)
                            .min_row_height(height)
                            .start_row(range.start)
                            .show(ui, |ui| {
                                for index in range {
                                    let row = &values[self.indices[index]];
                                    for (cell, (_, width)) in
                                        cells(row, &self.links, &self.countries)
                                            .into_iter()
                                            .zip(columns)
                                    {
                                        ui.add_sized(
                                            [*width, height],
                                            egui::Label::new(cell.text).truncate(),
                                        )
                                        .on_hover_text(cell.hover);
                                    }
                                    ui.end_row();
                                }
                            });
                    });
            });
    }
}

fn rows(view: &TableView) -> &[Row] {
    match view {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows,
        TableView::Unavailable { .. } => &[],
    }
}

fn limitation(view: &TableView) -> Option<String> {
    match view {
        TableView::Complete(_) => None,
        TableView::Truncated { missing, .. } => Some(format!("truncated, {missing} rows missing")),
        TableView::Unavailable { reason } => Some(format!("unavailable, {reason}")),
    }
}

fn table_name(table: TableId) -> &'static str {
    match table {
        TableId::Clock => "Clock",
        TableId::Campaign => "Campaign",
        TableId::Countries => "Countries",
        TableId::CountryLinks => "Country references",
        TableId::Provinces => "Provinces",
        TableId::States => "States",
        TableId::Divisions => "Divisions",
    }
}

#[derive(Debug, PartialEq, Eq)]
enum CountryMatch {
    Included,
    Excluded,
    Unresolved,
}

fn country_match(row: &Row, country: Option<u32>, links: &BTreeMap<u32, u32>) -> CountryMatch {
    let Some(country) = country else {
        return CountryMatch::Included;
    };
    let references = match row {
        Row::Clock(_) | Row::Campaign(_) => return CountryMatch::Included,
        Row::Country(row) => return slot_match(row.canonical_slot, country),
        Row::CountryLink(row) => return slot_match(row.canonical_slot, country),
        Row::Province(row) => [row.owner, row.controller, 0],
        Row::State(row) => [row.owner, row.controller, 0],
        Row::Division(row) => [row.roster_owner, row.logical_owner, row.expeditionary_owner],
    };
    let resolved = references.map(|raw| resolve_reference(raw, links));
    if resolved.contains(&Some(country)) {
        CountryMatch::Included
    } else if references
        .iter()
        .any(|raw| *raw != 0 && !links.contains_key(raw))
    {
        CountryMatch::Unresolved
    } else {
        CountryMatch::Excluded
    }
}

fn slot_match(slot: u32, country: u32) -> CountryMatch {
    if slot == country {
        CountryMatch::Included
    } else {
        CountryMatch::Excluded
    }
}

fn resolve_reference(raw: u32, links: &BTreeMap<u32, u32>) -> Option<u32> {
    // Zero remains the recorded sentinel even when the link table has a zero row.
    (raw != 0).then(|| links.get(&raw).copied()).flatten()
}

fn columns(table: TableId) -> &'static [(&'static str, f32)] {
    match table {
        TableId::Clock => &[
            ("Hour", 100.0),
            ("Play time counter", 140.0),
            ("Load timestamp", 140.0),
            ("Seed", 110.0),
        ],
        TableId::Campaign => &[("Campaign ID", 340.0)],
        TableId::Countries => &[
            ("ID", 70.0),
            ("Country slot", 90.0),
            ("Tag", 70.0),
            ("Recorded name", 190.0),
            ("Political power", 115.0),
            ("Command power", 115.0),
            ("Base stability (%)", 130.0),
            ("Base war support (%)", 155.0),
            ("Convoys", 110.0),
        ],
        TableId::CountryLinks => &[
            ("Reference ID", 100.0),
            ("Country slot", 100.0),
            ("Country", 120.0),
        ],
        TableId::Provinces => &[
            ("Province ID", 90.0),
            ("State ID", 90.0),
            ("Owner", 160.0),
            ("Controller", 160.0),
        ],
        TableId::States => &[("State ID", 90.0), ("Owner", 160.0), ("Controller", 160.0)],
        TableId::Divisions => &[
            ("Division (type / ID)", 175.0),
            ("Roster owner", 160.0),
            ("Logical owner", 160.0),
            ("Expeditionary owner", 160.0),
            ("Province ID", 90.0),
            ("Template (type / ID)", 175.0),
            ("Hit points", 115.0),
            ("Organisation", 115.0),
        ],
    }
}

struct Cell {
    text: String,
    hover: String,
}

impl Cell {
    fn handle(raw: u64) -> Self {
        let kind = raw as u32;
        let id = (raw >> 32) as u32;
        Self {
            text: format!("{kind} / {id}"),
            hover: format!("Recorded 64-bit handle: {raw} (0x{raw:016x})\nType: {kind}\nID: {id}"),
        }
    }

    fn integer(value: impl ToString) -> Self {
        let text = value.to_string();
        Self {
            hover: format!("Recorded integer: {text}"),
            text,
        }
    }

    fn bytes(bytes: &[u8]) -> Self {
        let text = String::from_utf8_lossy(bytes).into_owned();
        Self {
            hover: format!("{text}\nRecorded bytes: {bytes:02x?}"),
            text,
        }
    }

    fn fixed(raw: Option<i64>, percent: bool, unavailable: &str) -> Self {
        match raw {
            Some(raw) => Self {
                text: fixed(raw, percent),
                hover: format!(
                    "Recorded integer: {raw}\nSigned 64-bit fixed point, divided by 100000.{}",
                    if percent {
                        " Displayed as a base percentage."
                    } else {
                        ""
                    }
                ),
            },
            None => Self {
                text: "Unavailable".into(),
                hover: unavailable.into(),
            },
        }
    }

    fn country(raw: u32, links: &BTreeMap<u32, u32>, countries: &BTreeMap<u32, String>) -> Self {
        if raw == 0 {
            return Self {
                text: "None (0)".into(),
                hover: "Recorded country reference: 0\nNo country reference was recorded.".into(),
            };
        }
        match resolve_reference(raw, links) {
            Some(slot) => Self {
                text: format!(
                    "{} ({raw})",
                    countries
                        .get(&slot)
                        .cloned()
                        .unwrap_or_else(|| format!("Slot {slot}"))
                ),
                hover: format!("Recorded country reference: {raw}\nResolved country slot: {slot}"),
            },
            None => Self {
                text: format!("Unresolved ({raw})"),
                hover: format!(
                    "Recorded country reference: {raw}\nNo country is inferred from an unresolved reference."
                ),
            },
        }
    }
}

fn cells(row: &Row, links: &BTreeMap<u32, u32>, countries: &BTreeMap<u32, String>) -> Vec<Cell> {
    match row {
        Row::Clock(row) => vec![
            Cell::integer(row.hour),
            Cell::integer(row.play_time),
            Cell::integer(row.play_session_started_at),
            Cell::integer(row.seed),
        ],
        Row::Campaign(row) => vec![Cell::bytes(&row.id)],
        Row::Country(row) => vec![
            Cell::integer(row.id),
            Cell::integer(row.canonical_slot),
            Cell::bytes(&row.tag),
            Cell::bytes(&row.name),
            Cell::fixed(
                row.political_power_raw,
                false,
                "No political-status value was recorded.",
            ),
            Cell::fixed(Some(row.command_power_raw), false, ""),
            Cell::fixed(Some(row.stability_raw), true, ""),
            Cell::fixed(Some(row.war_support_raw), true, ""),
            Cell::fixed(
                row.convoys_raw,
                false,
                "The engine's convoy cache was invalid at this hour.",
            ),
        ],
        Row::CountryLink(row) => vec![
            Cell::integer(row.id),
            Cell::integer(row.canonical_slot),
            Cell {
                text: countries
                    .get(&row.canonical_slot)
                    .cloned()
                    .unwrap_or_else(|| "Unavailable".into()),
                hover: format!(
                    "Country slot {}. The name comes from this hour's country table.",
                    row.canonical_slot
                ),
            },
        ],
        Row::Province(row) => vec![
            Cell::integer(row.id),
            Cell::integer(row.state),
            Cell::country(row.owner, links, countries),
            Cell::country(row.controller, links, countries),
        ],
        Row::State(row) => vec![
            Cell::integer(row.id),
            Cell::country(row.owner, links, countries),
            Cell::country(row.controller, links, countries),
        ],
        Row::Division(row) => vec![
            Cell::handle(row.id),
            Cell::country(row.roster_owner, links, countries),
            Cell::country(row.logical_owner, links, countries),
            Cell::country(row.expeditionary_owner, links, countries),
            Cell::integer(row.province),
            Cell::handle(row.template),
            Cell::fixed(Some(row.hit_points), false, ""),
            Cell::fixed(Some(row.organisation), false, ""),
        ],
    }
}

/// Round only for display, without losing integer precision through an f64.
fn fixed(raw: i64, percent: bool) -> String {
    let divisor = if percent { 1_000 } else { 100_000 };
    let hundredths = (i128::from(raw).abs() * 100 + divisor / 2) / divisor;
    let sign = if raw < 0 && hundredths != 0 { "-" } else { "" };
    format!("{sign}{}.{:02}", hundredths / 100, hundredths % 100)
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::{CountryLinkRow, CountryRow, DivisionRow, ProvinceRow, StateRow};

    #[test]
    fn display_rounding_preserves_large_native_values_and_signed_ties() {
        assert_eq!(fixed(1500, false), "0.02");
        assert_eq!(fixed(-1500, false), "-0.02");
        assert_eq!(fixed(-499, false), "0.00");
        assert_eq!(fixed(70_000, true), "70.00");
        assert_eq!(fixed(i64::MAX, false), "92233720368547.76");
        assert_eq!(fixed(i64::MIN, false), "-92233720368547.76");
        assert_eq!(fixed(i64::MAX, true), "9223372036854775.81");
        assert_eq!(fixed(26_429_000, false), "264.29");
        let handle = Cell::handle(0xffff_ffff_0000_1269);
        assert_eq!(handle.text, "4713 / 4294967295");
        assert!(handle.hover.contains("0xffffffff00001269"));
    }

    #[test]
    fn country_filter_resolves_aliases_and_keeps_unknowns_distinct() {
        let links = [(0, 4), (12, 4), (91, 4), (50, 2)].into();
        let mut province = ProvinceRow {
            id: 1,
            state: 3,
            owner: 50,
            controller: 91,
        };
        assert_eq!(
            country_match(&Row::Province(province.clone()), Some(4), &links),
            CountryMatch::Included
        );
        assert_eq!(
            country_match(&Row::Province(province.clone()), Some(2), &links),
            CountryMatch::Included
        );
        assert_eq!(
            country_match(&Row::Province(province.clone()), Some(8), &links),
            CountryMatch::Excluded
        );
        province.controller = 0;
        assert_eq!(
            country_match(&Row::Province(province.clone()), Some(4), &links),
            CountryMatch::Excluded
        );
        assert_eq!(
            country_match(&Row::Province(province.clone()), None, &links),
            CountryMatch::Included
        );
        province.controller = 999;
        assert_eq!(
            country_match(&Row::Province(province), Some(4), &links),
            CountryMatch::Unresolved
        );
        assert_eq!(
            country_match(
                &Row::Province(ProvinceRow {
                    owner: 0,
                    controller: 0,
                    ..Default::default()
                }),
                Some(4),
                &links
            ),
            CountryMatch::Excluded
        );
        assert_eq!(
            country_match(
                &Row::State(StateRow {
                    id: 3,
                    owner: 12,
                    controller: 999
                }),
                Some(4),
                &links
            ),
            CountryMatch::Included
        );
        assert_eq!(
            country_match(
                &Row::State(StateRow {
                    id: 3,
                    owner: 12,
                    controller: 50
                }),
                Some(4),
                &links
            ),
            CountryMatch::Included
        );
        assert_eq!(
            country_match(
                &Row::Country(CountryRow {
                    id: 12,
                    canonical_slot: 4,
                    ..Default::default()
                }),
                Some(4),
                &links
            ),
            CountryMatch::Included
        );
        assert_eq!(
            country_match(
                &Row::CountryLink(CountryLinkRow {
                    id: 0,
                    canonical_slot: 0
                }),
                Some(0),
                &links
            ),
            CountryMatch::Included
        );
        let mut division = DivisionRow {
            roster_owner: 12,
            logical_owner: 50,
            expeditionary_owner: 91,
            ..Default::default()
        };
        assert_eq!(
            country_match(&Row::Division(division.clone()), Some(4), &links),
            CountryMatch::Included
        );
        assert_eq!(
            country_match(&Row::Division(division.clone()), Some(2), &links),
            CountryMatch::Included
        );
        division.roster_owner = 50;
        assert_eq!(
            country_match(&Row::Division(division.clone()), Some(4), &links),
            CountryMatch::Included
        );
        division.expeditionary_owner = 0;
        assert_eq!(
            country_match(&Row::Division(division.clone()), Some(4), &links),
            CountryMatch::Excluded
        );
        division.expeditionary_owner = 999;
        assert_eq!(
            country_match(&Row::Division(division.clone()), Some(4), &links),
            CountryMatch::Unresolved
        );
        assert_eq!(
            country_match(&Row::Division(division), Some(2), &links),
            CountryMatch::Included
        );
    }
}
