use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn walk_projected(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    slots: std::ops::Range<u32>,
    mut output: Buffer<'_, Row>,
    mut sources: Option<Buffer<'_, Source>>,
    naval: bool,
    mut visit: impl FnMut(usize, usize, &mut Row, Admitted),
    mut owner_visit: impl FnMut(u32, Option<usize>),
    scored: bool,
    mut score_visit: impl FnMut(usize, &ScoreLeaf),
) -> GameScoreWalk {
    let mut map = Projection::new(output.len());
    let mut source = sources.as_ref().map(|rows| Projection::new(rows.len()));
    let mut naval = naval.then(|| Projection::new(usize::MAX));
    let mut score = scored.then(|| Projection::new(usize::MAX));
    let result = (|| {
        let reader = Reader {
            memory,
            base,
            regions: RegionMemo::new(),
        };
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let manager = reader.pointer(game_state, cgame_state::STRATEGIC_AIR_MANAGER)?;
        if manager == 0 {
            if slots.start == 0 {
                for slot in 0..countries.count() {
                    owner_visit(slot, None);
                }
            }
            return Err("strategic air manager absent");
        }
        let vtable =
            read::<u64>(memory, manager).ok_or("strategic air manager vtable unreadable")?;
        let expected = engine_sites::at(base, cstrategic_air_manager::VTABLE_RVA)
            .ok_or("module address overflow")?;
        if vtable != expected as u64 {
            return Err("strategic air manager vtable mismatch");
        }
        let owners = reader.array(manager, cstrategic_air_manager::COUNTRY_AIR)?;
        if owners.count == 0 {
            if slots.start == 0 {
                for slot in 0..countries.count() {
                    owner_visit(slot, None);
                }
            }
            return Err("strategic air manager has no country slots");
        }
        for slot in slots.start..slots.end.min(owners.count) {
            let owner_pointer = reader.element(owners, slot)?;
            if owner_pointer == 0 {
                owner_visit(slot, None);
                continue;
            }
            reader.vtable(
                owner_pointer,
                cstrategic_air::VTABLE_RVA,
                "strategic air country vtable mismatch",
            )?;
            let owner = reader.field::<u32>(owner_pointer, cstrategic_air::COUNTRY)?;
            let owner_country = if slot == 0 && score.is_some() {
                let country =
                    crate::game_score_inputs::read_slot_zero(memory, base, countries.arrays()[0])?
                        .ok_or("strategic air slot zero owner has no catalogue object")?;
                if country.id != owner {
                    return Err("strategic air slot zero owner differs from catalogue object");
                }
                country
            } else {
                countries.resolve(owner)?
            };
            let owner_slot = owner_country.canonical_slot;
            if owner_slot != slot {
                return Err("strategic air owner differs from canonical country slot");
            }
            owner_visit(slot, Some(owner_pointer));
            let pools = reader.array(owner_pointer, cstrategic_air::POOLS)?;
            let mut pool_ordinal = 0_u32;
            for pool_index in 0..pools.count {
                let pool = reader.element(pools, pool_index)?;
                reader.vtable(
                    pool,
                    cair_wing_pool::VTABLE_RVA,
                    "air wing pool vtable mismatch",
                )?;
                if reader.pointer(pool, cair_wing_pool::OWNER)? != owner_pointer {
                    return Err("air wing pool owner differs from roster");
                }
                let wings = reader.array(pool, cair_wing_pool::WINGS)?;
                let required = map
                    .len
                    .checked_add(wings.count as usize)
                    .ok_or("air wing roster demand overflow")?;
                output.ensure(required)?;
                map.capacity = output.len();
                if let (Some(source), Some(sources)) = (&mut source, &mut sources) {
                    let required = source
                        .len
                        .checked_add(wings.count as usize)
                        .ok_or("air wing source demand overflow")?;
                    sources.ensure(required)?;
                    source.capacity = sources.len();
                }
                let air_base = match reader.field::<u64>(pool, cair_wing_pool::AIR_BASE_REFERENCE) {
                    Ok(value) => value,
                    Err(reason) => {
                        map.fail(reason);
                        if let Some(source) = &mut source {
                            source.fail(reason);
                        }
                        if let Some(naval) = &mut naval {
                            naval.fail(reason);
                        }
                        if score.is_none() {
                            return Err(reason);
                        }
                        0
                    }
                };
                memory.prefetch(wings.data, (wings.count as usize).saturating_mul(8));
                for wing_index in 0..wings.count {
                    let wing = reader.element(wings, wing_index)?;
                    if wing == 0 {
                        return Err("air wing roster contains null wing");
                    }
                    hint_wing(memory, wing);
                    let ordinal = pool_ordinal;
                    pool_ordinal = pool_ordinal
                        .checked_add(1)
                        .ok_or("air wing pool ordinal overflow")?;
                    let map_needs = map.needs_row();
                    let source_needs = source.as_ref().is_some_and(Projection::needs_row);
                    let naval_needs = naval.as_ref().is_some_and(Projection::needs_row);
                    let score_needs = score.as_ref().is_some_and(Projection::needs_row);
                    if !map_needs {
                        map.omit();
                    }
                    if let Some(source) = &mut source
                        && !source_needs
                    {
                        source.omit();
                    }
                    if let Some(naval) = &mut naval
                        && !naval_needs
                    {
                        naval.omit();
                    }
                    if let Some(score) = &mut score
                        && !score_needs
                    {
                        score.omit();
                    }
                    let details = map_needs || source_needs || naval_needs;
                    if details || score_needs {
                        match reader.row(
                            &reader.row_fields(wing, details),
                            wing,
                            pool,
                            owner,
                            ordinal,
                            air_base,
                            details,
                            |leaf| {
                                if let Some(score) = score.as_mut().filter(|_| score_needs) {
                                    match leaf {
                                        Ok(leaf) => {
                                            score_visit(wing, &leaf);
                                            score.len += 1;
                                        }
                                        Err(reason) => score.fail(reason),
                                    }
                                }
                            },
                        ) {
                            Ok(Some(mut row)) => {
                                row.owner_pointer = owner_country.pointer;
                                row.owner_slot = owner_slot;
                                visit(
                                    wing,
                                    pool,
                                    &mut row,
                                    Admitted {
                                        map: map_needs,
                                        sources: source_needs,
                                        naval: naval_needs,
                                        name_header: None,
                                    },
                                );
                                if map_needs {
                                    output[map.len] = row;
                                    map.len += 1;
                                }
                                if let Some(source) = source.as_mut().filter(|_| source_needs) {
                                    if let Some(destination) = sources
                                        .as_deref_mut()
                                        .and_then(|rows| rows.get_mut(source.len))
                                    {
                                        *destination = Source {
                                            pointer: wing,
                                            pool,
                                            row,
                                        };
                                        source.len += 1;
                                    } else {
                                        source.fail("air wing source projection storage missing");
                                    }
                                }
                                if let Some(naval) = naval.as_mut().filter(|_| naval_needs) {
                                    naval.len += 1;
                                }
                            }
                            Ok(None) => {}
                            Err(reason) => {
                                if map_needs {
                                    map.fail(reason);
                                }
                                if let Some(source) = source.as_mut().filter(|_| source_needs) {
                                    source.fail(reason);
                                }
                                if let Some(naval) = naval.as_mut().filter(|_| naval_needs) {
                                    naval.fail(reason);
                                }
                            }
                        }
                    }
                    if map.failure.is_some()
                        && source
                            .as_ref()
                            .is_none_or(|source| source.failure.is_some())
                        && naval.as_ref().is_none_or(|naval| naval.failure.is_some())
                        && score.as_ref().is_none_or(|score| score.failure.is_some())
                    {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    })();
    let ended = result.unwrap_or_else(|reason| {
        map.fail(reason);
        if let Some(source) = &mut source {
            source.fail(reason);
        }
        if let Some(naval) = &mut naval {
            naval.fail(reason);
        }
        if let Some(score) = &mut score {
            score.fail(reason);
        }
        true
    });
    GameScoreWalk {
        shared: SharedWalk {
            map: map.outcome(),
            sources: source.map_or(Outcome::Rows { len: 0 }, |source| source.outcome()),
            naval: naval.map_or(Outcome::Rows { len: 0 }, |naval| naval.outcome()),
        },
        score: score.map_or(Outcome::Rows { len: 0 }, |score| score.outcome()),
        ended,
        row_fallbacks: 0,
    }
}
