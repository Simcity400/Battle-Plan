
use engine_native_memory::{Memory, cpdx_array::Array, msvc_string, read};
use engine_native_parts::{modifier_accumulator, modifier_definition};

mod repeats;
pub use repeats::RepeatCache;

pub const REQUEST_CAPACITY: &str = "attrition modifier request arena capacity exceeded";
pub const ROOT_CAPACITY: &str = "attrition modifier root arena capacity exceeded";
pub const NODE_CAPACITY: &str = "attrition modifier node arena capacity exceeded";
pub const TEXT_CAPACITY: &str = "attrition modifier text arena capacity exceeded";
pub const DEPTH_CAPACITY: &str = "attrition modifier depth arena capacity exceeded";
pub const STACK_HEADROOM: &str = "attrition modifier census worker stack headroom exhausted";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Request {
    pub accumulator: usize,
    pub key: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RawNode {
    pub children: u32,
    pub value: i64,
    pub name: [usize; 2],
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Root {
    pub accumulator: usize,
    pub key: u32,
    pub nodes: [usize; 2],
    pub failure: Option<&'static str>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    address: usize,
    children: Array,
    header: [u8; 16],
    next: u32,
}

pub struct Buffers<'a> {
    pub requests: engine_native_memory::walk::Buffer<'a, Request>,
    pub request_len: usize,
    pub request_required: usize,
    pub roots: engine_native_memory::walk::Buffer<'a, Root>,
    pub root_len: usize,
    pub root_required: usize,
    pub nodes: engine_native_memory::walk::Buffer<'a, RawNode>,
    pub node_len: usize,
    pub node_required: usize,
    pub text: engine_native_memory::walk::Buffer<'a, u8>,
    pub text_len: usize,
    pub text_required: usize,
    pub stack: engine_native_memory::walk::Buffer<'a, Frame>,
    pub stack_required: usize,
    pub repeats: Option<&'a mut RepeatCache>,
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .filter(|_| pointer != 0)
        .ok_or("attrition modifier pointer invalid or overflow")
}

fn add(value: &mut usize, amount: usize) -> Result<(), &'static str> {
    *value = value
        .checked_add(amount)
        .ok_or("attrition modifier demand overflow")?;
    Ok(())
}

fn child(memory: &impl Memory, children: Array, ordinal: u32) -> Result<usize, &'static str> {
    let pointer = children
        .element(ordinal, 8)
        .and_then(|at| read::<u64>(memory, at))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .filter(|pointer| *pointer != 0)
        .ok_or("attrition modifier child unreadable or null")?;
    at(pointer, 0x10)
}

#[derive(Clone, Copy)]
struct StackLimits {
    low: usize,
    high: usize,
}

impl StackLimits {
    #[cfg(windows)]
    fn current() -> Result<Self, &'static str> {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetCurrentThreadStackLimits(low: *mut usize, high: *mut usize);
        }
        let mut limits = Self { low: 0, high: 0 };
        unsafe { GetCurrentThreadStackLimits(&mut limits.low, &mut limits.high) };
        if limits.low == 0 || limits.low >= limits.high {
            Err("attrition modifier worker stack limits unavailable")
        } else {
            Ok(limits)
        }
    }

    #[cfg(not(windows))]
    fn current() -> Result<Self, &'static str> {
        Err("attrition modifier worker stack limits unavailable")
    }

    #[inline(always)]
    fn check(&self) -> Result<(), &'static str> {
        let marker = 0_u8;
        let pointer = std::ptr::addr_of!(marker) as usize;
        if pointer >= self.high || pointer.saturating_sub(self.low) < 64 * 1024 {
            Err(STACK_HEADROOM)
        } else {
            Ok(())
        }
    }
}

