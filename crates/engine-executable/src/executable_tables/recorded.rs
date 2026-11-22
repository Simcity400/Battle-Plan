
use std::io;

use crate::availability::{Verified, Withheld};
use crate::build::ExecutableIdentity;

use super::{
    CURSORS, ExecutableTables, FOLDING, auxiliary, commands, cursors, folding, keywords,
    leak_string, numeric, registries, variables,
};

const MAGIC: &[u8; 4] = b"H4XT";
const VERSION: u16 = 3;

pub fn encode(tables: &ExecutableTables) -> Vec<u8> {
    let mut out = Out(Vec::with_capacity(1 << 20));
    out.0.extend_from_slice(MAGIC);
    out.0.extend_from_slice(&VERSION.to_le_bytes());
    out.u64(tables.identity.size);
    out.bytes(&tables.identity.sha256);
    out.verified(&tables.version, |out, version| out.str(version));
    out.verified(&tables.keywords, keywords_out);
    out.verified(&tables.registries, registries_out);
    out.verified(&tables.auxiliary, auxiliary_out);
    out.verified(&tables.variables, variables_out);
    out.verified(&tables.numeric, numeric_out);
    out.verified(&tables.commands, commands_out);
    out.verified(&tables.folding, |out, table| {
        let letters: Vec<_> = table.letters.iter().collect();
        out.list(&letters, |out, (point, letter)| {
            out.u32(**point);
            out.u8(**letter);
        });
    });
    out.verified(&tables.cursors, |out, table| {
        out.list(&table.slots, |out, slot| match slot {
            cursors::CursorSlot::Path(path) => {
                out.u8(0);
                out.str(path);
            }
            cursors::CursorSlot::Unusable(reason) => {
                out.u8(1);
                out.str(reason);
            }
        });
        let bindings: Vec<_> = table
            .bindings
            .iter()
            .filter_map(|(&action, entry)| entry.as_ref().ok().map(|&slot| (action, slot)))
            .collect();
        out.list(&bindings, |out, (action, slot)| {
            out.u8(*action as u8);
            out.u32(*slot);
        });
    });
    out.0
}

pub fn decode(bytes: &[u8]) -> io::Result<ExecutableTables> {
    let mut input = In(bytes);
    if input.take(4)? != MAGIC {
        return Err(invalid("not recorded executable tables"));
    }
    let version = u16::from_le_bytes(input.take(2)?.try_into().unwrap());
    if !(1..=VERSION).contains(&version) {
        return Err(invalid(format!(
            "recorded executable tables version {version} is not one this build reads (1 to {VERSION})"
        )));
    }
    let identity = ExecutableIdentity {
        size: input.u64()?,
        sha256: input.take(32)?.try_into().unwrap(),
    };
    let tables = ExecutableTables {
        identity,
        version: input.verified(|input| input.string())?,
        keywords: input.verified(keywords_in)?,
        registries: input.verified(registries_in)?,
        auxiliary: input.verified(auxiliary_in)?,
        variables: input.verified(variables_in)?,
        numeric: input.verified(numeric_in)?,
        commands: input.verified(commands_in)?,
        folding: if version < 2 {
            Err(Withheld::new(FOLDING, "not recorded in this recording"))
        } else {
            input.verified(|input| {
                folding::FoldingTable::from_pairs(
                    input
                        .list(|input| Ok((input.u32()?, u32::from(input.u8()?))))?
                        .into_iter(),
                )
            })?
        },
        cursors: if version < 3 {
            Err(Withheld::new(CURSORS, "not recorded in this recording"))
        } else {
            input.verified(|input| {
                let slots = input.list(|input| match input.u8()? {
                    0 => Ok(cursors::CursorSlot::validated(input.string()?)),
                    1 => Ok(cursors::CursorSlot::Unusable(input.string()?)),
                    _ => Err(invalid("unknown recorded cursor slot tag")),
                })?;
                let bindings = input.list(|input| {
                    let action = cursors::CursorAction::from_u8(input.u8()?)
                        .ok_or_else(|| invalid("unknown recorded cursor action"))?;
                    Ok((action, input.u32()?))
                })?;
                Ok(cursors::CursorTable::from_slots(slots, bindings))
            })?
        },
    };
    if !input.0.is_empty() {
        return Err(invalid("recorded executable tables have trailing bytes"));
    }
    Ok(tables)
}

