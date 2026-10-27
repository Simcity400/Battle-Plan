use map_display_data::{self as data, LayerResult, Snapshot};
use map_products::{FogMemos, ProductContent, ProductHour};
use product_codec::Product;
use replay_frame::frame::{DecodedFrame, ViewIdentity, view_reads};
use serde_json::{Value, json};
use std::{fs, io, path::Path, time::Instant};
use viewer_foundation::{
    map::perspective::{Choice, Country},
    profiling::{BuilderSpans, current_thread_cpu_time},
};

pub struct Pilot {
    content: ProductContent,
    fog: FogMemos,
    snow: map_products::display::SnowMemo,
}

impl Pilot {
    pub fn load(
        layers: &installed_content_core::ResolvedLayers,
        user: &Path,
        language: &str,
    ) -> io::Result<Self> {
        let content = ProductContent::load(layers, user, language)?;
        // Close lazy snow content inside the cold-content stopwatch, including named failures.
        content.prepare_display_content()?;
        Ok(Self {
            content,
            fog: FogMemos::new(8),
            snow: map_products::display::SnowMemo::new(8),
        })
    }

    pub fn content_witness(&self, output: &Path) -> io::Result<Value> {
        let witness = self.content.reads.witness()?;
        let mut entries = 0u64;
        let mut bytes = 0u64;
        let mut file = std::io::BufWriter::new(fs::File::create(output)?);
        witness.visit(|_, entry| {
            use std::io::Write;
            entries += 1;
            bytes += entry.len() as u64;
            file.write_all(&(entry.len() as u64).to_le_bytes())?;
            file.write_all(entry)
        })?;
        use std::io::Write;
        file.flush()?;
        Ok(json!({"digest":witness.digest, "entries":entries, "canonicalBytes":bytes}))
    }

    pub fn observe(
        &self,
        frame: &DecodedFrame,
        all_countries: bool,
        folder: &Path,
        mut emit: impl FnMut(Value) -> io::Result<()>,
    ) -> io::Result<()> {
        fs::create_dir_all(folder)?;
        let mut choices = vec![("recorded".to_owned(), Country::Recorded, None)];
        let mut census_complete = true;
        if all_countries {
            match viewer_foundation::map::tables::core::<replay_data::rows::tables::Countries>(
                frame,
            ) {
                Ok(countries) => {
                    for country in countries {
                        choices.push((
                            format!("country-{}-{}", country.id, country.canonical_slot),
                            Country::Tag(String::from_utf8_lossy(&country.tag).into_owned()),
                            Some((country.id, country.canonical_slot)),
                        ));
                    }
                }
                Err(error) => {
                    census_complete = false;
                    emit(
                        json!({"measurement":"display-products-country-census", "branch":frame.branch,
                        "hour":frame.hour,"status":"unavailable","reason":error.to_string()}),
                    )?;
                }
            }
        }
        emit(
            json!({"measurement":"display-products-country-census", "branch":frame.branch,"hour":frame.hour,
            "requests":choices.len(),"allCountries":all_countries,"censusComplete":census_complete,"identityScope":"branch/hour/native-id/canonical-slot; no inferred continuity across unread hours"}),
        )?;
        for (key, country, instance) in choices {
            for fog in [true, false] {
                let choice = Choice {
                    country: country.clone(),
                    fog,
                };
                let clock = Clock::new();
                let (built, reads) = view_reads::record(|| {
                    let hour = ProductHour::new(&self.content, frame, 0.0);
                    if let Some((id, slot)) = instance {
                        hour.seed_country(&choice, id, slot)?;
                    }
                    hour.fog(&choice, &self.fog, &mut BuilderSpans::new(false))
                });
                let timing = clock.finish();
                let source = source_views(frame, &reads);
                let result = built.as_ref().map(|v| v.display());
                let label = format!("{key}-fog-{}", u8::from(fog));
                self.record(
                    frame,
                    "fog",
                    &label,
                    result.as_ref().map_err(|error| *error),
                    timing,
                    &source,
                    folder,
                    &mut emit,
                )?;
                if let Ok(value) = result {
                    let included = json!({"includedIn":"fog", "scope":"no additional projection"});
                    self.record(
                        frame,
                        "limitations",
                        &label,
                        Ok(&value.limitations),
                        included.clone(),
                        &source,
                        folder,
                        &mut emit,
                    )?;
                    self.record(
                        frame,
                        "header",
                        &label,
                        Ok(&value.header),
                        included.clone(),
                        &source,
                        folder,
                        &mut emit,
                    )?;
                    self.record(
                        frame,
                        "map-text",
                        &label,
                        Ok(&value.map_text),
                        included.clone(),
                        &source,
                        folder,
                        &mut emit,
                    )?;
                    self.record(
                        frame,
                        "music",
                        &label,
                        Ok(&value.music),
                        included.clone(),
                        &source,
                        folder,
                        &mut emit,
                    )?;
                    self.record(
                        frame,
                        "battles",
                        &label,
                        Ok(&value.battles),
                        included,
                        &source,
                        folder,
                        &mut emit,
                    )?;
                    let clock = Clock::new();
                    let before = replay_data::measurement::snapshot();
                    let (exact, reads) = view_reads::record(|| consume_fog(value));
                    let exact = exact?;
                    let source_unchanged = before == replay_data::measurement::snapshot();
                    emit(
                        json!({"measurement":"display-products-consume", "layer":"fog", "branch":frame.branch,"hour":frame.hour,
                        "selector":label,"canonicalEqual":exact,"timing":clock.finish(),"sourceTableViews":reads.len(),
                        "readerOperationsUnchanged":source_unchanged,"scope":"closed DTO decode/install adapter; not the complete viewer source-free gate"}),
                    )?;
                    if !exact || !reads.is_empty() || !source_unchanged {
                        return Err(io::Error::other(
                            "fog display transport changed bytes or read source",
                        ));
                    }
                }
            }
        }
        let hour = ProductHour::new(&self.content, frame, 0.0);
        let clock = Clock::new();
        let (weather, reads) = view_reads::record(|| hour.weather_field());
        let timing = clock.finish();
        self.record(
            frame,
            "weather",
            "global",
            weather.as_ref(),
            timing,
            &source_views(frame, &reads),
            folder,
            &mut emit,
        )?;
        let clock = Clock::new();
        let (snow, reads) = view_reads::record(|| {
            self.content
                .snow_content()
                .and_then(|content| hour.snow(&content.cells, &content.winter, &self.snow))
        });
        let timing = clock.finish();
        self.record(
            frame,
            "snow",
            "global",
            snow.as_deref(),
            timing,
            &source_views(frame, &reads),
            folder,
            &mut emit,
        )?;
        Ok(())
    }

