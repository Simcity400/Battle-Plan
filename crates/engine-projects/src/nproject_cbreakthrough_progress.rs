
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x02aa_49b0);
pub const TREE_HEADER: usize = 0x08;
pub const TREE_SIZE: usize = 0x10;

use engine_native_memory::{
    Memory,
    memory::{RecordField, RecordLayout, RecordSources},
    read, walk::Outcome,
};
use engine_native::ccountry;
use crate::{nproject_breakthrough_node::{self as node, Links}, nproject_cprogram_status};

const SENTINEL_LEN: usize = node::IS_NIL + 1;
const SENTINEL_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: SENTINEL_LEN,
}];
const SENTINEL_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&SENTINEL_FIELDS, SENTINEL_LEN).expect("valid breakthrough sentinel layout");

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub owner: u32,
    pub specialization: u32,
    pub points: i32,
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
) -> Outcome {
    walk_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
) -> Outcome {
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let expected_bt =
            engine_sites::at(base, VTABLE_RVA).ok_or("breakthrough vtable overflow")? as u64;
        let mut found_country = false;
        let mut len = 0;
        let mut unread_rows = 0_u32;
        let mut batch_countries = [engine_native_memory::country_identity::Country {
            pointer: 0,
            canonical_slot: 0,
            id: 0,
        }; nproject_cprogram_status::BATCH];
        let mut statuses = [nproject_cprogram_status::BatchedStatus::EMPTY;
            nproject_cprogram_status::BATCH];
        let mut header_ptrs = [0_usize; nproject_cprogram_status::BATCH];
        let mut header_slots = [0_usize; nproject_cprogram_status::BATCH];
        let mut sentinel_bytes = [0_u8; nproject_cprogram_status::BATCH * SENTINEL_LEN];
        let mut slot = 1_u32;
        while slot < countries.count() {
            let mut n = 0_usize;
            while slot < countries.count() && n < nproject_cprogram_status::BATCH {
                let cur = slot;
                slot += 1;
                let Some(country) = countries.at(cur)? else {
                    continue;
                };
                found_country = true;
                batch_countries[n] = country;
                n += 1;
            }
            if n == 0 {
                break;
            }
            nproject_cprogram_status::read_batch(
                memory,
                base,
                &countries,
                &batch_countries[..n],
                &mut statuses,
            )?;
            let mut header_count = 0_usize;
            let mut parsed = [None; nproject_cprogram_status::BATCH];
            for (i, entry) in statuses[..n].iter().enumerate() {
                if let Some(block) = &entry.block {
                    let bt = &block[nproject_cprogram_status::BREAKTHROUGH..];
                    if u64::from_le_bytes(bt[..8].try_into().unwrap()) == expected_bt {
                        let header = usize::try_from(u64::from_le_bytes(
                            bt[TREE_HEADER..TREE_HEADER + 8].try_into().unwrap(),
                        ))
                        .ok();
                        let count = u32::try_from(u64::from_le_bytes(
                            bt[TREE_SIZE..TREE_SIZE + 8].try_into().unwrap(),
                        ))
                        .ok();
                        if let (Some(header), Some(count)) = (header, count)
                            && header != 0
                            && header.checked_add(SENTINEL_LEN).is_some()
                        {
                            parsed[i] = Some((header, count));
                            header_ptrs[header_count] = header;
                            header_slots[header_count] = i;
                            header_count += 1;
                        }
                    }
                }
            }
            let sentinels_copied = if header_count > 0 {
                memory.copy_records(
                    RecordSources::Pointers(&header_ptrs[..header_count]),
                    &SENTINEL_LAYOUT,
                    &mut sentinel_bytes[..header_count * SENTINEL_LEN],
                )
            } else {
                0
            };
            let mut sentinel_index = [usize::MAX; nproject_cprogram_status::BATCH];
            for k in 0..sentinels_copied {
                sentinel_index[header_slots[k]] = k;
            }
            for (i, entry) in statuses[..n].iter().enumerate() {
                let outcome = if let (Some((header, count)), k) = (parsed[i], sentinel_index[i])
                    && k != usize::MAX
                {
                    let raw = &sentinel_bytes[k * SENTINEL_LEN..(k + 1) * SENTINEL_LEN];
                    walk_with_sentinel(memory, header, count, raw, entry.country.id, output.suffix(len))
                } else {
                    walk_status_buffer(memory, base, entry.status, entry.country.id, output.suffix(len))
                };
                match outcome {
                    Outcome::Rows { len: count } => len += count,
                    Outcome::Truncated {
                        len: count,
                        unread_rows: omitted,
                    } => {
                        len += count;
                        unread_rows = unread_rows
                            .checked_add(omitted)
                            .ok_or("breakthrough omitted count overflow")?;
                    }
                    Outcome::Unavailable { reason } => return Err(reason),
                }
            }
        }
        if !found_country {
            return Err("breakthrough country roster contains no countries");
        }
        Ok(if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

fn walk_with_sentinel(
    memory: &impl Memory,
    header: usize,
    count: u32,
    raw: &[u8],
    owner: u32,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
) -> Outcome {
    let result = (|| {
        let nil_byte = raw[node::IS_NIL];
        if nil_byte > 1 {
            return Err("breakthrough tree nil flag invalid");
        }
        let ptr = |offset| {
            usize::try_from(u64::from_le_bytes(
                raw[offset..offset + 8].try_into().unwrap(),
            ))
            .map_err(|_| "breakthrough tree pointer overflow")
        };
        let sentinel = Links {
            left: ptr(node::LEFT)?,
            parent: ptr(node::PARENT)?,
            right: ptr(node::RIGHT)?,
            nil: nil_byte != 0,
        };
        if !sentinel.nil {
            return Err("breakthrough tree sentinel is not nil");
        }
        if count == 0 {
            if sentinel.left != header || sentinel.parent != header || sentinel.right != header {
                return Err("empty breakthrough tree has live links");
            }
            return Ok(Outcome::Rows { len: 0 });
        }
        let root = Links::read(memory, sentinel.parent)?;
        if root.nil || root.parent != header {
            return Err("breakthrough root parent mismatch");
        }
        output.ensure(count as usize)?;
        let mut current = sentinel.left;
        let mut previous_key = None;
        for ordinal in 0..count {
            if current == header {
                return Err("breakthrough tree shorter than declared size");
            }
            if ordinal as usize == output.len() {
                return Ok(Outcome::Truncated {
                    len: output.len(),
                    unread_rows: count - ordinal,
                });
            }
            let links = Links::read(memory, current)?;
            if links.nil {
                return Err("breakthrough tree reached foreign sentinel");
            }
            let (specialization, points) = node::value(memory, current)?;
            if previous_key.is_some_and(|key| key >= specialization as i32) {
                return Err("breakthrough keys duplicated or unordered");
            }
            output[ordinal as usize] = Row {
                owner,
                specialization,
                points,
            };
            previous_key = Some(specialization as i32);
            let last = current;
            current = successor(memory, header, current, links)?;
            if ordinal + 1 == count && (current != header || sentinel.right != last) {
                return Err("breakthrough tree exceeds declared size or maximum differs");
            }
        }
        Ok(Outcome::Rows {
            len: count as usize,
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

pub fn walk_status_buffer(
    memory: &impl Memory,
    base: usize,
    status: usize,
    owner: u32,
    output: engine_native_memory::walk::Buffer<'_, Row>,
) -> Outcome {
    let Some(address) = status.checked_add(nproject_cprogram_status::BREAKTHROUGH) else {
        return Outcome::Unavailable {
            reason: "breakthrough status address overflow",
        };
    };
    walk_progress_buffer(memory, base, address, owner, output)
}

pub fn walk_progress_buffer(
    memory: &impl Memory,
    base: usize,
    address: usize,
    owner: u32,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
) -> Outcome {
    let result = (|| {
        let expected = engine_sites::at(base, VTABLE_RVA).ok_or("breakthrough vtable overflow")?;
        if read::<u64>(memory, address) != Some(expected as u64) {
            return Err("breakthrough vtable unreadable or mismatched");
        }
        let header_address = address
            .checked_add(TREE_HEADER)
            .ok_or("breakthrough address overflow")?;
        let fields = read::<[u8; 16]>(memory, header_address)
            .ok_or("breakthrough tree header unreadable")?;
        let header = usize::try_from(u64::from_le_bytes(fields[..8].try_into().unwrap()))
            .map_err(|_| "breakthrough tree pointer overflow")?;
        let count = u32::try_from(u64::from_le_bytes(fields[8..].try_into().unwrap()))
            .map_err(|_| "breakthrough row count cannot be represented")?;
        let sentinel = Links::read(memory, header)?;
        if !sentinel.nil {
            return Err("breakthrough tree sentinel is not nil");
        }
        if count == 0 {
            if sentinel.left != header || sentinel.parent != header || sentinel.right != header {
                return Err("empty breakthrough tree has live links");
            }
            return Ok(Outcome::Rows { len: 0 });
        }
        let root = Links::read(memory, sentinel.parent)?;
        if root.nil || root.parent != header {
            return Err("breakthrough root parent mismatch");
        }
        output.ensure(count as usize)?;
        let mut current = sentinel.left;
        let mut previous_key = None;
        for ordinal in 0..count {
            if current == header {
                return Err("breakthrough tree shorter than declared size");
            }
            if ordinal as usize == output.len() {
                return Ok(Outcome::Truncated {
                    len: output.len(),
                    unread_rows: count - ordinal,
                });
            }
            let links = Links::read(memory, current)?;
            if links.nil {
                return Err("breakthrough tree reached foreign sentinel");
            }
            let (specialization, points) = node::value(memory, current)?;
            if previous_key.is_some_and(|key| key >= specialization as i32) {
                return Err("breakthrough keys duplicated or unordered");
            }
            output[ordinal as usize] = Row {
                owner,
                specialization,
                points,
            };
            previous_key = Some(specialization as i32);
            let last = current;
            current = successor(memory, header, current, links)?;
            if ordinal + 1 == count && (current != header || sentinel.right != last) {
                return Err("breakthrough tree exceeds declared size or maximum differs");
            }
        }
        Ok(Outcome::Rows {
            len: count as usize,
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

struct Chain {
    checkpoint: usize,
    power: u64,
    distance: u64,
}

impl Chain {
    fn new(start: usize) -> Self {
        Self {
            checkpoint: start,
            power: 1,
            distance: 0,
        }
    }

    fn visit(&mut self, address: usize) -> Result<(), &'static str> {
        self.distance += 1;
        if address == self.checkpoint {
            return Err("breakthrough tree link cycle");
        }
        if self.distance == self.power {
            self.checkpoint = address;
            self.power = self.power.saturating_mul(2);
            self.distance = 0;
        }
        Ok(())
    }
}

fn successor(
    memory: &impl Memory,
    header: usize,
    mut current: usize,
    links: Links,
) -> Result<usize, &'static str> {
    if links.right != header {
        let mut candidate = links.right;
        let mut chain = Chain::new(current);
        loop {
            chain.visit(candidate)?;
            let child = Links::read(memory, candidate)?;
            if child.nil {
                return Err("breakthrough tree reached foreign sentinel");
            }
            if child.parent != current {
                return Err("breakthrough child parent mismatch");
            }
            if child.left == header {
                return Ok(candidate);
            }
            current = candidate;
            candidate = child.left;
        }
    }
    let mut parent = links.parent;
    let mut chain = Chain::new(current);
    loop {
        if parent == header {
            return Ok(header);
        }
        chain.visit(parent)?;
        let ancestor = Links::read(memory, parent)?;
        if ancestor.nil {
            return Err("breakthrough tree reached foreign sentinel");
        }
        if ancestor.left == current {
            return Ok(parent);
        }
        if ancestor.right != current {
            return Err("breakthrough parent child mismatch");
        }
        current = parent;
        parent = ancestor.parent;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    const IMAGE: usize = 0x1000;
    const ADDRESS: usize = 0x2000;
    const HEADER: usize = 0x2080;
    const MINIMUM: usize = 0x20c0;
    const ROOT: usize = 0x2100;
    const MAXIMUM: usize = 0x2140;

    fn write(bytes: &mut [u8], address: usize, value: &[u8]) {
        let offset = address - ADDRESS;
        bytes[offset..offset + value.len()].copy_from_slice(value);
    }

    fn tree() -> Vec<u8> {
        let mut bytes = vec![0; 0x180];
        write(
            &mut bytes,
            ADDRESS,
            &((IMAGE + VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        write(
            &mut bytes,
            ADDRESS + TREE_HEADER,
            &(HEADER as u64).to_le_bytes(),
        );
        write(&mut bytes, ADDRESS + TREE_SIZE, &3_u64.to_le_bytes());
        for (address, left, parent, right, nil, key, points) in [
            (HEADER, MINIMUM, ROOT, MAXIMUM, 1_u8, 0_u32, 0_i32),
            (MINIMUM, HEADER, ROOT, HEADER, 0, 7, i32::MIN),
            (ROOT, MINIMUM, HEADER, MAXIMUM, 0, 11, 65537),
            (MAXIMUM, HEADER, ROOT, HEADER, 0, 15, i32::MAX),
        ] {
            write(
                &mut bytes,
                address + node::LEFT,
                &(left as u64).to_le_bytes(),
            );
            write(
                &mut bytes,
                address + node::PARENT,
                &(parent as u64).to_le_bytes(),
            );
            write(
                &mut bytes,
                address + node::RIGHT,
                &(right as u64).to_le_bytes(),
            );
            write(&mut bytes, address + node::IS_NIL, &[nil]);
            write(
                &mut bytes,
                address + node::SPECIALIZATION_HANDLE,
                &key.to_le_bytes(),
            );
            write(&mut bytes, address + node::POINTS, &points.to_le_bytes());
        }
        bytes
    }

    fn capture(bytes: &[u8], output: &mut Vec<Row>) -> Outcome {
        walk_progress_buffer(&SliceMemory {
                base: ADDRESS,
                bytes,
            }, IMAGE, ADDRESS, 19, engine_native_memory::walk::Buffer::Growing(output))
    }

    #[test]
    fn successor_recovers_ancestors_and_preserves_signed_native_points() {
        let bytes = tree();
        let mut rows = vec![Row::default(); 3];
        assert_eq!(capture(&bytes, &mut rows), Outcome::Rows { len: 3 });
        assert_eq!(
            rows,
            [
                Row {
                    owner: 19,
                    specialization: 7,
                    points: i32::MIN
                },
                Row {
                    owner: 19,
                    specialization: 11,
                    points: 65537
                },
                Row {
                    owner: 19,
                    specialization: 15,
                    points: i32::MAX
                },
            ]
        );
    }

    #[test]
    fn token_bits_do_not_assume_native_unsigned_order_and_duplicates_fail() {
        let mut bytes = tree();
        write(
            &mut bytes,
            MINIMUM + node::SPECIALIZATION_HANDLE,
            &u32::MAX.to_le_bytes(),
        );
        let mut rows = vec![Row::default(); 3];
        assert_eq!(capture(&bytes, &mut rows), Outcome::Rows { len: 3 });
        assert_eq!(rows.iter().map(|row| row.specialization).collect::<Vec<_>>(), [u32::MAX, 11, 15]);
        write(
            &mut bytes,
            ROOT + node::SPECIALIZATION_HANDLE,
            &u32::MAX.to_le_bytes(),
        );
        assert_eq!(
            capture(&bytes, &mut rows),
            Outcome::Unavailable {
                reason: "breakthrough keys duplicated or unordered"
            }
        );
    }

    #[test]
    fn the_count_probe_reads_no_row_payload_and_reports_every_row() {
        let bytes = tree();
        struct MissingRootPayload<'a>(&'a [u8]);
        impl Memory for MissingRootPayload<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                assert_ne!(
                    address,
                    ROOT + node::SPECIALIZATION_HANDLE,
                    "the count probe read a row payload"
                );
                SliceMemory {
                    base: ADDRESS,
                    bytes: self.0,
                }
                .copy(address, destination)
            }
        }
        assert_eq!(
            walk_progress_buffer(&MissingRootPayload(&bytes), IMAGE, ADDRESS, 19, engine_native_memory::walk::Buffer::Count),
            Outcome::Truncated {
                len: 0,
                unread_rows: 3
            }
        );
    }

    #[test]
    fn inconsistent_size_sentinel_and_cycles_fail_instead_of_returning_prefix() {
        let original = tree();
        for count in [2_u64, 4] {
            let mut bytes = original.clone();
            write(&mut bytes, ADDRESS + TREE_SIZE, &count.to_le_bytes());
            assert!(matches!(
                capture(&bytes, &mut vec![Row::default(); 4]),
                Outcome::Unavailable { .. }
            ));
        }
        let mut bytes = original.clone();
        write(
            &mut bytes,
            ROOT + node::PARENT,
            &(ROOT as u64).to_le_bytes(),
        );
        assert!(matches!(
            capture(&bytes, &mut vec![Row::default(); 3]),
            Outcome::Unavailable { .. }
        ));
        let mut bytes = original;
        write(
            &mut bytes,
            MAXIMUM + node::LEFT,
            &(MAXIMUM as u64).to_le_bytes(),
        );
        write(
            &mut bytes,
            MAXIMUM + node::PARENT,
            &(MAXIMUM as u64).to_le_bytes(),
        );
        assert!(matches!(
            capture(&bytes, &mut vec![Row::default(); 3]),
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn empty_tree_requires_empty_native_links_and_faults_are_explicit() {
        let mut bytes = tree();
        write(&mut bytes, ADDRESS + TREE_SIZE, &0_u64.to_le_bytes());
        assert!(matches!(
            capture(&bytes, &mut Vec::new()),
            Outcome::Unavailable { .. }
        ));
        for offset in [node::LEFT, node::PARENT, node::RIGHT] {
            write(&mut bytes, HEADER + offset, &(HEADER as u64).to_le_bytes());
        }
        assert_eq!(capture(&bytes, &mut Vec::new()), Outcome::Rows { len: 0 });
        assert!(matches!(
            capture(&bytes[..0x99], &mut Vec::new()),
            Outcome::Unavailable { .. }
        ));
    }
}
