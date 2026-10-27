use crate::{FogProduct, ProductHour};
use map_display_data as data;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::{DecodedFrame, TableView};
use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Mutex},
};
use viewer_foundation::map::tables::Limitations;

pub fn weather_field(frame: &DecodedFrame) -> io::Result<data::WeatherField> {
    let mut out = data::WeatherField {
        branch: frame.branch,
        hour: frame.hour,
        ..Default::default()
    };
    for r in viewer_foundation::table_access::rows::<tables::WeatherRegions>(frame)?.rows() {
        if let Some(kind) = [r.light_rain, r.heavy_rain, r.snow, r.blizzard, r.sandstorm]
            .iter()
            .position(|v| *v != 0)
        {
            out.kinds.insert(r.id, kind);
        }
    }
    Ok(out)
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
                viewer_world::map::animated_map_text::events::Prepared::from_display(fog.map_text)?,
            ),
            music: fog.music.map(Arc::new),
            battles: fog.battles.map(Arc::new),
        })
    }
}

pub struct SnowMemo {
    values: Mutex<VecDeque<Arc<map_snow::SnowMud>>>,
    capacity: usize,
}
impl SnowMemo {
    pub fn new(capacity: usize) -> Self {
        Self {
            values: Mutex::new(VecDeque::new()),
            capacity,
        }
    }
    fn intern(&self, snow: map_snow::SnowMud) -> Arc<map_snow::SnowMud> {
        let mut values = self
            .values
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(value) = values.iter().find(|v| ***v == snow) {
            return Arc::clone(value);
        }
        let snow = Arc::new(snow);
        if self.capacity != 0 {
            values.push_back(Arc::clone(&snow));
        }
        while values.len() > self.capacity {
            values.pop_front();
        }
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
        let links = viewer_foundation::map::tables::core::<tables::CountryLinks>(self.frame)?
            .iter()
            .map(|r| (r.id, r.canonical_slot))
            .collect();
        let tags = viewer_foundation::map::tables::core::<tables::Countries>(self.frame)?
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
        choice: &viewer_foundation::map::perspective::Choice,
        id: u32,
        slot: u32,
    ) -> io::Result<()> {
        let (links, tags) = self.country_maps()?;
        if !viewer_foundation::map::tables::core::<tables::Countries>(self.frame)?
            .iter()
            .any(|row| row.id == id && row.canonical_slot == slot)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "country instance is not present this hour",
            ));
        }
        let projected = viewer_foundation::map::memo::project(
            self.frame,
            viewer_foundation::map::perspective::VISION_TABLES,
        );
        let vision = viewer_foundation::map::perspective::Vision::read_resolved(
            &projected,
            &links,
            &tags,
            choice.fog,
            (
                slot,
                viewer_foundation::map::perspective::Resolution::Requested,
            ),
            viewer_foundation::map::visibility::FogContent::loaded(&self.content.fog),
        )?;
        self.seed_vision(choice, Arc::new(vision));
        Ok(())
    }

    pub fn snow_grid(&self, cells: &map_snow::SnowCells) -> io::Result<map_snow::SnowMud> {
        map_snow::grid_from_cells(cells, self.frame)
    }

    pub fn snow(
        &self,
        cells: &map_snow::SnowCells,
        winter: &impl map_snow::winter::Factors,
        memo: &SnowMemo,
    ) -> io::Result<Arc<map_snow::SnowMud>> {
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
    pub cells: map_snow::SnowCells,
    pub winter: map_snow::winter::Winter,
}
impl SnowContent {
    pub fn new(
        map: &installed_content_core::map::MapData,
        permanent: &[bool],
        winter: map_snow::winter::Winter,
    ) -> io::Result<Self> {
        Ok(Self {
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
        use std::io::Write;
        self.reads.operand_stream(b"snow-cells-and-winter/v1", |mut out| {
            snow.cells.write_identity(&mut out)?;
            for value in snow
                .winter
                .defines
                .per_province
                .into_iter()
                .chain(snow.winter.defines.total)
            {
                out.write_all(&value.to_bits().to_le_bytes())?;
            }
            out.write_all(&(snow.winter.records.len() as u64).to_le_bytes())?;
            for records in &snow.winter.records {
                out.write_all(&(records.len() as u64).to_le_bytes())?;
                for record in records {
                    out.write_all(&record.province.to_le_bytes())?;
                    out.write_all(&record.weight.to_bits().to_le_bytes())?;
                }
            }
            Ok(())
        })?;
        Ok(())
    }
    pub fn snow_content(&self) -> io::Result<&SnowContent> {
        self.snow
            .get_or_init(|| {
                let result = self
                    .snow_loader
                    .as_ref()
                    .ok_or_else(|| io::Error::other("snow content not configured"))
                    .and_then(|loader| loader.snow(&self.map));
                result
                    .and_then(|snow| {
                        self.record_snow_content(&snow)?;
                        Ok(snow)
                    })
                    .map_err(|e| (e.kind(), e.to_string()))
            })
            .as_ref()
            .map_err(|(kind, reason)| io::Error::new(*kind, reason.clone()))
    }
}
impl crate::loader::Loader {
    pub fn snow(&self, map: &installed_content_core::map::MapData) -> io::Result<SnowContent> {
        self.reads.load(|layers| {
            let (definitions, defaults) = installed_content_core::terrain::read(layers)?;
            let terrain = installed_content_image::bmp::decode_indexed(
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
            let defines = installed_content_core::defines::read(layers)?;
            let regions = installed_content_regions::regions::read(layers)?.regions;
            let winter = map_snow::winter::Winter::load(layers, &defines, map, &regions)?;
            Ok(SnowContent { cells, winter })
        })
    }
}
