//! Share source views while preserving each member's standalone column or error.
use super::super::aliases::private::{self, Followed, Supply, TableResolution};
use super::super::decoded_sections::Key;
use super::{
    Check, Context, LaterSection, RawColumn, keyframe_refusal, raw_stage, table_updates_admitted,
};
use crate::{
    Hour,
    frame::{SharedView, TableUpdate, TableView, recorded::Updates},
    rows::{TABLE_COUNT, TableId, compatibility::HISTORY},
};
use engine_allocation as allocation;
use std::{cell::OnceCell, io};

#[cfg(test)]
mod tests;

/// Group only stored alias tables connected through shared source dependencies.
fn grouped(version: u16) -> [bool; TABLE_COUNT] {
    let stored = || {
        private::TABLES
            .iter()
            .copied()
            .filter(move |&table| private::stored(table, version))
    };
    let mut parent: [usize; TABLE_COUNT] = std::array::from_fn(|at| at);
    let root = |parent: &[usize; TABLE_COUNT], mut at: usize| {
        while parent[at] != at {
            at = parent[at];
        }
        at
    };
    for table in stored() {
        for &source in private::dependencies(table) {
            let (from, to) = (root(&parent, table.index()), root(&parent, source.index()));
            parent[from] = to;
        }
    }
    let mut count = [0_usize; TABLE_COUNT];
    for table in stored() {
        count[root(&parent, table.index())] += 1;
    }
    let mut grouped = [false; TABLE_COUNT];
    for table in stored() {
        grouped[table.index()] = count[root(&parent, table.index())] > 1;
    }
    grouped
}

/// The group's members, each after the members it reads.
fn members(version: u16) -> io::Result<Vec<TableId>> {
    let grouped = grouped(version);
    let mut members = allocation::vec(private::TABLES.len())?;
    while let Some(table) = private::TABLES.iter().copied().find(|&table| {
        grouped[table.index()]
            && !members.contains(&table)
            && private::dependencies(table)
                .iter()
                .all(|source| !grouped[source.index()] || members.contains(source))
    }) {
        members.push(table);
    }
    Ok(members)
}

/// Whether `table`'s own read takes its column from its read's group.
pub(super) fn resolves(context: &Context<'_>, table: TableId) -> bool {
    #[cfg(test)]
    if private::hooks::with(|hooks| hooks.group_old.unwrap_or(false)) {
        return false;
    }
    grouped(context.format.version())[table.index()]
}

/// Catalogued layout gaps are decoded even when a selection omits the table.
fn decodes(context: &Context<'_>, table: TableId) -> bool {
    let version = context.format.version();
    let change = &HISTORY[table.index()];
    change.present(version)
        && (change.table_gap(table, version).is_some()
            || context
                .selected
                .is_none_or(|selected| selected.contains(&table)))
}

type Outcome = (TableId, io::Result<Updates>);

/// A read's alias group: each member it resolved.
pub(super) struct Group {
    resolved: Vec<Outcome>,
    /// Why the group could not begin: every member's read reports it.
    failure: Option<io::Error>,
}

impl Group {
    fn failed(failure: io::Error) -> Self {
        Self {
            resolved: Vec::new(),
            failure: Some(failure),
        }
    }

    fn outcome(&self, table: TableId) -> Option<io::Result<Updates>> {
        if let Some(failure) = &self.failure {
            return Some(Err(rebuild(failure)));
        }
        let (_, result) = self.resolved.iter().find(|(member, _)| *member == table)?;
        Some(match result {
            Ok(column) => Ok(column.clone()),
            Err(error) => Err(rebuild(error)),
        })
    }
}

/// The first member resolves the group; cached members can remain outside its outcomes.
pub(super) fn column(
    context: &Context<'_>,
    table: TableId,
    check: &mut Check<'_>,
) -> Option<io::Result<Updates>> {
    let mut resolved = false;
    // Keep caller errors intact instead of rebuilding them as ordinary member failures.
    let mut stopped = None;
    let group = context.group.get_or_init(|| {
        resolved = true;
        match resolve(context, check) {
            Ok(group) => group,
            Err(error) => {
                stopped = Some(error);
                Group {
                    resolved: Vec::new(),
                    failure: None,
                }
            }
        }
    });
    if let Some(error) = stopped {
        return Some(Err(error));
    }
    if resolved && let Err(error) = enter(context, group, check) {
        return Some(Err(error));
    }
    group.outcome(table)
}

