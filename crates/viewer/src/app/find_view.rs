
use super::{Viewer, head_rows, live_of};
use crate::document::Document;
use eframe::egui;
use engine_executable::executable_tables::folding::FoldingTable;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use map_display_data::landscape::{LabelName, Labels};
use viewer_layers_political::labels::Stamped;
use std::{collections::BTreeMap, sync::Arc};
use viewer_combat::find_view::{
    Control, EditKey, Field, FindView,
    index::{Entry, Fold, Index, Target, UNFOLDED_NAMES},
};
use viewer_gpu::camera::Camera;
use viewer_gpu::camera::motion::Floor;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Key {
    branch: replay_data::BranchId,
    hour: replay_data::Hour,
    ours: u32,
    labels: Result<usize, &'static str>,
}

const LABELS_NOT_DRAWN: &str = "victory points are left out: the map's labels have not been laid \
     out for the viewing country";
const LABELS_LOADING: &str = "victory points are loading: the map's labels have not been laid out \
     for this hour yet";

struct Built {
    index: Index,
    capital: Option<u32>,
    refused: BTreeMap<String, usize>,
    fold: Fold<'static>,
}

fn fold<'t>(table: Result<&'t FoldingTable, String>, refused: &mut BTreeMap<String, usize>) -> Fold<'t> {
    match table {
        Ok(table) => Fold::with_table(&table.letters),
        Err(withheld) => {
            refused.insert(withheld, 1);
            Fold::default()
        }
    }
}

#[derive(Default)]
pub struct FindState {
    pub view: FindView,
    pub open: bool,
    pub field: Field,
    index: Option<(Key, Result<Built, String>)>,
    searched: Option<(String, u64)>,
    generation: u64,
    results: Vec<(String, Target)>,
    pub refused: BTreeMap<String, usize>,
    pub window_refused: BTreeMap<String, usize>,
    pending: Option<Target>,
}

impl FindState {
    pub fn typing(&self) -> bool {
        self.open && self.field.focused
    }

    pub fn results(&self) -> Vec<String> {
        self.results.iter().map(|(text, _)| text.clone()).collect()
    }

    pub fn fingerprint(&self) -> u64 {
        self.generation
            .wrapping_mul(31)
            .wrapping_add(self.results.len() as u64)
            .wrapping_add(self.searched.as_ref().map_or(0, |(query, _)| query.len() as u64) << 32)
    }

    pub fn toggle(&mut self, now: f32) {
        if self.open {
            self.close();
        } else {
            self.open = true;
            self.field.open(now);
            self.searched = None;
            self.results.clear();
        }
    }

    pub fn close(&mut self) {
        self.open = false;
        self.field.focused = false;
    }

    pub fn click(&mut self, control: Control) {
        match control {
            Control::Result(index) => {
                if let Some((_, target)) = self.results.get(index) {
                    self.pending = Some(*target);
                    self.close();
                }
            }
            Control::Find => {
                if let Some((_, target)) = self.results.first() {
                    self.pending = Some(*target);
                    self.close();
                }
            }
            Control::Home => {
                if let Some((_, Ok(built))) = &self.index
                    && let Some(capital) = built.capital
                {
                    self.pending = Some(Target::State(capital));
                }
            }
            Control::Back => self.close(),
            Control::Field | Control::Window => {}
        }
    }

    pub fn take_pending(&mut self, document: &Document, camera_x: f32) -> Option<[f32; 2]> {
        let target = self.pending.take()?;
        let state = |id: u32| document.radar_centers.get(id).ok();
        let point = match target {
            Target::Country { capital } => state(capital?)?.map(|value| value as f32),
            Target::State(id) => state(id)?.map(|value| value as f32),
            Target::Province(id) => document.province_geometry.centroid(id)?,
        };
        let width = document.map.width as f32;
        let turns = ((camera_x - point[0]) / width).round();
        Some([point[0] + turns * width, point[1]])
    }

