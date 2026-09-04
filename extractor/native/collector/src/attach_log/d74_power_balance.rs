use super::*;

const A729_SHA256: &str = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
const MANAGER_OFF: usize = 0x450;
const RECORD_STRIDE: usize = 0x190;
const OVERRIDE_STRIDE: usize = 0x50;
const DAILY_KEY: u32 = 0x98;
const WEEKLY_KEY: u32 = 0x99;
const RETRIES: usize = 3;

const CENSUS_COPY_BUDGET_US: u64 = 8_000;

const MAX_MODIFIER_ROWS: i32 = 65_536;

struct CopyBudget {
    started: Instant,
    budget: Duration,
    #[cfg(test)]
    checks: std::cell::Cell<u32>,
}

impl CopyBudget {
    fn new(budget_us: u64) -> Self {
        Self {
            started: Instant::now(),
            budget: Duration::from_micros(budget_us),
            #[cfg(test)]
            checks: std::cell::Cell::new(0),
        }
    }

    fn verdict(&self, elapsed: Duration) -> Result<(), &'static str> {
        if elapsed > self.budget {
            return Err("power_balance_census_copy_budget_exceeded");
        }
        Ok(())
    }

    fn check(&self) -> Result<(), &'static str> {
        #[cfg(test)]
        self.checks.set(self.checks.get().saturating_add(1));
        self.verdict(self.started.elapsed())
    }
}

#[derive(Clone, Copy)]
pub(super) struct AttachProfile {
    manager: usize,
    daily_flags: u32,
    weekly_flags: u32,
}

pub(super) fn resolve_attach_profile(base: usize, game_state: usize) -> Option<AttachProfile> {
    let bytes = std::fs::read(process_path()?).ok()?;
    if format!("{:x}", Sha256::digest(bytes)) != A729_SHA256
        || read_u64_stable(base.checked_add(GAME_STATE_SINGLETON_RVA)?)? as usize != game_state
    {
        return None;
    }
    let manager = read_u64_stable(game_state.checked_add(MANAGER_OFF)?)? as usize;
    let definitions = read_u64_stable(base.checked_add(MODIFIER_DEFINITION_TABLE_RVA)?)? as usize;
    if !is_map_control_heap_pointer(manager) || !is_map_control_heap_pointer(definitions) {
        return None;
    }
    let definition = |key: u32| -> Option<(u32, u32)> {
        let row = definitions.checked_add((key as usize).checked_mul(0x78)?)?;
        Some((
            read_u32_stable(row.checked_add(0x60)?)?,
            read_u32_stable(row.checked_add(0x68)?)?,
        ))
    };
    let (daily_flags, daily_eligibility) = definition(DAILY_KEY)?;
    let (weekly_flags, weekly_eligibility) = definition(WEEKLY_KEY)?;
    if daily_eligibility != 0 || weekly_eligibility != 0 {
        return None;
    }
    Some(AttachProfile {
        manager,
        daily_flags,
        weekly_flags,
    })
}

#[derive(Clone, PartialEq, Eq)]
struct CopiedString {
    object: [u8; 32],
    heap: Vec<u8>,
}

#[derive(PartialEq, Eq)]
struct CopiedCountry {
    id: u32,
    vtable: u64,
    object_id: u32,
    modifiers: Vec<u8>,
}

#[derive(PartialEq, Eq)]
struct CopiedOverride {
    side: CopiedString,
    gfx: CopiedString,
}

struct CopiedRecord {
    fixed: Vec<u8>,
    balance: CopiedString,
    left: CopiedString,
    right: CopiedString,
    trending: CopiedString,
    range: CopiedString,
    members: Vec<u8>,
    _range_vector: Vec<u8>,
    overrides: Vec<CopiedOverride>,
}