/// Source-only members enter the memo and cache under their own table keys.
fn enter(context: &Context<'_>, group: &Group, check: &mut Check<'_>) -> io::Result<()> {
    for (table, result) in &group.resolved {
        check.run()?;
        let Ok(column) = result else {
            continue;
        };
        if decodes(context, *table) || context.memo[table.index()].get().is_some() {
            continue;
        }
        let entered = context.replay.decoded_sections.read_updates(
            Key::new(context.replay, context.index, *table),
            context.hours.len(),
            false,
            || {
                #[cfg(test)]
                super::super::watched::note_decoded();
                #[cfg(any(test, feature = "test-support"))]
                super::super::test_work::decoded(context.index, *table, context.hours.len());
                check.run()?;
                Ok(column.clone())
            },
        );
        if check.failed() {
            entered?;
        } else if let Ok(updates) = entered {
            let _ = context.memo[table.index()].set(updates);
        }
    }
    Ok(())
}

fn resolve(context: &Context<'_>, check: &mut Check<'_>) -> io::Result<Group> {
    let tables = match members(context.format.version()) {
        Ok(tables) => tables,
        Err(failure) => return Ok(Group::failed(failure)),
    };
    let stored = match allocation::collect_vec(tables.iter().map(|_| OnceCell::new())) {
        Ok(stored) => stored,
        Err(failure) => return Ok(Group::failed(failure)),
    };
    let mut state = match State::new(context, &tables, &stored) {
        Ok(state) => state,
        Err(failure) => return Ok(Group::failed(failure)),
    };
    state.run(check)?;
    let group = match state.outcomes(check) {
        Ok(resolved) => Group {
            resolved,
            failure: None,
        },
        Err(failure) if check.failed() => return Err(failure),
        Err(failure) => Group::failed(failure),
    };
    #[cfg(test)]
    verify(context, &group);
    Ok(group)
}

/// Allocation failures and caller stops never retry through the original raw reader.
fn raw_column(
    context: &Context<'_>,
    table: TableId,
    check: &mut Check<'_>,
) -> io::Result<RawColumn> {
    raw_stage(context, table, true, check).or_else(|error| {
        if check.failed() || allocation::is_allocation_error(&error) {
            Err(error)
        } else {
            let _table = replay_data::measurement::table(table);
            replay_data::measurement::count(replay_data::measurement::Counter::DirectRetries, 1);
            raw_stage(context, table, false, check)
        }
    })
}

/// Use only ready cached columns here: waiting could deadlock on another group member.
fn decoded(context: &Context<'_>, table: TableId) -> Option<Updates> {
    if let Some(updates) = context.memo[table.index()].get() {
        return Some(updates.clone());
    }
    let updates = context.replay.decoded_sections.ready(
        Key::new(context.replay, context.index, table),
        context.hours.len(),
        false,
    )?;
    let _ = context.memo[table.index()].set(updates.clone());
    Some(updates)
}

/// Rebuilt errors must preserve kind, operating-system code and text.
fn rebuild(error: &io::Error) -> io::Error {
    if let Some(code) = error.raw_os_error() {
        return io::Error::from_raw_os_error(code);
    }
    let kind = error.kind();
    let text = |value: &dyn std::fmt::Display| crate::codec::try_format(format_args!("{value}"));
    match text(error) {
        Ok(message)
            if error.get_ref().is_none() && text(&kind).is_ok_and(|bare| bare == message) =>
        {
            io::Error::from(kind)
        }
        Ok(message) => io::Error::new(kind, message),
        Err(failure) => failure,
    }
}

/// This sentinel never escapes: outcomes substitute the first failed source's error.
fn stopped() -> io::Error {
    io::Error::from(io::ErrorKind::Other)
}

/// A member's keyframe and later marks, kept while its resolution reads them.
struct Stored {
    key: Vec<u8>,
    later: LaterSection,
}

/// The group's resolution in progress.
struct State<'c, 'm> {
    context: &'c Context<'c>,
    stored: &'m [OnceCell<Stored>],
    members: Vec<Member<'m>>,
    plains: Vec<Plain>,
}

