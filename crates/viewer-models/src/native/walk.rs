use crate::native::*;

type PlayerKey = (usize, u32, u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Row {
    pub group: usize,
    pub instance: usize,
    pub mesh: usize,
    pub library: usize,
    pub player: usize,
    pub hidden: bool,
}

#[derive(Default)]
pub(in crate) struct Layout {
    pub(super) rows: Vec<Row>,
    pub(super) players: Vec<PlayerKey>,
    libraries: Vec<usize>,
    keyed: Vec<(PlayerKey, usize)>,
    pub(super) installed: bool,
    pub(super) by_group: Vec<Vec<usize>>,
}

pub(super) enum Routes<'a> {
    Installed(std::slice::Iter<'a, Row>),
    Direct(std::slice::Iter<'a, (usize, usize)>),
}

impl<'a> Iterator for Routes<'a> {
    type Item = (usize, usize, Option<&'a Row>);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Installed(rows) => rows.next().map(|row| (row.group, row.instance, Some(row))),
            Self::Direct(rows) => rows.next().map(|&(group, instance)| (group, instance, None)),
        }
    }
}

impl Layout {
    pub(super) fn routes<'a>(&'a self, field: &'a ModelField) -> Routes<'a> {
        if self.installed {
            Routes::Installed(self.rows.iter())
        } else {
            Routes::Direct(field.native_order.iter())
        }
    }

    pub fn rebuild(
        &mut self,
        field: &ModelField,
        mesh_count: usize,
        alternatives: &BTreeMap<usize, usize>,
        redirects: &BTreeMap<usize, usize>,
        hidden: &BTreeSet<PlayerKey>,
    ) {
        self.installed = false;
        self.rows.clear();
        self.players.clear();
        self.keyed.clear();
        self.libraries.clear();
        self.by_group.clear();
        self.by_group.resize_with(field.groups.len(), Vec::new);
        for (position, &(group, _)) in field.native_order.iter().enumerate() {
            if let Some(positions) = self.by_group.get_mut(group) {
                positions.push(position);
            }
        }
        let count = field.native_order.len();
        if self.rows.try_reserve(count).is_err()
            || self.players.try_reserve(count).is_err()
            || self.keyed.try_reserve(count).is_err()
            || self.libraries.try_reserve(mesh_count).is_err()
        {
            return;
        }
        self.libraries.resize(mesh_count, 0);
        for &mesh in alternatives.values() {
            if let Some(library) = self.libraries.get_mut(mesh) {
                *library = 1;
            }
        }
        for &(group_index, instance_index) in &field.native_order {
            let (Some(group), Some(instance)) = (
                field.groups.get(group_index),
                field.instances.get(instance_index),
            ) else {
                return;
            };
            let (seed, identity) = instance.animation_key();
            let key = (
                redirects.get(&group.mesh).copied().unwrap_or(group.mesh),
                seed,
                identity,
            );
            self.keyed.push((key, self.rows.len()));
            self.rows.push(Row {
                group: group_index,
                instance: instance_index,
                mesh: group.mesh,
                library: self.libraries.get(group.mesh).copied().unwrap_or(0),
                player: 0,
                hidden: hidden.contains(&(group.mesh, seed, identity)),
            });
        }
        self.keyed.sort_unstable();
        for &(key, row) in &self.keyed {
            if self.players.last() != Some(&key) {
                self.players.push(key);
            }
            self.rows[row].player = self.players.len() - 1;
        }
        self.installed = true;
    }

    pub fn set_hidden(&mut self, hidden: &BTreeSet<PlayerKey>) {
        if !self.installed {
            return;
        }
        for row in &mut self.rows {
            let (_, seed, identity) = self.players[row.player];
            row.hidden = hidden.contains(&(row.mesh, seed, identity));
        }
    }
}

#[derive(Default)]
struct ActorHandle {
    invocation: u64,
    mesh: usize,
    frozen: bool,
    value: Option<ActorPoses>,
}

#[derive(Default)]
pub(super) struct FrameHandles {
    invocation: u64,
    actors: Vec<ActorHandle>,
}

impl FrameHandles {
    pub fn begin(&mut self, count: usize) -> bool {
        if self
            .actors
            .try_reserve(count.saturating_sub(self.actors.len()))
            .is_err()
        {
            return false;
        }
        self.invocation = self.invocation.wrapping_add(1);
        if self.invocation == 0 {
            self.actors.clear();
            self.invocation = 1;
        }
        self.actors.resize_with(count, ActorHandle::default);
        true
    }

    pub fn resolve(
        &mut self,
        player: usize,
        mesh: usize,
        frozen: bool,
        build: impl FnOnce() -> ActorPoses,
    ) -> ActorPoses {
        let handle = &mut self.actors[player];
        if handle.invocation != self.invocation || handle.mesh != mesh || handle.frozen != frozen {
            *handle = ActorHandle {
                invocation: self.invocation,
                mesh,
                frozen,
                value: Some(build()),
            };
        }
        Arc::clone(handle.value.as_ref().unwrap())
    }
}

#[cfg(test)]
mod tests;