impl CopiedRecord {
    fn equal_within_budget(&self, other: &Self, budget: &CopyBudget) -> Result<bool, &'static str> {
        let Self {
            fixed,
            balance,
            left,
            right,
            trending,
            range,
            members,
            _range_vector,
            overrides,
        } = self;
        budget.check()?;
        if fixed != &other.fixed
            || balance != &other.balance
            || left != &other.left
            || right != &other.right
            || trending != &other.trending
            || range != &other.range
            || members != &other.members
            || _range_vector != &other._range_vector
            || overrides.len() != other.overrides.len()
        {
            return Ok(false);
        }
        for (mine, theirs) in overrides.iter().zip(&other.overrides) {
            budget.check()?;
            if mine != theirs {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

pub(super) struct CopiedCensus {
    frame_hour: u64,
    elapsed_wall_ms: u64,
    manager_header: [u8; 16],
    countries: Vec<CopiedCountry>,
    records: Vec<CopiedRecord>,
    daily_flags: u32,
    weekly_flags: u32,
}

impl CopiedCensus {
    fn equal_within_budget(&self, other: &Self, budget: &CopyBudget) -> Result<bool, &'static str> {
        let Self {
            frame_hour,
            elapsed_wall_ms,
            manager_header,
            countries,
            records,
            daily_flags,
            weekly_flags,
        } = self;
        if *frame_hour != other.frame_hour
            || *elapsed_wall_ms != other.elapsed_wall_ms
            || *manager_header != other.manager_header
            || *daily_flags != other.daily_flags
            || *weekly_flags != other.weekly_flags
            || countries.len() != other.countries.len()
            || records.len() != other.records.len()
        {
            return Ok(false);
        }
        for (mine, theirs) in countries.iter().zip(&other.countries) {
            budget.check()?;
            if mine != theirs {
                return Ok(false);
            }
        }
        for (mine, theirs) in records.iter().zip(&other.records) {
            if !mine.equal_within_budget(theirs, budget)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn admit_country_count(count: usize) -> Result<(), &'static str> {
    if count > d74_grammar::MAX_COUNTRIES {
        return Err("country_count_exceeds_payload_limit");
    }
    Ok(())
}

fn copy_exact(address: usize, len: usize) -> Option<Vec<u8>> {
    if len == 0 {
        return Some(Vec::new());
    }
    let mut bytes = vec![0; len];
    read_into(address, &mut bytes).then_some(bytes)
}

fn copy_string(address: usize) -> Option<CopiedString> {
    if !is_map_control_heap_pointer(address) {
        return None;
    }
    let mut object = [0; 32];
    read_into(address, &mut object).then_some(())?;
    let size = usize::try_from(read_u64_from_slice(&object, 0x10)?).ok()?;
    let capacity = usize::try_from(read_u64_from_slice(&object, 0x18)?).ok()?;
    if size > d74_grammar::MAX_TOKEN_BYTES || capacity < size {
        return None;
    }
    let heap = if capacity > 15 {
        let pointer = usize::try_from(read_u64_from_slice(&object, 0)?).ok()?;
        if size > 0 && !is_map_control_heap_pointer(pointer) {
            return None;
        }
        copy_exact(pointer, size)?
    } else {
        Vec::new()
    };
    Some(CopiedString { object, heap })
}

fn decode_string(value: &CopiedString) -> Result<String, &'static str> {
    let size =
        usize::try_from(read_u64_from_slice(&value.object, 0x10).ok_or("string_header_truncated")?)
            .map_err(|_| "string_size_overflow")?;
    let capacity = read_u64_from_slice(&value.object, 0x18).ok_or("string_header_truncated")?;
    let bytes = if capacity > 15 {
        value.heap.as_slice()
    } else {
        value
            .object
            .get(..size)
            .ok_or("inline_string_size_invalid")?
    };
    if bytes.len() != size {
        return Err("string_copy_size_mismatch");
    }
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| "string_not_utf8")
}

fn copy_modifiers(container: usize) -> Option<Vec<u8>> {
    let mut header = [0; 16];
    read_into(container, &mut header).then_some(())?;
    let pointer = usize::try_from(read_u64_from_slice(&header, 0)?).ok()?;
    let count = read_i32_from_slice(&header, 0xC)?;
    if count < 0 || count > MAX_MODIFIER_ROWS {
        return None;
    }
    let len = (count as usize).checked_mul(0x10)?;
    if len > 0 && !is_map_control_heap_pointer(pointer) {
        return None;
    }
    copy_exact(pointer, len)
}

fn modifier_value(rows: &[u8], key: u32, flags: u32) -> Result<i64, &'static str> {
    if rows.len() % 0x10 != 0 {
        return Err("modifier_rows_truncated");
    }
    let mut prior = None;
    for row in rows.chunks_exact(0x10) {
        let row_key = read_u32_from_slice(row, 0).ok_or("modifier_row_truncated")?;
        if prior.is_some_and(|value| value >= row_key) {
            return Err("modifier_keys_not_strictly_sorted");
        }
        prior = Some(row_key);
    }
    let raw = compact_country_modifier_raw_in_sorted_rows(
        rows.len() / 0x10,
        key,
        |index| {
            let row = rows.get(index * 0x10..(index + 1) * 0x10)?;
            read_u32_from_slice(row, 0)
        },
        |index| {
            let row = rows.get(index * 0x10..(index + 1) * 0x10)?;
            read_i64_from_slice(row, 8)
        },
    )
    .ok_or("modifier_keys_not_strictly_sorted")?;
    Ok(match raw {
        CompactCountryModifierRaw::Missing => 0,
        CompactCountryModifierRaw::Found(raw) => compact_country_modifier_apply_flags(raw, flags),
    })
}

fn copy_once(
    state: &CollectorState,
    frame_hour: u64,
    elapsed_wall_ms: u64,
    budget: &CopyBudget,
) -> Result<CopiedCensus, &'static str> {
    budget.check()?;
    let profile = state
        .power_balance_attach_profile()
        .ok_or("a729_power_balance_attach_profile_unavailable")?;
    admit_country_count(state.country_identity.rows.len())?;
    let mut countries = Vec::with_capacity(state.country_identity.rows.len());
    for row in &state.country_identity.rows {
        budget.check()?;
        if !is_map_control_heap_pointer(row.object_ptr) {
            return Err("country_object_pointer_invalid");
        }
        let vtable = read_u64(row.object_ptr).ok_or("country_vtable_unreadable")?;
        let object_id = read_u32(row.object_ptr + OBJ_TAG_DB_ID).ok_or("country_id_unreadable")?;
        let container = row
            .object_ptr
            .checked_add(CCOUNTRY_MODIFIER_CONTAINER_OFF)
            .ok_or("country_modifier_container_overflow")?;
        let modifiers = copy_modifiers(container).ok_or("country_modifier_container_unreadable")?;
        countries.push(CopiedCountry {
            id: row.tag_db_id,
            vtable,
            object_id,
            modifiers,
        });
    }

