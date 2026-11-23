use crate::{FogProduct, ProductHour};
use map_display_data as data;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::{DecodedFrame, TableView};
use std::{
    io,
    sync::{Arc, Mutex},
};
use viewer_foundation::tables::Limitations;

fn weather_kind(row: &replay_data::rows::WeatherRegionRow) -> Option<usize> {
    [
        row.light_rain,
        row.heavy_rain,
        row.snow,
        row.blizzard,
        row.sandstorm,
    ]
    .iter()
    .position(|&flag| flag != 0)
}

pub fn weather_field(frame: &DecodedFrame) -> io::Result<data::WeatherField> {
    let mut kinds = data::copy_index::CopyIndex::default();
    for row in dashboard_text::table_access::rows::<tables::WeatherRegions>(frame)?.rows() {
        if let Some(kind) = weather_kind(row) {
            kinds = kinds.inserted(row.id, kind)?;
        }
    }
    Ok(data::WeatherField {
        branch: frame.branch,
        hour: frame.hour,
        kinds,
    })
}

#[derive(Default)]
pub(crate) struct WeatherMemo {
    source: Option<replay_frame::frame::ViewIdentity>,
    kinds: data::copy_index::CopyIndex<u32, usize>,
}
impl WeatherMemo {
    pub fn project(&mut self, frame: &DecodedFrame) -> io::Result<data::WeatherField> {
        let view = &frame.tables[TableId::WeatherRegions.index()];
        let changes = replay_frame::frame::HourInput::new(frame).changes().since(
            TableId::WeatherRegions,
            self.source.as_ref(),
            weather_fields(),
        );
        let kinds = if matches!(changes, replay_frame::frame::TableChanges::Unchanged) {
            self.kinds.clone()
        } else if let replay_frame::frame::TableChanges::Rows(changes) = changes {
            let rows = frame.complete::<tables::WeatherRegions>()?;
            let mut kinds = self.kinds.clone();
            for change in changes.changed() {
                let key = change.key;
                if !change.inserted && change.fields & weather_fields() == 0 {
                    continue;
                }
                let id = u32::try_from(key).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "weather region key exceeds u32")
                })?;
                let index = rows.binary_search_by_key(&id, |row| row.id).map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "weather delta key absent from its certified result",
                    )
                })?;
                let kind = weather_kind(&rows[index]);
                if kinds.get(&id).copied() != kind {
                    if let Some(kind) = kind {
                        kinds = kinds.inserted(id, kind)?;
                    } else {
                        kinds = kinds.removed(&id)?;
                    }
                }
            }
            for &key in changes.deleted() {
                if let Ok(id) = u32::try_from(key) {
                    if kinds.contains_key(&id) {
                        kinds = kinds.removed(&id)?;
                    }
                }
            }
            kinds
        } else {
            let kinds = weather_field(frame)?.kinds;
            if kinds == self.kinds {
                self.kinds.clone()
            } else {
                kinds
            }
        };
        self.source = Some(view.identity());
        self.kinds = kinds.clone();
        Ok(data::WeatherField {
            branch: frame.branch,
            hour: frame.hour,
            kinds,
        })
    }
}
pub(crate) fn weather_fields() -> u64 {
    static FIELDS: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *FIELDS.get_or_init(|| {
        (0..TableId::WeatherRegions.schema().len())
            .filter(|&field| {
                TableId::WeatherRegions
                    .field_name(field)
                    .is_some_and(|name| {
                        ["light_rain", "heavy_rain", "snow", "blizzard", "sandstorm"]
                            .contains(&name)
                    })
            })
            .fold(0, |mask, field| mask | (1u64 << field))
    })
}

impl FogProduct {
    pub fn display(&self) -> data::Fog {
        data::Fog {
            intel: Arc::clone(&self.intel),
            limitations: self.limitations.lines(),
            header: (*self.header).clone(),
            map_text: self.map_text.display(),
            music: self.music.as_deref().cloned(),
            battles: self.battles.as_deref().cloned(),
        }
    }
    pub fn from_display(fog: data::Fog) -> io::Result<Self> {
        Ok(Self {
            intel: fog.intel,
            limitations: Limitations::from_lines(fog.limitations),
            header: Arc::new(fog.header),
            map_text: Arc::new(
                viewer_map_text::animated_map_text::events::Prepared::from_display(fog.map_text)?,
            ),
            music: fog.music.map(Arc::new),
            battles: fog.battles.map(Arc::new),
            music_inputs: None,
        })
    }
}