struct Member<'m> {
    table: TableId,
    run: Run<'m>,
}

enum Run<'m> {
    /// Not needed so far.
    Idle,
    Decoded(Updates),
    /// Resolving, its next hour index first.
    Resolving(Box<Resolving<'m>>),
    /// Its hour is running.
    Busy,
    Done(io::Result<Updates>, Vec<Entry>),
}

struct Resolving<'m> {
    next: usize,
    raw: std::vec::IntoIter<TableUpdate>,
    resolution: TableResolution<'m>,
    entries: Vec<Entry>,
}

/// A source one member has loaded.
struct Entry {
    table: TableId,
    from: Origin,
    followed: Followed,
    shared: Option<bool>,
}

enum Origin {
    /// A plain source's group view: `plains[index]`.
    Plain(usize),
    /// A member's resolved table: `members[index]`.
    Member(usize),
    Column(Updates),
}

/// A plain source's column and the group's view of it.
struct Plain {
    table: TableId,
    column: Updates,
    view: Followed,
}

impl<'c, 'm> State<'c, 'm> {
    fn new(
        context: &'c Context<'c>,
        tables: &[TableId],
        stored: &'m [OnceCell<Stored>],
    ) -> io::Result<Self> {
        let mut members = allocation::vec(tables.len())?;
        members.extend(tables.iter().map(|&table| Member {
            table,
            run: Run::Idle,
        }));
        // Every source a member reads, at most.
        let plains = allocation::vec(
            tables
                .iter()
                .map(|&table| private::dependencies(table).len())
                .sum(),
        )?;
        Ok(Self {
            context,
            stored,
            members,
            plains,
        })
    }

    fn run(&mut self, check: &mut Check<'_>) -> io::Result<()> {
        let context = self.context;
        for member in 0..self.members.len() {
            check.run()?;
            if decodes(context, self.members[member].table) {
                self.start(member, check)?;
            }
        }
        for at in 0..context.hours.len() {
            check.run()?;
            for member in 0..self.members.len() {
                check.run()?;
                if matches!(&self.members[member].run, Run::Resolving(resolving) if resolving.next == at)
                {
                    // A member that stops keeps why.
                    if let Err(error) = self.hour(member, check)
                        && check.failed()
                    {
                        return Err(error);
                    }
                }
            }
        }
        for member in &mut self.members {
            check.run()?;
            member.run = match std::mem::replace(&mut member.run, Run::Idle) {
                Run::Resolving(resolving) => {
                    let Resolving {
                        resolution,
                        entries,
                        ..
                    } = *resolving;
                    let result = resolution.finish(context.complete).and_then(Updates::new);
                    Run::Done(result, released(entries))
                }
                run => run,
            };
        }
        Ok(())
    }

    fn start(&mut self, member: usize, check: &mut Check<'_>) -> io::Result<()> {
        check.run()?;
        let table = self.members[member].table;
        let run = match decoded(self.context, table) {
            Some(column) => Run::Decoded(column),
            None => match self.raw(member, check) {
                Ok(run) => run,
                Err(error) if check.failed() => return Err(error),
                Err(error) => Run::Done(Err(error), Vec::new()),
            },
        };
        self.members[member].run = run;
        Ok(())
    }

