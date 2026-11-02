use crate::{
    BoundarySummary, CallContext, CellValue, Coverage, Declaration, DeclarationId,
    DependencyTarget, DictionaryString, FieldId, FilterInsertion, Hour, IndexId, Interval, Key,
    Lookup, ObservationStream, ProductCell, ProductFieldId, ProvisionalRule, ProvisionalWindow,
    Publication, PublicationRecord, RelationId, Row, Rows, Subscription, TableId, Timer,
    WindowClosures, WindowDependencies, WindowExtent, WindowField,
};
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplayKey<'a> {
    pub key: Key<'a>,
    pub extent: WindowExtent,
}

#[derive(Clone, Copy)]
pub struct ReplayScope<'a> {
    segment: Interval,
    rule: DeclarationId,
    definition: &'a ProvisionalRule,
    subscription: &'a Subscription,
    keys: &'a [ReplayKey<'a>],
    dependencies: &'a dyn WindowDependencies,
    hours: Interval,
}

impl<'a> ReplayScope<'a> {
    pub fn new(
        segment: Interval,
        rule: DeclarationId,
        keys: &'a [ReplayKey<'a>],
        dependencies: &'a dyn WindowDependencies,
        subscription: &'a Subscription,
        closures: &dyn WindowClosures,
    ) -> io::Result<Self> {
        let Some(Declaration::Provisional(definition)) = subscription.declarations().get(rule.0)
        else {
            return Err(io::ErrorKind::InvalidInput.into());
        };
        if keys.is_empty() || keys.windows(2).any(|pair| pair[0].key >= pair[1].key) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let mut last = segment.first();
        for item in keys {
            if item.extent
                != definition
                    .bound
                    .extent(segment, closures.closed_at(rule, item.key)?)?
                || !definition
                    .inputs
                    .iter()
                    .any(|input| input.keys.contains(item.key))
            {
                return Err(io::ErrorKind::InvalidInput.into());
            }
            last = last.max(item.extent.hours.last());
        }
        let scope = Self {
            segment,
            rule,
            definition,
            subscription,
            keys,
            dependencies,
            hours: Interval::new(segment.first(), last).unwrap(),
        };
        dependencies.visit(&mut |dependency| {
            let item = scope
                .key(dependency.key)
                .ok_or(io::ErrorKind::InvalidInput)?;
            if dependency.hours.intersection(item.extent.hours) != Some(dependency.hours)
                || !definition.outputs.iter().any(|output| {
                    output.target == dependency.target
                        && output.keys.contains(dependency.key)
                        && output.fields.contains(&dependency.field)
                })
            {
                return Err(io::ErrorKind::InvalidInput.into());
            }
            Ok(())
        })?;
        Ok(scope)
    }
    pub fn keys(self) -> &'a [ReplayKey<'a>] {
        self.keys
    }
    pub fn segment(self) -> Interval {
        self.segment
    }
    pub fn rule(self) -> DeclarationId {
        self.rule
    }
    pub fn hours(self) -> Interval {
        self.hours
    }
    pub fn dependencies(self) -> &'a dyn WindowDependencies {
        self.dependencies
    }
    pub fn key(self, key: Key<'_>) -> Option<ReplayKey<'a>> {
        self.keys
            .binary_search_by_key(&key, |item| item.key)
            .ok()
            .map(|slot| self.keys[slot])
    }
    pub fn active(self, hour: Hour, key: Key<'_>) -> bool {
        self.key(key).is_some_and(|item| {
            item.extent
                .hours
                .intersection(Interval::hour(hour))
                .is_some()
        })
    }
    pub fn keeps(
        self,
        hour: Hour,
        declaration: DeclarationId,
        key: Key<'_>,
        field: Option<WindowField>,
    ) -> bool {
        self.active(hour, key)
            && self.definition.inputs.iter().any(|input| {
                input.declaration == declaration
                    && input.keys.contains(key)
                    && field.is_none_or(|field| input.fields.contains(&field))
            })
    }
    pub fn timer_due(self, hour: Hour, timer: Timer<'_>) -> bool {
        timer.hour <= hour
            && self.active(hour, timer.key)
            && self.definition.inputs.iter().any(|input| {
                matches!(
                    self.subscription.declarations()[input.declaration.0],
                    Declaration::Timer
                ) && input.keys.contains(timer.key)
            })
    }
    pub fn begin(
        self,
        family: &mut dyn crate::Family,
        incoming: &dyn BoundarySummary,
        opens: bool,
        context: &mut dyn CallContext,
        out: &mut ReplayPublication<'_>,
    ) -> io::Result<()> {
        out.require_scope(self)?;
        if context.hour() != self.hours.first() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let summary = ScopedSummary {
            scope: self,
            summary: incoming,
        };
        family.begin(
            &crate::Start {
                hours: self.segment,
                opens,
                boundary: crate::StartingBoundary::Replay {
                    summary: &summary,
                    scope: self,
                },
            },
            &mut ReplayContext::new(self, context),
            out,
        )
    }
    pub fn hour(
        self,
        family: &mut dyn crate::Family,
        at: &crate::At<'_>,
        context: &mut dyn CallContext,
        out: &mut ReplayPublication<'_>,
    ) -> io::Result<()> {
        out.require_scope(self)?;
        if context.hour() != at.hour || self.hours.intersection(Interval::hour(at.hour)).is_none() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        for event in at.events {
            match *event {
                crate::Event::Changed {
                    table, key, field, ..
                }
                | crate::Event::Unknown {
                    table, key, field, ..
                } => self.table_field(at.hour, table, key, field)?,
                crate::Event::Appeared {
                    table, key, fields, ..
                } => {
                    self.require_key(at.hour, key)?;
                    for &field in fields.fields() {
                        self.table_field(at.hour, table, key, field)?;
                    }
                }
                crate::Event::Gone { key, .. } => self.require_key(at.hour, key)?,
                crate::Event::Joined { value, member, .. }
                | crate::Event::Left { value, member, .. } => {
                    if !self.active(at.hour, value) && !self.active(at.hour, member) {
                        return Err(io::ErrorKind::InvalidInput.into());
                    }
                }
                crate::Event::Tick { due, .. } => {
                    if due.iter().any(|&timer| !self.timer_due(at.hour, timer)) {
                        return Err(io::ErrorKind::InvalidInput.into());
                    }
                }
                _ => {}
            }
        }
        family.hour(at, &mut ReplayContext::new(self, context), out)
    }
    pub fn end<'f>(
        self,
        family: &'f mut dyn crate::Family,
        at: &crate::End<'_>,
        context: &mut dyn CallContext,
        out: &mut ReplayPublication<'_>,
    ) -> io::Result<crate::Boundary<'f>> {
        out.require_scope(self)?;
        if at.at.hour != self.hours.last() || context.hour() != at.at.hour {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        family.end(at, &mut ReplayContext::new(self, context), out)
    }
    fn require_key(self, hour: Hour, key: Key<'_>) -> io::Result<()> {
        if self.active(hour, key) {
            Ok(())
        } else {
            Err(io::ErrorKind::InvalidInput.into())
        }
    }
    fn table_field(
        self,
        hour: Hour,
        table: TableId,
        key: Key<'_>,
        field: FieldId,
    ) -> io::Result<()> {
        self.require_key(hour, key)?;
        if self.definition.inputs.iter().any(|input| input.keys.contains(key)
            && input.fields.contains(&WindowField::Table(field))
            && matches!(&self.subscription.declarations()[input.declaration.0],
                Declaration::Stream { table: t, .. } | Declaration::Watch { table: t, .. }
                | Declaration::Lookup { table: t, .. } | Declaration::Previous { table: t, .. } if *t == table)) {
            Ok(())
        } else { Err(io::ErrorKind::InvalidInput.into()) }
    }
}

