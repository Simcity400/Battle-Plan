
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x02a0_0e18);
pub const SECONDARY_VTABLE_RVA: Rva = Rva(0x02a0_0e50);
pub const SECONDARY_BASE: usize = 0x18;
pub const OCCUPATION_DATA_TABLE: usize = 0x48;
pub const OCCUPATION_DATA_ROWS: usize = 0x60;
pub const GARRISON_TEMPLATE: usize = 0x78;
pub const GARRISON_PRIORITY: usize = 8;

pub fn template_priority(
    memory: &impl Memory,
    base: usize,
    country: usize,
) -> Result<i32, &'static str> {
    let holder = status(memory, base, country)?;
    holder
        .checked_add(GARRISON_PRIORITY)
        .and_then(|at| read::<i32>(memory, at))
        .ok_or("garrison reinforcement priority unreadable")
}
pub const GROUP_TEMPLATE: usize = 0xc8;
pub const STATE_TEMPLATES: usize = 0xf8;
pub const ASSIGNED_MANPOWER: usize = 0x90;
pub const DEFAULT_LAW: usize = 0x118;
pub const RESET_RVA: Rva = Rva(0x0107_beb0);

use engine_native_memory::{Memory, read, walk::Outcome};
use engine_native::{ccountry, creferenced_division_template};
use crate::{
    ccountry_occupation_data::{self, LawRow, Scope},
    coccupation_law,
    cstate_garrison_data::{self, ManpowerRow},
    manpower_value_entry,
    pdx_robin_hood_table,
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub garrison_template: Option<u64>,
    pub default_law_present: u8,
    pub law_start: usize,
    pub law_len: usize,
    pub occupied: Option<ccountry::RosterSpan>,
    pub occupied_reason: Option<&'static str>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReserveManpowerRow {
    pub country: u32,
    pub ordinal: u32,
    pub source_country: u32,
    pub amount: i32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TemplateSelection {
    pub country: u32,
    pub occupied: Option<u32>,
    pub state: Option<u32>,
    pub template: Option<u64>,
}

pub fn visit_template_selections(
    memory: &impl Memory,
    base: usize,
    country: engine_native_memory::country_identity::Country,
    countries: &ccountry::Reader<'_, impl Memory>,
    mut visit: impl FnMut(TemplateSelection) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let parent = status(memory, base, country.pointer)?;
    let template = |at: usize| {
        let pointer = read::<u64>(memory, at)
            .and_then(|p| usize::try_from(p).ok())
            .ok_or("garrison template selection unreadable")?;
        if pointer == 0 {
            Ok(None)
        } else {
            creferenced_division_template::identity(memory, base, pointer).map(Some)
        }
    };
    visit(TemplateSelection {
        country: country.id,
        template: template(
            parent
                .checked_add(GARRISON_TEMPLATE)
                .ok_or("garrison selection address overflow")?,
        )?,
        ..Default::default()
    })?;
    pdx_robin_hood_table::visit(
        memory,
        parent
            .checked_add(OCCUPATION_DATA_TABLE)
            .ok_or("garrison selection address overflow")?,
        |entry| {
            let occupied =
                ccountry_occupation_data::identity(memory, base, parent, entry, countries)?;
            visit(TemplateSelection {
                country: country.id,
                occupied: Some(occupied),
                state: None,
                template: template(
                    entry
                        .value
                        .checked_add(GROUP_TEMPLATE)
                        .ok_or("garrison selection address overflow")?,
                )?,
            })?;
            pdx_robin_hood_table::visit(
                memory,
                entry
                    .value
                    .checked_add(STATE_TEMPLATES)
                    .ok_or("garrison selection address overflow")?,
                |state| {
                    visit(TemplateSelection {
                        country: country.id,
                        occupied: Some(occupied),
                        state: Some(state.key),
                        template: Some(creferenced_division_template::identity(
                            memory,
                            base,
                            state.value,
                        )?),
                    })
                },
            )
        },
    )
}

pub struct Buffers<'a> {
    pub settings: &'a mut Vec<Row>,
    pub laws: &'a mut Vec<LawRow>,
    pub group_laws: &'a mut Vec<ccountry_occupation_data::GroupLawRow>,
    pub garrisons: &'a mut Vec<cstate_garrison_data::Row>,
    pub manpower: &'a mut Vec<ManpowerRow>,
    pub reserve_manpower: &'a mut Vec<ReserveManpowerRow>,
    pub names: &'a mut Vec<u8>,
    pub equipment: &'a mut Vec<cstate_garrison_data::equipment::Row>,
}

