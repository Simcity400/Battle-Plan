use crate::invalid;
use installed_content_core::ResolvedLayers;
use map_snow::{SnowCells, SnowMud, winter::Winter};
use replay::container::Replay;
use replay_data::{BranchId, Hour};
use replay_frame::frame::DecodedFrame;
use std::{io, sync::Arc};
use store::{SourceKey, Store, Writer};

mod background;
mod binding;
mod codec;
mod reader;
pub use background::Background;
pub use binding::Key;
pub use codec::Limits;
pub use reader::{Reader, Restore};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Coverage {
    pub endpoints: u64,
    pub first: Hour,
    pub last: Hour,
}

pub const STRIDE: Hour = 40;
pub const PRODUCER_VERSION: u32 = 1;
pub const CONTRACT: &[u8] =
    b"snow-mud/cells4/retained-edges/raw-winter-f32-bits/no-perspective-dependence/v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Available,
    Unavailable(String),
    Unsupported(String),
}

pub struct Inputs {
    pub cells: SnowCells,
    pub winter: Winter,
}
impl Inputs {
    pub fn load(layers: &ResolvedLayers) -> io::Result<Self> {
        let map = installed_content_core::map::load(layers)?;
        let (definitions, defaults) = installed_content_core::terrain::read(layers)?;
        let terrain = installed_content_image::bmp::decode_indexed(
            &layers.read(&format!("map/{}", defaults.terrain))?,
        )?;
        if [terrain.width, terrain.height] != [map.width, map.height] {
            return Err(invalid("snow terrain dimensions differ from map"));
        }
        let cells = SnowCells::new(&map, &map_snow::permanent_mask(&definitions, &terrain))?;
        let defines = installed_content_core::defines::read(layers)?;
        let regions = installed_content_regions::regions::read(layers)?.regions;
        let winter = Winter::load(layers, &defines, &map, &regions)?;
        Ok(Self { cells, winter })
    }
    pub fn limits(&self) -> Limits {
        Limits::new(&self.cells, &self.winter)
    }
    pub fn build(&self, frame: &DecodedFrame) -> io::Result<SnowMud> {
        let _cpu = replay_frame::cpu::scope("export", Some("snow.product"));
        map_snow::build(&self.cells, frame, &self.winter)
    }
}

pub struct Session {
    pub key: Key,
    inputs: Result<Inputs, Status>,
    prefix_length: u64,
    prefix_digest: [u8; 32],
    previous: Option<(binding::Binding, [u8; 32])>,
}

fn cache_validation(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
    )
}
fn reuse_result(result: io::Result<()>) -> io::Result<bool> {
    match result {
        Ok(()) => Ok(true),
        Err(error) if cache_validation(&error) => Ok(false),
        Err(error) => Err(error),
    }
}
fn saved_binding(store: &Store, current: &SourceKey) -> io::Result<Option<binding::Binding>> {
    let Some(previous) = store.saved_key()? else {
        return Ok(None);
    };
    let section = "map-export-snow-v1/binding";
    let result = if &previous == current {
        store.read(current, section, 0)
    } else {
        store.read_prefix(&previous, current, section, 0)
    }
    .and_then(|range| range.map(binding::Binding::read).transpose());
    match result {
        Err(error) if cache_validation(&error) => Ok(None),
        result => result,
    }
}