pub struct ScopedSummary<'a> {
    pub scope: ReplayScope<'a>,
    pub summary: &'a dyn BoundarySummary,
}
impl BoundarySummary for ScopedSummary<'_> {
    fn value(&self, key: Key<'_>, field: ProductFieldId) -> io::Result<ProductCell<'_>> {
        if self.scope.key(key).is_none() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        self.summary.value(key, field)
    }
    fn bytes(&self) -> u64 {
        size_of::<Self>() as u64
    }
}

pub struct ReplayPublication<'a> {
    scope: ReplayScope<'a>,
    out: &'a mut dyn Publication,
}
impl<'a> ReplayPublication<'a> {
    pub fn new(scope: ReplayScope<'a>, out: &'a mut dyn Publication) -> Self {
        Self { scope, out }
    }
    fn require_scope(&self, scope: ReplayScope<'_>) -> io::Result<()> {
        if self.scope.segment == scope.segment
            && self.scope.rule == scope.rule
            && std::ptr::eq(self.scope.keys, scope.keys)
            && std::ptr::eq(self.scope.dependencies, scope.dependencies)
            && std::ptr::eq(self.scope.subscription, scope.subscription)
        {
            Ok(())
        } else {
            Err(io::ErrorKind::InvalidInput.into())
        }
    }
}
impl Publication for ReplayPublication<'_> {
    fn publish(&mut self, record: PublicationRecord<'_>) -> io::Result<()> {
        self.scope.require_key(record.hour, record.key)?;
        if !self
            .scope
            .dependencies
            .hours(record.key, record.field, DependencyTarget::Publication)?
            .is_some_and(|hours| hours.intersection(Interval::hour(record.hour)).is_some())
        {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        self.out.publish(record)
    }
    // Coverage belongs to the full pass, and replay cannot create new provisional windows.
    fn coverage(&mut self, _: Coverage<'_>) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn provisional(&mut self, _: ProvisionalWindow<'_>) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

pub struct ReplayContext<'a> {
    scope: ReplayScope<'a>,
    inner: &'a mut dyn CallContext,
}
impl<'a> ReplayContext<'a> {
    // The wrapped reader owns only this batch's retained inputs and timers.
    pub fn new(scope: ReplayScope<'a>, inner: &'a mut dyn CallContext) -> Self {
        Self { scope, inner }
    }
    fn declaration(&self, declaration: DeclarationId) -> io::Result<()> {
        if self
            .scope
            .definition
            .inputs
            .iter()
            .any(|input| input.declaration == declaration)
        {
            Ok(())
        } else {
            Err(io::ErrorKind::InvalidInput.into())
        }
    }
}
impl CallContext for ReplayContext<'_> {
    fn hour(&self) -> Hour {
        self.inner.hour()
    }
    fn observation(&self, declaration: DeclarationId) -> io::Result<&dyn ObservationStream> {
        self.declaration(declaration)?;
        self.inner.observation(declaration)
    }
    fn exact(&self, table: TableId, key: Key<'_>, field: FieldId) -> io::Result<CellValue<'_>> {
        self.scope.table_field(self.hour(), table, key, field)?;
        self.inner.exact(table, key, field)
    }
    fn previous(&self, table: TableId, key: Key<'_>, field: FieldId) -> io::Result<CellValue<'_>> {
        self.scope.table_field(self.hour(), table, key, field)?;
        self.inner.previous(table, key, field)
    }
    fn historical(
        &self,
        index: IndexId,
        key: Key<'_>,
        field: ProductFieldId,
        hour: Hour,
    ) -> io::Result<ProductCell<'_>> {
        self.scope.require_key(self.hour(), key)?;
        if !self.scope.definition.inputs.iter().any(|input| input.keys.contains(key)
            && input.fields.contains(&WindowField::Product(field))
            && matches!(self.scope.subscription.declarations()[input.declaration.0], Declaration::Historical { index: i, .. } if i == index)) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        self.inner.historical(index, key, field, hour)
    }
    fn prefix(
        &self,
        declaration: DeclarationId,
        components: &[Key<'_>],
        visitor: &mut dyn FnMut(Row<'_>) -> io::Result<()>,
    ) -> io::Result<Lookup<'_, ()>> {
        self.declaration(declaration)?;
        self.inner.prefix(declaration, components, &mut |row| {
            if self.scope.keeps(self.hour(), declaration, row.key, None) {
                visitor(row)?;
            }
            Ok(())
        })
    }
    fn secondary(
        &self,
        declaration: DeclarationId,
        value: Key<'_>,
        visitor: &mut dyn FnMut(Row<'_>) -> io::Result<()>,
    ) -> io::Result<Lookup<'_, ()>> {
        self.declaration(declaration)?;
        self.inner.secondary(declaration, value, &mut |row| {
            if self.scope.keeps(self.hour(), declaration, row.key, None) {
                visitor(row)?;
            }
            Ok(())
        })
    }
    fn members(
        &self,
        relation: RelationId,
        value: Key<'_>,
        visitor: &mut dyn FnMut(Key<'_>) -> io::Result<()>,
    ) -> io::Result<Lookup<'_, ()>> {
        self.scope.require_key(self.hour(), value)?;
        self.inner.members(relation, value, visitor)
    }
    fn snapshot(&self, _: TableId) -> io::Result<Lookup<'_, &dyn Rows>> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn take_watch(&mut self, declaration: DeclarationId) -> io::Result<bool> {
        self.declaration(declaration)?;
        self.inner.take_watch(declaration)
    }
    fn schedule(&mut self, timer: Timer<'_>) -> io::Result<()> {
        self.scope.require_key(self.hour(), timer.key)?;
        if !self.scope.definition.inputs.iter().any(|input| {
            matches!(
                self.scope.subscription.declarations()[input.declaration.0],
                Declaration::Timer
            ) && input.keys.contains(timer.key)
        }) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        if !self.scope.timer_due(timer.hour.max(self.hour()), timer) {
            self.inner.cancel(timer.id)?;
            return Ok(());
        }
        self.inner.schedule(timer)
    }
    fn cancel(&mut self, id: Key<'_>) -> io::Result<bool> {
        self.inner.cancel(id)
    }
    fn close_window(&mut self, rule: DeclarationId, key: Key<'_>) -> io::Result<()> {
        self.scope.require_key(self.hour(), key)?;
        if rule != self.scope.rule {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        self.inner.close_window(rule, key)
    }
    fn insert_filter(
        &mut self,
        declaration: DeclarationId,
        key: Key<'_>,
    ) -> io::Result<FilterInsertion<'_>> {
        self.declaration(declaration)?;
        self.scope.require_key(self.hour(), key)?;
        self.inner.insert_filter(declaration, key)
    }
    fn remove_filter(&mut self, declaration: DeclarationId, key: Key<'_>) -> io::Result<bool> {
        self.declaration(declaration)?;
        self.scope.require_key(self.hour(), key)?;
        self.inner.remove_filter(declaration, key)
    }
    fn dictionary(&self, id: u32) -> io::Result<Lookup<'_, &str>> {
        self.inner.dictionary(id)
    }
    fn dictionary_additions(&self) -> &[DictionaryString<'_>] {
        self.inner.dictionary_additions()
    }
}