#[derive(Default)]
pub struct SnowMemo {
    values: Mutex<Vec<std::sync::Weak<map_display_data::SnowMud>>>,
}
impl SnowMemo {
    pub fn new() -> Self {
        Self::default()
    }
    fn intern(&self, snow: map_display_data::SnowMud) -> Arc<map_display_data::SnowMud> {
        self.intern_shared(Arc::new(snow))
    }
    fn intern_shared(&self, snow: Arc<map_display_data::SnowMud>) -> Arc<map_display_data::SnowMud> {
        let mut values = self
            .values
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        values.retain(|value| value.strong_count() != 0);
        if let Some(value) = values
            .iter()
            .rev()
            .filter_map(std::sync::Weak::upgrade)
            .find(|value| Arc::ptr_eq(value, &snow) || **value == *snow)
        {
            return value;
        }
        values.push(Arc::downgrade(&snow));
        snow
    }
}

impl ProductHour<'_> {
    pub(crate) fn country_maps(
        &self,
    ) -> io::Result<(
        std::collections::BTreeMap<u32, u32>,
        std::collections::BTreeMap<u32, String>,
    )> {
        let links = viewer_foundation::tables::core::<tables::CountryLinks>(self.frame)?
            .iter()
            .map(|r| (r.id, r.canonical_slot))
            .collect();
        let tags = viewer_foundation::tables::core::<tables::Countries>(self.frame)?
            .iter()
            .map(|r| {
                (
                    r.canonical_slot,
                    String::from_utf8_lossy(&r.tag).into_owned(),
                )
            })
            .collect();
        Ok((links, tags))
    }

    pub fn seed_country(
        &self,
        choice: &viewer_foundation::perspective::Choice,
        id: u32,
        slot: u32,
    ) -> io::Result<()> {
        let (links, tags) = self.country_maps()?;
        if !viewer_foundation::tables::core::<tables::Countries>(self.frame)?
            .iter()
            .any(|row| row.id == id && row.canonical_slot == slot)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "country instance is not present this hour",
            ));
        }
        let projected = viewer_runtime::memo::project(
            self.frame,
            viewer_foundation::perspective::VISION_TABLES,
        );
        let vision = viewer_foundation::perspective::Vision::read_resolved(
            &projected,
            &links,
            &tags,
            choice.fog,
            (
                slot,
                viewer_foundation::perspective::Resolution::Requested,
            ),
            viewer_foundation::visibility::FogContent::loaded(&self.content.fog),
        )?;
        self.seed_vision(choice, Arc::new(vision));
        Ok(())
    }

    pub fn snow_grid(&self, cells: &map_snow::SnowCells) -> io::Result<map_display_data::SnowMud> {
        map_snow::grid_from_cells(cells, self.frame)
    }

    pub fn snow(
        &self,
        cells: &map_snow::SnowCells,
        winter: &impl map_snow::winter::Factors,
        memo: &SnowMemo,
    ) -> io::Result<Arc<map_display_data::SnowMud>> {
        let built = map_snow::build(cells, self.frame, winter)?;
        Ok(memo.intern(built))
    }
}

pub fn evidence(frame: &DecodedFrame, tables: &[TableId]) -> Vec<data::Evidence> {
    tables
        .iter()
        .map(|&table| {
            let view = &*frame.tables[table.index()];
            let status = match view {
                TableView::Complete(rows) if rows.is_empty() => data::Availability::KnownEmpty {},
                TableView::Complete(_) => data::Availability::Known {},
                TableView::Truncated { missing, .. } => data::Availability::Unknown {
                    reason: format!("{missing} missing rows"),
                    evidence_hour: frame.hour,
                    scope: format!("{table:?}"),
                    continuity: "truncated source census".into(),
                },
                TableView::Unavailable { reason } => data::Availability::Unavailable {
                    reason: reason.clone(),
                    extent: format!("branch {} hour {}", frame.branch, frame.hour),
                },
                TableView::NotRecorded { mode } => data::Availability::NotRecorded {
                    component: format!("{table:?}"),
                    capture_contract: mode.to_string(),
                },
            };
            data::Evidence {
                component: format!("{table:?}"),
                supplying_hour: frame.tables[table.index()].sampled_at(),
                status,
            }
        })
        .collect()
}

