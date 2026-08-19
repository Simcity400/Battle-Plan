use std::collections::BTreeSet;

// @leaf-source-authority crate=d11_grammar root=extractor/native/d11_grammar files=8 sha256=cd0e1eed4d8767af23c3d745c2ff369f1fa78e514c9a77ab4f6ef9acdfc54631
pub const DEPLOYMENT_QUEUE_SCHEMA_VERSION: u32 = d11_grammar::deployment::SchemaVersion::V2.wire();
pub const DEPLOYMENT_QUEUE_HEADER_LEN: usize = d11_grammar::deployment::HEADER_LEN;
pub const DEPLOYMENT_CONVEYOR_ID_TYPE: u32 = d11_grammar::deployment::CONVEYOR_ID_TYPE;
pub const DEPLOYMENT_LINE_ID_TYPE: u32 = d11_grammar::deployment::LINE_ID_TYPE;
pub const DEPLOYMENT_TEMPLATE_ID_TYPE: u32 = d11_grammar::deployment::TEMPLATE_ID_TYPE;
pub const DEPLOYMENT_EQUIPMENT_ID_TYPE: u32 = d11_grammar::deployment::EQUIPMENT_ID_TYPE;

const COUNTRY_FLAG_QUEUE_COMPLETE: u32 = d11_grammar::deployment::CountryFlag::QueueComplete.bit();
const CONVEYOR_FLAG_CLOSED: u32 = d11_grammar::deployment::ConveyorFlag::Closed.bit();
const LINE_FLAG_TRAINING: u32 = d11_grammar::deployment::LineFlag::Training.bit();
const LINE_FLAG_MAX_TRAINING: u32 = d11_grammar::deployment::LineFlag::MaxTraining.bit();
const LINE_FLAG_MANPOWER_HAVE: u32 = d11_grammar::deployment::LineFlag::ManpowerHave.bit();
const LINE_FLAG_MANPOWER_NEED: u32 = d11_grammar::deployment::LineFlag::ManpowerNeed.bit();
const LINE_FLAG_MAX_MANPOWER: u32 = d11_grammar::deployment::LineFlag::MaxManpower.bit();
const LINE_FLAG_NAME_ORDER: u32 = d11_grammar::deployment::LineFlag::NameOrder.bit();
const LINE_KNOWN_FLAGS: u32 = d11_grammar::deployment::LINE_KNOWN_FLAGS;
const EQUIPMENT_FLAG_ARCHETYPE: u32 = d11_grammar::deployment::EquipmentFlag::Archetype.bit();
const MAX_STRING_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct DeploymentEquipmentRow {
    pub ordinal: u32,
    pub equipment_id: u32,
    pub archetype: Option<String>,
    pub amount: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeploymentLineRow {
    pub ordinal: u32,
    pub line_id: u32,
    pub name_order: Option<i32>,
    pub amount: i32,
    pub training: Option<f64>,
    pub max_training: Option<f64>,
    pub manpower_have: Option<i64>,
    pub manpower_need: Option<i64>,
    pub max_manpower: Option<i64>,
    pub equipment_in_transit: Vec<DeploymentEquipmentRow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeploymentConveyorRow {
    pub ordinal: u32,
    pub conveyor_id: u32,
    pub name: String,
    pub template_id: u32,
    pub amount: i32,
    pub location: u32,
    pub priority: u32,
    pub role: String,
    pub closed: bool,
    pub lines: Vec<DeploymentLineRow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeploymentCountryRow {
    pub tag_db_id: u32,
    pub queue_complete: bool,
    pub conveyors: Vec<DeploymentConveyorRow>,
}

impl d11_grammar::deployment::EquipmentEncoding for DeploymentEquipmentRow {
    fn ordinal(&self) -> u32 {
        self.ordinal
    }
    fn equipment_id(&self) -> u32 {
        self.equipment_id
    }
    fn id_type(&self) -> u32 {
        DEPLOYMENT_EQUIPMENT_ID_TYPE
    }
    fn flags(&self) -> u32 {
        u32::from(self.archetype.is_some()) * EQUIPMENT_FLAG_ARCHETYPE
    }
    fn amount_bits(&self) -> u64 {
        self.amount.to_bits()
    }
    fn archetype(&self) -> Option<&[u8]> {
        self.archetype.as_deref().map(str::as_bytes)
    }
}

impl d11_grammar::deployment::LineEncoding for DeploymentLineRow {
    type Equipment = DeploymentEquipmentRow;

    fn ordinal(&self) -> u32 {
        self.ordinal
    }
    fn line_id(&self) -> u32 {
        self.line_id
    }
    fn id_type(&self) -> u32 {
        DEPLOYMENT_LINE_ID_TYPE
    }
    fn name_order(&self) -> i32 {
        self.name_order.unwrap_or(0)
    }
    fn amount(&self) -> i32 {
        self.amount
    }
    fn flags(&self) -> u32 {
        u32::from(self.training.is_some()) * LINE_FLAG_TRAINING
            | u32::from(self.max_training.is_some()) * LINE_FLAG_MAX_TRAINING
            | u32::from(self.manpower_have.is_some()) * LINE_FLAG_MANPOWER_HAVE
            | u32::from(self.manpower_need.is_some()) * LINE_FLAG_MANPOWER_NEED
            | u32::from(self.max_manpower.is_some()) * LINE_FLAG_MAX_MANPOWER
            | u32::from(self.name_order.is_some()) * LINE_FLAG_NAME_ORDER
    }
    fn training_bits(&self) -> u64 {
        self.training.unwrap_or(0.0).to_bits()
    }
    fn max_training_bits(&self) -> u64 {
        self.max_training.unwrap_or(0.0).to_bits()
    }
    fn manpower_have(&self) -> i64 {
        self.manpower_have.unwrap_or(0)
    }
    fn manpower_need(&self) -> i64 {
        self.manpower_need.unwrap_or(0)
    }
    fn max_manpower(&self) -> i64 {
        self.max_manpower.unwrap_or(0)
    }
    fn reserved(&self) -> u32 {
        0
    }
    fn equipment(&self) -> &[Self::Equipment] {
        &self.equipment_in_transit
    }
}

impl d11_grammar::deployment::ConveyorEncoding for DeploymentConveyorRow {
    type Line = DeploymentLineRow;

    fn ordinal(&self) -> u32 {
        self.ordinal
    }
    fn conveyor_id(&self) -> u32 {
        self.conveyor_id
    }
    fn id_type(&self) -> u32 {
        DEPLOYMENT_CONVEYOR_ID_TYPE
    }
    fn template_id(&self) -> u32 {
        self.template_id
    }
    fn template_id_type(&self) -> u32 {
        DEPLOYMENT_TEMPLATE_ID_TYPE
    }
    fn amount(&self) -> i32 {
        self.amount
    }
    fn location(&self) -> u32 {
        self.location
    }
    fn priority(&self) -> u32 {
        self.priority
    }
    fn flags(&self) -> u32 {
        u32::from(self.closed) * CONVEYOR_FLAG_CLOSED
    }
    fn name(&self) -> &[u8] {
        self.name.as_bytes()
    }
    fn role(&self) -> &[u8] {
        self.role.as_bytes()
    }
    fn lines(&self) -> &[Self::Line] {
        &self.lines
    }
}

impl d11_grammar::deployment::CountryEncoding for DeploymentCountryRow {
    type Conveyor = DeploymentConveyorRow;

    fn tag_db_id(&self) -> u32 {
        self.tag_db_id
    }
    fn flags(&self) -> u32 {
        u32::from(self.queue_complete) * COUNTRY_FLAG_QUEUE_COMPLETE
    }
    fn reserved(&self) -> u32 {
        0
    }
    fn conveyors(&self) -> &[Self::Conveyor] {
        &self.conveyors
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeploymentQueueSummary {
    pub country_rows: u32,
    pub complete_country_rows: u32,
    pub unknown_country_rows: u32,
    pub conveyor_rows: u32,
    pub line_rows: u32,
    pub equipment_rows: u32,
    pub changed_country_rows: u32,
    pub status_flags: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeploymentQueueFrame {
    pub elapsed_wall_ms: u64,
    pub changed_country_rows: u32,
    pub status_flags: u32,
    pub countries: Vec<DeploymentCountryRow>,
}

pub fn encode_deployment_queue_payload(
    elapsed_wall_ms: u64,
    changed_country_rows: u32,
    status_flags: u32,
    countries: &[DeploymentCountryRow],
) -> Result<Vec<u8>, String> {
    let country_count = u32::try_from(countries.len())
        .map_err(|_| "deployment country count exceeds u32".to_string())?;
    if changed_country_rows > country_count {
        return Err(format!(
            "deployment changed_country_rows {changed_country_rows} > country_count {country_count}"
        ));
    }
    let _conveyor_count = countries.iter().try_fold(0u32, |total, country| {
        total
            .checked_add(
                u32::try_from(country.conveyors.len())
                    .map_err(|_| "deployment country conveyor count exceeds u32".to_string())?,
            )
            .ok_or_else(|| "deployment conveyor count overflow".to_string())
    })?;
    let _line_count = countries.iter().try_fold(0u32, |total, country| {
        country.conveyors.iter().try_fold(total, |total, conveyor| {
            total
                .checked_add(
                    u32::try_from(conveyor.lines.len())
                        .map_err(|_| "deployment conveyor line count exceeds u32".to_string())?,
                )
                .ok_or_else(|| "deployment line count overflow".to_string())
        })
    })?;
    let _equipment_count = countries.iter().try_fold(0u32, |total, country| {
        country.conveyors.iter().try_fold(total, |total, conveyor| {
            conveyor.lines.iter().try_fold(total, |total, line| {
                total
                    .checked_add(
                        u32::try_from(line.equipment_in_transit.len())
                            .map_err(|_| "deployment equipment count exceeds u32".to_string())?,
                    )
                    .ok_or_else(|| "deployment equipment count overflow".to_string())
            })
        })
    })?;

    for country in countries {
        if country.tag_db_id == 0 {
            return Err("deployment country has zero tag_db_id".into());
        }
        if !country.queue_complete && !country.conveyors.is_empty() {
            return Err(format!(
                "deployment country {} is unknown but carries conveyor rows",
                country.tag_db_id
            ));
        }
        for conveyor in &country.conveyors {
            validate_encoded_string(&conveyor.name)?;
            validate_encoded_string(&conveyor.role)?;
            for line in &conveyor.lines {
                for equipment in &line.equipment_in_transit {
                    if let Some(archetype) = equipment.archetype.as_deref() {
                        validate_encoded_string(archetype)?;
                    }
                }
            }
        }
    }

    d11_grammar::deployment::encode_v2(
        d11_grammar::deployment::HeaderWords {
            elapsed_wall_ms,
            changed_country_rows,
            status_flags,
        },
        countries,
    )
    .map_err(format_deployment_encode_error)
}

fn format_deployment_encode_error(error: d11_grammar::deployment::DqEncodeError) -> String {
    match error {
        d11_grammar::deployment::DqEncodeError::CountOverflow { .. } => {
            "deployment count exceeds u32".to_string()
        }
        d11_grammar::deployment::DqEncodeError::LengthOverflow { .. }
        | d11_grammar::deployment::DqEncodeError::LengthDoesNotFitU32 { .. } => {
            "deployment payload length overflow".to_string()
        }
        d11_grammar::deployment::DqEncodeError::Allocation { .. } => {
            "deployment payload allocation failed".to_string()
        }
        d11_grammar::deployment::DqEncodeError::PreflightMismatch {
            declared,
            written,
        } => format!(
            "deployment encoder preflight declared {declared} byte(s) and wrote {written}; the row source changed between the two passes"
        ),
    }
}

fn validate_encoded_string(value: &str) -> Result<(), String> {
    if value.len() > MAX_STRING_BYTES {
        return Err(format!(
            "deployment string length {} exceeds {MAX_STRING_BYTES}",
            value.len()
        ));
    }
    Ok(())
}

pub fn decode_deployment_queue_payload(payload: &[u8]) -> Result<DeploymentQueueSummary, String> {
    let frame = decode_deployment_queue_frame(payload)?;
    let mut summary = DeploymentQueueSummary {
        country_rows: frame.countries.len() as u32,
        changed_country_rows: frame.changed_country_rows,
        status_flags: frame.status_flags,
        ..DeploymentQueueSummary::default()
    };
    for country in &frame.countries {
        if country.queue_complete {
            summary.complete_country_rows += 1;
        } else {
            summary.unknown_country_rows += 1;
        }
        summary.conveyor_rows += country.conveyors.len() as u32;
        for conveyor in &country.conveyors {
            summary.line_rows += conveyor.lines.len() as u32;
            for line in &conveyor.lines {
                summary.equipment_rows += line.equipment_in_transit.len() as u32;
            }
        }
    }
    Ok(summary)
}

pub fn decode_deployment_queue_frame(payload: &[u8]) -> Result<DeploymentQueueFrame, String> {
    let country_cursor =
        d11_grammar::deployment::walk(payload).map_err(format_deployment_grammar_violation)?;
    decode_deployment_queue_frame_from_cursor(payload, country_cursor)
}

pub fn decode_deployment_queue_frame_from_cursor(
    payload: &[u8],
    country_cursor: d11_grammar::deployment::CountryCursor,
) -> Result<DeploymentQueueFrame, String> {
    let preview = d11_grammar::deployment::read_header_preview(payload)
        .map_err(format_deployment_grammar_violation)?;
    let header = *country_cursor.header();
    if preview.elapsed_wall_ms != header.elapsed_wall_ms
        || preview.country_count != header.country_count
        || preview.changed_country_rows != header.changed_country_rows
        || preview.status_flags != header.status_flags
        || preview.schema_version != header.schema_version.wire()
        || preview.declared_conveyors != header.declared_conveyors
        || preview.declared_lines != header.declared_lines
        || preview.declared_equipment != header.declared_equipment
        || preview.reserved != header.reserved
    {
        return Err(format!(
            "deployment cursor was opened over a different carrier: cursor header says schema v{} elapsed {} countries {}, these bytes say v{} elapsed {} countries {}",
            header.schema_version.wire(),
            header.elapsed_wall_ms,
            header.country_count,
            preview.schema_version,
            preview.elapsed_wall_ms,
            preview.country_count,
        ));
    }
    let elapsed_wall_ms = header.elapsed_wall_ms;
    let country_count = header.country_count;
    let changed_country_rows = header.changed_country_rows;
    let status_flags = header.status_flags;
    let declared_conveyors = header.declared_conveyors;
    let declared_lines = header.declared_lines;
    let declared_equipment = header.declared_equipment;
    // old-format support before release, so a v1 carrier is REFUSED here
    if preview.schema_version == d11_grammar::deployment::SchemaVersion::V1.wire() {
        return Err(format!(
            "deployment schema v{} is refused: the recorder writes v{}, and no pre-release support exists for the older layout",
            d11_grammar::deployment::SchemaVersion::V1.wire(),
            d11_grammar::deployment::SchemaVersion::V2.wire(),
        ));
    }
    if changed_country_rows > country_count {
        return Err("deployment changed country count exceeds country count".into());
    }
    let mut country_cursor = country_cursor
        .precheck_declared_counts(payload)
        .map_err(format_deployment_grammar_violation)?;

    let mut tags = BTreeSet::new();
    let mut conveyor_ids = BTreeSet::new();
    let mut line_ids = BTreeSet::new();
    let mut countries = Vec::with_capacity(country_count as usize);
    let mut observed_conveyors = 0u32;
    let mut observed_lines = 0u32;
    let mut observed_equipment = 0u32;
    loop {
        let (country_index, country_row, conveyors_block) = match country_cursor
            .next(payload)
            .map_err(format_deployment_grammar_violation)?
        {
            d11_grammar::deployment::CountryStep::Country {
                index,
                row,
                conveyors,
            } => (index, row, conveyors),
            d11_grammar::deployment::CountryStep::End(end) => {
                observed_conveyors = end.totals.conveyors;
                observed_lines = end.totals.lines;
                observed_equipment = end.totals.equipment;
                break;
            }
        };
        let tag_db_id = country_row.tag_db_id;
        let flags = country_row.flags;
        let conveyor_count = country_row.conveyor_count;
        let country_reserved = country_row.reserved;
        if tag_db_id == 0 || !tags.insert(tag_db_id) {
            return Err(format!(
                "deployment country {country_index} invalid/duplicate tag {tag_db_id}"
            ));
        }
        if flags & !COUNTRY_FLAG_QUEUE_COMPLETE != 0 || country_reserved != 0 {
            return Err(format!(
                "deployment country {tag_db_id} invalid flags/reserved"
            ));
        }
        if flags & COUNTRY_FLAG_QUEUE_COMPLETE == 0 {
            if conveyor_count != 0 {
                return Err(format!(
                    "deployment unknown country {tag_db_id} carries conveyors"
                ));
            }
        }
        let queue_complete = flags & COUNTRY_FLAG_QUEUE_COMPLETE != 0;
        if conveyor_count > declared_conveyors.saturating_sub(observed_conveyors) {
            return Err(format!(
                "deployment country {tag_db_id} conveyor count exceeds declared remainder"
            ));
        }
        let mut conveyor_cursor = conveyors_block
            .open(payload)
            .map_err(format_deployment_grammar_violation)?;
        let mut conveyors = Vec::with_capacity(conveyor_count as usize);
        while conveyor_cursor.remaining() != 0 {
            let fixed = conveyor_cursor
                .next(payload)
                .map_err(format_deployment_grammar_violation)?;
            let conveyor_ordinal = fixed.index as u32;
            let row = fixed.row;
            let ordinal = row.ordinal;
            let conveyor_id = row.conveyor_id;
            let id_type = row.id_type;
            let template_id = row.template_id;
            let template_type = row.template_id_type;
            let amount = row.amount;
            let location = row.location;
            let priority = row.priority;
            let conveyor_flags = row.flags;
            let line_count = row.line_count;
            if ordinal != conveyor_ordinal
                || conveyor_id == 0
                || id_type != DEPLOYMENT_CONVEYOR_ID_TYPE
                || !conveyor_ids.insert(conveyor_id)
                || template_id == 0
                || template_type != DEPLOYMENT_TEMPLATE_ID_TYPE
                || amount < 0
                || conveyor_flags & !CONVEYOR_FLAG_CLOSED != 0
            {
                return Err(format!(
                    "deployment invalid conveyor identity/body {conveyor_id}"
                ));
            }
            let name_item = fixed
                .name
                .read_name(payload)
                .map_err(format_deployment_grammar_violation)?;
            let name = deployment_string(payload, name_item.name, "deployment conveyor name")?;
            let role_item = name_item
                .role
                .read_role(payload)
                .map_err(format_deployment_grammar_violation)?;
            let role = deployment_string(payload, role_item.row.role, "deployment conveyor role")?;
            observed_conveyors = observed_conveyors
                .checked_add(1)
                .ok_or_else(|| "deployment conveyor count overflow".to_string())?;
            if line_count > declared_lines.saturating_sub(observed_lines) {
                return Err(format!(
                    "deployment conveyor {conveyor_id} line count exceeds declared remainder"
                ));
            }
            let mut line_cursor = role_item
                .lines
                .open(payload)
                .map_err(format_deployment_grammar_violation)?;
            let mut lines = Vec::with_capacity(line_count as usize);
            while line_cursor.remaining() != 0 {
                let fixed = line_cursor
                    .next(payload)
                    .map_err(format_deployment_grammar_violation)?;
                let line_ordinal = fixed.index as u32;
                let row = fixed.row;
                let ordinal = row.ordinal;
                let line_id = row.line_id;
                let id_type = row.id_type;
                let name_order = row.name_order;
                let amount = row.amount;
                let line_flags = row.flags;
                let training = f64::from_bits(row.training_bits);
                let max_training = f64::from_bits(row.max_training_bits);
                let manpower_have = row.manpower_have;
                let manpower_need = row.manpower_need;
                let max_manpower = row.max_manpower;
                let equipment_count = row.equipment_count;
                let line_reserved = row.reserved;
                if ordinal != line_ordinal
                    || line_id == 0
                    || id_type != DEPLOYMENT_LINE_ID_TYPE
                    || !line_ids.insert(line_id)
                    || amount < 0
                    || line_flags & !LINE_KNOWN_FLAGS != 0
                    || line_reserved != 0
                    || (line_flags & LINE_FLAG_NAME_ORDER == 0 && name_order != 0)
                {
                    return Err(format!("deployment invalid line identity/body {line_id}"));
                }
                validate_optional_float(line_flags, LINE_FLAG_TRAINING, training, "training")?;
                validate_optional_float(
                    line_flags,
                    LINE_FLAG_MAX_TRAINING,
                    max_training,
                    "max_training",
                )?;
                validate_optional_nonnegative_i64(
                    line_flags,
                    LINE_FLAG_MANPOWER_HAVE,
                    manpower_have,
                    "manpower_have",
                )?;
                validate_optional_nonnegative_i64(
                    line_flags,
                    LINE_FLAG_MANPOWER_NEED,
                    manpower_need,
                    "manpower_need",
                )?;
                validate_optional_nonnegative_i64(
                    line_flags,
                    LINE_FLAG_MAX_MANPOWER,
                    max_manpower,
                    "max_manpower",
                )?;
                observed_lines = observed_lines
                    .checked_add(1)
                    .ok_or_else(|| "deployment line count overflow".to_string())?;
                if equipment_count > declared_equipment.saturating_sub(observed_equipment) {
                    return Err(format!(
                        "deployment line {line_id} equipment count exceeds declared remainder"
                    ));
                }
                let mut equipment_cursor = fixed
                    .equipment
                    .open(payload)
                    .map_err(format_deployment_grammar_violation)?;
                let mut equipment_in_transit = Vec::with_capacity(equipment_count as usize);
                while equipment_cursor.remaining() != 0 {
                    let item = equipment_cursor
                        .next(payload)
                        .map_err(format_deployment_grammar_violation)?;
                    let equipment_ordinal = item.index as u32;
                    let row = item.row;
                    let ordinal = row.ordinal;
                    let equipment_id = row.equipment_id;
                    let equipment_type = row.id_type;
                    let equipment_flags = row.flags;
                    let amount = f64::from_bits(row.amount_bits);
                    let archetype = row
                        .archetype
                        .map(|span| deployment_string(payload, span, "deployment archetype"))
                        .transpose()?;
                    equipment_cursor = item.next;
                    if ordinal != equipment_ordinal
                        || equipment_id == 0
                        || equipment_type != DEPLOYMENT_EQUIPMENT_ID_TYPE
                        || equipment_flags & !EQUIPMENT_FLAG_ARCHETYPE != 0
                        || !amount.is_finite()
                        || amount < 0.0
                    {
                        return Err(format!(
                            "deployment invalid equipment row {equipment_id}:{equipment_type}"
                        ));
                    }
                    observed_equipment = observed_equipment
                        .checked_add(1)
                        .ok_or_else(|| "deployment equipment count overflow".to_string())?;
                    equipment_in_transit.push(DeploymentEquipmentRow {
                        ordinal,
                        equipment_id,
                        archetype,
                        amount,
                    });
                }
                line_cursor = equipment_cursor
                    .into_end()
                    .map_err(format_deployment_grammar_violation)?;
                lines.push(DeploymentLineRow {
                    ordinal,
                    line_id,
                    name_order: if line_flags & LINE_FLAG_NAME_ORDER != 0 {
                        Some(name_order)
                    } else {
                        None
                    },
                    amount,
                    training: (line_flags & LINE_FLAG_TRAINING != 0).then_some(training),
                    max_training: (line_flags & LINE_FLAG_MAX_TRAINING != 0)
                        .then_some(max_training),
                    manpower_have: (line_flags & LINE_FLAG_MANPOWER_HAVE != 0)
                        .then_some(manpower_have),
                    manpower_need: (line_flags & LINE_FLAG_MANPOWER_NEED != 0)
                        .then_some(manpower_need),
                    max_manpower: (line_flags & LINE_FLAG_MAX_MANPOWER != 0)
                        .then_some(max_manpower),
                    equipment_in_transit,
                });
            }
            conveyor_cursor = line_cursor
                .into_end()
                .map_err(format_deployment_grammar_violation)?;
            conveyors.push(DeploymentConveyorRow {
                ordinal,
                conveyor_id,
                name,
                template_id,
                amount,
                location,
                priority,
                role,
                closed: conveyor_flags & CONVEYOR_FLAG_CLOSED != 0,
                lines,
            });
        }
        countries.push(DeploymentCountryRow {
            tag_db_id,
            queue_complete,
            conveyors,
        });
        country_cursor = conveyor_cursor
            .into_end()
            .map_err(format_deployment_grammar_violation)?;
    }
    let observed_countries = u32::try_from(countries.len())
        .map_err(|_| "deployment decoded country count exceeds u32".to_string())?;
    if (
        observed_countries,
        observed_conveyors,
        observed_lines,
        observed_equipment,
    ) != (
        country_count,
        declared_conveyors,
        declared_lines,
        declared_equipment,
    ) {
        return Err(format!(
            "deployment declared/observed count mismatch: declared {country_count}/{declared_conveyors}/{declared_lines}/{declared_equipment}, observed {observed_countries}/{observed_conveyors}/{observed_lines}/{observed_equipment}"
        ));
    }
    Ok(DeploymentQueueFrame {
        elapsed_wall_ms,
        changed_country_rows,
        status_flags,
        countries,
    })
}

fn deployment_string(
    payload: &[u8],
    span: d11_grammar::Span,
    label: &str,
) -> Result<String, String> {
    if span.len > MAX_STRING_BYTES {
        return Err(format!(
            "{label} length {} exceeds {MAX_STRING_BYTES}",
            span.len
        ));
    }
    Ok(std::str::from_utf8(&payload[span.start..span.end])
        .map_err(|_| format!("{label} is not UTF-8"))?
        .to_string())
}
pub fn format_deployment_grammar_violation(
    violation: d11_grammar::deployment::Violation,
) -> String {
    use d11_grammar::deployment::{
        Block, DqCallerDefect as CallerDefect, DqSection as Section,
        DqViolationKind as ViolationKind,
    };
    match violation.kind {
        ViolationKind::Need {
            section,
            needed,
            available,
        } => {
            let label = match section {
                Section::Header => "deployment header",
                Section::Country => "deployment country row",
                Section::Conveyor => "deployment conveyor row",
                Section::ConveyorName => "deployment conveyor name",
                Section::ConveyorRole => "deployment conveyor role",
                Section::Line => "deployment line row",
                Section::Equipment => "deployment equipment row",
                Section::Archetype => "deployment archetype",
            };
            if matches!(
                section,
                Section::ConveyorName | Section::ConveyorRole | Section::Archetype
            ) {
                let declared_len = needed
                    .saturating_sub(violation.at)
                    .saturating_sub(d11_grammar::deployment::STRING_PREFIX_LEN);
                if declared_len > MAX_STRING_BYTES {
                    return format!("{label} length {declared_len} exceeds {MAX_STRING_BYTES}");
                }
            }
            format!("{label} truncated: need {needed}, have {available}")
        }
        ViolationKind::UnknownSchema { version, reserved }
        | ViolationKind::HeaderReserved {
            version,
            value: reserved,
        } => {
            format!("deployment invalid schema/reserved {version}/{reserved}")
        }
        ViolationKind::CountCannotFit {
            block,
            count,
            remaining,
            minimum_row_len,
        } => {
            let label = match block {
                Block::Countries => "country",
                Block::Conveyors if violation.at == d11_grammar::deployment::HEADER_LEN => {
                    "declared conveyor"
                }
                Block::Lines if violation.at == d11_grammar::deployment::HEADER_LEN => {
                    "declared line"
                }
                Block::Equipment if violation.at == d11_grammar::deployment::HEADER_LEN => {
                    "declared equipment"
                }
                Block::Conveyors => "conveyor",
                Block::Lines => "line",
                Block::Equipment => "equipment",
            };
            format!(
                "deployment {label} count {count} exceeds remaining payload bound {}",
                remaining / minimum_row_len
            )
        }
        ViolationKind::TrailingBytes {
            consumed,
            available,
        } => {
            format!("deployment trailing bytes: consumed {consumed} of {available}")
        }
        ViolationKind::ArithmeticOverflow {
            operation,
            left,
            right,
        } => {
            format!("deployment count overflow ({operation:?}, {left}, {right})")
        }
        ViolationKind::CallerDefect(CallerDefect::BlockNotConsumed { block, remaining }) => {
            format!("deployment caller did not consume {remaining} {block:?} row(s)")
        }
    }
}
fn validate_optional_float(flags: u32, flag: u32, value: f64, label: &str) -> Result<(), String> {
    if flags & flag != 0 {
        if !value.is_finite() || value < 0.0 {
            return Err(format!("deployment {label} is invalid"));
        }
    } else if value.to_bits() != 0 {
        return Err(format!("deployment {label} payload present without flag"));
    }
    Ok(())
}

fn validate_optional_nonnegative_i64(
    flags: u32,
    flag: u32,
    value: i64,
    label: &str,
) -> Result<(), String> {
    if flags & flag != 0 {
        if value < 0 {
            return Err(format!("deployment {label} is negative"));
        }
    } else if value != 0 {
        return Err(format!("deployment {label} payload present without flag"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exact_country() -> DeploymentCountryRow {
        DeploymentCountryRow {
            tag_db_id: 7,
            queue_complete: true,
            conveyors: vec![DeploymentConveyorRow {
                ordinal: 0,
                conveyor_id: 254,
                name: "Infantry 1939 A".into(),
                template_id: 1546,
                amount: 1,
                location: 1537,
                priority: 1,
                role: "infantry".into(),
                closed: false,
                lines: vec![DeploymentLineRow {
                    ordinal: 0,
                    line_id: 6483,
                    name_order: Some(21),
                    amount: 1,
                    training: Some(0.09841),
                    max_training: Some(1.0),
                    manpower_have: Some(12_160),
                    manpower_need: Some(12_160),
                    max_manpower: Some(12_160),
                    equipment_in_transit: vec![DeploymentEquipmentRow {
                        ordinal: 0,
                        equipment_id: 4730,
                        archetype: Some("anti_tank_equipment_1".into()),
                        amount: 16.0,
                    }],
                }],
            }],
        }
    }

    #[test]
    fn exact_nested_carrier_round_trips_and_counts() {
        let country = exact_country();
        let payload = encode_deployment_queue_payload(9, 1, 0, &[country.clone()]).unwrap();
        let summary = decode_deployment_queue_payload(&payload).unwrap();
        assert_eq!(summary.country_rows, 1);
        assert_eq!(summary.complete_country_rows, 1);
        assert_eq!(summary.conveyor_rows, 1);
        assert_eq!(summary.line_rows, 1);
        assert_eq!(summary.equipment_rows, 1);
        let frame = decode_deployment_queue_frame(&payload).unwrap();
        assert_eq!(frame.elapsed_wall_ms, 9);
        assert_eq!(frame.countries, vec![country]);
    }

    /// recorder writes, and it writes schema v2. There is no pre-release support
    #[test]
    fn gate_refuses_a_deployment_carrier_written_in_schema_v1() {
        let words = d11_grammar::deployment::HeaderWords {
            elapsed_wall_ms: 9,
            changed_country_rows: 0,
            status_flags: 0,
        };
        let payload = d11_grammar::deployment::encode_v1(words, &[]).unwrap();
        assert_eq!(
            u32::from_le_bytes([payload[20], payload[21], payload[22], payload[23]]),
            d11_grammar::deployment::SchemaVersion::V1.wire(),
            "the fixture must really be a v1 frame or this test proves nothing"
        );

        let error = decode_deployment_queue_payload(&payload)
            .expect_err("a schema v1 deployment carrier must be refused");
        assert!(
            error.contains("schema v1 is refused"),
            "the refusal must name the version it refused, got: {error}"
        );

        let v2 = encode_deployment_queue_payload(9, 0, 0, &[]).unwrap();
        decode_deployment_queue_payload(&v2).expect("schema v2 is what ships");
    }

    #[test]
    fn gate_refuses_v1_bytes_carried_in_on_a_v2_cursor() {
        let words = d11_grammar::deployment::HeaderWords {
            elapsed_wall_ms: 9,
            changed_country_rows: 0,
            status_flags: 0,
        };
        let v1 = d11_grammar::deployment::encode_v1(words, &[]).unwrap();
        let v2 = encode_deployment_queue_payload(9, 0, 0, &[]).unwrap();
        assert_eq!(
            v1.len(),
            v2.len(),
            "the substitution needs two frames of one size"
        );

        let cursor = d11_grammar::deployment::walk(&v2).expect("v2 walks");
        let error = decode_deployment_queue_frame_from_cursor(&v1, cursor)
            .expect_err("v1 bytes must be refused however the cursor was opened");
        assert!(
            error.contains("schema v1 is refused") || error.contains("different carrier"),
            "expected a version or carrier-mismatch refusal, got: {error}"
        );
    }

    #[test]
    fn gate_refuses_a_same_version_cursor_opened_over_a_different_frame() {
        let first = encode_deployment_queue_payload(1, 0, 0, &[]).unwrap();
        let second = encode_deployment_queue_payload(2, 0, 0, &[]).unwrap();
        assert_eq!(first.len(), second.len());
        assert_ne!(first, second, "the two frames must actually differ");

        let cursor = d11_grammar::deployment::walk(&first).expect("v2 walks");
        let error = decode_deployment_queue_frame_from_cursor(&second, cursor)
            .expect_err("a cursor from another frame must be refused");
        assert!(
            error.contains("different carrier"),
            "expected a carrier-mismatch refusal, got: {error}"
        );
    }

    #[test]
    fn gate_refuses_a_cursor_already_advanced_past_the_first_country() {
        let country = DeploymentCountryRow {
            tag_db_id: 7,
            queue_complete: true,
            conveyors: Vec::new(),
        };
        let payload = encode_deployment_queue_payload(9, 1, 0, &[country]).unwrap();
        assert_eq!(
            decode_deployment_queue_payload(&payload)
                .unwrap()
                .country_rows,
            1
        );

        let cursor = d11_grammar::deployment::walk(&payload).expect("v2 walks");
        let advanced = match cursor.next(&payload).expect("first country") {
            d11_grammar::deployment::CountryStep::Country { conveyors, .. } => conveyors
                .open(&payload)
                .expect("empty conveyor block opens")
                .into_end()
                .expect("empty conveyor block closes"),
            d11_grammar::deployment::CountryStep::End(_) => {
                panic!("the fixture must carry one country")
            }
        };

        let error = decode_deployment_queue_frame_from_cursor(&payload, advanced)
            .expect_err("an advanced cursor must not decode as an empty frame");
        assert!(
            error.contains("declared/observed count mismatch"),
            "expected a declared/observed refusal, got: {error}"
        );
    }

    #[test]
    fn proven_empty_and_unknown_are_distinct() {
        let countries = [
            DeploymentCountryRow {
                tag_db_id: 1,
                queue_complete: true,
                conveyors: vec![],
            },
            DeploymentCountryRow {
                tag_db_id: 2,
                queue_complete: false,
                conveyors: vec![],
            },
        ];
        let summary = decode_deployment_queue_payload(
            &encode_deployment_queue_payload(9, 2, 0, &countries).unwrap(),
        )
        .unwrap();
        assert_eq!(summary.complete_country_rows, 1);
        assert_eq!(summary.unknown_country_rows, 1);
    }

    #[test]
    fn serializer_backed_absence_round_trips_without_fabricated_defaults() {
        let mut country = exact_country();
        let line = &mut country.conveyors[0].lines[0];
        line.name_order = None;
        line.equipment_in_transit[0].archetype = None;
        let payload = encode_deployment_queue_payload(9, 1, 0, &[country.clone()]).unwrap();
        let summary = decode_deployment_queue_payload(&payload).unwrap();
        assert_eq!(summary.line_rows, 1);
        assert_eq!(summary.equipment_rows, 1);
        assert_eq!(
            decode_deployment_queue_frame(&payload).unwrap().countries,
            vec![country]
        );
    }

    #[test]
    fn unknown_country_cannot_smuggle_rows() {
        let mut country = exact_country();
        country.queue_complete = false;
        assert!(encode_deployment_queue_payload(9, 1, 0, &[country])
            .unwrap_err()
            .contains("unknown but carries"));
    }

    #[test]
    fn decoder_rejects_wrong_identity_type_and_duplicate_ids() {
        let mut payload = encode_deployment_queue_payload(9, 1, 0, &[exact_country()]).unwrap();
        payload[64..68].copy_from_slice(&63u32.to_le_bytes());
        assert!(decode_deployment_queue_payload(&payload)
            .unwrap_err()
            .contains("invalid conveyor"));

        let mut country = exact_country();
        let mut second = country.conveyors[0].clone();
        second.ordinal = 1;
        country.conveyors.push(second);
        let payload = encode_deployment_queue_payload(9, 1, 0, &[country]).unwrap();
        assert!(decode_deployment_queue_payload(&payload)
            .unwrap_err()
            .contains("invalid conveyor"));
    }

    #[test]
    fn decoder_rejects_trailing_bytes_and_count_drift() {
        let mut trailing = encode_deployment_queue_payload(9, 1, 0, &[exact_country()]).unwrap();
        trailing.push(0);
        assert!(decode_deployment_queue_payload(&trailing)
            .unwrap_err()
            .contains("trailing bytes"));

        let mut drift = encode_deployment_queue_payload(9, 1, 0, &[exact_country()]).unwrap();
        drift[32..36].copy_from_slice(&2u32.to_le_bytes());
        assert!(decode_deployment_queue_payload(&drift)
            .unwrap_err()
            .contains("count mismatch"));
    }

    #[test]
    fn decoder_rejects_header_counts_before_capacity_allocation() {
        let mut payload = vec![0u8; DEPLOYMENT_QUEUE_HEADER_LEN];
        payload[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        payload[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        payload[20..24].copy_from_slice(&DEPLOYMENT_QUEUE_SCHEMA_VERSION.to_le_bytes());
        let error = decode_deployment_queue_frame(&payload).unwrap_err();
        assert!(error.contains("country count"), "{error}");

        for (offset, label) in [
            (24, "declared conveyor"),
            (28, "declared line"),
            (32, "declared equipment"),
        ] {
            let mut payload = vec![0u8; DEPLOYMENT_QUEUE_HEADER_LEN];
            payload[20..24].copy_from_slice(&DEPLOYMENT_QUEUE_SCHEMA_VERSION.to_le_bytes());
            payload[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            let error = decode_deployment_queue_frame(&payload).unwrap_err();
            assert!(error.contains(label), "{label}: {error}");
        }
    }
}