fn keywords_out(out: &mut Out, table: &keywords::KeywordTables) {
    out.list(&table.names, |out, (id, name)| {
        out.u32(*id);
        out.str(name);
    });
    out.list(&table.modifier_names, |out, name| out.str(name));
}

fn keywords_in(input: &mut In) -> io::Result<keywords::KeywordTables> {
    let names = input.list(|input| Ok((input.u32()?, input.leaked()?)))?;
    let modifier_names = input.list(In::leaked)?;
    Ok(keywords::KeywordTables::recorded(names, modifier_names))
}

fn trigger_reader_out(out: &mut Out, reader: &registries::TriggerReader) {
    match reader {
        registries::TriggerReader::Assign => out.u8(0),
        registries::TriggerReader::Comparison => out.u8(1),
        registries::TriggerReader::Body => out.u8(2),
        registries::TriggerReader::Own(identity) => {
            out.u8(3);
            out.u64(*identity);
        }
    }
}

fn registries_out(out: &mut Out, table: &registries::RegistryTables) {
    out.list(&table.triggers, |out, (id, name)| {
        out.u32(*id);
        out.str(name);
    });
    out.list(&table.trigger_classes, |out, (id, mask, reader)| {
        out.u32(*id);
        out.u32(*mask);
        trigger_reader_out(out, reader);
    });
    out.list(&table.trigger_key_readers, |out, (id, reader)| {
        out.u32(*id);
        out.u64(*reader);
    });
    out.list(&table.effects, |out, (id, name)| {
        out.u32(*id);
        out.str(name);
    });
    out.list(&table.effect_classes, |out, (id, value, key)| {
        out.u32(*id);
        out.u64(*value);
        out.u64(*key);
    });
    out.list(&table.effect_identities, |out, (id, name, identity)| {
        out.u32(*id);
        out.str(name);
        out.u64(*identity);
    });
    let context: Vec<_> = table.context_effect_names.iter().collect();
    out.list(&context, |out, (identity, name)| {
        out.u64(**identity);
        out.str(name);
    });
    let unmatched: Vec<_> = table.unmatched.iter().collect();
    out.list(&unmatched, |out, (identity, stand_in)| {
        out.u64(**identity);
        out.u64(**stand_in);
    });
}

fn registries_in(input: &mut In) -> io::Result<registries::RegistryTables> {
    let mut table = registries::RegistryTables {
        triggers: input.list(|input| Ok((input.u32()?, input.leaked()?)))?,
        trigger_classes: input.list(|input| {
            let id = input.u32()?;
            let mask = input.u32()?;
            let reader = match input.u8()? {
                0 => registries::TriggerReader::Assign,
                1 => registries::TriggerReader::Comparison,
                2 => registries::TriggerReader::Body,
                3 => registries::TriggerReader::Own(input.u64()?),
                _ => return Err(invalid("unknown recorded trigger reader")),
            };
            Ok((id, mask, reader))
        })?,
        trigger_key_readers: input.list(|input| Ok((input.u32()?, input.u64()?)))?,
        effects: input.list(|input| Ok((input.u32()?, input.leaked()?)))?,
        effect_classes: input.list(|input| Ok((input.u32()?, input.u64()?, input.u64()?)))?,
        effect_identities: input
            .list(|input| Ok((input.u32()?, input.leaked()?, input.u64()?)))?,
        ..Default::default()
    };
    table.context_effect_names = input
        .list(|input| Ok((input.u64()?, input.leaked()?)))?
        .into_iter()
        .collect();
    table.unmatched = input
        .list(|input| Ok((input.u64()?, input.u64()?)))?
        .into_iter()
        .collect();
    Ok(table)
}