    fn search(&mut self) {
        let Some((_, Ok(built))) = &self.index else {
            self.results.clear();
            return;
        };
        let query = self.field.text().to_owned();
        if self.searched.as_ref() == Some(&(query.clone(), self.generation)) {
            return;
        }
        self.refused = built.refused.clone();
        if built.index.unfolded > 0 {
            self.refused
                .insert(UNFOLDED_NAMES.to_owned(), built.index.unfolded);
        }
        self.results = match built.index.search(&query, built.fold) {
            Ok(found) => found
                .into_iter()
                .map(|entry| (entry.display.clone(), entry.target))
                .collect(),
            Err(why) => {
                self.refused.insert(why.to_owned(), 1);
                Vec::new()
            }
        };
        self.searched = Some((query, self.generation));
    }
}

fn key_of(frame: &DecodedFrame, ours: u32, drawn: Option<&Stamped>) -> (Key, Result<Arc<Labels>, &'static str>) {
    let labels = drawn_labels(drawn, ours, frame.branch, frame.hour);
    let key = Key { branch: frame.branch, hour: frame.hour, ours, labels: labels.as_ref().map(|labels| Arc::as_ptr(labels) as usize).map_err(|why| *why) };
    (key, labels)
}

fn drawn_labels(drawn: Option<&Stamped>, ours: u32, branch: replay_data::BranchId, hour: replay_data::Hour) -> Result<Arc<Labels>, &'static str> {
    match drawn {
        Some(drawn) if drawn.ours == ours => match drawn.branch == branch && drawn.hours.contains(&hour) {
            true => Ok(Arc::clone(&drawn.labels)),
            false => Err(LABELS_LOADING),
        },
        _ => Err(LABELS_NOT_DRAWN),
    }
}

fn victory_point_entries(
    labels: &Labels,
    defines: &viewer_map_icons::labels::LabelDefines,
    refused: &mut BTreeMap<String, usize>,
) -> Vec<Entry> {
    labels
        .candidates
        .iter()
        .filter(|candidate| candidate.victory_points >= 1)
        .filter_map(|candidate| {
            let name = match &candidate.name {
                LabelName::Value { text: Some(text) } if !text.is_empty() => text.clone(),
                LabelName::Value { .. } => return None,
                LabelName::Refused { reason } => {
                    *refused
                        .entry(format!("province {} is left out: {reason}", candidate.province))
                        .or_insert(0) += 1;
                    return None;
                }
            };
            let side = candidate
                .controller
                .and_then(|slot| labels.allegiances.get(&slot).copied())
                .unwrap_or(viewer_foundation::allegiance::Allegiance::Other.index());
            let shape =
                viewer_map_icons::labels::shape_group(false, candidate.victory_points, defines);
            let frame_number = shape * 5 + side + 1;
            Some(Entry {
                display: format!("{} {name}", viewer_combat::find_view::victory_point_icon(frame_number)),
                name,
                secondary: String::new(),
                target: Target::Province(candidate.province),
            })
        })
        .collect()
}

fn build(
    document: &Document,
    frame: &DecodedFrame,
    ours: u32,
    labels: Result<&Labels, &str>,
) -> Result<Built, String> {
    let (mut entries, mut refused) =
        viewer_combat::find_view::countries_and_states(&document.combat, frame)
            .map_err(|e| e.to_string())?;
    match labels {
        Ok(labels) => entries.extend(victory_point_entries(
            labels,
            &document.labels.defines,
            &mut refused,
        )),
        Err(why) => {
            refused.insert(why.to_owned(), 1);
        }
    }
    let raw = frame
        .complete::<tables::Countries>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.canonical_slot == ours)
        .map(|row| row.id);
    let capital = match raw {
        Some(raw) => frame
            .complete::<tables::Politics>()
            .map_err(|e| e.to_string())?
            .iter()
            .find(|row| row.country == raw && row.capital > 0)
            .map(|row| row.capital as u32),
        None => None,
    };
    let table = match document.content.tables {
        Some(tables) => tables.folding.as_ref().map_err(ToString::to_string),
        None => Err("diacritic folding table withheld: the recording's executable tables are not loaded".to_owned()),
    };
    let fold = fold(table, &mut refused);
    Ok(Built {
        index: Index::build(entries, fold),
        capital,
        refused,
        fold,
    })
}