    fn record<T: Product>(
        &self,
        frame: &DecodedFrame,
        layer: &str,
        selector: &str,
        result: Result<&T, &io::Error>,
        projection: Value,
        source_views: &Value,
        folder: &Path,
        emit: &mut impl FnMut(Value) -> io::Result<()>,
    ) -> io::Result<()> {
        let clock = Clock::new();
        let result = match result {
            Ok(value) => LayerResult::Value {
                payload: product_codec::encode(value)?,
            },
            Err(error) => LayerResult::Failed {
                kind: format!("{:?}", error.kind()),
                display: error.to_string(),
            },
        };
        let (raw, stored) = match &result {
            LayerResult::Value { payload } => (
                payload.len(),
                zstd::bulk::compress(payload, 3)?.len().min(payload.len()),
            ),
            LayerResult::Failed { .. } => (0, 0),
        };
        let snapshot = Snapshot {
            branch: frame.branch,
            hour: frame.hour,
            selector: selector.to_owned(),
            content: self.content.reads.witness()?.digest,
            evidence: map_products::display::evidence(frame, &replay_data::rows::TableId::ALL),
            result,
        };
        let bytes = product_codec::encode(&snapshot)?;
        let before = replay_data::measurement::snapshot();
        let (round_trip, reads) = view_reads::record(|| -> io::Result<bool> {
            let decoded: Snapshot = product_codec::decode(&bytes)?;
            if let LayerResult::Value { payload } = &decoded.result {
                let value: T = product_codec::decode(payload)?;
                if product_codec::encode(&value)? != *payload {
                    return Ok(false);
                }
            }
            Ok(product_codec::encode(&decoded)? == bytes)
        });
        let canonical_equal = round_trip?;
        let source_unchanged = before == replay_data::measurement::snapshot();
        if !canonical_equal || !reads.is_empty() || !source_unchanged {
            return Err(io::Error::other(
                "display codec changed bytes or read source",
            ));
        }
        let identity = data::identity(&snapshot)?;
        let codec = clock.finish();
        let name = format!("{}-{}-{layer}-{selector}.bin", frame.branch, frame.hour);
        fs::write(folder.join(&name), &bytes)?;
        emit(
            json!({"measurement":"display-products-cost","layer":layer,"selector":selector,
            "branch":frame.branch,"hour":frame.hour,"projection":projection,"codec":codec,
            "payloadRawBytes":raw,"payloadStoredBytes":stored,"snapshotBytes":bytes.len(),"compression":"zstd-3-or-raw",
            "identity":identity.digest,"schema":product_codec::schema_digest::<T>(),"snapshotSchema":product_codec::schema_digest::<Snapshot>(),
            "contract":data::CONTRACT,"snapshot":name,"status":match snapshot.result { LayerResult::Value { .. } => "value", LayerResult::Failed { .. } => "failed" },
            "sourcePath":"borrowed-reconstruction","sourceViews":source_views,
            "canonicalEqual":canonical_equal,"consumeSourceTableViews":reads.len(),"consumeReaderOperationsUnchanged":source_unchanged,
            "evidenceScope":"full source-view status census; sourceViews lists distinct observed table dereferences (fog ancillary shares parent census)",
            "isFogBreakdown":matches!(layer,"limitations"|"header"|"map-text"|"music"|"battles"),
            "byteScope":"full hourly operands, not I/P or a product-store estimate; fog includes its ancillary breakdown"}),
        )
    }
}

fn source_views(frame: &DecodedFrame, views: &[ViewIdentity]) -> Value {
    json!(
        replay_data::rows::TableId::ALL
            .iter()
            .filter(|table| views.iter().any(|v| v.is(&frame.tables[table.index()])))
            .map(|table| format!("{table:?}"))
            .collect::<Vec<_>>()
    )
}

fn consume_fog(value: data::Fog) -> io::Result<bool> {
    let bytes = product_codec::encode(&value)?;
    let restored = map_products::FogProduct::from_display(product_codec::decode(&bytes)?)?;
    Ok(product_codec::encode(&restored.display())? == bytes)
}

struct Clock {
    wall: Instant,
    cpu: Option<std::time::Duration>,
}
impl Clock {
    fn new() -> Self {
        Self {
            wall: Instant::now(),
            cpu: current_thread_cpu_time(),
        }
    }
    fn finish(self) -> Value {
        json!({"wallUs":self.wall.elapsed().as_secs_f64()*1e6,
        "cpuUs":self.cpu.zip(current_thread_cpu_time()).and_then(|(a,b)| b.checked_sub(a)).map(|d| d.as_secs_f64()*1e6)})
    }
}
