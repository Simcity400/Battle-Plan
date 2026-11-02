use super::{
    gate::{self, Consumer, Event},
    invalid,
};
use core_spool::{
    Evidence, EvidenceRef, Kind, Native, PayloadBlock, Record, RecordView, Type, Value, Writer,
};
use sha2::{Digest, Sha256};
use std::io::{self, Write};

const CELL_NOT_RECORDED: u8 = 1;

#[derive(Default)]
pub struct Canonical {
    hash: Sha256,
    event: Option<Event>,
}
impl Canonical {
    pub fn finish(self) -> [u8; 32] {
        self.hash.finalize().into()
    }
    fn bytes(&mut self, b: &[u8]) {
        self.hash.update(b);
    }
    fn byte(&mut self, b: u8) {
        self.bytes(&[b]);
    }
    fn blob(&mut self, b: &[u8]) {
        self.bytes(&(b.len() as u64).to_le_bytes());
        self.bytes(b);
    }
    fn optional_u32(&mut self, v: Option<u32>) {
        self.byte(u8::from(v.is_some()));
        if let Some(v) = v {
            self.bytes(&v.to_le_bytes());
        }
    }
    fn optional_i32(&mut self, v: Option<i32>) {
        self.byte(u8::from(v.is_some()));
        if let Some(v) = v {
            self.bytes(&v.to_le_bytes());
        }
    }
    fn cell_prefix(&mut self, field: u16, recorded: bool) {
        self.byte(1);
        self.bytes(&field.to_le_bytes());
        self.byte(u8::from(recorded));
    }
    pub fn cell_borrow(&mut self, field: u16, recorded: bool, value: &gate::Value<'_>) {
        self.event_bytes(self.event.unwrap());
        self.cell_prefix(field, recorded);
        if !recorded {
            return;
        }
        macro_rules! scalar {
            ($tag:expr,$v:expr) => {{
                self.byte($tag);
                self.bytes(&$v.to_le_bytes());
            }};
        }
        macro_rules! optional {
            ($tag:expr,$v:expr) => {{
                self.byte($tag);
                self.byte(u8::from($v.is_some()));
                if let Some(v) = $v {
                    self.bytes(&v.to_le_bytes());
                }
            }};
        }
        match value {
            gate::Value::U8(v) => scalar!(0, v),
            gate::Value::I8(v) => scalar!(1, v),
            gate::Value::U16(v) => scalar!(2, v),
            gate::Value::I16(v) => scalar!(3, v),
            gate::Value::U32(v) => scalar!(4, v),
            gate::Value::I32(v) => scalar!(5, v),
            gate::Value::U64(v) => scalar!(6, v),
            gate::Value::I64(v) => scalar!(7, v),
            gate::Value::Bool(v) => {
                self.byte(8);
                self.byte(u8::from(*v));
            }
            gate::Value::OptionalI32(v) => optional!(9, v),
            gate::Value::OptionalU32(v) => optional!(10, v),
            gate::Value::OptionalI64(v) => optional!(11, v),
            gate::Value::OptionalU64(v) => optional!(12, v),
            gate::Value::Text(id, text) => {
                self.byte(13);
                self.bytes(&id.to_le_bytes());
                self.byte(u8::from(text.is_some()));
                if let Some(b) = text {
                    self.blob(b);
                }
            }
            gate::Value::U32List(v) => {
                self.byte(14);
                self.bytes(&(v.len() as u64).to_le_bytes());
                for n in v {
                    self.bytes(&n.to_le_bytes());
                }
            }
            gate::Value::U64List(v) => {
                self.byte(15);
                self.bytes(&(v.len() as u64).to_le_bytes());
                for n in v {
                    self.bytes(&n.to_le_bytes());
                }
            }
            gate::Value::Opaque(v) => {
                self.byte(16);
                self.blob(v);
            }
        }
    }
    pub fn spool_record(
        &mut self,
        hour: i64,
        table: u32,
        field: u32,
        record: RecordView<'_>,
        payload: &mut PayloadBlock<'_>,
        source_kind: Option<u8>,
    ) -> io::Result<()> {
        match record.kind {
            Kind::RowStart | Kind::Gone | Kind::Read => {
                let key = key(record.key, payload)?;
                self.event(Event {
                    hour: i32::try_from(hour).map_err(|_| invalid("Gate hour overflow"))?,
                    kind: if record.kind == Kind::Gone {
                        2
                    } else if record.kind == Kind::Read {
                        4
                    } else {
                        record.value[0]
                    },
                    table: u16::try_from(table).map_err(|_| invalid("Gate table overflow"))?,
                    key,
                });
            }
            Kind::Unknown => {
                self.event_bytes(Event {
                    hour: hour as i32,
                    kind: match source_kind {
                        Some(kind @ (1 | 3)) => kind,
                        _ => return Err(invalid("unknown cell lacks bench source kind")),
                    },
                    table: table as u16,
                    key: key(record.key, payload)?,
                });
                self.cell_prefix(
                    u16::try_from(field).map_err(|_| invalid("Gate field overflow"))?,
                    false,
                );
            }
            Kind::Appeared if record.value_type == Type::Unit => self.event(Event {
                hour: hour as i32,
                kind: 1,
                table: table as u16,
                key: key(record.key, payload)?,
            }),
            Kind::Appeared | Kind::Changed => {
                self.event_bytes(Event {
                    hour: hour as i32,
                    kind: if record.kind == Kind::Appeared { 1 } else { 3 },
                    table: table as u16,
                    key: key(record.key, payload)?,
                });
                self.cell_prefix(
                    u16::try_from(field).map_err(|_| invalid("Gate field overflow"))?,
                    true,
                );
                match record.value_type {
                    Type::U8
                    | Type::I8
                    | Type::U16
                    | Type::I16
                    | Type::U32
                    | Type::I32
                    | Type::U64
                    | Type::I64
                    | Type::Bool => {
                        self.byte(record.value_type as u8 - 1);
                        self.bytes(record.value);
                    }
                    Type::OptionalI32
                    | Type::OptionalU32
                    | Type::OptionalI64
                    | Type::OptionalU64 => {
                        self.byte(record.value_type as u8 - 1);
                        self.bytes(if record.value[0] == 0 {
                            &record.value[..1]
                        } else {
                            record.value
                        });
                    }
                    Type::Text => {
                        self.byte(13);
                        self.bytes(&record.value[..5]);
                        let b = record.blob().unwrap();
                        self.bytes(&b.len.to_le_bytes());
                        payload.blob(b, |b| {
                            self.bytes(b);
                            Ok(())
                        })?;
                    }
                    Type::U32List | Type::U64List | Type::Bytes => {
                        let b = record.blob().unwrap();
                        let (tag, width) = match record.value_type {
                            Type::U32List => (14, 4),
                            Type::U64List => (15, 8),
                            _ => (16, 1),
                        };
                        self.byte(tag);
                        self.bytes(&(b.len / width).to_le_bytes());
                        payload.blob(b, |b| {
                            self.bytes(b);
                            Ok(())
                        })?;
                    }
                    _ => return Err(invalid("unexpected Gate cell lane")),
                }
            }
            Kind::Status | Kind::NotRecorded | Kind::Refused => {
                let evidence = EvidenceRef::decode(record.value);
                self.event = Some(Event {
                    hour: hour as i32,
                    kind: 0,
                    table: table as u16,
                    key: key(record.key, payload)?,
                });
                self.event_bytes(self.event.unwrap());
                self.byte(2);
                self.byte(evidence.code);
                self.optional_u32(
                    evidence
                        .missing
                        .map(u32::try_from)
                        .transpose()
                        .map_err(|_| invalid("Gate missing count overflow"))?,
                );
                let has_reason = evidence.code == 3;
                self.byte(u8::from(has_reason));
                if has_reason {
                    let mut len = 0;
                    payload.reason(evidence.reason, |b| {
                        len += b.len() as u64;
                        Ok(())
                    })?;
                    self.bytes(&len.to_le_bytes());
                    payload.reason(evidence.reason, |b| {
                        self.bytes(b);
                        Ok(())
                    })?;
                }
                self.optional_i32(
                    evidence
                        .sample
                        .map(i32::try_from)
                        .transpose()
                        .map_err(|_| invalid("Gate sample hour overflow"))?,
                );
            }
            Kind::VisibleKey => self.visible_key(key(record.key, payload)?),
            Kind::SegmentStart | Kind::Discontinuity => {
                self.event = Some(Event {
                    hour: hour as i32,
                    kind: if record.kind == Kind::SegmentStart {
                        5
                    } else {
                        6
                    },
                    table: table as u16,
                    key: key(record.key, payload)?,
                });
                self.flags(record.value[0]);
            }
            _ => return Err(invalid("unexpected Gate spool event")),
        }
        Ok(())
    }
}
fn key(id: core_spool::KeyId<'_>, payload: &mut PayloadBlock<'_>) -> io::Result<u128> {
    let mut bytes = [0; 16];
    let mut at = 0;
    let len = payload.key(id, |b| {
        if b.len() > bytes.len() - at {
            return Err(invalid("Gate key exceeds u128"));
        }
        bytes[at..at + b.len()].copy_from_slice(b);
        at += b.len();
        Ok(())
    })?;
    if len != 16 {
        return Err(invalid("Gate key width changed"));
    }
    Ok(u128::from_le_bytes(bytes))
}
impl Canonical {
    fn event_bytes(&mut self, event: Event) {
        self.byte(0);
        self.bytes(&event.hour.to_le_bytes());
        self.byte(event.kind);
        self.bytes(&event.table.to_le_bytes());
        self.bytes(&event.key.to_le_bytes());
    }
}
impl Consumer for Canonical {
    fn event(&mut self, event: Event) {
        self.event = Some(event);
        if matches!(event.kind, 1 | 2 | 4) {
            self.event_bytes(event);
        }
    }
    fn cell(&mut self, field: u16, recorded: bool, value: gate::Value<'_>) {
        self.cell_borrow(field, recorded, &value);
    }
    fn status(
        &mut self,
        status: u8,
        missing: Option<u32>,
        reason: Option<&[u8]>,
        sample: Option<i32>,
    ) {
        self.event_bytes(self.event.unwrap());
        self.byte(2);
        self.byte(status);
        self.optional_u32(missing);
        self.byte(u8::from(reason.is_some()));
        if let Some(b) = reason {
            self.blob(b);
        }
        self.optional_i32(sample);
    }
    fn visible_key(&mut self, key: u128) {
        self.event_bytes(self.event.unwrap());
        self.byte(3);
        self.bytes(&key.to_le_bytes());
    }
    fn flags(&mut self, flags: u8) {
        self.event_bytes(self.event.unwrap());
        self.byte(4);
        self.byte(flags);
    }
}

