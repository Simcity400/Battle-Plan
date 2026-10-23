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
mod index;
mod reader;
mod records;
pub mod witness;
pub use background::Background;
pub use binding::Key;
pub use codec::Limits;
pub use reader::{Headroom, LayerProduct, Reader, Restore};
pub use records::{
    CONTRACT, Cursor, FORMAT, LayerStatus, Manifest, Payload, Producer, SNOW, SNOW_CONTRACT,
    TABLE_SECTION, Variant, payload_section, schema_digest,
};
pub use witness::Witness;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Coverage {
    pub endpoints: u64,
    pub first: Hour,
    pub last: Hour,
}

pub const STRIDE: Hour = 40;
pub const PRODUCER_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Available,
    Unavailable(String),
    Unsupported(String),
}

pub struct Inputs {
    pub cells: SnowCells,
    pub winter: Winter,
    pub witness: Witness,
}
impl Inputs {
    pub fn load(layers: &ResolvedLayers) -> io::Result<Self> {
        let (inputs, witness) = Witness::capture(layers, Self::load_raw)?;
        let mut inputs = inputs?;
        inputs.witness = witness;
        Ok(inputs)
    }
    fn load_raw(layers: &ResolvedLayers) -> io::Result<Self> {
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
        Ok(Self {
            cells,
            winter,
            witness: Witness::default(),
        })
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
    content_witness: Witness,
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
    let section = "map-export-v2/binding";
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
        let (inputs, content_witness) = if let Some(layers) = layers {
            let (inputs, witness) = Witness::capture(layers, |layers| {
                if layers.presentation().shows_visual_mods() {
                    return Err(invalid("snow export: visual-mod post-processing"));
                }
                Inputs::load_raw(layers)
            })?;
            let inputs = inputs.map_err(|error| {
                if layers.presentation().shows_visual_mods() {
                    Status::Unsupported(error.to_string())
                } else {
                    Status::Unavailable(error.to_string())
                }
            });
            (inputs, witness)
        } else {
            (
                Err(Status::Unavailable(
                    "snow: game content not configured".into(),
                )),
                Witness::default(),
            )
        };
        check()?;
        let mut session =
            Self::from_inputs(replay, inputs, prefix_length, store, source, &mut check)?;
        session.key.content = content_witness.digest;
        session.key.content_entries = content_witness.entries;
        session.content_witness = content_witness;
        Ok(session)
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
        let content_witness = inputs
            .as_ref()
            .map(|inputs| inputs.witness.clone())
            .unwrap_or_default();
        let mut key = Key::for_inputs(
            inputs.as_ref().map(|i| (&i.cells, &i.winter)),
            PRODUCER_VERSION,
        )?;
        key.content = content_witness.digest;
        key.content_entries = content_witness.entries;
        key.recording = recording;
        key.anchor = old
            .as_ref()
            .filter(|old| Some(old.prefix_digest) == previous)
            .map_or(recording, |old| old.key.anchor);
        check()?;
        Ok(Self {
            key,
            inputs,
            prefix_length,
            prefix_digest,
            previous: old.zip(previous),
            content_witness,
        })
    }
    pub fn ready(store: &Store, source: &SourceKey, key: &Key) -> io::Result<bool> {
        Ok(saved_binding(store, source)?.is_some_and(|binding| binding.key == *key))
    }
    pub fn reusable(
        &self,
        store: &Store,
        current: &SourceKey,
        branches: impl IntoIterator<Item = (BranchId, Option<Cursor>)>,
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
        branches: impl IntoIterator<Item = (BranchId, Option<Cursor>)>,
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
        branches: impl IntoIterator<Item = (BranchId, Option<Cursor>)>,
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
        reuse_result((|| {
            witness::verify(
                &self.key,
                |ordinal| read(&self.key.section("witness"), ordinal),
                &mut check,
            )?;
            for bucket in 0..=u8::MAX {
                let section = self.key.section(&format!("index/{bucket:02x}"));
                let mut ordinal = 0;
                while let Some(range) = read(&section, ordinal)? {
                    check()?;
                    let entry = index::Entry::read(range)?;
                    if entry.digest[0] != bucket {
                        return Err(invalid("map export index is in the wrong bucket"));
                    }
                    ordinal += 1;
                }
            }
            for (branch, accepted) in branches {
                let cursor = read(&self.key.section(&format!("cursor/{branch}")), 0)?
                    .map(|range| Cursor::read(range, &self.key, branch))
                    .transpose()?;
                if cursor != accepted {
                    return Err(invalid(
                        "map export cursor differs from accepted walk frontier",
                    ));
                }
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
                        .ok_or_else(|| invalid("map export inherited index is dangling"))?;
                    let header = codec::header(&range)?;
                    if header.branch != branch
                        || header.kind != 0
                        || header.layer != 0
                        || previous_hour.is_some_and(|hour| hour >= header.hour)
                    {
                        return Err(invalid(
                            "map export inherited index has wrong provenance or order",
                        ));
                    }
                    let body = codec::unpack(
                        range,
                        &self.key,
                        Some((branch, header.hour)),
                        0,
                        header.raw,
                    )?;
                    let manifest: Manifest = product_codec::decode(&body)?;
                    records::validate_manifest(&self.key, &manifest)?;
                    for (&layer, status) in &manifest.layers {
                        check()?;
                        match status {
                            LayerStatus::Available { payload } => {
                                let range =
                                    read(&payload_section(payload.section)?, payload.ordinal)?
                                        .ok_or_else(|| {
                                            invalid("map export inherited payload is dangling")
                                        })?;
                                let maximum = if layer == SNOW {
                                    self.inputs
                                        .as_ref()
                                        .map_err(|_| {
                                            invalid("snow payload without admitted inputs")
                                        })?
                                        .limits()
                                        .maximum()?
                                } else {
                                    usize::MAX
                                };
                                let bytes = read_payload(
                                    range, &self.key, layer, false, payload, maximum, &mut 0,
                                )?;
                                if layer == SNOW {
                                    let inputs = self.inputs.as_ref().map_err(|_| {
                                        invalid("snow payload without admitted inputs")
                                    })?;
                                    codec::decode_product(&bytes, inputs.limits())?;
                                }
                            }
                            LayerStatus::Unavailable { .. }
                            | LayerStatus::Unsupported { .. }
                            | LayerStatus::PartError { .. } => {}
                        }
                    }
                    for (&table, payload) in &manifest.tables {
                        check()?;
                        let range = read(&payload_section(payload.section)?, payload.ordinal)?
                            .ok_or_else(|| {
                                invalid("map export inherited table payload is dangling")
                            })?;
                        read_payload(range, &self.key, table, true, payload, usize::MAX, &mut 0)?;
                    }
                    previous_hour = Some(header.hour);
                    check()?;
                }
                if count != 0 && cursor.is_none() {
                    return Err(invalid("map export endpoint cursor is missing"));
                }
                if let Some(cursor) = cursor {
                    if previous_hour.is_some_and(|last| last > cursor.hour) {
                        return Err(invalid("map export cursor precedes its endpoints"));
                    }
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
        for &layer in self.key.producers.keys() {
            check()?;
            if count(&payload_section(u32::from(layer))?)?.unwrap_or(0) != 0 {
                return Ok(true);
            }
        }
        if count(&self.key.section("tables"))?.unwrap_or(0) != 0 {
            return Ok(true);
        }
        for bucket in 0..=u8::MAX {
            check()?;
            if count(&self.key.section(&format!("index/{bucket:02x}")))?.unwrap_or(0) != 0 {
                return Ok(true);
            }
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
        writer.ensure_section(&self.key.section("tables"))?;
        writer.ensure_section(&self.key.section("witness"))?;
        for &layer in self.key.producers.keys() {
            writer.ensure_section(&payload_section(u32::from(layer))?)?;
        }
        for bucket in 0..=u8::MAX {
            writer.ensure_section(&self.key.section(&format!("index/{bucket:02x}")))?;
        }
        let bytes = binding::Binding {
            key: self.key.clone(),
            prefix_length: self.prefix_length,
            prefix_digest: self.prefix_digest,
        }
        .encode();
        writer.write_at(&self.key.section("binding"), 0, bytes.as_slice())?;
        if self.key.content_entries > 0
            && writer.read_at(&self.key.section("witness"), 0)?.is_none()
        {
            self.content_witness.publish(writer, &self.key)?;
        }
        Ok(())
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
            .map(|range| Cursor::read(range, &self.key, frame.branch))
            .transpose()?
            .map(|cursor| cursor.hour);
        self.observe_after(writer, frame, branch_last, previous)
    }
    pub fn observe_after(
        &self,
        writer: &mut Writer,
        frame: &DecodedFrame,
        branch_last: Hour,
        previous: Option<Hour>,
    ) -> io::Result<()> {
        self.observe_in_block_after(writer, frame, branch_last, previous, None)
    }
    pub fn observe_in_block_after(
        &self,
        writer: &mut Writer,
        frame: &DecodedFrame,
        branch_last: Hour,
        previous: Option<Hour>,
        block: Option<u64>,
    ) -> io::Result<()> {
        let progress = self.key.section(&format!("cursor/{}", frame.branch));
        if previous.is_some_and(|hour| hour >= frame.hour) {
            return Err(invalid("snow export hour did not advance"));
        }
        let endpoint = endpoint_kind(frame.hour, branch_last, previous);
        if endpoint != 0 {
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
            write_snow_endpoint(
                writer,
                &self.key,
                frame.branch,
                frame.hour,
                value.as_ref().map_err(Clone::clone),
                self.inputs.as_ref().ok().map(Inputs::limits),
                endpoint,
            )?;
        }
        let cursor = Cursor {
            hour: frame.hour,
            block,
        }
        .encode(&self.key, frame.branch)?;
        writer.write_at(&progress, 0, cursor.as_slice())?;
        Ok(())
    }
}

pub fn endpoint_kind(hour: Hour, last: Hour, previous: Option<Hour>) -> u8 {
    u8::from(hour.rem_euclid(STRIDE) == 0)
        | (u8::from(hour == last) << 1)
        | (u8::from(previous.is_none()) << 2)
        | (u8::from(previous.is_some() && previous.and_then(|h| h.checked_add(1)) != Some(hour))
            << 3)
}

pub fn write_endpoint(
    writer: &mut Writer,
    key: &Key,
    branch: BranchId,
    hour: Hour,
    value: Result<&SnowMud, Status>,
    limits: Option<Limits>,
) -> io::Result<()> {
    write_snow_endpoint(writer, key, branch, hour, value, limits, 1)
}
fn write_snow_endpoint(
    writer: &mut Writer,
    key: &Key,
    branch: BranchId,
    hour: Hour,
    value: Result<&SnowMud, Status>,
    limits: Option<Limits>,
    endpoint: u8,
) -> io::Result<()> {
    let _cpu = replay_frame::cpu::scope("export-io", Some("snow.store"));
    let status = match value {
        Ok(snow) => {
            let limits = limits
                .ok_or_else(|| invalid("available snow endpoint has no geometry admission"))?;
            let bytes = codec::encode_product(snow, limits)?;
            LayerStatus::Available {
                payload: intern_layer(writer, key, SNOW, branch, hour, &bytes)?,
            }
        }
        Err(status @ (Status::Unavailable(_) | Status::Unsupported(_))) => {
            let unsupported = matches!(&status, Status::Unsupported(_));
            let reason = match status {
                Status::Unavailable(reason) | Status::Unsupported(reason) => reason,
                _ => unreachable!(),
            };
            let (kind, body) =
                codec::status_body(if unsupported { 2 } else { 1 }, &reason, limits)?;
            let reason = codec::decode_reason(&body)?;
            if kind == 1 {
                LayerStatus::Unavailable { reason }
            } else {
                LayerStatus::Unsupported { reason }
            }
        }
        Err(Status::Available) => return Err(invalid("available snow status needs a product")),
    };
    write_manifest(
        writer,
        key,
        branch,
        hour,
        &Manifest {
            endpoint,
            layers: [(SNOW, status)].into_iter().collect(),
            tables: Default::default(),
        },
    )
}

pub fn write_manifest(
    writer: &mut Writer,
    key: &Key,
    branch: BranchId,
    hour: Hour,
    manifest: &Manifest,
) -> io::Result<()> {
    records::validate_manifest(key, manifest)?;
    for payload in manifest
        .layers
        .values()
        .filter_map(|status| {
            if let LayerStatus::Available { payload } = status {
                Some(payload)
            } else {
                None
            }
        })
        .chain(manifest.tables.values())
    {
        let range = writer
            .read_at(&payload_section(payload.section)?, payload.ordinal)?
            .ok_or_else(|| invalid("map export manifest payload has not been written"))?;
        let header = codec::header(&range)?;
        let owner = (payload.section & 0xffff) as u16;
        let table = payload.section & TABLE_SECTION != 0;
        if header.digest != payload.digest
            || header.raw as u64 != payload.raw
            || (header.branch, header.hour) != (payload.branch, payload.hour)
            || header.layer != owner
            || header.kind != if table { 4 } else { 3 }
            || header.key
                != if table {
                    key.table_digest()
                } else {
                    key.layer_digest(owner)?
                }
        {
            return Err(invalid("map export manifest payload provenance differs"));
        }
    }
    if let Some(range) = writer.read_at(&key.section(&format!("cursor/{branch}")), 0)? {
        let cursor = Cursor::read(range, key, branch)?;
        if cursor.hour >= hour {
            return Err(invalid("map export endpoint did not advance"));
        }
    }
    let body = product_codec::encode(manifest)?;
    let packet = codec::pack_for(key.digest(), branch, hour, 0, 0, &body)?;
    writer.write(&key.records(branch), packet.as_slice())?;
    let cursor = Cursor { hour, block: None }.encode(key, branch)?;
    writer.write_at(
        &key.section(&format!("cursor/{branch}")),
        0,
        cursor.as_slice(),
    )?;
    Ok(())
}

pub fn intern_layer(
    writer: &mut Writer,
    key: &Key,
    layer: u16,
    branch: BranchId,
    hour: Hour,
    bytes: &[u8],
) -> io::Result<Payload> {
    if layer == 0 {
        return Err(invalid("map export layer zero is reserved"));
    }
    index::intern(writer, key, u32::from(layer), branch, hour, bytes)
}
pub fn intern_table(
    writer: &mut Writer,
    key: &Key,
    table: u16,
    branch: BranchId,
    hour: Hour,
    bytes: &[u8],
) -> io::Result<Payload> {
    index::intern(
        writer,
        key,
        TABLE_SECTION | u32::from(table),
        branch,
        hour,
        bytes,
    )
}

fn read_payload(
    range: store::Range,
    key: &Key,
    layer: u16,
    table: bool,
    payload: &Payload,
    admitted: usize,
    peak: &mut usize,
) -> io::Result<Vec<u8>> {
    read_payload_admitted(
        range,
        key,
        layer,
        table,
        payload,
        admitted,
        usize::MAX,
        0,
        peak,
    )
}
fn read_payload_admitted(
    range: store::Range,
    key: &Key,
    layer: u16,
    table: bool,
    payload: &Payload,
    admitted: usize,
    headroom: usize,
    resident: usize,
    peak: &mut usize,
) -> io::Result<Vec<u8>> {
    let raw = usize::try_from(payload.raw)
        .map_err(|_| invalid("map export payload exceeds address space"))?;
    if raw > admitted {
        return Err(invalid(
            "map export raw length not admitted by live headroom",
        ));
    }
    let header = codec::header(&range)?;
    if header.raw != raw || header.digest != payload.digest {
        return Err(invalid(
            "map export manifest payload length or digest differs",
        ));
    }
    if !table {
        key.layer_digest(layer)?;
    }
    let owner = (payload.section & 0xffff) as u16;
    let stored_table = payload.section & TABLE_SECTION != 0;
    let bytes = codec::unpack_for_admitted(
        range,
        if stored_table {
            key.table_digest()
        } else {
            key.layer_digest(owner)?
        },
        Some((payload.branch, payload.hour)),
        if stored_table { 4 } else { 3 },
        owner,
        admitted,
        headroom,
        resident,
        peak,
    )?;
    Ok(bytes)
}

#[cfg(test)]
mod tests;
