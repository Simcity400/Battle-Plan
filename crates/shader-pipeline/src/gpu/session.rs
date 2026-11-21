use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Default)]
pub(super) struct GroupClock {
    revision: AtomicU64,
    pub hint: AtomicBool,
}
impl GroupClock {
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }
    pub fn invalidate(&self) {
        self.revision.fetch_add(1, Ordering::SeqCst);
        self.hint.store(false, Ordering::Release);
    }
    fn touch(
        &self,
        revision: u64,
        constants: &GroupStamp,
        textures: &GroupStamp,
        frame: u64,
    ) -> bool {
        if self.revision() != revision {
            return false;
        }
        constants.touch_ordered(frame);
        textures.touch_ordered(frame);
        self.hint.store(false, Ordering::Release);
        self.revision() == revision
    }
}

#[derive(Clone)]
pub(super) struct GroupStamp(Arc<AtomicU64>);
impl GroupStamp {
    pub fn new(frame: u64) -> Self {
        Self(Arc::new(AtomicU64::new(frame)))
    }
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
    pub fn set(&self, frame: u64) {
        self.0.store(frame, Ordering::Release);
    }
    fn touch_ordered(&self, frame: u64) {
        self.0.store(frame, Ordering::SeqCst);
    }
}

#[derive(Clone)]
pub(super) struct GroupResources {
    pub pipeline: wgpu::RenderPipeline,
    pub read_only: super::ReadOnly,
    pub groups: [wgpu::BindGroup; 3],
}

#[derive(Clone)]
pub(super) enum ResourceHandle {
    Inline(GroupResources),
    Shared(Arc<GroupResources>),
}
impl ResourceHandle {
    pub fn get(&self) -> &GroupResources {
        match self {
            Self::Inline(value) => value,
            Self::Shared(value) => value,
        }
    }
}

pub(super) struct ResolvedGroups {
    pub resources: GroupResources,
    pub constants: Option<GroupStamp>,
    pub textures: Option<GroupStamp>,
    pub clock: Option<Arc<GroupClock>>,
    pub revision: u64,
}

struct BufferEntry {
    buffer: wgpu::Buffer,
    resources: Arc<GroupResources>,
    constants: GroupStamp,
    textures: GroupStamp,
    clock: Arc<GroupClock>,
    revision: u64,
}
impl BufferEntry {
    fn touch(&self, frame: u64) -> bool {
        self.clock
            .touch(self.revision, &self.constants, &self.textures, frame)
    }
}
struct ProgramIdentity {
    owner: Arc<Mutex<Groups>>,
    pipeline: wgpu::RenderPipeline,
    textures: wgpu::BindGroup,
}
impl ProgramIdentity {
    fn matches(&self, program: &BindingProgram) -> bool {
        Arc::ptr_eq(&self.owner, &program.groups)
            && self.pipeline == program.pipeline
            && self.textures == program.textures
    }
}
#[derive(Default)]
struct ProgramEntry {
    identity: Option<ProgramIdentity>,
    buffers: Vec<BufferEntry>,
    last: usize,
}

#[derive(Default)]
pub struct BindingSessionStorage {
    programs: Vec<ProgramEntry>,
}
impl BindingSessionStorage {
    fn clear(&mut self) {
        for entry in &mut self.programs {
            entry.identity = None;
            entry.buffers.clear();
            entry.last = 0;
        }
    }
}

pub struct BindingSession<'a> {
    arena: &'a Arena,
    frame: u64,
    storage: BindingSessionStorage,
}
impl<'a> BindingSession<'a> {
    pub fn new(arena: &'a Arena) -> Self {
        Self::with_storage(arena, BindingSessionStorage::default())
    }

    pub fn with_storage(arena: &'a Arena, mut storage: BindingSessionStorage) -> Self {
        storage.clear();
        Self {
            arena,
            frame: arena.frame(),
            storage,
        }
    }

