use naga::{Expression as E, Handle, TypeInner as T};
use std::{collections::BTreeMap, ops::Range};

#[derive(Clone)]
struct Region {
    binding: u32,
    offset: u32,
    ty: T,
}

fn children(module: &naga::Module, region: &Region) -> Vec<Region> {
    let scalar = |scalar| T::Scalar(scalar);
    let items: Vec<_> = match &region.ty {
        T::Struct { members, .. } => members
            .iter()
            .map(|member| (member.offset, module.types[member.ty].inner.clone()))
            .collect(),
        T::Array {
            base,
            size: naga::ArraySize::Constant(count),
            stride,
        } => (0..count.get())
            .map(|index| (index * stride, module.types[*base].inner.clone()))
            .collect(),
        T::Matrix {
            columns,
            rows,
            scalar,
        } => {
            let stride = if *rows == naga::VectorSize::Tri {
                4
            } else {
                *rows as u32
            } * u32::from(scalar.width);
            (0..*columns as u32)
                .map(|index| {
                    (
                        index * stride,
                        T::Vector {
                            size: *rows,
                            scalar: *scalar,
                        },
                    )
                })
                .collect()
        }
        T::Vector {
            size,
            scalar: value,
        } => (0..*size as u32)
            .map(|index| (index * u32::from(value.width), scalar(*value)))
            .collect(),
        _ => Vec::new(),
    };
    items
        .into_iter()
        .map(|(offset, ty)| Region {
            binding: region.binding,
            offset: region.offset + offset,
            ty,
        })
        .collect()
}

fn pointer(
    module: &naga::Module,
    expressions: &naga::Arena<E>,
    expression: Handle<E>,
) -> Vec<Region> {
    match &expressions[expression] {
        E::GlobalVariable(global) => {
            let global = &module.global_variables[*global];
            if global.space != naga::AddressSpace::Uniform {
                return Vec::new();
            }
            global
                .binding
                .as_ref()
                .map(|binding| Region {
                    binding: binding.binding,
                    offset: 0,
                    ty: module.types[global.ty].inner.clone(),
                })
                .into_iter()
                .collect()
        }
        E::AccessIndex { base, index } => pointer(module, expressions, *base)
            .iter()
            .flat_map(|region| children(module, region).into_iter().nth(*index as usize))
            .collect(),
        E::Access { base, index } => {
            let index = constant_index(module, expressions, *index);
            pointer(module, expressions, *base)
                .iter()
                .flat_map(|region| {
                    children(module, region)
                        .into_iter()
                        .enumerate()
                        .filter(move |(i, _)| index.is_none_or(|index| *i == index as usize))
                        .map(|(_, region)| region)
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

fn constant_index(
    module: &naga::Module,
    expressions: &naga::Arena<E>,
    expression: Handle<E>,
) -> Option<u32> {
    match expressions[expression] {
        E::Literal(naga::Literal::U32(index)) => Some(index),
        E::Literal(naga::Literal::I32(index)) if index >= 0 => Some(index as u32),
        E::Constant(value) => constant_index(
            module,
            &module.global_expressions,
            module.constants[value].init,
        ),
        E::ZeroValue(_) => Some(0),
        _ => None,
    }
}

fn add(module: &naga::Module, region: &Region, out: &mut BTreeMap<u32, Vec<Range<u32>>>) {
    if let T::Scalar(scalar) = region.ty {
        out.entry(region.binding)
            .or_default()
            .push(region.offset..region.offset + u32::from(scalar.width));
    } else {
        for child in children(module, region) {
            add(module, &child, out);
        }
    }
}

fn function(
    module: &naga::Module,
    function: &naga::Function,
    info: &naga::valid::FunctionInfo,
    out: &mut BTreeMap<u32, Vec<Range<u32>>>,
) {
    if function.arguments.iter().any(|argument| {
        matches!(
            module.types[argument.ty].inner,
            T::Pointer {
                space: naga::AddressSpace::Uniform,
                ..
            } | T::ValuePointer {
                space: naga::AddressSpace::Uniform,
                ..
            }
        )
    }) {
        for (_, global) in module.global_variables.iter() {
            if global.space == naga::AddressSpace::Uniform
                && let Some(binding) = &global.binding
                && let T::Struct { span, .. } = module.types[global.ty].inner
            {
                out.entry(binding.binding).or_default().push(0..span);
            }
        }
    }
    for (handle, expression) in function.expressions.iter() {
        let E::Load { pointer: source } = expression else {
            continue;
        };
        if info[handle].ref_count == 0 {
            continue;
        }
        let regions = pointer(module, &function.expressions, *source);
        let mut projections = 0;
        let mut components = Vec::new();
        for (_, use_) in function.expressions.iter() {
            match use_ {
                E::AccessIndex { base, index } if *base == handle => {
                    projections += 1;
                    components.push(*index);
                }
                E::Swizzle {
                    vector,
                    size,
                    pattern,
                } if *vector == handle => {
                    projections += 1;
                    components.extend(pattern[..*size as usize].iter().map(|value| *value as u32));
                }
                _ => {}
            }
        }
        for region in regions {
            if projections == info[handle].ref_count && matches!(region.ty, T::Vector { .. }) {
                let children = children(module, &region);
                for index in &components {
                    if let Some(child) = children.get(*index as usize) {
                        add(module, child, out);
                    }
                }
            } else {
                add(module, &region, out);
            }
        }
    }
}

pub(crate) fn reflect(
    module: &naga::Module,
    info: &naga::valid::ModuleInfo,
) -> BTreeMap<u32, Vec<Range<u32>>> {
    let mut out = BTreeMap::new();
    for (handle, body) in module.functions.iter() {
        function(module, body, &info[handle], &mut out);
    }
    for (index, entry) in module.entry_points.iter().enumerate() {
        function(
            module,
            &entry.function,
            info.get_entry_point(index),
            &mut out,
        );
    }
    for ranges in out.values_mut() {
        ranges.sort_unstable_by_key(|range| (range.start, range.end));
        let mut merged: Vec<Range<u32>> = Vec::new();
        for range in ranges.drain(..) {
            if let Some(last) = merged.last_mut()
                && range.start <= last.end
            {
                last.end = last.end.max(range.end);
            } else {
                merged.push(range);
            }
        }
        *ranges = merged;
    }
    out
}

impl super::Buffer {
    pub fn read_bytes(&self, block: &[u8], out: &mut Vec<u8>) {
        for range in &self.reads {
            let held =
                (range.start as usize).min(block.len())..(range.end as usize).min(block.len());
            let zeros = (range.end - range.start) as usize - held.len();
            out.extend_from_slice(&block[held]);
            out.extend(std::iter::repeat_n(0u8, zeros));
        }
    }

    pub fn check_initialized(&self, physical: u32, known: &[Range<u32>]) -> Result<(), String> {
        for read in &self.reads {
            let end = read.end.min(physical);
            let mut start = read.start;
            while start < end {
                let next = known
                    .iter()
                    .filter(|range| range.start <= start)
                    .map(|range| range.end)
                    .max()
                    .unwrap_or(start);
                if next <= start {
                    return Err(format!(
                        "native {} constant buffer b{} reads unavailable bytes {}..{} (physical size {})",
                        if self.binding.is_multiple_of(2) {
                            "vertex"
                        } else {
                            "pixel"
                        },
                        self.binding / 2,
                        start,
                        end,
                        physical
                    ));
                }
                start = next;
            }
        }
        Ok(())
    }
}