pub struct BuffersBuffer<'a> {
    pub settings: engine_native_memory::walk::Buffer<'a, Row>,
    pub laws: engine_native_memory::walk::Buffer<'a, LawRow>,
    pub group_laws: engine_native_memory::walk::Buffer<'a, ccountry_occupation_data::GroupLawRow>,
    pub garrisons: engine_native_memory::walk::Buffer<'a, cstate_garrison_data::Row>,
    pub manpower: engine_native_memory::walk::Buffer<'a, ManpowerRow>,
    pub reserve_manpower: engine_native_memory::walk::Buffer<'a, ReserveManpowerRow>,
    pub names: engine_native_memory::walk::Buffer<'a, u8>,
    pub equipment: engine_native_memory::walk::Buffer<'a, cstate_garrison_data::equipment::Row>,
}

impl<'a> From<Buffers<'a>> for BuffersBuffer<'a> {
    fn from(value: Buffers<'a>) -> Self {
        Self {
            settings: engine_native_memory::walk::Buffer::Growing(value.settings),
            laws: engine_native_memory::walk::Buffer::Growing(value.laws),
            group_laws: engine_native_memory::walk::Buffer::Growing(value.group_laws),
            garrisons: engine_native_memory::walk::Buffer::Growing(value.garrisons),
            manpower: engine_native_memory::walk::Buffer::Growing(value.manpower),
            reserve_manpower: engine_native_memory::walk::Buffer::Growing(value.reserve_manpower),
            names: engine_native_memory::walk::Buffer::Growing(value.names),
            equipment: engine_native_memory::walk::Buffer::Growing(value.equipment),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub settings: Outcome,
    pub laws: Outcome,
    pub group_laws: Outcome,
    pub garrisons: Outcome,
    pub manpower: Outcome,
    pub reserve_manpower: Outcome,
    pub bytes_len: usize,
    pub required_bytes: usize,
    pub equipment_len: usize,
    pub required_equipment: usize,
    pub fallback_law: Option<(usize, usize)>,
    pub occupied_len: usize,
    pub required_occupied: usize,
}

struct Occupied<'a> {
    rows: engine_native_memory::walk::Buffer<'a, u32>,
    len: usize,
    required: usize,
}

impl Occupied<'_> {
    fn read(
        &mut self,
        memory: &impl Memory,
        base: usize,
        status: usize,
    ) -> Result<ccountry::RosterSpan, &'static str> {
        let source = status
            .checked_add(OCCUPATION_DATA_ROWS)
            .and_then(|at| engine_native_memory::cpdx_array::Array::read(memory, at, 8))
            .ok_or("occupied-country roster unreadable or invalid")?;
        let count = source.count as usize;
        self.required = self
            .required
            .checked_add(count)
            .ok_or("occupied-country roster demand overflow")?;
        self.rows.ensure(self.required)?;
        if count > self.rows.len().saturating_sub(self.len) {
            return Err("occupied-country roster arena capacity exceeded");
        }
        let span = ccountry::RosterSpan {
            start: self.len,
            count,
        };
        engine_native_memory::cpdx_array::pointers(
            memory,
            source,
            "occupied-country roster pointer unreadable",
            |ordinal, pointer| {
                let fields = read::<[u8; 32]>(memory, pointer)
                    .ok_or("occupied-country roster group unreadable")?;
                if engine_sites::at(base, ccountry_occupation_data::VTABLE_RVA).map(|p| p as u64)
                    != Some(u64::from_le_bytes(fields[..8].try_into().unwrap()))
                {
                    return Err("occupied-country roster group vtable mismatch");
                }
                if u64::from_le_bytes(fields[8..16].try_into().unwrap()) != status as u64 {
                    return Err("occupied-country roster group parent mismatch");
                }
                let country = usize::try_from(u64::from_le_bytes(fields[24..].try_into().unwrap()))
                    .map_err(|_| "occupied-country roster country address overflow")?;
                let identity = read::<[u8; 12]>(memory, country)
                    .ok_or("occupied-country roster country unreadable")?;
                if engine_sites::at(base, ccountry::VTABLE_RVA).map(|p| p as u64)
                    != Some(u64::from_le_bytes(identity[..8].try_into().unwrap()))
                {
                    return Err("occupied-country roster country vtable mismatch");
                }
                let id = u32::from_le_bytes(identity[8..].try_into().unwrap());
                self.rows[span.start + ordinal as usize] = id;
                Ok(())
            },
        )?;
        self.len += count;
        Ok(span)
    }
}

pub fn occupied_demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<usize, &'static str> {
    let countries = ccountry::Reader::new(memory, base, state)?;
    let mut required = 0_usize;
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        let pointer = status(memory, base, country.pointer)?;
        let source = pointer
            .checked_add(OCCUPATION_DATA_ROWS)
            .and_then(|at| engine_native_memory::cpdx_array::Array::read(memory, at, 8))
            .ok_or("occupied-country roster unreadable or invalid")?;
        required = required
            .checked_add(source.count as usize)
            .ok_or("occupied-country roster demand overflow")?;
    }
    Ok(required)
}