pub fn capture(
    memory: &impl Memory,
    base: usize,
    buffers: &mut Buffers<'_>,
) -> Result<(), &'static str> {
    let supplied = buffers.request_len;
    buffers.request_required = buffers.request_required.max(supplied);
    if supplied > buffers.requests.len() || buffers.request_required > supplied {
        return Err(REQUEST_CAPACITY);
    }
    buffers.requests[..supplied].sort_unstable();
    let mut unique = 0;
    for index in 0..supplied {
        let request = buffers.requests[index];
        if unique == 0 || buffers.requests[unique - 1] != request {
            buffers.requests[unique] = request;
            unique += 1;
        }
    }
    buffers.request_len = unique;
    buffers.root_len = 0;
    buffers.root_required = unique;
    buffers.node_len = 0;
    buffers.node_required = 0;
    buffers.text_len = 0;
    buffers.text_required = 0;
    buffers.stack_required = 0;
    buffers.roots.ensure(unique)?;
    let limits = StackLimits::current();
    let mut repeats = buffers.repeats.take();
    for index in 0..unique {
        let request = buffers.requests[index];
        let first_node = buffers.node_len;
        let first_text = buffers.text_len;
        let mut failure = None;
        let result = match repeats.as_deref_mut() {
            Some(cache) => {
                if index == 0 || buffers.requests[index - 1].accumulator != request.accumulator {
                    cache.clear();
                }
                let memory = repeats::Repeats {
                    inner: memory,
                    cache: std::cell::RefCell::new(cache),
                };
                capture_root(&memory, base, request, true, buffers, &mut failure, &limits)
            }
            None => capture_root(memory, base, request, true, buffers, &mut failure, &limits),
        };
        failure = failure.or(result.err());
        let nodes = if failure.is_some() {
            buffers.node_len = first_node;
            buffers.text_len = first_text;
            [first_node, 0]
        } else {
            [first_node, buffers.node_len - first_node]
        };
        if let Some(root) = buffers.roots.get_mut(index) {
            *root = Root {
                accumulator: request.accumulator,
                key: request.key,
                nodes,
                failure,
            };
            buffers.root_len += 1;
        }
    }
    buffers.repeats = repeats;
    if unique > buffers.roots.len() {
        Err(ROOT_CAPACITY)
    } else {
        Ok(())
    }
}

fn capture_root(
    memory: &impl Memory,
    base: usize,
    request: Request,
    store: bool,
    buffers: &mut Buffers<'_>,
    failure: &mut Option<&'static str>,
    limits: &Result<StackLimits, &'static str>,
) -> Result<(), &'static str> {
    let metadata = modifier_definition::operands(memory, base, request.key)?;
    let mut address = request.accumulator;
    let mut depth = 0;
    loop {
        if address == 0
            || buffers.stack[..depth]
                .iter()
                .any(|frame| frame.address == address)
        {
            return Err("attrition modifier null node or ancestor cycle");
        }
        buffers.stack.ensure(
            depth
                .checked_add(1)
                .ok_or("attrition modifier depth overflow")?,
        )?;
        if depth == buffers.stack.len() {
            if store {
                failure.get_or_insert(DEPTH_CAPACITY);
            }
            let limits = limits.as_ref().map_err(|reason| *reason)?;
            limits.check()?;
            census_without_scratch(
                memory,
                address,
                request.key,
                metadata,
                store,
                buffers,
                failure,
                depth,
                None,
                limits,
            )?;
        } else {
            let (children, header) = capture_node(
                memory,
                address,
                request.key,
                metadata,
                store,
                buffers,
                failure,
            )?;
            buffers.stack_required = buffers.stack_required.max(depth + 1);
            buffers.stack[depth] = Frame {
                address,
                children,
                header,
                next: 0,
            };
            depth += 1;
        }
        loop {
            if depth == 0 {
                return Ok(());
            }
            let frame = &mut buffers.stack[depth - 1];
            if frame.next == frame.children.count {
                if !store {
                    validate_children(memory, frame.address, frame.header)?;
                }
                depth -= 1;
                continue;
            }
            address = child(memory, frame.children, frame.next)?;
            frame.next += 1;
            break;
        }
    }
}

