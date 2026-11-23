use super::*;

pub fn walk_country_buffer<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    country: engine_native_memory::country_identity::Country,
    context: Result<Context, &'static str>,
    inputs: &[research_inputs::Row],
    output: engine_native_memory::walk::Buffer<'_, Row>,
) -> Outcome {
    if !inputs.iter().any(|r| r.slot.technology_present != 0) {
        return Outcome::Rows { len: 0 };
    }
    let mut out = Output {
        rows: output,
        len: 0,
        omitted: 0,
        current: Row {
            country: country.id,
            index: u32::MAX,
            ..Row::default()
        },
    };
    let result = (|| {
        let context = context?;
        let accumulator = at(country.pointer, engine_native_memory::country_identity::MODIFIER_ACCUMULATOR)?;
        let (raw, metadata) = modifier(
            memory,
            base,
            accumulator,
            crate::modifier_keys::RESEARCH_SPEED_FACTOR,
        )?;
        out.modified(Kind::CountrySpeed, raw, metadata, 0, 0, Text::default())?;
        out.value(Kind::AheadFactor, context.ahead_factor)?;
        out.value(Kind::MinimumSpeed, context.minimum)?;
        out.value(Kind::SharingMaximum, context.sharing_maximum)?;
        country_sources(memory, base, accumulator, &mut out)?;

        let status = inputs
            .iter()
            .find(|r| r.slot.technology_present != 0)
            .unwrap()
            .status_pointer;
        let sharing = sharing::Country::read(memory, base, countries, country, status)?;
        let (raw, metadata) = if sharing.is_empty() {
            (0, None)
        } else {
            modifier(
                memory,
                base,
                accumulator,
                crate::modifier_keys::RESEARCH_SHARING_PER_COUNTRY_BONUS_FACTOR,
            )?
        };
        out.modified(
            Kind::SharingMultiplier,
            raw,
            metadata,
            0,
            0,
            Text::default(),
        )?;
        let mut witnesses = sharing::Witnesses::default();
        let mut licenses = None;
        let mut scientist = None;
        for input in inputs.iter().filter(|r| r.slot.technology_present != 0) {
            if input.status_pointer != status {
                return Err("research modifier owner changed");
            }
            out.current = Row {
                country: country.id,
                index: input.slot.index,
                technology_handle: input.slot.technology_handle,
                ..Row::default()
            };
            out.value(Kind::Technology, i64::from(input.slot.technology_handle))?;
            out.value(Kind::Year, i64::from(input.template_year))?;
            let tech = input.technology_pointer;
            let mut cached = [0; 24];
            let mut xp = [0; 12];
            let mut equipment = [0; 16];
            let mut lists = [0; 64];
            if memory.copy_scattered_prefetched(
                [
                    at(tech, 0x190)?,
                    at(tech, 0x1e8)?,
                    at(tech, 0x68)?,
                    at(tech, 0x110)?,
                ],
                &mut [
                    &mut cached[..],
                    &mut xp[..],
                    &mut equipment[..],
                    &mut lists[..],
                ],
            ) != 4
            {
                return Err("research modifier operands unreadable");
            }
            out.value(Kind::CategorySpeed, i64_at(&cached, 0))?;
            out.push(
                Kind::DesignTeam,
                i64_at(&cached, 16),
                i64_at(&xp, 4),
                u32::from_le_bytes(xp[4..8].try_into().unwrap()),
                u32::from_le_bytes(xp[8..12].try_into().unwrap()),
                Text::default(),
            )?;
            out.push(
                Kind::Experience,
                if xp[0] != 0 {
                    read::<i64>(memory, at(input.template_pointer, 0x538)?)
                        .ok_or("research XP modifier unreadable")?
                } else {
                    0
                },
                i64::from(xp[0]),
                0,
                0,
                Text::default(),
            )?;
            let ideas = array(&lists[..16], 8)?;
            let scripts = array(&lists[24..40], 40)?;
            let categories = array(&lists[48..64], 4)?;
            pointer_pages(memory, ideas, |pointer| {
                let bytes = read::<[u8; 16]>(memory, at(pointer, 0x38)?)
                    .ok_or("research idea contribution unreadable")?;
                let raw = i64_at(&bytes, 0);
                if raw != 0 {
                    let idea = pointer_from(&bytes[8..])?;
                    if read::<u64>(memory, idea)
                        != Some(site(base, engine_native_parts::cidea::VTABLE_RVA)? as u64)
                    {
                        return Err("research idea identity mismatch");
                    }
                    out.push(
                        Kind::IdeaSource,
                        raw,
                        0,
                        0,
                        0,
                        text(memory, at(idea, engine_native_parts::cidea::TOKEN)?)?,
                    )?;
                }
                Ok(())
            })?;
            for index in 0..scripts.count {
                let entry = scripts
                    .element(index, 40)
                    .ok_or("research script address overflow")?;
                let source = pointer(memory, entry)?;
                let raw = read::<i64>(memory, at(source, 0x30)?)
                    .ok_or("research script contribution unreadable")?;
                if raw != 0 {
                    out.push(
                        Kind::ScriptSource,
                        raw,
                        0,
                        0,
                        0,
                        text(memory, at(entry, 8)?)?,
                    )?;
                }
            }
            let equipment = array(&equipment, 8)?;
            let license = if equipment.count == 0 {
                false
            } else {
                let roster = match licenses {
                    Some(roster) => roster,
                    None => {
                        let production =
                            pointer(memory, at(country.pointer, ccountry::PRODUCTION_STATUS)?)?;
                        let roster = array_at(memory, at(production, 0x1b0)?, 8)?;
                        licenses = Some(roster);
                        roster
                    }
                };
                licensed(memory, equipment, roster)?
            };
            out.value(Kind::License, if license { context.license } else { 0 })?;
            scientist_bonus(
                memory,
                base,
                country,
                countries,
                categories,
                &mut scientist,
                &mut out,
            )?;
            sharing.capture(memory, base, countries, input, &mut witnesses, &mut out)?;
        }
        Ok(())
    })();
    match result {
        Err(reason) => Outcome::Unavailable { reason },
        Ok(()) if out.omitted != 0 => Outcome::Truncated {
            len: out.len,
            unread_rows: out.omitted,
        },
        Ok(()) => Outcome::Rows { len: out.len },
    }
}