pub struct SnowContent {
    fold: Mutex<map_snow::fold::Fold>,
    pub cells: map_snow::SnowCells,
    pub winter: map_snow::winter::Winter,
}
impl SnowContent {
    pub fn project(
        &self,
        frame: &DecodedFrame,
        memo: &SnowMemo,
    ) -> io::Result<Arc<map_display_data::SnowMud>> {
        let projected = self
            .fold
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .project(&self.cells, frame, &self.winter)?;
        Ok(memo.intern_shared(projected))
    }
    pub fn new(
        map: &content_reader_map::map::MapData,
        permanent: &[bool],
        winter: map_snow::winter::Winter,
    ) -> io::Result<Self> {
        Ok(Self {
            fold: Mutex::new(Default::default()),
            cells: map_snow::SnowCells::new(map, permanent)?,
            winter,
        })
    }
}
impl crate::ProductContent {
    pub fn with_snow_loader(mut self, loader: crate::loader::Loader) -> Self {
        self.snow_loader = Some(loader);
        self
    }
    pub fn with_snow(self, value: io::Result<SnowContent>) -> Self {
        let value = value.and_then(|snow| {
            self.record_snow_content(&snow)?;
            Ok(snow)
        });
        if let Err(error) = &value {
            self.reads.session_error(error);
        }
        let _ = self.snow.set(value.map_err(|e| (e.kind(), e.to_string())));
        self
    }
    fn record_snow_content(&self, snow: &SnowContent) -> io::Result<()> {
        self.reads
            .operand_stream(b"snow-cells-and-winter/v1", |mut bytes| {
                snow.cells.write_identity(&mut bytes)?;
                for value in snow
                    .winter
                    .defines
                    .per_province
                    .into_iter()
                    .chain(snow.winter.defines.total)
                {
                    bytes.write_all(&value.to_bits().to_le_bytes())?;
                }
                bytes.write_all(&(snow.winter.records.len() as u64).to_le_bytes())?;
                for records in &snow.winter.records {
                    bytes.write_all(&(records.len() as u64).to_le_bytes())?;
                    for record in records {
                        bytes.write_all(&record.province.to_le_bytes())?;
                        bytes.write_all(&record.weight.to_bits().to_le_bytes())?;
                    }
                }
                Ok(())
            })
    }
    pub fn snow_content(&self) -> io::Result<&SnowContent> {
        if self.snow.get().is_none() {
            let _building = self
                .snow_building
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.snow.get().is_none() {
                let result = self
                    .snow_loader
                    .as_ref()
                    .ok_or_else(|| io::Error::other("snow content not configured"))
                    .and_then(|loader| read_snow(loader, &self.map));
                let result = result.and_then(|snow| {
                    self.record_snow_content(&snow)?;
                    Ok(snow)
                });
                if let Err(error) = &result {
                    if error.kind() == io::ErrorKind::WouldBlock
                        || error.kind() == io::ErrorKind::OutOfMemory
                    {
                        return Err(io::Error::new(error.kind(), error.to_string()));
                    }
                }
                let _ = self.snow.set(result.map_err(|e| (e.kind(), e.to_string())));
            }
        }
        self.snow
            .get()
            .expect("completed snow content")
            .as_ref()
            .map_err(|(kind, reason)| io::Error::new(*kind, reason.clone()))
    }
}
fn read_snow(loader: &crate::loader::Loader, map: &content_reader_map::map::MapData) -> io::Result<SnowContent> {
    loader.reads.load_retryable(|layers| {
        let (definitions, defaults) = content_reader_key_names::terrain::read(layers)?;
        let terrain = content_reader_image::bmp::decode_indexed(
            &layers.read(&format!("map/{}", defaults.terrain))?,
        )?;
        if [terrain.width, terrain.height] != [map.width, map.height] {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "snow terrain dimensions differ from map",
            ));
        }
        let cells =
            map_snow::SnowCells::new(map, &map_snow::permanent_mask(&definitions, &terrain))?;
        let defines = content_reader_defines::defines::read(layers)?;
        let regions = content_reader_regions::regions::read(layers)?.regions;
        let winter = map_snow::winter::Winter::load(layers, &defines, map, &regions)?;
        Ok(SnowContent {
            cells,
            winter,
            fold: Mutex::new(Default::default()),
        })
    })
}