fn auxiliary_out(out: &mut Out, table: &auxiliary::AuxiliaryTables) {
    out.verified(&table.equipment_categories, |out, rows| {
        out.list(rows, |out, (id, mask, name)| {
            out.u32(*id);
            out.u64(*mask);
            out.str(name);
        })
    });
    out.verified(&table.category_names, |out, rows| {
        out.list(rows, |out, (name, mask)| {
            out.str(name);
            out.u64(*mask);
        })
    });
    out.verified(&table.stat_metadata, |out, rows| {
        out.list(rows, |out, (name, kind, flag)| {
            out.str(name);
            out.str(kind);
            out.u8(u8::from(*flag));
        })
    });
    for part in [
        &table.ai_strategy_types,
        &table.missions,
        &table.equipment_missions,
        &table.execution_types,
        &table.order_types,
        &table.selectors,
    ] {
        out.verified(part, |out, rows| {
            out.list(rows, |out, (name, value)| {
                out.str(name);
                out.u32(*value);
            })
        });
    }
    out.verified(&table.rules, |out, rows| {
        out.list(rows, |out, (name, group, fulfilled, default)| {
            out.str(name);
            out.str(group);
            out.u8(u8::from(*fulfilled));
            out.u8(u8::from(*default));
        })
    });
    out.verified(&table.air_graph_channels, |out, channels| {
        channels.iter().for_each(|channel| out.u32(*channel))
    });
    out.verified(&table.air_graph_colors, |out, colors| {
        colors
            .iter()
            .flatten()
            .for_each(|part| out.u32(part.to_bits()))
    });
    out.verified(&table.air_graph_scale, |out, scale| out.u32(scale.to_bits()));
    out.verified(&table.air_detection_override_prefix, |out, prefix| {
        out.str(prefix)
    });
}

fn auxiliary_in(input: &mut In) -> io::Result<auxiliary::AuxiliaryTables> {
    let named = |input: &mut In| {
        input.verified(|input| input.list(|input| Ok((input.leaked()?, input.u32()?))))
    };
    Ok(auxiliary::AuxiliaryTables {
        equipment_categories: input.verified(|input| {
            input.list(|input| Ok((input.u32()?, input.u64()?, input.leaked()?)))
        })?,
        category_names: input
            .verified(|input| input.list(|input| Ok((input.leaked()?, input.u64()?))))?,
        stat_metadata: input.verified(|input| {
            input.list(|input| Ok((input.leaked()?, input.leaked()?, input.u8()? != 0)))
        })?,
        ai_strategy_types: named(input)?,
        missions: named(input)?,
        equipment_missions: named(input)?,
        execution_types: named(input)?,
        order_types: named(input)?,
        selectors: named(input)?,
        rules: input.verified(|input| {
            input.list(|input| {
                Ok((
                    input.leaked()?,
                    input.leaked()?,
                    input.u8()? != 0,
                    input.u8()? != 0,
                ))
            })
        })?,
        air_graph_channels: input.verified(|input| {
            let mut channels = [0; auxiliary::AIR_GRAPH_DISPLAYS];
            for channel in &mut channels {
                *channel = input.u32()?;
            }
            Ok(channels)
        })?,
        air_graph_colors: input.verified(|input| {
            let mut colors = [[0.0; 4]; auxiliary::AIR_GRAPH_DISPLAYS];
            for part in colors.iter_mut().flatten() {
                *part = f32::from_bits(input.u32()?);
            }
            Ok(colors)
        })?,
        air_graph_scale: input.verified(|input| Ok(f32::from_bits(input.u32()?)))?,
        air_detection_override_prefix: input.verified(In::leaked)?,
    })
}

fn variables_out(out: &mut Out, table: &variables::VariableTables) {
    for names in [
        &table.global,
        &table.country,
        &table.state,
        &table.unit_leader,
        &table.industrial_organisation,
        &table.project,
    ] {
        out.list(names, |out, name| out.str(name));
    }
}

fn variables_in(input: &mut In) -> io::Result<variables::VariableTables> {
    Ok(variables::VariableTables {
        global: input.list(In::leaked)?,
        country: input.list(In::leaked)?,
        state: input.list(In::leaked)?,
        unit_leader: input.list(In::leaked)?,
        industrial_organisation: input.list(In::leaked)?,
        project: input.list(In::leaked)?,
    })
}

fn numeric_out(out: &mut Out, table: &numeric::NumericTables) {
    out.list(&table.properties, |out, property| {
        out.str(property.name);
        match property.identity {
            numeric::NumericIdentity::Scope {
                scope,
                ordinal,
                handler_rva,
            } => {
                out.u8(0);
                out.u8(scope as u8);
                out.u32(ordinal);
                out.u32(handler_rva);
            }
            numeric::NumericIdentity::Callback {
                vtable_rva,
                value_reader_rva,
                input_reader_rva,
            } => {
                out.u8(1);
                out.u32(vtable_rva);
                out.u32(value_reader_rva);
                out.u32(input_reader_rva);
            }
        }
    });
}

