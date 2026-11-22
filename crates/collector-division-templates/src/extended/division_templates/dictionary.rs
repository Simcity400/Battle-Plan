use engine_allocation as allocation;
use engine_native::{creferenced_division_template as native, walk::Outcome};

fn text(arena: &[u8], span: native::Span) -> Option<&[u8]> {
    arena.get(span.start..span.start.checked_add(span.len)?)
}

fn append(bytes: &mut Vec<u8>, value: &[u8]) -> Result<native::Span, &'static str> {
    allocation::reserve(bytes, value.len()).map_err(|_| "template byte arena allocation failed")?;
    let start = bytes.len();
    bytes.extend_from_slice(value);
    Ok(native::Span {
        start,
        len: value.len(),
    })
}

pub fn derive(
    memory: &impl engine_native::Memory,
    icons: &[native::IconRow],
    outcome: Outcome,
    names: &[u8],
    rows: &mut Vec<native::Row>,
    bytes: &mut Vec<u8>,
) -> (Outcome, u64) {
    rows.clear();
    bytes.clear();
    let mut rereads = 0;
    let derived = (|| -> Result<(), &'static str> {
        if let Outcome::Unavailable { reason } = outcome {
            return Err(reason);
        }
        allocation::reserve(rows, icons.len()).map_err(|_| "template row allocation failed")?;
        for icon in icons {
            if let Some(reason) = icon.unit_type_failure {
                return Err(reason);
            }
            let walked = icon.display_name.zip(icon.name_key).and_then(|(display, key)| {
                Some((text(names, display.literal)?, text(names, key)?))
            });
            let [name, name_key] = match walked {
                Some((literal, key)) => [append(bytes, literal)?, append(bytes, key)?],
                None => {
                    rereads += 1;
                    let start = bytes.len();
                    native::copy_names(memory, icon.pointer, bytes, start)?
                }
            };
            rows.push(native::Row {
                template: icon.template,
                owner: icon.owner,
                name,
                name_key,
                unit_type: icon.unit_type,
            });
        }
        Ok(())
    })();
    match derived {
        Ok(()) => (outcome, rereads),
        Err(reason) => {
            rows.clear();
            (Outcome::Unavailable { reason }, rereads)
        }
    }
}

#[cfg(test)]
mod tests;