pub(crate) struct Rows<'a, T> {
    pub data: engine_native_memory::walk::Buffer<'a, T>,
    pub len: usize,
    unread: u32,
    reason: Option<&'static str>,
}

impl<'a, T: Default> Rows<'a, T> {
    fn new(data: engine_native_memory::walk::Buffer<'a, T>) -> Self {
        Self {
            data,
            len: 0,
            unread: 0,
            reason: None,
        }
    }
    pub fn fits(&mut self) -> Result<bool, &'static str> {
        if self.reason.is_some() {
            return Ok(false);
        }
        self.data.ensure(
            self.len
                .checked_add(1)
                .ok_or("occupation row count overflow")?,
        )
    }
    pub fn remaining(&self) -> usize {
        if self.reason.is_some() {
            0
        } else if self.data.is_growing() {
            usize::MAX
        } else {
            self.data.len() - self.len
        }
    }
    pub fn push(&mut self, row: T) -> Result<(), &'static str> {
        if self.fits()? {
            self.data[self.len] = row;
            self.len += 1;
            Ok(())
        } else {
            self.omit()
        }
    }
    pub fn omit(&mut self) -> Result<(), &'static str> {
        self.omit_n(1)
    }
    pub fn omit_n(&mut self, count: u32) -> Result<(), &'static str> {
        self.unread = self
            .unread
            .checked_add(count)
            .ok_or("occupation omitted row count overflow")?;
        Ok(())
    }
    pub fn fail(&mut self, reason: &'static str) {
        self.reason.get_or_insert(reason);
    }
    fn outcome(&self) -> Outcome {
        match self.reason {
            Some(reason) => Outcome::Unavailable { reason },
            None if self.unread == 0 => Outcome::Rows { len: self.len },
            None => Outcome::Truncated {
                len: self.len,
                unread_rows: self.unread,
            },
        }
    }
}

pub(crate) fn status(
    memory: &impl Memory,
    base: usize,
    country: usize,
) -> Result<usize, &'static str> {
    let pointer = country
        .checked_add(ccountry::OCCUPATION_STATUS)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .ok_or("occupation status pointer unreadable")?;
    let primary =
        engine_sites::at(base, VTABLE_RVA).ok_or("occupation status primary vtable mismatch")?;
    if read::<u64>(memory, pointer) != Some(primary as u64) {
        return Err("occupation status primary vtable mismatch");
    }
    let secondary = engine_sites::at(base, SECONDARY_VTABLE_RVA)
        .ok_or("occupation status secondary vtable mismatch")?;
    if Some(secondary as u64)
        != pointer
            .checked_add(SECONDARY_BASE)
            .and_then(|p| read::<u64>(memory, p))
    {
        return Err("occupation status secondary vtable mismatch");
    }
    Ok(pointer)
}

fn settings(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    country: u32,
    rows: &mut Rows<'_, Row>,
    names: &mut coccupation_law::Names<'_>,
) -> Result<(), &'static str> {
    let template = pointer
        .checked_add(GARRISON_TEMPLATE)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .ok_or("garrison template pointer unreadable")?;
    let garrison_template = if template == 0 {
        None
    } else {
        let expected = engine_sites::at(base, creferenced_division_template::VTABLE_RVA)
            .ok_or("garrison template vtable mismatch")?;
        if read::<u64>(memory, template) != Some(expected as u64) {
            return Err("garrison template vtable mismatch");
        }
        let reference = template
            .checked_add(creferenced_division_template::TYPE)
            .and_then(|p| read::<u64>(memory, p))
            .ok_or("garrison template reference unreadable")?;
        if reference == 0 {
            return Err("garrison template reference is zero");
        }
        Some(reference)
    };
    let law = pointer
        .checked_add(DEFAULT_LAW)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .ok_or("default occupation law pointer unreadable")?;
    if law == 0 {
        return rows.push(Row {
            country,
            garrison_template,
            ..Row::default()
        });
    }
    let token = coccupation_law::literal(memory, base, law)?;
    match names.copy(memory, token, rows.fits()?)? {
        Some(law_start) => rows.push(Row {
            country,
            garrison_template,
            default_law_present: 1,
            law_start,
            law_len: token.len,
            ..Row::default()
        }),
        None => rows.omit(),
    }
}

pub fn walk(memory: &impl Memory, base: usize, game_state: usize, buffers: Buffers<'_>) -> Walk {
    walk_inner(memory, base, game_state, buffers.into(), None)
}

#[cfg(test)]
pub fn walk_with_occupied(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    buffers: Buffers<'_>,
    occupied: &mut Vec<u32>,
) -> Walk {
    walk_inner(
        memory,
        base,
        game_state,
        buffers.into(),
        Some(Occupied {
            rows: engine_native_memory::walk::Buffer::Growing(occupied),
            len: 0,
            required: 0,
        }),
    )
}