    fn raw(&mut self, member: usize, check: &mut Check<'_>) -> io::Result<Run<'m>> {
        let context = self.context;
        let table = self.members[member].table;
        let RawColumn {
            updates,
            key_marks,
            later,
        } = raw_column(context, table, check)?;
        let Some(key) = key_marks else {
            // Stored without marks, its raw column is its column.
            return Ok(Run::Done(Ok(Updates::new(updates)?), Vec::new()));
        };
        let stored: &'m [OnceCell<Stored>] = self.stored;
        let _ = stored[member].set(Stored { key, later });
        let stored = stored[member].get().expect("set just now");
        let resolution = TableResolution::new(
            table,
            context.hours.len(),
            &stored.key,
            stored.later.marks(),
        )?;
        Ok(Run::Resolving(allocation::boxed(Resolving {
            next: 0,
            raw: updates.into_iter(),
            resolution,
            entries: Vec::new(),
        })?))
    }

    /// Check keyframe refusals before columns; member failures are ordered in `outcomes`.
    fn source(&mut self, source: TableId, check: &mut Check<'_>) -> io::Result<Origin> {
        check.run()?;
        let context = self.context;
        if let Some(update) = keyframe_refusal(
            context.directory,
            context.format,
            context.replay.capture_mode,
            None,
            source,
        )? {
            return Ok(Origin::Column(Updates::repeat(
                update,
                context.hours.len(),
            )?));
        }
        let Some(member) = self
            .members
            .iter()
            .position(|member| member.table == source)
        else {
            if let Some(plain) = self.plains.iter().position(|plain| plain.table == source) {
                return Ok(Origin::Plain(plain));
            }
            allocation::reserve(&mut self.plains, 1)?;
            let column = table_updates_admitted(context, source, check)?;
            self.plains.push(Plain {
                table: source,
                column,
                view: Followed::default(),
            });
            return Ok(Origin::Plain(self.plains.len() - 1));
        };
        if matches!(self.members[member].run, Run::Idle) {
            self.start(member, check)?;
        }
        Ok(match &self.members[member].run {
            Run::Decoded(column) | Run::Done(Ok(column), _) => Origin::Column(column.clone()),
            _ => Origin::Member(member),
        })
    }

    fn pull(&mut self, member: usize, at: usize, check: &mut Check<'_>) -> io::Result<()> {
        loop {
            let next = match &self.members[member].run {
                Run::Resolving(resolving) => resolving.next,
                Run::Done(Err(_), _) => return Err(stopped()),
                Run::Busy => unreachable!("alias tables never read themselves"),
                _ => return Ok(()),
            };
            if next > at {
                return Ok(());
            }
            self.hour(member, check)?;
        }
    }

    /// Resolve `member`'s next hour; an error when it stops.
    fn hour(&mut self, member: usize, check: &mut Check<'_>) -> io::Result<()> {
        #[cfg(test)]
        let _step = super::super::check::Scope::enter(super::super::check::Step::Group(
            self.members[member].table,
            self.next(member),
        ));
        check.run()?;
        let table = self.members[member].table;
        let Run::Resolving(mut resolving) =
            std::mem::replace(&mut self.members[member].run, Run::Busy)
        else {
            unreachable!("only a resolving member resolves an hour");
        };
        let at = resolving.next;
        // A member reading a member that stopped stops with it.
        let result = if resolving.entries.iter().any(|entry| {
            matches!(entry.from, Origin::Member(source)
                if matches!(self.members[source].run, Run::Done(Err(_), _)))
        }) {
            Err(stopped())
        } else {
            let raw = resolving.raw.next().expect("one raw update an hour");
            #[cfg(test)]
            if private::hooks::with(|hooks| hooks.group_panic) == Some((table, at)) {
                panic!("{table:?} panics at hour index {at}");
            }
            let hours = self.context.hours;
            let mut sources = Sources {
                state: &mut *self,
                entries: &mut resolving.entries,
                table,
            };
            let result = resolving
                .resolution
                .hour(hours, at, raw, &mut sources, check);
            // The source views this hour read go, so each advances in place.
            resolving.resolution.release();
            result
        };
        match result {
            Ok(()) => {
                resolving.next += 1;
                self.members[member].run = Run::Resolving(resolving);
                Ok(())
            }
            Err(error) if check.failed() => Err(error),
            Err(error) => {
                self.members[member].run = Run::Done(Err(error), released(resolving.entries));
                Err(stopped())
            }
        }
    }

    fn next(&self, member: usize) -> usize {
        match &self.members[member].run {
            Run::Resolving(resolving) => resolving.next,
            _ => unreachable!("a member read on the group's view is resolving"),
        }
    }

    fn resolved(&self, member: usize) -> &SharedView {
        match &self.members[member].run {
            Run::Resolving(resolving) => resolving
                .resolution
                .resolved()
                .expect("a member read on the group's view resolved its hour"),
            _ => unreachable!("a member read on the group's view is resolving"),
        }
    }

    fn updates(&self, member: usize) -> &[TableUpdate] {
        match &self.members[member].run {
            Run::Resolving(resolving) => resolving.resolution.updates(),
            _ => unreachable!("a member read through its updates is resolving"),
        }
    }

    /// Preserve the first loaded source failure, not whichever member happened to finish first.
    fn outcomes(self, check: &mut Check<'_>) -> io::Result<Vec<Outcome>> {
        let mut resolved: Vec<Outcome> = allocation::vec(self.members.len())?;
        for member in self.members {
            check.run()?;
            let Run::Done(own, entries) = member.run else {
                continue;
            };
            let failed = entries.iter().find_map(|entry| match entry.from {
                Origin::Member(_) => resolved
                    .iter()
                    .find(|(table, _)| *table == entry.table)
                    .and_then(|(_, result)| result.as_ref().err()),
                _ => None,
            });
            let result = match failed {
                Some(error) => Err(rebuild(error)),
                None => own,
            };
            resolved.push((member.table, result));
        }
        Ok(resolved)
    }
}

/// A finished member's sources, without the views it kept of them.
fn released(mut entries: Vec<Entry>) -> Vec<Entry> {
    for entry in &mut entries {
        entry.followed.view = None;
    }
    entries
}

/// One member's sources while its hour resolves.
struct Sources<'s, 'c, 'm> {
    state: &'s mut State<'c, 'm>,
    entries: &'s mut Vec<Entry>,
    /// The member reading them.
    table: TableId,
}