fn numeric_in(input: &mut In) -> io::Result<numeric::NumericTables> {
    Ok(numeric::NumericTables {
        properties: input.list(|input| {
            let name = input.leaked()?;
            let identity = match input.u8()? {
                0 => numeric::NumericIdentity::Scope {
                    scope: match input.u8()? {
                        0 => numeric::NumericScope::Country,
                        1 => numeric::NumericScope::State,
                        2 => numeric::NumericScope::UnitLeader,
                        3 => numeric::NumericScope::Character,
                        _ => return Err(invalid("unknown recorded numeric scope")),
                    },
                    ordinal: input.u32()?,
                    handler_rva: input.u32()?,
                },
                1 => numeric::NumericIdentity::Callback {
                    vtable_rva: input.u32()?,
                    value_reader_rva: input.u32()?,
                    input_reader_rva: input.u32()?,
                },
                _ => return Err(invalid("unknown recorded numeric identity")),
            };
            Ok(numeric::NumericProperty { name, identity })
        })?,
    })
}

fn source_out(out: &mut Out, source: &commands::Source) {
    use commands::Source;
    match *source {
        Source::Country(offset) => {
            out.u8(0);
            out.u32(offset);
        }
        Source::Reference(offset, object) => {
            out.u8(1);
            out.u32(offset);
            out.u8(object as u8);
        }
        Source::References(offset, object) => {
            out.u8(2);
            out.u32(offset);
            out.u8(object as u8);
        }
        Source::Pointer(offset, object) => {
            out.u8(3);
            out.u32(offset);
            out.u8(object as u8);
        }
        Source::Province(offset) => {
            out.u8(4);
            out.u32(offset);
        }
        Source::Building { definition, state } => {
            out.u8(5);
            out.u32(definition);
            out.u32(state);
        }
        Source::NavalSide { combat, side } => {
            out.u8(6);
            out.u32(combat);
            out.u32(side);
        }
        Source::ScriptCountry { pointer, length } => {
            out.u8(7);
            out.u32(pointer);
            out.u32(length);
        }
    }
}

fn source_in(input: &mut In) -> io::Result<commands::Source> {
    use commands::Source;
    let object = |input: &mut In| {
        let byte = input.u8()?;
        commands::Object::ALL
            .get(usize::from(byte))
            .copied()
            .ok_or_else(|| invalid("unknown recorded command object"))
    };
    Ok(match input.u8()? {
        0 => Source::Country(input.u32()?),
        1 => Source::Reference(input.u32()?, object(input)?),
        2 => Source::References(input.u32()?, object(input)?),
        3 => Source::Pointer(input.u32()?, object(input)?),
        4 => Source::Province(input.u32()?),
        5 => Source::Building {
            definition: input.u32()?,
            state: input.u32()?,
        },
        6 => Source::NavalSide {
            combat: input.u32()?,
            side: input.u32()?,
        },
        7 => Source::ScriptCountry {
            pointer: input.u32()?,
            length: input.u32()?,
        },
        _ => return Err(invalid("unknown recorded command source")),
    })
}

fn commands_out(out: &mut Out, table: &commands::CommandTables) {
    use commands::{Target, Token};
    out.list(&table.kinds, |out, kind| {
        out.u32(kind.vtable);
        out.str(&kind.name);
        match kind.token {
            Token::Constant(value) => {
                out.u8(0);
                out.u32(value);
            }
            Token::Word(value) => {
                out.u8(1);
                out.u32(value);
            }
        }
        let (tag, offset) = match kind.target {
            Target::Unknown => (0, 0),
            Target::Country(offset) => (1, offset),
            Target::Group(offset) => (2, offset),
            Target::Unit(offset) => (3, offset),
            Target::Units(offset) => (4, offset),
            Target::Wings(offset) => (5, offset),
            Target::Sources(_) => (6, 0),
        };
        out.u8(tag);
        out.u32(offset);
        if let Target::Sources(sources) = kind.target {
            out.list(sources, source_out);
        }
    });
    out.list(&table.objects, |out, (vtable, mask)| {
        out.u32(*vtable);
        out.u32(*mask);
    });
}