pub struct Convert<'a> {
    pub writer: &'a mut Writer,
    pub hash: Canonical,
    pub error: Option<io::Error>,
    event: Event,
    started: bool,
    pub counts: [u64; 17],
    pub publish_each_hour: bool,
    pub on_publish: Option<&'a mut dyn FnMut() -> io::Result<()>>,
}
impl<'a> Convert<'a> {
    pub fn new(writer: &'a mut Writer) -> Self {
        Self {
            writer,
            hash: Canonical::default(),
            error: None,
            event: Event {
                hour: 0,
                kind: 0,
                table: 0,
                key: 0,
            },
            started: false,
            counts: [0; 17],
            publish_each_hour: false,
            on_publish: None,
        }
    }
    fn push(&mut self, kind: Kind, field: u16, key: u128, value: Value<'_>) {
        if self.error.is_some() {
            return;
        }
        self.counts[kind.code() as usize] += 1;
        if let Err(error) = self.writer.append(Record {
            hour: i64::from(self.event.hour),
            kind,
            table: u32::from(self.event.table),
            field: u32::from(field),
            key: &key.to_le_bytes(),
            value,
        }) {
            self.error = Some(error);
        }
    }
}
impl Consumer for Convert<'_> {
    fn event(&mut self, event: Event) {
        if self.publish_each_hour
            && self.started
            && event.hour != self.event.hour
            && self.error.is_none()
        {
            if let Err(error) = self.writer.publish_hour().and_then(|_| {
                if let Some(callback) = self.on_publish.as_mut() {
                    callback()
                } else {
                    Ok(())
                }
            }) {
                self.error = Some(error);
            }
        }
        self.started = true;
        self.event = event;
        self.hash.event(event);
        match event.kind {
            1 => self.push(Kind::Appeared, 0, event.key, Value::Unit),
            2 => self.push(Kind::Gone, 0, event.key, Value::Unit),
            4 => self.push(Kind::Read, 0, event.key, Value::Unit),
            _ => {}
        }
    }
    fn cell(&mut self, field: u16, recorded: bool, value: gate::Value<'_>) {
        self.hash.cell_borrow(field, recorded, &value);
        if !recorded {
            self.push(
                Kind::Unknown,
                field,
                self.event.key,
                Value::Evidence(Evidence {
                    code: CELL_NOT_RECORDED,
                    reason: b"Gate 1 cell not recorded",
                    first: i64::from(self.event.hour),
                    last: i64::from(self.event.hour),
                    sample: None,
                    missing: None,
                }),
            );
            return;
        }
        let value = match &value {
            gate::Value::U8(v) => Value::U8(*v),
            gate::Value::I8(v) => Value::I8(*v),
            gate::Value::U16(v) => Value::U16(*v),
            gate::Value::I16(v) => Value::I16(*v),
            gate::Value::U32(v) => Value::U32(*v),
            gate::Value::I32(v) => Value::I32(*v),
            gate::Value::U64(v) => Value::U64(*v),
            gate::Value::I64(v) => Value::I64(*v),
            gate::Value::Bool(v) => Value::Bool(*v),
            gate::Value::OptionalI32(v) => Value::OptionalI32(*v),
            gate::Value::OptionalU32(v) => Value::OptionalU32(*v),
            gate::Value::OptionalI64(v) => Value::OptionalI64(*v),
            gate::Value::OptionalU64(v) => Value::OptionalU64(*v),
            gate::Value::Text(id, text) => Value::Text {
                id: *id,
                text: *text,
            },
            gate::Value::U32List(v) => Value::U32List(v),
            gate::Value::U64List(v) => Value::U64List(v),
            gate::Value::Opaque(v) => Value::Bytes(v),
        };
        self.push(
            if self.event.kind == 1 {
                Kind::Appeared
            } else {
                Kind::Changed
            },
            field,
            self.event.key,
            value,
        );
    }
    fn status(
        &mut self,
        status: u8,
        missing: Option<u32>,
        reason: Option<&[u8]>,
        sample: Option<i32>,
    ) {
        self.hash.status(status, missing, reason, sample);
        self.push(
            if status == 0 {
                Kind::NotRecorded
            } else if status == 3 {
                Kind::Refused
            } else {
                Kind::Status
            },
            0,
            self.event.key,
            Value::Evidence(Evidence {
                code: status,
                reason: reason.unwrap_or_default(),
                first: i64::from(self.event.hour),
                last: i64::from(self.event.hour),
                sample: sample.map(i64::from),
                missing: missing.map(u64::from),
            }),
        );
    }
    fn visible_key(&mut self, key: u128) {
        self.hash.visible_key(key);
        self.push(Kind::VisibleKey, 0, key, Value::Unit);
    }
    fn flags(&mut self, flags: u8) {
        self.hash.flags(flags);
        self.push(
            if self.event.kind == 5 {
                Kind::SegmentStart
            } else {
                Kind::Discontinuity
            },
            0,
            self.event.key,
            Value::U8(flags),
        );
    }
}

pub struct SourceKinds<'a> {
    pub sink: &'a mut dyn Write,
    pub error: Option<io::Error>,
    kind: u8,
}
impl<'a> SourceKinds<'a> {
    pub fn new(sink: &'a mut dyn Write) -> Self {
        Self {
            sink,
            error: None,
            kind: 0,
        }
    }
}
impl Consumer for SourceKinds<'_> {
    fn status(&mut self, _: u8, _: Option<u32>, _: Option<&[u8]>, _: Option<i32>) {}
    fn visible_key(&mut self, _: u128) {}
    fn flags(&mut self, _: u8) {}
    fn event(&mut self, event: Event) {
        self.kind = event.kind;
    }
    fn cell(&mut self, _: u16, recorded: bool, _: gate::Value<'_>) {
        if !recorded && self.error.is_none() {
            if let Err(error) = self.sink.write_all(&[self.kind]) {
                self.error = Some(error);
            }
        }
    }
}