impl Supply for Sources<'_, '_, '_> {
    fn loaded(&self) -> usize {
        self.entries.len()
    }

    fn table(&self, index: usize) -> TableId {
        self.entries[index].table
    }

    fn position(&self, source: TableId) -> Option<usize> {
        self.entries.iter().position(|entry| entry.table == source)
    }

    fn load(&mut self, source: TableId, _at: usize, check: &mut Check<'_>) -> io::Result<usize> {
        allocation::reserve(self.entries, 1)?;
        let from = self.state.source(source, check)?;
        self.entries.push(Entry {
            table: source,
            from,
            followed: Followed::default(),
            shared: None,
        });
        Ok(self.entries.len() - 1)
    }

    fn changed_keys(
        &mut self,
        index: usize,
        at: usize,
        relevant: &dyn Fn(u128, u64) -> bool,
        keys: &mut Vec<(u128, u64)>,
        check: &mut Check<'_>,
    ) -> io::Result<bool> {
        let entry = &self.entries[index];
        match &entry.from {
            Origin::Column(column) => entry
                .followed
                .changed_keys(column, at, relevant, keys, check),
            Origin::Plain(plain) => entry.followed.changed_keys(
                &self.state.plains[*plain].column,
                at,
                relevant,
                keys,
                check,
            ),
            Origin::Member(member) => {
                let member = *member;
                self.state.pull(member, at, check)?;
                self.entries[index].followed.changed_keys(
                    self.state.updates(member),
                    at,
                    relevant,
                    keys,
                    check,
                )
            }
        }
    }

    fn complete(&self, index: usize) -> bool {
        let entry = &self.entries[index];
        entry.shared.unwrap_or_else(|| entry.followed.complete())
    }

    fn members(&self, index: usize, keys: &[u128], out: &mut Vec<u128>) -> io::Result<()> {
        // A source read through its rows' members is always followed alone.
        self.entries[index].followed.members(keys, out)
    }

    fn catch_up(
        &mut self,
        index: usize,
        hours: &[Hour],
        at: usize,
        check: &mut Check<'_>,
    ) -> io::Result<()> {
        if self.entries[index].followed.source_at > at {
            return Ok(());
        }
        // Member joins need a private view retaining both the preceding and current rows.
        let alone = private::reads_members(self.table, self.entries[index].table);
        let (source, plain_column) = match &self.entries[index].from {
            Origin::Column(_) => (None, None),
            Origin::Plain(plain) => (None, Some(*plain)),
            Origin::Member(member) => (Some(*member), None),
        };
        if let Some(member) = source {
            self.state.pull(member, at, check)?;
            if !alone && self.state.next(member) == at + 1 {
                let complete = matches!(&**self.state.resolved(member), TableView::Complete(_));
                self.entries[index].share(at, complete);
                return Ok(());
            }
            let entry = &mut self.entries[index];
            entry.alone();
            return entry.followed.catch_up(
                entry.table,
                self.state.updates(member),
                hours,
                at,
                check,
            );
        }
        if let Some(plain) = plain_column {
            let shared = &mut self.state.plains[plain];
            if !alone && shared.view.source_at <= at + 1 {
                if let Err(error) =
                    shared
                        .view
                        .catch_up(shared.table, &shared.column, hours, at, check)
                {
                    // A failed advance loses its view; the next reader must rebuild from the first hour.
                    shared.view = Followed::default();
                    return Err(error);
                }
                let complete = shared.view.complete();
                self.entries[index].share(at, complete);
                return Ok(());
            }
            let entry = &mut self.entries[index];
            entry.alone();
            return entry
                .followed
                .catch_up(entry.table, &shared.column, hours, at, check);
        }
        let entry = &mut self.entries[index];
        let Origin::Column(column) = &entry.from else {
            unreachable!("a column source")
        };
        entry
            .followed
            .catch_up(entry.table, column, hours, at, check)
    }

    fn view(&self, index: usize) -> SharedView {
        let entry = &self.entries[index];
        if entry.shared.is_none() {
            return entry.followed.view();
        }
        match &entry.from {
            Origin::Plain(plain) => self.state.plains[*plain].view.view(),
            Origin::Member(member) => self.state.resolved(*member).clone(),
            Origin::Column(_) => unreachable!("a column is followed alone"),
        }
    }
}