fn commands_in(input: &mut In) -> io::Result<commands::CommandTables> {
    use commands::{Target, Token};
    let kinds = input.list(|input| {
        let vtable = input.u32()?;
        let name = input.string()?;
        let token = match input.u8()? {
            0 => Token::Constant(input.u32()?),
            1 => Token::Word(input.u32()?),
            _ => return Err(invalid("unknown recorded command token")),
        };
        let tag = input.u8()?;
        let offset = input.u32()?;
        let target = match tag {
            0 => Target::Unknown,
            1 => Target::Country(offset),
            2 => Target::Group(offset),
            3 => Target::Unit(offset),
            4 => Target::Units(offset),
            5 => Target::Wings(offset),
            6 => Target::Sources(Box::leak(input.list(source_in)?.into_boxed_slice())),
            _ => return Err(invalid("unknown recorded command target")),
        };
        Ok(commands::Kind {
            vtable,
            name,
            token,
            target,
        })
    })?;
    let objects = input.list(|input| Ok((input.u32()?, input.u32()?)))?;
    Ok(commands::CommandTables::recorded(kinds, objects))
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

struct Out(Vec<u8>);

impl Out {
    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }

    fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }

    fn str(&mut self, text: &str) {
        self.u32(text.len() as u32);
        self.0.extend_from_slice(text.as_bytes());
    }

    fn list<T>(&mut self, items: &[T], mut item: impl FnMut(&mut Self, &T)) {
        self.u32(items.len() as u32);
        for value in items {
            item(self, value);
        }
    }


    fn verified<T>(&mut self, part: &Verified<T>, value: impl FnOnce(&mut Self, &T)) {
        match part {
            Ok(part) => {
                self.u8(0);
                value(self, part);
            }
            Err(withheld) => {
                self.u8(1);
                self.str(&withheld.part);
                self.str(&withheld.reason);
            }
        }
    }
}

struct In<'a>(&'a [u8]);

