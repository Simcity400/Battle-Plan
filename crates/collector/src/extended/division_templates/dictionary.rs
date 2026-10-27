//! DivisionTemplates derived every hour from the template icon walk
//! ([`crate::template_changes`]).
//!
//! A dictionary row records a template's handle, roster owner, literal name, fallback name key and
//! unit_type. The hourly icon walk ([`native::walk_icons_with_selected_details_growing`]) visits
//! the same rosters in the same order and supplies every template's handle, owner, literal, key and
//! unit-type operands (the unit-type override and the definition category [`native::walk`]
//! derives), read this hour or held under the template's unchanged writer certificate (every
//! run-time writer of the names is a hooked site, [`engine_mutations::template_mutations`]). Only
//! a template whose names the walk could not read is read again here; failing that, the table is
//! refused.
use engine_allocation as allocation;
use engine_native::{creferenced_division_template as native, walk::Outcome};

/// `arena[span]`.
fn text(arena: &[u8], span: native::Span) -> Option<&[u8]> {
    arena.get(span.start..span.start.checked_add(span.len)?)
}

/// Appends `value` to `bytes`, returning its span.
fn append(bytes: &mut Vec<u8>, value: &[u8]) -> Result<native::Span, &'static str> {
    allocation::reserve(bytes, value.len()).map_err(|_| "template byte arena allocation failed")?;
    let start = bytes.len();
    bytes.extend_from_slice(value);
    Ok(native::Span {
        start,
        len: value.len(),
    })
}

/// One hour's DivisionTemplates into `rows` and `bytes`, from this hour's
/// icon walk (`icons`, `outcome`, its name arena `names`), and the templates
/// whose names were read again.
pub(crate) fn derive(
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
                    native::copy_names_growing(memory, icon.pointer, bytes, start)?
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