pub(crate) fn dump(
    document: &Document,
    options: &crate::Options,
    query: &str,
) -> Result<(viewer_combat::Drawn, Vec<String>), String> {
    let frame = document.frame().map_err(|e| e.to_string())?;
    let ours = document.vision(&options.perspective).map_err(|e| e.to_string())?.ours();
    let labels = document.hour().and_then(|hour| hour.labels(ours)).map_err(|e| e.to_string())?;
    let built = build(document, frame, ours, Ok(&labels))?;
    let mut refused = built.refused;
    let results: Vec<String> = match built.index.search(query, built.fold) {
        Ok(found) => found.into_iter().map(|entry| entry.display.clone()).collect(),
        Err(why) => {
            refused.insert(why.to_owned(), 1);
            Vec::new()
        }
    };
    let mut lines: Vec<String> = results.iter().map(|text| format!("result: {text:?}")).collect();
    let state = viewer_combat::find_view::State { text: query, query, caret: None, results: &results, fingerprint: results.len() as u64 };
    let mut drawn = viewer_combat::Drawn::default();
    let mut window_refused = BTreeMap::new();
    let extent = viewer_gpu::ui_scale::extent(options.size, options.ui_scale.unwrap_or(1.0));
    FindView::default()
        .draw(&document.combat, &state, extent, false, options.presentation_time, true, None, &mut drawn, &mut window_refused)
        .map_err(|e| e.to_string())?;
    lines.extend(refused.iter().map(|(why, count)| format!("left out ({count}): {why}")));
    lines.extend(window_refused.iter().chain(&drawn.refused).map(|(why, count)| format!("not drawn ({count}): {why}")));
    Ok((drawn, lines))
}

impl Viewer {
    pub(super) fn refresh_find_view(&mut self, document: &Document) {
        if !self.picker.find.open {
            return;
        }
        let head = head_rows::head(live_of(&self.playhead_map));
        let Some(ours) = self.shown_vision().ok().map(|vision| vision.ours()) else {
            return;
        };
        let drawn = live_of(&self.playhead_map).and_then(|side| side.products().labels.as_ref());
        let held = self.picker.find.index.as_ref().map(|(built, _)| *built);
        let read = self.head_rows.find(head, |frame| {
            let (key, labels) = key_of(frame, ours, drawn);
            (held != Some(key)).then(|| (key, build(document, frame, ours, labels.as_deref().map_err(|why| *why))))
        });
        if let Some(Some((key, built))) = read {
            let find = &mut self.picker.find;
            if let Err(why) = &built {
                find.refused = BTreeMap::from([(why.clone(), 1)]);
            }
            find.index = Some((key, built));
            find.generation += 1;
        }
        self.picker.find.search();
    }

    pub(super) fn find_view_move(&mut self, document: &Document, camera: &mut Camera, floor: Floor<'_>) {
        if let Some(center) = self.picker.find.take_pending(document, camera.center[0]) {
            self.camera_manual = true;
            self.zoom.place(camera, center, Some(0.0), floor);
        }
    }