    let mut manager_header = [0; 16];
    read_into(
        profile
            .manager
            .checked_add(0x08)
            .ok_or("manager_header_overflow")?,
        &mut manager_header,
    )
    .then_some(())
    .ok_or("manager_header_unreadable")?;
    let base =
        usize::try_from(read_u64_from_slice(&manager_header, 0).ok_or("manager_header_truncated")?)
            .map_err(|_| "record_base_overflow")?;
    let capacity = read_i32_from_slice(&manager_header, 8).ok_or("manager_header_truncated")?;
    let count = read_i32_from_slice(&manager_header, 12).ok_or("manager_header_truncated")?;
    if count < 0 || capacity < count || count as usize > d74_grammar::MAX_RECORDS {
        return Err("manager_count_or_capacity_invalid");
    }
    let fixed_len = (count as usize)
        .checked_mul(RECORD_STRIDE)
        .ok_or("manager_record_array_size_overflow")?;
    if fixed_len > 0 && !is_map_control_heap_pointer(base) {
        return Err("manager_record_array_pointer_invalid");
    }
    budget.check()?;
    let fixed_all = copy_exact(base, fixed_len).ok_or("manager_record_array_unreadable")?;
    let mut records = Vec::with_capacity(count as usize);
    for ordinal in 0..count as usize {
        budget.check()?;
        let fixed = fixed_all
            .get(ordinal * RECORD_STRIDE..(ordinal + 1) * RECORD_STRIDE)
            .ok_or("manager_record_array_truncated")?
            .to_vec();
        if fixed.get(0x08).copied() == Some(0) {
            return Err("manager_record_validity_sentinel_invalid");
        }
        let ptr = |offset| {
            read_u64_from_slice(&fixed, offset)
                .map(|value| value as usize)
                .filter(|value| is_map_control_heap_pointer(*value))
        };
        let definition = ptr(0x10).ok_or("record_balance_definition_pointer_invalid")?;
        let left = ptr(0x18).ok_or("record_left_definition_pointer_invalid")?;
        let right = ptr(0x20).ok_or("record_right_definition_pointer_invalid")?;
        let trending = ptr(0x28).ok_or("record_trending_definition_pointer_invalid")?;
        let range = ptr(0xB0).ok_or("record_active_range_pointer_invalid")?;

        let members_base =
            read_u64_from_slice(&fixed, 0x38).ok_or("member_header_missing")? as usize;
        let member_count = read_i32_from_slice(&fixed, 0x44).ok_or("member_header_missing")?;
        if member_count < 0 || member_count as usize > d74_grammar::MAX_MEMBERS_PER_RECORD {
            return Err("member_count_invalid");
        }
        let member_len = (member_count as usize)
            .checked_mul(4)
            .ok_or("member_size_overflow")?;
        if member_len > 0 && !is_map_control_heap_pointer(members_base) {
            return Err("member_array_pointer_invalid");
        }
        let members = copy_exact(members_base, member_len).ok_or("member_array_unreadable")?;

        let ranges_base = read_u64_from_slice(&fixed, 0x68).ok_or("range_header_missing")? as usize;
        let range_count = read_i32_from_slice(&fixed, 0x74).ok_or("range_header_missing")?;
        if range_count < 0 || range_count as usize > d74_grammar::MAX_RECORDS {
            return Err("range_vector_count_invalid");
        }
        let range_len = (range_count as usize)
            .checked_mul(8)
            .ok_or("range_size_overflow")?;
        if range_len > 0 && !is_map_control_heap_pointer(ranges_base) {
            return Err("range_vector_pointer_invalid");
        }
        let range_vector = copy_exact(ranges_base, range_len).ok_or("range_vector_unreadable")?;

        let overrides_base =
            read_u64_from_slice(&fixed, 0x178).ok_or("override_header_missing")? as usize;
        let override_capacity =
            read_i32_from_slice(&fixed, 0x180).ok_or("override_header_missing")?;
        let override_count = read_i32_from_slice(&fixed, 0x184).ok_or("override_header_missing")?;
        if override_count < 0
            || override_capacity < override_count
            || override_count as usize > d74_grammar::MAX_OVERRIDES_PER_RECORD
        {
            return Err("override_count_or_capacity_invalid");
        }
        let override_len = (override_count as usize)
            .checked_mul(OVERRIDE_STRIDE)
            .ok_or("override_size_overflow")?;
        if override_len > 0 && !is_map_control_heap_pointer(overrides_base) {
            return Err("override_vector_pointer_invalid");
        }
        let mut overrides = Vec::with_capacity(override_count as usize);
        for index in 0..override_count as usize {
            budget.check()?;
            let element = overrides_base
                .checked_add(
                    index
                        .checked_mul(OVERRIDE_STRIDE)
                        .ok_or("override_address_overflow")?,
                )
                .ok_or("override_address_overflow")?;
            overrides.push(CopiedOverride {
                side: copy_string(
                    element
                        .checked_add(0x08)
                        .ok_or("override_side_address_overflow")?,
                )
                .ok_or("override_side_string_unreadable")?,
                gfx: copy_string(
                    element
                        .checked_add(0x30)
                        .ok_or("override_gfx_address_overflow")?,
                )
                .ok_or("override_gfx_string_unreadable")?,
            });
        }
        records.push(CopiedRecord {
            fixed,
            balance: copy_string(
                definition
                    .checked_add(0x10)
                    .ok_or("balance_string_address_overflow")?,
            )
            .ok_or("balance_identity_unreadable")?,
            left: copy_string(
                left.checked_add(0x10)
                    .ok_or("left_string_address_overflow")?,
            )
            .ok_or("left_identity_unreadable")?,
            right: copy_string(
                right
                    .checked_add(0x10)
                    .ok_or("right_string_address_overflow")?,
            )
            .ok_or("right_identity_unreadable")?,
            trending: copy_string(
                trending
                    .checked_add(0x10)
                    .ok_or("trending_string_address_overflow")?,
            )
            .ok_or("trending_identity_unreadable")?,
            range: copy_string(
                range
                    .checked_add(0x718)
                    .ok_or("range_string_address_overflow")?,
            )
            .ok_or("active_range_identity_unreadable")?,
            members,
            _range_vector: range_vector,
            overrides,
        });
    }
    Ok(CopiedCensus {
        frame_hour,
        elapsed_wall_ms,
        manager_header,
        countries,
        records,
        daily_flags: profile.daily_flags,
        weekly_flags: profile.weekly_flags,
    })
}