impl Entry {
    /// Brought up to hour index `at` on the group's view.
    fn share(&mut self, at: usize, complete: bool) {
        self.followed = Followed {
            source_at: at + 1,
            view: None,
        };
        self.shared = Some(complete);
    }

    /// A previously shared source must restart from its first hour when followed alone.
    fn alone(&mut self) {
        if self.shared.take().is_some() {
            self.followed = Followed::default();
        }
    }
}

/// Verify each member independently against standalone resolution using the group's sources.
#[cfg(test)]
fn verify(context: &Context<'_>, group: &Group) {
    if !private::hooks::with(|hooks| hooks.verify_group) || group.failure.is_some() {
        return;
    }
    let in_group = grouped(context.format.version());
    for (table, result) in &group.resolved {
        let alone = raw_column(context, *table, &mut Check::disabled())
            .and_then(|raw| {
                private::resolve(
                    *table,
                    context.hours,
                    raw.updates,
                    raw.key_marks.as_deref(),
                    raw.later.marks(),
                    context.complete,
                    |source| {
                        if let Some(update) = keyframe_refusal(
                            context.directory,
                            context.format,
                            context.replay.capture_mode,
                            None,
                            source,
                        )? {
                            return Updates::repeat(update, context.hours.len());
                        }
                        if !in_group[source.index()] {
                            return table_updates_admitted(context, source, &mut Check::disabled());
                        }
                        group
                            .outcome(source)
                            .or_else(|| decoded(context, source).map(Ok))
                            .unwrap_or_else(|| {
                                panic!(
                                    "{table:?} alone reads {source:?}, which the group never read"
                                )
                            })
                    },
                )
            })
            .and_then(Updates::new);
        same(*table, &alone, result, "verify");
    }
}

/// Compare updates and exact error identity, including allocation classification.
#[cfg(test)]
fn same(table: TableId, alone: &io::Result<Updates>, grouped: &io::Result<Updates>, what: &str) {
    match (alone, grouped) {
        (Ok(alone), Ok(grouped)) => {
            assert_eq!(alone.len(), grouped.len(), "{table:?} {what}");
            for at in 0..alone.len() {
                assert_eq!(
                    **alone.get(at),
                    **grouped.get(at),
                    "{table:?} hour index {at} {what}"
                );
            }
        }
        (Err(alone), Err(grouped)) => {
            assert_eq!(alone.kind(), grouped.kind(), "{table:?} {what}");
            assert_eq!(
                alone.raw_os_error(),
                grouped.raw_os_error(),
                "{table:?} {what}"
            );
            assert_eq!(alone.to_string(), grouped.to_string(), "{table:?} {what}");
            assert_eq!(
                allocation::is_allocation_error(alone),
                allocation::is_allocation_error(grouped),
                "{table:?} {what}"
            );
        }
        (alone, grouped) => panic!("{table:?} {what}: alone {alone:?}, grouped {grouped:?}"),
    }
}