#[inline(never)]
#[allow(
    clippy::too_many_arguments,
    reason = "Explicit per-root key, capture state and caller-owned arenas"
)]
fn capture_node(
    memory: &impl Memory,
    address: usize,
    key: u32,
    metadata: modifier_definition::Operands,
    store: bool,
    buffers: &mut Buffers<'_>,
    failure: &mut Option<&'static str>,
) -> Result<(Array, [u8; 16]), &'static str> {
    let header = read::<[u8; 16]>(memory, at(address, 0x18)?)
        .ok_or("attrition modifier children unreadable")?;
    let children = Array::from_header(header, 8).ok_or("attrition modifier children invalid")?;
    let raw = modifier_accumulator::lookup_at(memory, address, key)?;
    let value = if raw.present {
        metadata.effective(raw.raw)
    } else {
        0
    };
    let name_first = buffers.text_len;
    let mut name_len = 0;
    if value != 0 {
        let display = at(address, 0x48)?;
        let header = read::<[u8; msvc_string::STRIDE]>(memory, display)
            .ok_or("attrition modifier display unreadable")?;
        let name = msvc_string::Descriptor::from_header(memory, display, header)?;
        name_len = name.len;
        add(&mut buffers.text_required, name.len)?;
        if store {
            buffers.text.ensure(
                name_first
                    .checked_add(name.len)
                    .ok_or("attrition modifier text overflow")?,
            )?;
            if let Some(output) = name_first
                .checked_add(name.len)
                .and_then(|end| buffers.text.get_mut(name_first..end))
            {
                name.copy(memory, output)?;
                buffers.text_len += name.len;
            } else {
                failure.get_or_insert(TEXT_CAPACITY);
            }
        } else if read::<[u8; msvc_string::STRIDE]>(memory, display) != Some(header) {
            return Err("attrition modifier display header changed");
        }
    }
    add(&mut buffers.node_required, 1)?;
    if store {
        buffers.nodes.ensure(
            buffers
                .node_len
                .checked_add(1)
                .ok_or("attrition modifier node overflow")?,
        )?;
        if let Some(node) = buffers.nodes.get_mut(buffers.node_len) {
            *node = RawNode {
                children: children.count,
                value,
                name: [name_first, name_len],
            };
            buffers.node_len += 1;
        } else {
            failure.get_or_insert(NODE_CAPACITY);
        }
    }
    Ok((children, header))
}

fn validate_children(
    memory: &impl Memory,
    address: usize,
    header: [u8; 16],
) -> Result<(), &'static str> {
    if read::<[u8; 16]>(memory, at(address, 0x18)?) != Some(header) {
        Err("attrition modifier child header changed")
    } else {
        Ok(())
    }
}

struct Ancestor<'a> {
    address: usize,
    parent: Option<&'a Ancestor<'a>>,
}

#[inline(never)]
#[allow(
    clippy::too_many_arguments,
    reason = "Borrowed native ancestor frames replace short heap scratch"
)]
fn census_without_scratch(
    memory: &impl Memory,
    address: usize,
    key: u32,
    metadata: modifier_definition::Operands,
    store: bool,
    buffers: &mut Buffers<'_>,
    failure: &mut Option<&'static str>,
    prefix_depth: usize,
    ancestors: Option<&Ancestor<'_>>,
    limits: &StackLimits,
) -> Result<(), &'static str> {
    if address == 0
        || buffers.stack[..prefix_depth]
            .iter()
            .any(|frame| frame.address == address)
    {
        return Err("attrition modifier null node or ancestor cycle");
    }
    let mut previous = ancestors;
    let mut depth = prefix_depth + 1;
    while let Some(ancestor) = previous {
        if ancestor.address == address {
            return Err("attrition modifier null node or ancestor cycle");
        }
        depth += 1;
        previous = ancestor.parent;
    }
    let (children, header) = capture_node(memory, address, key, metadata, store, buffers, failure)?;
    buffers.stack_required = buffers.stack_required.max(depth);
    let current = Ancestor {
        address,
        parent: ancestors,
    };
    for ordinal in 0..children.count {
        let address = child(memory, children, ordinal)?;
        limits.check()?;
        census_without_scratch(
            memory,
            address,
            key,
            metadata,
            store,
            buffers,
            failure,
            prefix_depth,
            Some(&current),
            limits,
        )?;
    }
    if !store {
        validate_children(memory, address, header)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