fn census_within_budget(
    budget: &CopyBudget,
    copy: impl Fn(&CopyBudget) -> Result<CopiedCensus, &'static str>,
) -> Result<CopiedCensus, &'static str> {
    for _ in 0..RETRIES {
        let first = copy(budget)?;
        let second = copy(budget)?;
        if first.equal_within_budget(&second, budget)? {
            budget.check()?;
            return Ok(first);
        }
        budget.check()?;
    }
    Err("power_balance_data_changed_while_being_read")
}

pub(super) fn copy_census(
    state: &CollectorState,
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<CopiedCensus, &'static str> {
    census_within_budget(&CopyBudget::new(CENSUS_COPY_BUDGET_US), |budget| {
        copy_once(state, frame_hour, elapsed_wall_ms, budget)
    })
}

fn resolve_side_gfx_overrides(
    rows: impl IntoIterator<Item = (String, String)>,
) -> Vec<d74_grammar::SideGfxOverride> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for (side_id, gfx) in rows {
        if gfx.is_empty() || side_id.is_empty() {
            continue;
        }
        if !seen.insert(side_id.clone()) {
            continue;
        }
        out.push(d74_grammar::SideGfxOverride { side_id, gfx });
    }
    out
}

pub(super) fn build_payload(
    mut copied: CopiedCensus,
    state: &CollectorState,
) -> Result<d74_grammar::Payload, String> {
    if !state.country_identity.has_scalar_source()
        || country_roster_absence_claim_unbacked(&state.country_identity)
    {
        return Err("d18_country_roster_unavailable".into());
    }
    let country_vtable = state.country_identity.country_vtable;
    if copied
        .countries
        .iter()
        .any(|row| row.id == 0 || row.vtable as usize != country_vtable || row.object_id != row.id)
    {
        return Err("country_roster_identity_did_not_hold".into());
    }
    let tags = state
        .country_identity
        .rows
        .iter()
        .map(|row| (row.tag_db_id, row.tag.as_str()))
        .collect::<BTreeMap<_, _>>();
    copied.countries.sort_by_key(|row| row.id);
    let mut countries = Vec::with_capacity(copied.countries.len());
    for row in &copied.countries {
        let tag = tags
            .get(&row.id)
            .ok_or_else(|| format!("country_id_did_not_join_d18_roster:{}", row.id))?;
        countries.push(d74_grammar::Country {
            tag_db_id: row.id,
            tag: (*tag).to_owned(),
        });
    }
    let authority = d41_country_roster_authority_preimage(
        countries
            .iter()
            .map(|row| (row.tag_db_id, row.tag.as_str()))
            .collect(),
    )
    .ok_or_else(|| "d18_country_roster_digest_failed".to_string())?;
    let modifiers = copied
        .countries
        .iter()
        .map(|row| (row.id, row.modifiers.as_slice()))
        .collect::<BTreeMap<_, _>>();
    let mut records = Vec::with_capacity(copied.records.len());
    for (ordinal, record) in copied.records.into_iter().enumerate() {
        let mut members = Vec::with_capacity(record.members.len() / 4);
        for row in record.members.chunks_exact(4) {
            let id = read_u32_from_slice(row, 0).ok_or("member_id_truncated")?;
            let rows = modifiers
                .get(&id)
                .ok_or_else(|| format!("member_id_did_not_join_d18_roster:{id}"))?;
            members.push(d74_grammar::MemberAccumulator {
                tag_db_id: id,
                daily_raw_fixed5: modifier_value(rows, DAILY_KEY, copied.daily_flags)?,
                weekly_raw_fixed5: modifier_value(rows, WEEKLY_KEY, copied.weekly_flags)?,
            });
        }
        members.sort_by_key(|row| row.tag_db_id);
        let mut decoded_overrides = Vec::with_capacity(record.overrides.len());
        for row in record.overrides {
            decoded_overrides.push((decode_string(&row.side)?, decode_string(&row.gfx)?));
        }
        let side_gfx_overrides = resolve_side_gfx_overrides(decoded_overrides);
        records.push(d74_grammar::Record {
            manager_ordinal: u32::try_from(ordinal).map_err(|_| "manager_ordinal_overflow")?,
            balance_id: decode_string(&record.balance)?,
            left_side_id: decode_string(&record.left)?,
            right_side_id: decode_string(&record.right)?,
            trending_side_id: decode_string(&record.trending)?,
            active_range_id: decode_string(&record.range)?,
            value_raw_fixed5: read_i64_from_slice(&record.fixed, 0x30)
                .ok_or("record_value_truncated")?,
            members,
            side_gfx_overrides,
        });
    }
    Ok(d74_grammar::Payload {
        frame_hour: copied.frame_hour,
        status_flags: 0,
        elapsed_wall_ms: copied.elapsed_wall_ms,
        roster_identity_sha256: authority.sha256,
        countries,
        records,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn never_expires() -> CopyBudget {
        CopyBudget::new(u64::MAX)
    }

    fn already_expired() -> CopyBudget {
        let budget = CopyBudget::new(0);
        while budget.started.elapsed() == Duration::ZERO {
            core::hint::spin_loop();
        }
        budget
    }

    fn production_code() -> String {
        include_str!("d74_power_balance.rs")
            .replace("\r\n", "\n")
            .split_once("#[cfg(test)]\nmod tests {")
            .expect("this file has a test module")
            .0
            .lines()
            .map(|line| line.split("//").next().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn production_item(production: &str, opening: &str) -> String {
        let lines: Vec<&str> = production.lines().collect();
        let start = lines
            .iter()
            .position(|line| line.trim_start().starts_with(opening))
            .unwrap_or_else(|| panic!("the production text has no `{opening}`"));
        let indent = &lines[start][..lines[start].len() - lines[start].trim_start().len()];
        let closing = format!("{indent}}}");
        let end = start
            + 1
            + lines[start + 1..]
                .iter()
                .position(|line| *line == closing)
                .expect("the item closes at its own indentation");
        lines[start..=end].join("\n")
    }

    fn one_line(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn one_census_is_metered_by_one_budget_read_off_one_clock() {
        let production = production_code();
        assert!(
            production.contains("fn census_within_budget"),
            "the production text this test reads is the retry site's, not the \
             test module's"
        );
        assert!(
            !production.contains("/*"),
            "only line comments are stripped, so a block comment could hide any \
             of the counts below"
        );
        assert!(
            !production
                .lines()
                .any(|line| line.trim_start().starts_with("mod ")),
            "a child module's own file is out of this scan's reach and could \
             still see CopyBudget"
        );
        assert_eq!(
            production.matches("CopyBudget::new").count(),
            1,
            "copy_census builds the census's only budget; a second one anywhere \
             in production is a second allowance, which no unit test can see \
             because copy_once reads the live game. Naming the constructor \
             without calling it counts too"
        );
        assert_eq!(
            production.matches("CopyBudget::new(").count(),
            1,
            "the constructor is called where it is named, never bound to a name \
             that can then be called as often as it likes"
        );
        assert!(
            !production.contains("Self::new"),
            "inside the impl, Self::new is the same constructor under a name the \
             count above cannot see"
        );
        assert!(
            production_item(&production, "impl CopyBudget {").contains("fn check"),
            "the slice below must span the whole impl, not its first method"
        );
        let retries = one_line(&production_item(&production, "fn census_within_budget"));
        assert_eq!(
            retries.matches("return Ok(").count(),
            1,
            "the retry loop succeeds in exactly one place, the settled arm below; \
             an earlier return hands back a census that reached neither the \
             compare nor the settle"
        );
        assert!(
            retries.contains(
                "if first.equal_within_budget(&second, budget)? { budget.check()?; \
                 return Ok(first); } budget.check()?;"
            ),
            "the settle is checked AFTER the compare, on both arms: a compare \
             whose last unit crosses the deadline is a census that ran out, and \
             a check hoisted above it would return that census as if it had not"
        );
        assert_eq!(
            one_line(&production_item(&production, "fn new(budget_us: u64)")),
            "fn new(budget_us: u64) -> Self { Self { started: Instant::now(), \
             budget: Duration::from_micros(budget_us), #[cfg(test)] checks: \
             std::cell::Cell::new(0), } }",
            "the census starts at the instant the budget is built; a `started` \
             put anywhere but now buys an allowance no test can see"
        );
        assert_eq!(
            one_line(&production_item(&production, "fn check(&self)")),
            "fn check(&self) -> Result<(), &'static str> { #[cfg(test)] \
             self.checks.set(self.checks.get().saturating_add(1)); \
             self.verdict(self.started.elapsed()) }",
            "check is the delegate that hands verdict the census's own elapsed \
             time, untouched; nothing it could do to that time is visible to a \
             test that must not read a clock"
        );
        let constructions = production_item(&production, "impl CopyBudget {")
            .lines()
            .filter(|line| line.trim() == "Self {")
            .count()
            + production
                .lines()
                .map(str::trim)
                .filter(|line| {
                    line.contains("CopyBudget {")
                        && !line.starts_with("struct ")
                        && !line.starts_with("impl ")
                })
                .count();
        assert_eq!(
            constructions, 1,
            "only CopyBudget::new builds one; a struct literal elsewhere would \
             build a budget past it"
        );
        assert_eq!(
            production.matches("Instant::now()").count(),
            1,
            "the census is timed from the one instant CopyBudget::new takes"
        );
    }

    fn production_blocks(production: &str) -> Vec<String> {
        let lines: Vec<&str> = production.lines().collect();
        let mut blocks = Vec::new();
        let mut index = 0;
        while index < lines.len() {
            let line = lines[index];
            let top_level = !line.starts_with(' ') && !line.is_empty();
            let item =
                line.starts_with("fn ") || line.contains(" fn ") || line.starts_with("impl ");
            if top_level && item && (line.ends_with('{') || line.ends_with('(')) {
                let end = lines[index + 1..]
                    .iter()
                    .position(|later| *later == "}")
                    .map(|at| index + 1 + at)
                    .unwrap_or(lines.len() - 1);
                blocks.push(lines[index..=end].join("\n"));
                index = end + 1;
                continue;
            }
            index += 1;
        }
        blocks
    }

    const REPEATS: [&str; 24] = [
        "for ",
        "while ",
        "loop {",
        ".for_each(",
        ".try_for_each(",
        ".fold(",
        ".try_fold(",
        ".collect(",
        ".collect::",
        ".retain(",
        ".flat_map(",
        ".filter_map(",
        ".find_map(",
        ".map_while(",
        ".take_while(",
        ".skip_while(",
        ".inspect(",
        ".reduce(",
        ".partition(",
        ".any(",
        ".all(",
        ".position(",
        ".scan(",
        ".repeat(",
    ];

    fn repetitions(text: &str) -> usize {
        REPEATS
            .iter()
            .map(|repeat| text.matches(repeat).count())
            .sum()
    }

    #[test]
    fn no_live_walk_escapes_the_budget() {
        const LIVE_READS: [&str; 8] = [
            "read_into(",
            "read_u64(",
            "read_u32(",
            "read_u64_stable(",
            "read_u32_stable(",
            "copy_exact(",
            "copy_string(",
            "copy_modifiers(",
        ];
        let blocks = production_blocks(&production_code());
        assert!(
            blocks.len() >= 8,
            "the production text did not split into items: {} found",
            blocks.len()
        );
        let mut walkers = Vec::new();
        for block in &blocks {
            if !REPEATS.iter().any(|repeat| block.contains(repeat)) {
                continue;
            }
            if !LIVE_READS.iter().any(|call| block.contains(call)) {
                continue;
            }
            let name = block.lines().next().unwrap_or_default().trim().to_owned();
            assert!(
                block.contains("budget.check()?;"),
                "`{name}` walks live memory without reaching the budget"
            );
            walkers.push(name);
        }
        assert_eq!(
            walkers,
            vec!["fn copy_once(".to_string()],
            "copy_once is the only production item that walks live memory. A \
             helper that grows a loop over reads walks it too, however small \
             each read looks, and owes the budget a check per row"
        );
    }

    #[test]
    fn the_live_copy_reaches_the_budget_before_each_row_it_copies() {
        let copy_once = one_line(&production_item(&production_code(), "fn copy_once("));
        assert!(
            copy_once.starts_with(
                "fn copy_once( state: &CollectorState, frame_hour: u64, \
                 elapsed_wall_ms: u64, budget: &CopyBudget, ) -> \
                 Result<CopiedCensus, &'static str> { budget.check()?;"
            ),
            "the budget is reached before the first live read of the pass, not \
             only before each walk: with no countries the manager's whole record \
             array is copied before the record walk starts"
        );
        for walk in [
            "for row in &state.country_identity.rows { budget.check()?;",
            "for ordinal in 0..count as usize { budget.check()?;",
            "for index in 0..override_count as usize { budget.check()?;",
        ] {
            assert!(
                copy_once.contains(walk),
                "copy_once no longer opens `{walk}`. The whole header is pinned, \
                 not just the first statement, because an iterator adaptor in the \
                 header reads the game before the body reaches the budget"
            );
        }
        assert_eq!(
            copy_once.matches("copy_exact(base, fixed_len)").count(),
            1,
            "the manager's record array is copied once, by the bulk copy the \
             next assertion meters"
        );
        assert!(
            copy_once.contains("budget.check()?; let fixed_all = copy_exact(base, fixed_len)"),
            "the one bulk copy between the country walk and the record walk is \
             the largest single read of the pass and is reached through the \
             budget, not after the last country's check"
        );
        assert_eq!(
            repetitions(&copy_once),
            3,
            "copy_once repeats itself exactly three times, and the three pinned \
             above are those three. Any other repetition here -- another walk, \
             an iterator adaptor driving a closure, a bare loop -- reads the \
             game a row at a time behind whichever check came before it"
        );
    }

    #[test]
    fn the_copy_budget_expires_and_names_the_hour() {
        let budget = CopyBudget::new(0);
        while budget.started.elapsed() == Duration::ZERO {
            core::hint::spin_loop();
        }
        assert_eq!(
            budget.check(),
            Err("power_balance_census_copy_budget_exceeded")
        );
    }

    #[test]
    fn the_budget_expires_exactly_when_the_elapsed_time_passes_it() {
        let metered = CopyBudget::new(CENSUS_COPY_BUDGET_US);
        let budget = Duration::from_micros(CENSUS_COPY_BUDGET_US);
        assert_eq!(metered.budget, budget);
        for under in [
            Duration::ZERO,
            budget / 2,
            budget - Duration::from_nanos(1),
            budget,
        ] {
            assert_eq!(metered.verdict(under), Ok(()), "{under:?}");
        }
        for over in [budget + Duration::from_nanos(1), budget * 2, Duration::MAX] {
            assert_eq!(
                metered.verdict(over),
                Err("power_balance_census_copy_budget_exceeded"),
                "{over:?}"
            );
        }

        assert_eq!(never_expires().check(), Ok(()));
        assert_eq!(
            already_expired().check(),
            Err("power_balance_census_copy_budget_exceeded")
        );

        assert!(
            CENSUS_COPY_BUDGET_US >= 8_000,
            "the budget must stay clear of the worst measured census"
        );
    }

    fn copied_string(text: &str) -> CopiedString {
        let mut object = [0; 32];
        object[0x10..0x18].copy_from_slice(&(text.len() as u64).to_le_bytes());
        object[0x18..0x20].copy_from_slice(&64u64.to_le_bytes());
        CopiedString {
            object,
            heap: text.as_bytes().to_vec(),
        }
    }

    fn census_of(countries: usize, records: usize) -> CopiedCensus {
        CopiedCensus {
            frame_hour: 7,
            elapsed_wall_ms: 11,
            manager_header: [3; 16],
            countries: (0..countries)
                .map(|id| CopiedCountry {
                    id: id as u32,
                    vtable: 0x1234,
                    object_id: id as u32,
                    modifiers: vec![id as u8; 0x10],
                })
                .collect(),
            records: (0..records)
                .map(|ordinal| CopiedRecord {
                    fixed: vec![ordinal as u8; RECORD_STRIDE],
                    balance: copied_string("balance"),
                    left: copied_string("left"),
                    right: copied_string("right"),
                    trending: copied_string("left"),
                    range: copied_string("range"),
                    members: vec![1, 2, 3, 4],
                    _range_vector: vec![5, 6, 7, 8],
                    overrides: vec![CopiedOverride {
                        side: copied_string("left"),
                        gfx: copied_string("gfx"),
                    }],
                })
                .collect(),
            daily_flags: 0x21,
            weekly_flags: 0x42,
        }
    }

    fn census_shaped(countries: usize, records: usize, overrides: usize) -> CopiedCensus {
        let mut census = census_of(countries, records);
        for record in &mut census.records {
            record.overrides = (0..overrides)
                .map(|index| CopiedOverride {
                    side: copied_string(&format!("side{index}")),
                    gfx: copied_string("gfx"),
                })
                .collect();
        }
        census
    }

    fn compare_spending(
        mine: &CopiedCensus,
        theirs: &CopiedCensus,
    ) -> (Result<bool, &'static str>, usize) {
        let budget = never_expires();
        let outcome = mine.equal_within_budget(theirs, &budget);
        (outcome, budget.checks.get() as usize)
    }

    #[test]
    fn comparing_the_two_passes_is_spent_under_the_same_budget_as_the_copy() {
        let first = census_of(3, 2);
        let second = census_of(3, 2);
        let free = never_expires();
        assert_eq!(first.equal_within_budget(&second, &free), Ok(true));

        let spent = already_expired();
        assert_eq!(
            first.equal_within_budget(&second, &spent),
            Err("power_balance_census_copy_budget_exceeded")
        );
        assert_eq!(
            census_of(0, 1).equal_within_budget(&census_of(0, 1), &already_expired()),
            Err("power_balance_census_copy_budget_exceeded"),
            "the record walk carries the budget of its own, not the country walk's"
        );

        assert_eq!(
            first.equal_within_budget(&census_of(2, 2), &free),
            Ok(false)
        );
        assert_eq!(
            first.equal_within_budget(&census_of(3, 1), &free),
            Ok(false)
        );
        assert_eq!(
            first.equal_within_budget(&census_of(2, 2), &spent),
            Ok(false)
        );

        let mut third = census_of(3, 2);
        third.records[1].overrides[0].gfx = copied_string("other_gfx");
        assert_eq!(first.equal_within_budget(&third, &free), Ok(false));
        let mut fourth = census_of(3, 2);
        fourth.countries[2].modifiers = vec![0xFF; 0x10];
        assert_eq!(first.equal_within_budget(&fourth, &free), Ok(false));

        for (countries, records, overrides) in [(5, 4, 1), (9, 7, 3), (4, 5, 0), (0, 0, 0)] {
            let same = census_shaped(countries, records, overrides);
            assert_eq!(
                compare_spending(&same, &census_shaped(countries, records, overrides)),
                (Ok(true), countries + records + records * overrides),
                "one check per country, per record and per override, and no \
                 other budget the compare pass could be spending instead"
            );
        }

        let mut countries_differ = census_shaped(5, 4, 0);
        countries_differ.countries[2].modifiers = vec![0xFF; 0x10];
        assert_eq!(
            compare_spending(&census_shaped(5, 4, 0), &countries_differ),
            (Ok(false), 3),
            "the third country cost a check before it was read, not after"
        );

        let mut records_differ = census_shaped(0, 4, 0);
        records_differ.records[2].fixed = vec![0xFF; RECORD_STRIDE];
        assert_eq!(
            compare_spending(&census_shaped(0, 4, 0), &records_differ),
            (Ok(false), 3),
            "the third record cost a check before it was read, not after"
        );

        let mut overrides_differ = census_shaped(0, 1, 3);
        overrides_differ.records[0].overrides[1].gfx = copied_string("other_gfx");
        assert_eq!(
            compare_spending(&census_shaped(0, 1, 3), &overrides_differ),
            (Ok(false), 3),
            "the second override cost a check before it was read, not after"
        );
    }

    #[test]
    fn the_retry_loop_spends_every_pass_out_of_the_one_budget_it_was_given() {
        let budget = never_expires();
        let copies = std::cell::Cell::new(0usize);
        let copy = |budget: &CopyBudget| {
            copies.set(copies.get() + 1);
            budget.check()?;
            Ok(census_shaped(2, 1, 1))
        };
        assert!(census_within_budget(&budget, copy).is_ok());
        assert_eq!(copies.get(), 2, "one settled census is two copies");
        assert_eq!(
            budget.checks.get() as usize,
            2 + (2 + 1 + 1) + 1,
            "both copies, the compare of them and the settle all draw on the \
             budget handed in, and nothing draws on one of its own"
        );

        let churning = never_expires();
        let churned = std::cell::Cell::new(0usize);
        assert_eq!(
            census_within_budget(&churning, |budget| {
                churned.set(churned.get() + 1);
                budget.check()?;
                Ok(census_shaped(churned.get(), 0, 0))
            })
            .err(),
            Some("power_balance_data_changed_while_being_read")
        );
        assert_eq!(churned.get(), 2 * RETRIES);
        assert_eq!(churning.checks.get() as usize, 2 * RETRIES + RETRIES);

        let spent = already_expired();
        let attempted = std::cell::Cell::new(0usize);
        assert_eq!(
            census_within_budget(&spent, |budget| {
                attempted.set(attempted.get() + 1);
                budget.check()?;
                Ok(census_shaped(1, 1, 1))
            })
            .err(),
            Some("power_balance_census_copy_budget_exceeded")
        );
        assert_eq!(
            attempted.get(),
            1,
            "a budget already gone is not restarted by the retry loop"
        );
    }

    #[test]
    fn one_countrys_modifier_container_is_bounded_to_a_copy_the_budget_can_afford() {
        assert_eq!(MAX_MODIFIER_ROWS, 65_536);
        assert!(
            (MAX_MODIFIER_ROWS as usize) * 0x10 <= 1024 * 1024,
            "one indivisible modifier copy must stay inside a mebibyte"
        );
    }

    #[test]
    fn a_world_too_big_for_the_payload_is_refused_before_it_is_copied() {
        assert_eq!(admit_country_count(d74_grammar::MAX_COUNTRIES), Ok(()));
        assert_eq!(
            admit_country_count(d74_grammar::MAX_COUNTRIES + 1),
            Err("country_count_exceeds_payload_limit")
        );
    }

    #[test]
    fn copied_modifier_rows_must_be_strictly_sorted() {
        let mut rows = Vec::new();
        for (key, value) in [(0x99u32, 7i64), (0x98, 3)] {
            rows.extend_from_slice(&key.to_le_bytes());
            rows.extend_from_slice(&0u32.to_le_bytes());
            rows.extend_from_slice(&value.to_le_bytes());
        }
        assert_eq!(
            modifier_value(&rows, DAILY_KEY, 0),
            Err("modifier_keys_not_strictly_sorted")
        );
    }

    #[test]
    fn side_gfx_overrides_take_the_first_identity_and_skip_empty_gfx() {
        let resolved = resolve_side_gfx_overrides([
            ("FIN_fascism".to_string(), "GFX_first".to_string()),
            ("FIN_fascism".to_string(), "GFX_second".to_string()),
            ("FIN_democracy".to_string(), String::new()),
            (String::new(), "GFX_orphan".to_string()),
            ("FIN_neutrality".to_string(), "GFX_third".to_string()),
        ]);
        assert_eq!(
            resolved,
            vec![
                d74_grammar::SideGfxOverride {
                    side_id: "FIN_fascism".into(),
                    gfx: "GFX_first".into(),
                },
                d74_grammar::SideGfxOverride {
                    side_id: "FIN_neutrality".into(),
                    gfx: "GFX_third".into(),
                },
            ]
        );
    }
}