impl<'a> In<'a> {
    fn take(&mut self, count: usize) -> io::Result<&'a [u8]> {
        if self.0.len() < count {
            return Err(invalid("recorded executable tables are truncated"));
        }
        let (taken, rest) = self.0.split_at(count);
        self.0 = rest;
        Ok(taken)
    }

    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }


    fn string(&mut self) -> io::Result<String> {
        let length = self.u32()? as usize;
        String::from_utf8(self.take(length)?.to_vec())
            .map_err(|_| invalid("recorded executable table text is not UTF-8"))
    }

    fn leaked(&mut self) -> io::Result<&'static str> {
        self.string().map(leak_string)
    }

    fn list<T>(&mut self, mut item: impl FnMut(&mut Self) -> io::Result<T>) -> io::Result<Vec<T>> {
        let count = self.u32()? as usize;
        if count > self.0.len() {
            return Err(invalid("recorded executable table list is longer than its bytes"));
        }
        (0..count).map(|_| item(self)).collect()
    }


    fn verified<T>(
        &mut self,
        value: impl FnOnce(&mut Self) -> io::Result<T>,
    ) -> io::Result<Verified<T>> {
        Ok(match self.u8()? {
            0 => Ok(value(self)?),
            _ => {
                let part = self.string()?;
                Err(Withheld::new(part, self.string()?))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables(
        folding: Verified<folding::FoldingTable>,
        cursors: Verified<cursors::CursorTable>,
    ) -> ExecutableTables {
        fn withheld<T>(part: &'static str) -> Verified<T> {
            Err(Withheld::new(part, "synthetic"))
        }
        ExecutableTables {
            identity: ExecutableIdentity {
                size: 7,
                sha256: [3; 32],
            },
            version: withheld("version"),
            keywords: withheld("keywords"),
            registries: withheld("registries"),
            auxiliary: withheld("auxiliary"),
            variables: withheld("variables"),
            numeric: withheld("numeric"),
            commands: withheld("commands"),
            folding,
            cursors,
        }
    }

    #[test]
    fn the_folding_table_round_trips_and_an_earlier_recording_lacks_it_by_name() {
        let table = folding::FoldingTable::from_pairs([(0x100, 0x61), (0x1e9e, 0x53)]).unwrap();
        let cursor_table = cursors::CursorTable::from_raw_paths(
            [
                "gfx/cursors/alpha.cur".to_owned(),
                String::new(),
                "gfx/alt/beta.ani".to_owned(),
            ],
            3,
            [
                (cursors::CursorAction::Normal, 0),
                (cursors::CursorAction::NoMove, 1),
                (cursors::CursorAction::DragSelect, 2),
            ],
        );
        let encoded = encode(&tables(Ok(table.clone()), Ok(cursor_table.clone())));
        let decoded = decode(&encoded).unwrap();
        assert_eq!(decoded.folding, Ok(table.clone()));
        assert_eq!(decoded.cursors, Ok(cursor_table.clone()));
        let decoded_cursors = decoded.cursors.unwrap();
        assert_eq!(
            decoded_cursors.path_for(cursors::CursorAction::Normal),
            Ok("gfx/cursors/alpha.cur")
        );
        assert!(
            decoded_cursors
                .path_for(cursors::CursorAction::NoMove)
                .unwrap_err()
                .contains("empty cursor path")
        );
        assert_eq!(
            decoded_cursors.path_for(cursors::CursorAction::DragSelect),
            Ok("gfx/alt/beta.ani")
        );

        let withheld_folding = Withheld::new(FOLDING, "no run");
        let withheld_cursors = Withheld::new(CURSORS, "no anchor");
        let decoded = decode(&encode(&tables(
            Err(withheld_folding.clone()),
            Err(withheld_cursors.clone()),
        )))
        .unwrap();
        assert_eq!(decoded.folding, Err(withheld_folding));
        assert_eq!(decoded.cursors, Err(withheld_cursors));
        assert_eq!(decoded.identity().sha256, [3; 32]);

        let cursor_tail = 1
            + 4
            + (1 + 4 + "gfx/cursors/alpha.cur".len())
            + (1 + 4 + "empty cursor path".len())
            + (1 + 4 + "gfx/alt/beta.ani".len())
            + 4
            + 3 * (1 + 4);
        let mut version_two = encoded[..encoded.len() - cursor_tail].to_vec();
        version_two[4..6].copy_from_slice(&2_u16.to_le_bytes());
        let decoded_v2 = decode(&version_two).unwrap();
        assert_eq!(decoded_v2.folding, Ok(table));
        let cursors_v2 = decoded_v2.cursors.unwrap_err();
        assert_eq!(cursors_v2.part, CURSORS);
        assert_eq!(cursors_v2.reason, "not recorded in this recording");

        let mut version_one = version_two[..version_two.len() - (1 + 4 + 2 * 5)].to_vec();
        version_one[4..6].copy_from_slice(&1_u16.to_le_bytes());
        let decoded_v1 = decode(&version_one).unwrap();
        let folding_v1 = decoded_v1.folding.unwrap_err();
        assert_eq!(folding_v1.part, FOLDING);
        assert_eq!(folding_v1.reason, "not recorded in this recording");
        let cursors_v1 = decoded_v1.cursors.unwrap_err();
        assert_eq!(cursors_v1.part, CURSORS);
        assert_eq!(cursors_v1.reason, "not recorded in this recording");

        let mut later = encoded.clone();
        later[4..6].copy_from_slice(&(VERSION + 1).to_le_bytes());
        assert!(decode(&later).is_err());

        let mut conflicting = encoded[..encoded.len() - (4 + 3 * 5)].to_vec();
        conflicting.extend_from_slice(&3_u32.to_le_bytes());
        conflicting.push(cursors::CursorAction::Normal as u8);
        conflicting.extend_from_slice(&0_u32.to_le_bytes());
        conflicting.push(cursors::CursorAction::Normal as u8);
        conflicting.extend_from_slice(&2_u32.to_le_bytes());
        conflicting.push(cursors::CursorAction::DragSelect as u8);
        conflicting.extend_from_slice(&2_u32.to_le_bytes());
        let decoded_conflict = decode(&conflicting).unwrap().cursors.unwrap();
        let conflict_err = decoded_conflict
            .path_for(cursors::CursorAction::Normal)
            .unwrap_err();
        assert!(conflict_err.contains("normal"), "{conflict_err}");
        assert!(conflict_err.contains("conflicting"), "{conflict_err}");
        assert_eq!(
            decoded_conflict.path_for(cursors::CursorAction::DragSelect),
            Ok("gfx/alt/beta.ani")
        );
    }
}