pub fn walk_with_occupied_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    buffers: BuffersBuffer<'_>,
    occupied: engine_native_memory::walk::Buffer<'_, u32>,
) -> Walk {
    walk_inner(
        memory,
        base,
        game_state,
        buffers,
        Some(Occupied {
            rows: occupied,
            len: 0,
            required: 0,
        }),
    )
}

fn walk_inner(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    buffers: BuffersBuffer<'_>,
    mut occupied: Option<Occupied<'_>>,
) -> Walk {
    let mut setting_rows = Rows::new(buffers.settings);
    let mut law_rows = Rows::new(buffers.laws);
    let mut group_rows = Rows::new(buffers.group_laws);
    let mut garrison_rows = Rows::new(buffers.garrisons);
    let mut manpower_rows = Rows::new(buffers.manpower);
    let mut reserve_rows = Rows::new(buffers.reserve_manpower);
    let mut names = coccupation_law::Names::new_buffer(buffers.names);
    let mut equipment = cstate_garrison_data::equipment::Arena::new_buffer(buffers.equipment);
    let fallback_law = coccupation_law::fallback(memory, base)
        .ok()
        .and_then(|token| {
            names
                .copy(memory, token, true)
                .ok()
                .flatten()
                .map(|start| (start, token.len))
        });
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let states = ccountry_occupation_data::state_array(memory, game_state);
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let pointer = match status(memory, base, country.pointer) {
                Ok(pointer) => pointer,
                Err(reason) => {
                    setting_rows.fail(reason);
                    law_rows.fail(reason);
                    group_rows.fail(reason);
                    garrison_rows.fail(reason);
                    manpower_rows.fail(reason);
                    reserve_rows.fail(reason);
                    continue;
                }
            };
            let first_setting = setting_rows.len;
            if let Err(reason) = settings(
                memory,
                base,
                pointer,
                country.id,
                &mut setting_rows,
                &mut names,
            ) {
                setting_rows.fail(reason);
            }
            if setting_rows.len > first_setting
                && let Some(occupied) = &mut occupied
            {
                let row = &mut setting_rows.data[first_setting];
                match occupied.read(memory, base, pointer) {
                    Ok(span) => row.occupied = Some(span),
                    Err(reason) => row.occupied_reason = Some(reason),
                }
            }
            let reserve = pointer
                .checked_add(ASSIGNED_MANPOWER)
                .ok_or("occupation reserve manpower address overflow")
                .and_then(|container| {
                    manpower_value_entry::visit(
                        memory,
                        base,
                        container,
                        reserve_rows.remaining(),
                        |ordinal, source_country, amount| {
                            reserve_rows.push(ReserveManpowerRow {
                                country: country.id,
                                ordinal,
                                source_country,
                                amount,
                            })
                        },
                    )
                });
            match reserve {
                Ok(unread) => reserve_rows.omit_n(unread)?,
                Err(reason) => reserve_rows.fail(reason),
            }
            let graph = pointer
                .checked_add(OCCUPATION_DATA_TABLE)
                .ok_or("occupation data table address overflow")
                .and_then(|table| {
                    pdx_robin_hood_table::visit(memory, table, |entry| {
                        let occupied = ccountry_occupation_data::identity(
                            memory, base, pointer, entry, &countries,
                        )?;
                        let scope = Scope {
                            controller: country.id,
                            occupied,
                            pointer: entry.value,
                        };
                        if let Err(reason) = ccountry_occupation_data::group_law(
                            memory,
                            base,
                            &scope,
                            &mut group_rows,
                            &mut names,
                        ) {
                            group_rows.fail(reason);
                        }
                        if let Err(reason) = ccountry_occupation_data::laws(
                            memory,
                            base,
                            &scope,
                            &mut law_rows,
                            &mut names,
                        ) {
                            law_rows.fail(reason);
                        }
                        if let Err(reason) = states.and_then(|states| {
                            cstate_garrison_data::walk_data(
                                memory,
                                base,
                                states,
                                &scope,
                                &mut garrison_rows,
                                &mut manpower_rows,
                                &mut equipment,
                            )
                        }) {
                            garrison_rows.fail(reason);
                            manpower_rows.fail(reason);
                        }
                        Ok(())
                    })
                });
            if let Err(reason) = graph {
                law_rows.fail(reason);
                group_rows.fail(reason);
                garrison_rows.fail(reason);
                manpower_rows.fail(reason);
            }
        }
        Ok::<(), &'static str>(())
    })();
    if let Err(reason) = result {
        setting_rows.fail(reason);
        law_rows.fail(reason);
        group_rows.fail(reason);
        garrison_rows.fail(reason);
        manpower_rows.fail(reason);
        reserve_rows.fail(reason);
    }
    group_rows.data[..group_rows.len].sort_unstable_by_key(|row| (row.controller, row.occupied));
    if group_rows.data[..group_rows.len].windows(2).any(|rows| {
        (rows[0].controller, rows[0].occupied) == (rows[1].controller, rows[1].occupied)
    }) {
        group_rows.fail("duplicate occupied-country law");
    }
    law_rows.data[..law_rows.len]
        .sort_unstable_by_key(|row| (row.controller, row.occupied, row.state));
    if law_rows.data[..law_rows.len].windows(2).any(|rows| {
        (rows[0].controller, rows[0].occupied, rows[0].state)
            == (rows[1].controller, rows[1].occupied, rows[1].state)
    }) {
        law_rows.fail("duplicate occupation state law");
    }
    garrison_rows.data[..garrison_rows.len].sort_unstable_by_key(|row| row.state);
    if garrison_rows.data[..garrison_rows.len]
        .windows(2)
        .any(|rows| rows[0].state == rows[1].state)
    {
        garrison_rows.fail("duplicate state garrison");
    }
    manpower_rows.data[..manpower_rows.len].sort_unstable_by_key(|row| (row.state, row.ordinal));
    if manpower_rows.data[..manpower_rows.len]
        .windows(2)
        .any(|rows| (rows[0].state, rows[0].ordinal) == (rows[1].state, rows[1].ordinal))
    {
        manpower_rows.fail("duplicate state garrison manpower entry");
    }
    Walk {
        settings: setting_rows.outcome(),
        laws: law_rows.outcome(),
        group_laws: group_rows.outcome(),
        garrisons: garrison_rows.outcome(),
        manpower: manpower_rows.outcome(),
        reserve_manpower: reserve_rows.outcome(),
        bytes_len: names.len,
        required_bytes: names.required,
        occupied_len: occupied.as_ref().map_or(0, |arena| arena.len),
        required_occupied: occupied.as_ref().map_or(0, |arena| arena.required),
        equipment_len: equipment.len,
        required_equipment: equipment.required,
        fallback_law,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_parts::carmy_manpower_values;
    use engine_native::{cgame_state, cstate};
    use engine_native_memory::memory::SliceMemory;

    #[test]
    fn occupied_country_roster_keeps_native_order_and_preserves_settings_on_refusal() {
        let mut bytes = fixture();
        array(&mut bytes, 0x7000 + OCCUPATION_DATA_ROWS, 0x9200, 2);
        u64_at(&mut bytes, 0x9200, 0x8400);
        u64_at(&mut bytes, 0x9208, 0x8400);
        u64_at(
            &mut bytes,
            0x8400 + ccountry_occupation_data::OCCUPIED_COUNTRY_POINTER,
            0x5000,
        );
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        assert_eq!(occupied_demand(&memory, 0, BASE), Ok(2));
        {
            let mut settings = vec![Row::default(); 2];
            let mut laws = vec![LawRow::default(); 8];
            let mut garrisons = vec![cstate_garrison_data::Row::default(); 8];
            let mut manpower = vec![ManpowerRow::default(); 8];
            let mut reserve = vec![ReserveManpowerRow::default(); 8];
            let mut names = vec![0; 512];
            let mut occupied = Vec::new();
            let walk = walk_with_occupied(
                &memory,
                0,
                BASE,
                Buffers {
                    settings: &mut settings,
                    laws: &mut laws,
                    group_laws: &mut vec![ccountry_occupation_data::GroupLawRow::default(); 2],
                    garrisons: &mut garrisons,
                    manpower: &mut manpower,
                    reserve_manpower: &mut reserve,
                    names: &mut names,
                    equipment: &mut Vec::new(),
                },
                &mut occupied,
            );
            assert_eq!(walk.settings, Outcome::Rows { len: 2 });
            assert_eq!(walk.required_occupied, 2);
            assert_eq!(settings[0].default_law_present, 1);
            assert_eq!(settings[0].garrison_template, Some((7_u64 << 32) | 52));
            assert_eq!(occupied, [99, 99]);
            assert_eq!(
                settings[0].occupied,
                Some(ccountry::RosterSpan { start: 0, count: 2 })
            );
            assert_eq!(settings[0].occupied_reason, None);
            assert!(settings[1].occupied.is_some_and(|span| span.count == 0));
        }
    }

    #[test]
    fn template_selections_keep_defaults_inherited_groups_and_explicit_state_overrides() {
        let mut bytes = fixture();
        table(&mut bytes, 0x8400 + STATE_TEMPLATES, 0x8800, 0, 0xa200);
        u64_at(
            &mut bytes,
            0xa200,
            creferenced_division_template::VTABLE_RVA.0 as u64,
        );
        u64_at(
            &mut bytes,
            0xa200 + creferenced_division_template::TYPE,
            (9_u64 << 32) | 52,
        );
        u64_at(&mut bytes, 0x8800 + 24 + 16, u64::MAX);
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let countries = ccountry::Reader::new(&memory, 0, BASE).unwrap();
        let mut rows = Vec::new();
        visit_template_selections(
            &memory,
            0,
            countries.at(1).unwrap().unwrap(),
            &countries,
            |row| {
                rows.push(row);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            rows,
            [
                TemplateSelection {
                    country: 1,
                    occupied: None,
                    state: None,
                    template: Some((7_u64 << 32) | 52)
                },
                TemplateSelection {
                    country: 1,
                    occupied: Some(99),
                    state: None,
                    template: None
                },
                TemplateSelection {
                    country: 1,
                    occupied: Some(99),
                    state: Some(0),
                    template: Some((9_u64 << 32) | 52)
                },
            ]
        );
        u64_at(&mut bytes, 0xa200, 0);
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let countries = ccountry::Reader::new(&memory, 0, BASE).unwrap();
        assert!(
            visit_template_selections(
                &memory,
                0,
                countries.at(1).unwrap().unwrap(),
                &countries,
                |_| Ok(())
            )
            .is_err()
        );
    }

    #[test]
    fn garrison_priority_is_independent_of_template_selection_and_keeps_signed_words() {
        let mut bytes = fixture();
        for priority in [1_i32, 0, -7, i32::MIN, i32::MAX] {
            u32_at(&mut bytes, 0x7000 + GARRISON_PRIORITY, priority as u32);
            let memory = SliceMemory {
                base: BASE,
                bytes: &bytes,
            };
            assert_eq!(template_priority(&memory, 0, 0x3000).unwrap(), priority);
        }
    }

    const BASE: usize = 0x1000;

    fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
        bytes[address - BASE..address - BASE + value.len()].copy_from_slice(value);
    }
    fn u32_at(bytes: &mut [u8], address: usize, value: u32) {
        put(bytes, address, &value.to_le_bytes());
    }
    fn u64_at(bytes: &mut [u8], address: usize, value: u64) {
        put(bytes, address, &value.to_le_bytes());
    }
    fn array(bytes: &mut [u8], address: usize, data: u64, count: u32) {
        u64_at(bytes, address, data);
        u32_at(bytes, address + 8, count);
        u32_at(bytes, address + 12, count);
    }
    fn table(bytes: &mut [u8], address: usize, data: u64, key: u32, value: u64) {
        u64_at(bytes, address, data);
        u32_at(bytes, address + 8, 1);
        u32_at(bytes, address + 12, 1);
        put(bytes, data as usize + 4, &[1]);
        u32_at(bytes, data as usize + 8, key);
        u64_at(bytes, data as usize + 16, value);
    }
    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0xc000];
        array(&mut bytes, BASE + cgame_state::COUNTRIES, 0x2000, 3);
        array(&mut bytes, BASE + cgame_state::COUNTRY_LINKS, 0x2100, 100);
        array(&mut bytes, BASE + cgame_state::STATES, 0x2400, 3);
        u64_at(&mut bytes, 0x2008, 0x3000);
        u64_at(&mut bytes, 0x2010, 0x5000);
        u32_at(&mut bytes, 0x2104, 1);
        u32_at(&mut bytes, 0x2108, 2);
        u32_at(&mut bytes, 0x2100 + 99 * 4, 2);
        for (country, id, status) in [(0x3000, 1, 0x7000), (0x5000, 99, 0x7200)] {
            u64_at(&mut bytes, country, ccountry::VTABLE_RVA.0 as u64);
            u32_at(&mut bytes, country + ccountry::ID, id);
            u64_at(
                &mut bytes,
                country + ccountry::OCCUPATION_STATUS,
                status as u64,
            );
            u64_at(&mut bytes, status, VTABLE_RVA.0 as u64);
            u64_at(
                &mut bytes,
                status + SECONDARY_BASE,
                SECONDARY_VTABLE_RVA.0 as u64,
            );
            u64_at(
                &mut bytes,
                status + ASSIGNED_MANPOWER,
                carmy_manpower_values::VTABLE_RVA.0 as u64,
            );
        }
        u64_at(&mut bytes, 0x7000 + GARRISON_TEMPLATE, 0xa100);
        u64_at(
            &mut bytes,
            0xa100,
            creferenced_division_template::VTABLE_RVA.0 as u64,
        );
        u64_at(
            &mut bytes,
            0xa100 + creferenced_division_template::TYPE,
            (7_u64 << 32) | 52,
        );
        u64_at(&mut bytes, 0x7000 + DEFAULT_LAW, 0xa000);
        u64_at(&mut bytes, 0xa000, coccupation_law::VTABLE_RVA.0 as u64);
        put(&mut bytes, 0xa000 + coccupation_law::NAME, b"civilian");
        u64_at(&mut bytes, 0xa000 + coccupation_law::NAME + 16, 8);
        u64_at(&mut bytes, 0xa000 + coccupation_law::NAME + 24, 15);
        array(&mut bytes, 0x7000 + ASSIGNED_MANPOWER + 8, 0xb000, 1);
        u32_at(&mut bytes, 0xb000, 99);
        put(&mut bytes, 0xb004, &(-17_i32).to_le_bytes());
        table(
            &mut bytes,
            0x7000 + OCCUPATION_DATA_TABLE,
            0x8000,
            2,
            0x8400,
        );
        u64_at(
            &mut bytes,
            0x8400,
            ccountry_occupation_data::VTABLE_RVA.0 as u64,
        );
        u64_at(
            &mut bytes,
            0x8400 + ccountry_occupation_data::PARENT_STATUS,
            0x7000,
        );
        u32_at(
            &mut bytes,
            0x8400 + ccountry_occupation_data::OCCUPIED_COUNTRY,
            99,
        );
        table(
            &mut bytes,
            0x8400 + ccountry_occupation_data::LAW_TABLE,
            0x8200,
            1,
            0xa000,
        );
        table(
            &mut bytes,
            0x8400 + ccountry_occupation_data::GARRISON_TABLE,
            0x8600,
            1,
            0x9000,
        );
        u64_at(&mut bytes, 0x2408, 0xb100);
        u64_at(&mut bytes, 0xb100, cstate::VTABLE_RVA.0 as u64);
        u32_at(&mut bytes, 0xb100 + cstate::ID, 1);
        u64_at(
            &mut bytes,
            0x9000,
            cstate_garrison_data::VTABLE_RVA.0 as u64,
        );
        u64_at(&mut bytes, 0x9000 + cstate_garrison_data::STATE, 0xb100);
        u64_at(
            &mut bytes,
            0x9000 + cstate_garrison_data::OCCUPATION_DATA,
            0x8400,
        );
        u64_at(
            &mut bytes,
            0x9000 + cstate_garrison_data::STRENGTH_RATIO,
            i64::MAX as u64,
        );
        u64_at(
            &mut bytes,
            0x9000 + cstate_garrison_data::GARRISON_REQUIRED,
            i64::MIN as u64,
        );
        put(
            &mut bytes,
            0x9000 + cstate_garrison_data::REQUIRED_MANPOWER,
            &i32::MAX.to_le_bytes(),
        );
        let manpower = 0x9000 + cstate_garrison_data::ASSIGNED_MANPOWER;
        u64_at(
            &mut bytes,
            manpower,
            carmy_manpower_values::VTABLE_RVA.0 as u64,
        );
        array(&mut bytes, manpower + 8, 0xb800, 2);
        u32_at(&mut bytes, 0xb800, 99);
        put(&mut bytes, 0xb804, &i32::MIN.to_le_bytes());
        u32_at(&mut bytes, 0xb808, 1);
        put(&mut bytes, 0xb80c, &i32::MAX.to_le_bytes());
        bytes
    }

    #[test]
    fn shared_graph_retains_aliases_literal_laws_all_manpower_and_native_widths() {
        let bytes = fixture();
        let mut settings = vec![Row::default(); 2];
        let mut laws = vec![LawRow::default(); 1];
        let mut garrisons = vec![cstate_garrison_data::Row::default(); 1];
        let mut manpower = vec![ManpowerRow::default(); 2];
        let mut reserve = vec![ReserveManpowerRow::default(); 1];
        let mut names = vec![0; 16];
        let result = walk(
            &SliceMemory {
                base: BASE,
                bytes: &bytes,
            },
            0,
            BASE,
            Buffers {
                settings: &mut settings,
                laws: &mut laws,
                group_laws: &mut vec![ccountry_occupation_data::GroupLawRow::default(); 2],
                garrisons: &mut garrisons,
                manpower: &mut manpower,
                reserve_manpower: &mut reserve,
                names: &mut names,
                equipment: &mut Vec::new(),
            },
        );
        assert_eq!(
            result,
            Walk {
                settings: Outcome::Rows { len: 2 },
                laws: Outcome::Rows { len: 1 },
                group_laws: Outcome::Rows { len: 1 },
                garrisons: Outcome::Rows { len: 1 },
                manpower: Outcome::Rows { len: 2 },
                reserve_manpower: Outcome::Rows { len: 1 },
                bytes_len: 16,
                required_bytes: 16,
                occupied_len: 0,
                required_occupied: 0,
                equipment_len: 0,
                required_equipment: 0,
                fallback_law: None,
            }
        );
        assert_eq!(names, *b"civiliancivilian");
        assert_eq!(
            (
                settings[0].garrison_template,
                settings[1].default_law_present
            ),
            (Some((7_u64 << 32) | 52), 0)
        );
        assert_eq!((laws[0].occupied, garrisons[0].occupied), (99, 99));
        assert_eq!(garrisons[0].equipment, None);
        assert_eq!(
            garrisons[0].equipment_failure,
            Some("garrison required equipment pool vtable mismatch")
        );
        assert_eq!(
            (garrisons[0].strength_ratio, garrisons[0].garrison_required),
            (i64::MAX, i64::MIN)
        );
        assert_eq!(
            (
                manpower[0].source_country,
                manpower[0].amount,
                manpower[1].amount
            ),
            (99, i32::MIN, i32::MAX)
        );
        assert_eq!(
            (
                reserve[0].country,
                reserve[0].source_country,
                reserve[0].amount
            ),
            (1, 99, -17)
        );
    }

    #[test]
    fn law_faults_are_scoped_and_raw_zero_state_law_keys_remain_visible() {
        let mut bytes = fixture();
        u32_at(&mut bytes, 0x8208, 0);
        let mut settings = vec![Row::default(); 2];
        let mut laws = vec![LawRow::default(); 1];
        let mut garrisons = vec![cstate_garrison_data::Row::default(); 1];
        let mut manpower = vec![ManpowerRow::default(); 2];
        let mut reserve = vec![ReserveManpowerRow::default(); 1];
        let mut names = vec![0; 16];
        let result = walk(
            &SliceMemory {
                base: BASE,
                bytes: &bytes,
            },
            0,
            BASE,
            Buffers {
                settings: &mut settings,
                laws: &mut laws,
                group_laws: &mut vec![ccountry_occupation_data::GroupLawRow::default(); 2],
                garrisons: &mut garrisons,
                manpower: &mut manpower,
                reserve_manpower: &mut reserve,
                names: &mut names,
                equipment: &mut Vec::new(),
            },
        );
        assert_eq!(result.laws, Outcome::Rows { len: 1 });
        assert_eq!(laws[0].state, 0);
        u64_at(&mut bytes, 0xa000, 123);
        let result = walk(
            &SliceMemory {
                base: BASE,
                bytes: &bytes,
            },
            0,
            BASE,
            Buffers {
                settings: &mut settings,
                laws: &mut laws,
                group_laws: &mut vec![ccountry_occupation_data::GroupLawRow::default(); 2],
                garrisons: &mut garrisons,
                manpower: &mut manpower,
                reserve_manpower: &mut reserve,
                names: &mut names,
                equipment: &mut Vec::new(),
            },
        );
        assert!(matches!(result.settings, Outcome::Unavailable { .. }));
        assert!(matches!(result.laws, Outcome::Unavailable { .. }));
        assert_eq!(result.garrisons, Outcome::Rows { len: 1 });
        assert_eq!(result.manpower, Outcome::Rows { len: 2 });
        assert_eq!(result.reserve_manpower, Outcome::Rows { len: 1 });
    }

    #[test]
    fn identical_raw_law_state_keys_in_distinct_occupied_scopes_are_independent() {
        let mut bytes = fixture();
        u32_at(&mut bytes, 0x8208, 0);
        u32_at(&mut bytes, 0x7000 + OCCUPATION_DATA_TABLE + 8, 2);
        u32_at(&mut bytes, 0x7000 + OCCUPATION_DATA_TABLE + 12, 2);
        put(&mut bytes, 0x8018 + 4, &[1]);
        u32_at(&mut bytes, 0x8018 + 8, 1);
        u64_at(&mut bytes, 0x8018 + 16, 0x8800);
        u64_at(
            &mut bytes,
            0x8800,
            ccountry_occupation_data::VTABLE_RVA.0 as u64,
        );
        u64_at(
            &mut bytes,
            0x8800 + ccountry_occupation_data::PARENT_STATUS,
            0x7000,
        );
        u32_at(
            &mut bytes,
            0x8800 + ccountry_occupation_data::OCCUPIED_COUNTRY,
            1,
        );
        table(
            &mut bytes,
            0x8800 + ccountry_occupation_data::LAW_TABLE,
            0x8a00,
            0,
            0xa000,
        );
        let mut settings = vec![Row::default(); 2];
        let mut laws = vec![LawRow::default(); 2];
        let mut garrisons = vec![cstate_garrison_data::Row::default(); 1];
        let mut manpower = vec![ManpowerRow::default(); 2];
        let mut reserve = vec![ReserveManpowerRow::default(); 1];
        let mut names = vec![0; 24];
        let result = walk(
            &SliceMemory {
                base: BASE,
                bytes: &bytes,
            },
            0,
            BASE,
            Buffers {
                settings: &mut settings,
                laws: &mut laws,
                group_laws: &mut vec![ccountry_occupation_data::GroupLawRow::default(); 2],
                garrisons: &mut garrisons,
                manpower: &mut manpower,
                reserve_manpower: &mut reserve,
                names: &mut names,
                equipment: &mut Vec::new(),
            },
        );
        assert_eq!(result.laws, Outcome::Rows { len: 2 });
        assert_eq!(
            laws.iter().map(|row| (row.controller, row.occupied, row.state)).collect::<Vec<_>>(),
            [(1, 1, 0), (1, 99, 0)]
        );
    }
}