    pub(super) fn find_view_keys(
        &mut self,
        ui: &egui::Ui,
        document: &Document,
        extent: [f32; 2],
        now: f32,
        filter: super::CameraKeyFilter,
    ) -> bool {
        if !self.picker.find.open {
            return false;
        }
        let shortcuts: Vec<_> = viewer_combat::find_view::shortcuts(&document.combat, extent)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(control, text)| Some((control, super::picker::parse_shortcut(&text)?)))
            .collect();
        let events = ui.input(|input| input.events.clone());
        let find = &mut self.picker.find;
        let typing = find.field.focused;
        for event in events {
            match event {
                egui::Event::Text(text) if find.field.focused => {
                    find.field.insert(&text, now);
                }
                egui::Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    repeat,
                    modifiers,
                } => {
                    let edit = match key {
                        egui::Key::Backspace => Some(EditKey::Backspace),
                        egui::Key::Delete => Some(EditKey::Delete),
                        egui::Key::ArrowLeft => Some(EditKey::Left),
                        egui::Key::ArrowRight => Some(EditKey::Right),
                        egui::Key::Home => Some(EditKey::Home),
                        egui::Key::End => Some(EditKey::End),
                        _ => None,
                    };
                    if find.field.focused
                        && let Some(edit) = edit
                    {
                        find.field.key(edit, now);
                        continue;
                    }
                    let types = !(modifiers.ctrl || modifiers.alt || modifiers.command)
                        && (key.symbol_or_name().chars().count() == 1 || key == egui::Key::Space);
                    if repeat || (find.field.focused && types) {
                        continue;
                    }
                    if filter.blocks(physical_key.unwrap_or(key)) {
                        continue;
                    }
                    if let Some((control, _)) = shortcuts
                        .iter()
                        .find(|(_, (wanted, wanted_key))| *wanted_key == key && modifiers.matches_exact(*wanted))
                    {
                        find.click(*control);
                    }
                }
                _ => {}
            }
        }
        typing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built(entries: Vec<Entry>, capital: Option<u32>) -> Built {
        Built {
            index: Index::build(entries, Fold::default()),
            capital,
            refused: BTreeMap::new(),
            fold: Fold::default(),
        }
    }

    #[test]
    fn victory_points_take_only_the_labels_drawn_for_the_viewing_country_at_the_rows_branch_and_hour() {
        let labels = Arc::new(Labels::default());
        let drawn = Stamped { labels: Arc::clone(&labels), ours: 3, branch: 7, hours: 40..=44 };
        assert!(drawn_labels(Some(&drawn), 3, 7, 44).is_ok_and(|taken| Arc::ptr_eq(&taken, &labels)));
        assert!(drawn_labels(Some(&drawn), 3, 7, 40).is_ok(), "unchanged since the hour they were made");
        assert_eq!(drawn_labels(Some(&drawn), 3, 7, 45).err(), Some(LABELS_LOADING), "laid out before this hour: a rename now is not in them");
        assert_eq!(drawn_labels(Some(&drawn), 3, 7, 39).err(), Some(LABELS_LOADING), "made after this hour");
        assert_eq!(drawn_labels(Some(&drawn), 3, 8, 44).err(), Some(LABELS_LOADING), "another branch's");
        assert_eq!(drawn_labels(Some(&drawn), 4, 7, 44).err(), Some(LABELS_NOT_DRAWN), "another country's labels");
        assert_eq!(drawn_labels(None, 3, 7, 44).err(), Some(LABELS_NOT_DRAWN), "the layer draws none: no earlier labels stand in");
    }

    #[test]
    fn the_index_key_and_its_labels_are_the_frame_it_is_built_over() {
        let labels = Arc::new(Labels::default());
        let drawn = Stamped { labels: Arc::clone(&labels), ours: 3, branch: 1, hours: 40..=44 };
        let (key, taken) = key_of(&viewer_foundation::test_support::test_frame(44), 3, Some(&drawn));
        assert_eq!((key.branch, key.hour, key.ours, key.labels), (1, 44, 3, Ok(Arc::as_ptr(&labels) as usize)));
        assert!(taken.is_ok_and(|taken| Arc::ptr_eq(&taken, &labels)));
        let (key, taken) = key_of(&viewer_foundation::test_support::test_frame(45), 3, Some(&drawn));
        assert_eq!((key.hour, key.labels), (45, Err(LABELS_LOADING)), "a frame published after the labels: the rename is not in them");
        assert_eq!(taken.err(), Some(LABELS_LOADING));
    }

    #[test]
    fn the_recorded_folding_table_folds_accents_and_a_withheld_one_is_named() {
        let entries = || vec![Entry { name: "Bérgen".into(), secondary: String::new(), display: "Bérgen".into(), target: Target::State(12) }];
        let mut refused = BTreeMap::new();
        let table = FoldingTable { letters: BTreeMap::from([(u32::from('é'), b'e')]) };
        let folded = fold(Ok(&table), &mut refused);
        let index = Index::build(entries(), folded);
        assert_eq!(index.search("berg", folded).unwrap().len(), 1);
        assert!(refused.is_empty());
        let withheld = engine_executable::availability::Withheld::new("diacritic folding table", "not recorded");
        let unfolded = fold(Err(withheld.to_string()), &mut refused);
        assert_eq!(Index::build(entries(), unfolded).unfolded, 1);
        assert_eq!(refused.keys().collect::<Vec<_>>(), ["diacritic folding table withheld: not recorded"]);
    }

    fn state() -> FindState {
        let mut find = FindState::default();
        find.index = Some((
            Key {
                branch: Default::default(),
                hour: Default::default(),
                ours: 1,
                labels: Err(LABELS_NOT_DRAWN),
            },
            Ok(built(
                vec![
                    Entry {
                        name: "Berlin".into(),
                        secondary: String::new(),
                        display: "Berlin".into(),
                        target: Target::Province(6521),
                    },
                    Entry {
                        name: "Bergen".into(),
                        secondary: "12".into(),
                        display: "Bergen".into(),
                        target: Target::State(12),
                    },
                ],
                Some(64),
            )),
        ));
        find.toggle(0.0);
        find
    }

    #[test]
    fn victory_points_come_from_the_maps_labels_and_rank_by_where_the_query_falls() {
        use map_display_data::landscape::LabelCandidate;
        let candidate = |province, victory_points, name: LabelName| LabelCandidate {
            province,
            victory_points,
            capital: false,
            controller: Some(1),
            name,
        };
        let named = |text: &str| LabelName::Value {
            text: Some(text.to_owned()),
        };
        let labels = Labels {
            perspective: Some("ITA".to_owned()),
            candidates: Arc::new(vec![
                candidate(9904, 40, named("Roma")),
                candidate(1, 0, named("No points")),
                candidate(2, 5, LabelName::Value { text: None }),
                candidate(3, 5, LabelName::Refused {
                    reason: "no owner".to_owned(),
                }),
            ]),
            allegiances: BTreeMap::from([(1, 2)]),
        };
        use viewer_foundation::mode_visibility::{PriorityIcon, Priorities};
        let defines = viewer_map_icons::labels::LabelDefines {
            after: vec![0.0, 9.0, 20.0],
            dot_cutoff: [0.0; 2],
            text_cutoff: [0.0; 2],
            max_points_for_percent: 1.0,
            capital_cutoff: [0.0; 2],
            priorities: Priorities::uniform([0; PriorityIcon::ALL.len()]),
        };
        let mut refused = BTreeMap::new();
        let mut entries = victory_point_entries(&labels, &defines, &mut refused);
        assert_eq!(entries.len(), 1);
        assert_eq!(refused.len(), 1);
        assert_eq!(
            entries[0].display,
            format!(
                "{}onmap_victorypoints_strip|8 Roma",
                viewer_native_gui::native_text::ICON
            )
        );
        entries.insert(
            0,
            Entry {
                name: "Emilia Romagna".into(),
                secondary: "4".into(),
                display: "Emilia Romagna".into(),
                target: Target::State(4),
            },
        );
        let index = Index::build(entries, Fold::default());
        let found: Vec<_> = index
            .search("Roma", Fold::default())
            .unwrap()
            .into_iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(found, ["Roma", "Emilia Romagna"]);
    }

    #[test]
    fn return_goes_to_the_first_result_and_closes_selecting_nothing() {
        let mut find = state();
        find.field.insert("ber", 0.0);
        find.search();
        assert_eq!(find.results(), ["Berlin", "Bergen"]);
        find.click(Control::Find);
        assert!(!find.open);
        assert_eq!(find.pending, Some(Target::Province(6521)));
    }

    #[test]
    fn home_goes_to_the_viewing_capital_and_keeps_the_view_open() {
        let mut find = state();
        find.click(Control::Home);
        assert!(find.open);
        assert_eq!(find.pending, Some(Target::State(64)));
        find.click(Control::Back);
        assert!(!find.open);
        find.toggle(1.0);
        assert!(find.typing());
        assert_eq!(find.field.text(), "");
    }
}