impl Session {
    pub fn bind(
        replay: &Replay,
        layers: Option<&ResolvedLayers>,
        prefix_length: u64,
        store: &Store,
        source: &SourceKey,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<Self> {
        let _cpu = replay_frame::cpu::scope("export-bind", Some("snow.binding"));
        check()?;
        let inputs = layers.map_or_else(
            || {
                Err(Status::Unavailable(
                    "snow: game content not configured".into(),
                ))
            },
            |layers| {
                if layers.presentation().shows_visual_mods() {
                    return Err(Status::Unsupported(
                        "snow export: visual-mod post-processing".into(),
                    ));
                }
                Inputs::load(layers).map_err(|error| Status::Unavailable(error.to_string()))
            },
        );
        check()?;
        Self::from_inputs(replay, inputs, prefix_length, store, source, check)
    }
    fn from_inputs(
        replay: &Replay,
        inputs: Result<Inputs, Status>,
        prefix_length: u64,
        store: &Store,
        source: &SourceKey,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<Self> {
        let old = saved_binding(store, source)?;
        let (recording, prefix_digest, previous) = binding::witness(
            replay,
            prefix_length,
            old.as_ref().map(|b| b.prefix_length),
            &mut check,
        )?;
        let mut key = Key::for_inputs(
            inputs.as_ref().map(|i| (&i.cells, &i.winter)),
            PRODUCER_VERSION,
        )?;
        key.recording = recording;
        check()?;
        Ok(Self {
            key,
            inputs,
            prefix_length,
            prefix_digest,
            previous: old.zip(previous),
        })
    }
    pub fn ready(store: &Store, source: &SourceKey, key: &Key) -> io::Result<bool> {
        Ok(saved_binding(store, source)?.is_some_and(|binding| binding.key == *key))
    }
    pub fn reusable(
        &self,
        store: &Store,
        current: &SourceKey,
        branches: impl IntoIterator<Item = BranchId>,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<bool> {
        check()?;
        if !Self::ready(store, current, &self.key)? {
            return Ok(false);
        }
        self.validate(store, current, branches, check)
    }
    // The caller must first accept the old recording-index and session metadata prefix.
    pub fn can_inherit(
        &self,
        store: &Store,
        current: &SourceKey,
        branches: impl IntoIterator<Item = BranchId>,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<bool> {
        let _cpu = replay_frame::cpu::scope("export-bind", Some("snow.prefix"));
        check()?;
        let Some((old, proof)) = &self.previous else {
            return Ok(false);
        };
        if old.key.digest() != self.key.digest()
            || old.prefix_length > self.prefix_length
            || old.prefix_digest != *proof
            || saved_binding(store, current)?.is_none_or(|binding| binding.key != old.key)
        {
            return Ok(false);
        }
        self.validate(store, current, branches, check)
    }
    fn validate(
        &self,
        store: &Store,
        current: &SourceKey,
        branches: impl IntoIterator<Item = BranchId>,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<bool> {
        check()?;
        let Some(previous) = store.saved_key()? else {
            return Ok(false);
        };
        let read = |section: &str, ordinal| {
            if &previous == current {
                store.read(current, section, ordinal)
            } else {
                store.read_prefix(&previous, current, section, ordinal)
            }
        };
        let maximum = match &self.inputs {
            Ok(inputs) => inputs.limits().maximum()?,
            Err(Status::Unavailable(reason) | Status::Unsupported(reason)) => {
                codec::reason_length(reason)?
            }
            Err(Status::Available) => return Err(invalid("snow session has no available inputs")),
        };
        reuse_result((|| {
            for branch in branches {
                check()?;
                let section = self.key.records(branch);
                let count = if &previous == current {
                    store.range_count(current, &section)?
                } else {
                    store.prefix_range_count(&previous, current, &section)?
                }
                .unwrap_or(0);
                let mut previous_hour = None;
                for ordinal in 0..count {
                    check()?;
                    let range = read(&section, ordinal)?
                        .ok_or_else(|| invalid("snow inherited index is dangling"))?;
                    let header = codec::header(&range)?;
                    if header.branch != branch
                        || header.kind > 2
                        || previous_hour.is_some_and(|hour| hour >= header.hour)
                    {
                        return Err(invalid(
                            "snow inherited index has wrong provenance, order or status",
                        ));
                    }
                    let body = codec::unpack(
                        range,
                        &self.key,
                        Some((branch, header.hour)),
                        header.kind,
                        if header.kind == 0 { 40 } else { maximum },
                    )?;
                    check()?;
                    if header.kind == 0 {
                        if body.len() != 40 {
                            return Err(invalid("snow inherited manifest length differs"));
                        }
                        let payload = u64::from_le_bytes(body[..8].try_into().unwrap());
                        let range = read(&self.key.section("payloads"), payload)?
                            .ok_or_else(|| invalid("snow inherited payload is dangling"))?;
                        let limits = self
                            .inputs
                            .as_ref()
                            .map_err(|_| invalid("snow payload without admitted inputs"))?
                            .limits();
                        let bytes = codec::unpack(range, &self.key, None, 3, limits.maximum()?)?;
                        if codec::digest(&bytes) != body[8..] {
                            return Err(invalid("snow inherited payload digest differs"));
                        }
                        codec::decode_product(&bytes, limits)?;
                    } else {
                        codec::decode_reason(&body)?;
                    }
                    check()?;
                    previous_hour = Some(header.hour);
                }
            }
            check()
        })())
    }
    pub fn has_history(
        &self,
        store: &Store,
        current: &SourceKey,
        branches: impl IntoIterator<Item = BranchId>,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<bool> {
        check()?;
        let Some(previous) = store.saved_key()? else {
            return Ok(false);
        };
        let count = |section: &str| {
            if &previous == current {
                store.range_count(current, section)
            } else {
                store.prefix_range_count(&previous, current, section)
            }
        };
        if count(&self.key.section("payloads"))?.unwrap_or(0) != 0 {
            return Ok(true);
        }
        for branch in branches {
            check()?;
            if count(&self.key.records(branch))?.unwrap_or(0) != 0
                || count(&self.key.section(&format!("cursor/{branch}")))?.unwrap_or(0) != 0
            {
                return Ok(true);
            }
        }
        check()?;
        Ok(false)
    }
    pub fn initialise(&self, writer: &mut Writer) -> io::Result<()> {
        let bytes = binding::Binding {
            key: self.key.clone(),
            prefix_length: self.prefix_length,
            prefix_digest: self.prefix_digest,
        }
        .encode();
        writer.write_at(&self.key.section("binding"), 0, bytes.as_slice())
    }
    pub fn inherit(&self, writer: &mut Writer) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("export-io", Some("snow.inherit"));
        if let Some(range) = writer.read_at(&self.key.section("binding"), 0)? {
            let old = binding::Binding::read(range)?;
            if old.key != self.key
                && self.previous.as_ref().is_none_or(|(binding, proof)| {
                    binding.key != old.key
                        || binding.prefix_digest != *proof
                        || binding.key.digest() != self.key.digest()
                })
            {
                return Err(invalid(
                    "snow inheritance binding changed after prefix verification",
                ));
            }
        }
        self.initialise(writer)
    }
    pub fn observe(
        &self,
        writer: &mut Writer,
        frame: &DecodedFrame,
        branch_last: Hour,
    ) -> io::Result<()> {
        let progress = self.key.section(&format!("cursor/{}", frame.branch));
        let previous = writer
            .read_at(&progress, 0)?
            .map(|file| -> io::Result<Hour> {
                let bytes = codec::read_exact_range(file, 4)?;
                Ok(Hour::from_le_bytes(bytes.try_into().unwrap()))
            })
            .transpose()?;
        self.observe_after(writer, frame, branch_last, previous)
    }
    pub fn observe_after(
        &self,
        writer: &mut Writer,
        frame: &DecodedFrame,
        branch_last: Hour,
        previous: Option<Hour>,
    ) -> io::Result<()> {
        let progress = self.key.section(&format!("cursor/{}", frame.branch));
        if previous.is_some_and(|hour| hour >= frame.hour) {
            return Err(invalid("snow export hour did not advance"));
        }
        let endpoint = frame.hour.rem_euclid(STRIDE) == 0
            || frame.hour == branch_last
            || previous.and_then(|hour| hour.checked_add(1)) != Some(frame.hour);
        if endpoint {
            let value = match &self.inputs {
                Ok(inputs) => {
                    match &*frame.tables[replay_data::rows::TableId::WeatherProvinces.index()] {
                        replay_frame::frame::TableView::Unavailable { reason } => {
                            Err(Status::Unavailable(reason.clone()))
                        }
                        replay_frame::frame::TableView::NotRecorded { mode } => {
                            Err(Status::Unavailable(format!(
                                "WeatherProvinces not recorded in {mode:?}"
                            )))
                        }
                        _ => inputs
                            .build(frame)
                            .map_err(|error| Status::Unavailable(error.to_string())),
                    }
                }
                Err(status) => Err(status.clone()),
            };
            write_endpoint(
                writer,
                &self.key,
                frame.branch,
                frame.hour,
                value.as_ref().map_err(Clone::clone),
                self.inputs.as_ref().ok().map(Inputs::limits),
            )?;
        }
        writer.write_at(&progress, 0, frame.hour.to_le_bytes().as_slice())?;
        Ok(())
    }
}

pub fn write_endpoint(
    writer: &mut Writer,
    key: &Key,
    branch: BranchId,
    hour: Hour,
    value: Result<&SnowMud, Status>,
    limits: Option<Limits>,
) -> io::Result<()> {
    let _cpu = replay_frame::cpu::scope("export-io", Some("snow.store"));
    let (status, body) = match value {
        Ok(snow) => {
            let limits = limits
                .ok_or_else(|| invalid("available snow endpoint has no geometry admission"))?;
            let bytes = codec::encode_product(snow, limits)?;
            let digest = codec::digest(&bytes);
            let payloads = key.section("payloads");
            let mut index = 0;
            let payload = loop {
                let Some(range) = writer.read_at(&payloads, index)? else {
                    let packet = codec::pack(key, branch, hour, 3, &bytes)?;
                    break writer.write(&payloads, packet.as_slice())?;
                };
                let candidate = (|| {
                    let header = codec::header(&range)?;
                    if header.digest != digest {
                        return Ok(false);
                    }
                    Ok(codec::unpack(range, key, None, 3, limits.maximum()?)? == bytes)
                })();
                match candidate {
                    Ok(true) => break index,
                    Ok(false) => {}
                    Err(error) if cache_validation(&error) => {}
                    Err(error) => return Err(error),
                }
                index += 1;
            };
            let mut body = payload.to_le_bytes().to_vec();
            body.extend_from_slice(&digest);
            (0, body)
        }
        Err(Status::Unavailable(reason)) => codec::status_body(1, &reason, limits)?,
        Err(Status::Unsupported(name)) => codec::status_body(2, &name, limits)?,
        Err(Status::Available) => return Err(invalid("available snow status needs a product")),
    };
    let packet = codec::pack(key, branch, hour, status, &body)?;
    let section = key.records(branch);
    writer.write(&section, packet.as_slice())?;
    Ok(())
}

#[cfg(test)]
mod tests;