    pub fn bind_ordered<'b>(
        &mut self, key: usize, program: &BindingProgram,
        mut constants: impl Iterator<Item = Option<&'b [u8]>>,
    ) -> Result<Bindings, String> {
        (|| {
            self.sync_frame();
            let count = program.buffers.len();
            let mut blocks = [(&[][..], 0); BUFFERS];
            for (index, spec) in program.buffers.iter().enumerate() {
                let bytes = constants.next().flatten().ok_or_else(|| format!(
                    "native constant buffer {} has not been bound", spec.binding / 2))?;
                blocks[index] = (bytes, u64::from(spec.size));
            }
            let mut offsets = [0; BUFFERS];
            let source = self.arena.constants(&blocks[..count], &mut offsets[..count])?;
            self.finish_bind(key, program, source, offsets, &blocks[..count])
        })().map_err(|why| format!("{}: {why}", program.label))
    }

    fn sync_frame(&mut self) {
        let frame = self.arena.frame();
        if self.frame != frame {
            self.storage.clear();
            self.frame = frame;
        }
    }

    pub fn into_storage(mut self) -> BindingSessionStorage {
        self.storage.clear();
        self.storage
    }

    pub fn bind_with<'b>(
        &mut self,
        key: usize,
        program: &BindingProgram,
        constants: impl Fn(u32) -> Option<&'b [u8]>,
    ) -> Result<Bindings, String> {
        self.bind(key, program, constants)
            .map_err(|why| format!("{}: {why}", program.label))
    }

    fn bind<'b>(
        &mut self,
        key: usize,
        program: &BindingProgram,
        constants: impl Fn(u32) -> Option<&'b [u8]>,
    ) -> Result<Bindings, String> {
        self.sync_frame();
        let count = program.buffers.len();
        let mut blocks = [(&[][..], 0); BUFFERS];
        for (block, spec) in blocks.iter_mut().zip(&program.buffers) {
            let bytes = constants(spec.binding).ok_or_else(|| {
                format!(
                    "native constant buffer {} has not been bound",
                    spec.binding / 2
                )
            })?;
            *block = (bytes, u64::from(spec.size));
        }
        let mut offsets = [0; BUFFERS];
        let source = self
            .arena
            .constants(&blocks[..count], &mut offsets[..count])?;
        self.finish_bind(key, program, source, offsets, &blocks[..count])
    }

    fn finish_bind(
        &mut self, key: usize, program: &BindingProgram, source: wgpu::Buffer,
        offsets: [u32; BUFFERS], _blocks: &[(&[u8], u64)],
    ) -> Result<Bindings, String> {
        let frame = self.frame;
        let count = program.buffers.len();
        self.storage.programs.resize_with(
            self.storage.programs.len().max(key + 1),
            ProgramEntry::default,
        );
        let entry = &mut self.storage.programs[key];
        if entry
            .identity
            .as_ref()
            .is_none_or(|identity| !identity.matches(program))
        {
            entry.identity = Some(ProgramIdentity {
                owner: Arc::clone(&program.groups),
                pipeline: program.pipeline.clone(),
                textures: program.textures.clone(),
            });
            entry.buffers.clear();
            entry.last = 0;
        }
        let found = entry
            .buffers
            .get(entry.last)
            .filter(|last| last.buffer == source)
            .map(|_| entry.last)
            .or_else(|| entry.buffers.iter().position(|last| last.buffer == source));
        let resources =
            if let Some(index) = found.filter(|index| entry.buffers[*index].touch(frame)) {
                entry.last = index;
                Arc::clone(&entry.buffers[index].resources)
            } else {
                let resolved = program.resolve_groups(self.arena, source.clone(), true)?;
                let resources = Arc::new(resolved.resources);
                let value = BufferEntry {
                    buffer: source,
                    resources: Arc::clone(&resources),
                    constants: resolved.constants.expect("captured use"),
                    textures: resolved.textures.expect("captured use"),
                    clock: resolved.clock.expect("captured clock"),
                    revision: resolved.revision,
                };
                if let Some(index) = found {
                    entry.buffers[index] = value;
                    entry.last = index;
                } else {
                    entry.last = entry.buffers.len();
                    entry.buffers.push(value);
                }
                resources
            };
        Ok(Bindings {
            resources: ResourceHandle::Shared(resources),
            offsets,
            count,
            #[cfg(any(test, feature = "test-support"))]
            proof: preparation_bytes(&program.buffers, _blocks),
        })
    }
}

#[cfg(test)]
#[path = "session/tests.rs"]
mod tests;
