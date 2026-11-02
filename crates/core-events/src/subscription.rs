use crate::{
    CellValue, Count, DependencyTarget, FieldId, Hour, IndexId, Interval, Key, ObservationId,
    ProductFieldId, ReadEvidence, RelationId, TableId, WindowBound,
};
use std::{fmt, io, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadRule {
    Acquired,
    Supplied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeclarationId(pub usize);

#[derive(Clone)]
pub enum WindowKeys {
    All,
    // Canonical keys in strictly increasing numeric order.
    Keys(Arc<[Arc<[u8]>]>),
    Where(Arc<dyn Fn(Key<'_>) -> bool + Send + Sync>),
}

impl WindowKeys {
    pub fn contains(&self, key: Key<'_>) -> bool {
        match self {
            Self::All => true,
            Self::Keys(keys) => keys
                .binary_search_by(|bytes| {
                    bytes
                        .len()
                        .cmp(&key.varint().len())
                        .then_with(|| bytes.iter().rev().cmp(key.varint().iter().rev()))
                })
                .is_ok(),
            Self::Where(predicate) => predicate(key),
        }
    }
    fn valid(&self) -> bool {
        match self {
            Self::Keys(keys) => {
                keys.iter().all(|bytes| Key::new(bytes).is_ok())
                    && keys
                        .windows(2)
                        .all(|pair| Key::new(&pair[0]).unwrap() < Key::new(&pair[1]).unwrap())
            }
            _ => true,
        }
    }
}

#[derive(Clone)]
pub struct WindowInput {
    pub declaration: DeclarationId,
    pub keys: WindowKeys,
    // Empty selects call facts only, never all of a declaration's fields.
    pub fields: Arc<[WindowField]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowField {
    Table(FieldId),
    Product(ProductFieldId),
}

#[derive(Clone)]
pub struct WindowOutput {
    pub keys: WindowKeys,
    pub fields: Arc<[ProductFieldId]>,
    pub target: DependencyTarget,
}

#[derive(Clone)]
pub struct ProvisionalRule {
    pub bound: WindowBound,
    // Keep initial state, transitions and call facts for these declarations from the segment's first hour.
    pub inputs: Arc<[WindowInput]>,
    pub outputs: Arc<[WindowOutput]>,
}

impl ProvisionalRule {
    pub fn keeps(
        &self,
        segment: Interval,
        hour: Hour,
        declaration: DeclarationId,
        key: Key<'_>,
        field: Option<WindowField>,
        closed_at: Option<Hour>,
    ) -> bool {
        self.bound
            .extent(segment, closed_at)
            .is_ok_and(|extent| extent.hours.intersection(Interval::hour(hour)).is_some())
            && self.inputs.iter().any(|input| {
                input.declaration == declaration
                    && input.keys.contains(key)
                    && field.is_none_or(|field| input.fields.contains(&field))
            })
    }
}

#[derive(Clone)]
pub enum Filter {
    All,
    // The reader owns the mutable set; handlers edit it only through CallContext.
    Keys,
    Where {
        field: FieldId,
        predicate: Arc<dyn Fn(CellValue<'_>) -> bool + Send + Sync>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FieldRef {
    pub table: TableId,
    pub field: FieldId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RelationStep {
    pub source: FieldRef,
    pub target: TableId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relation {
    pub id: RelationId,
    pub steps: Arc<[RelationStep]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LookupShape {
    Exact,
    Prefix { components: usize },
    Secondary(Relation),
    Previous,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexedStatus<'a> {
    Complete,
    Truncated {
        missing: Count<'a>,
    },
    Unavailable {
        reason: crate::Reason<'a>,
    },
    NotRecorded,
    ReadInPart,
    Refused {
        reason: crate::Reason<'a>,
        hours: crate::Interval,
    },
}

#[derive(Clone, Copy)]
pub struct TableMetadata<'a> {
    pub table: TableId,
    pub written_fields: &'a [u64],
    pub inserted: Count<'a>,
    pub deleted: Count<'a>,
    pub status: IndexedStatus<'a>,
    pub sample_hour: Option<Hour>,
    pub acquired: Option<ReadEvidence<'a>>,
    pub supplied: Option<ReadEvidence<'a>>,
}

#[derive(Clone, Copy)]
pub struct Metadata<'a> {
    pub hour: Hour,
    pub tables: &'a [TableMetadata<'a>],
    pub unavailable: Option<crate::ObservationUnknown<'a>>,
}

pub type ObservationPredicate = dyn for<'a> Fn(&Metadata<'a>) -> crate::Truth<'a> + Send + Sync;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ObserveWake {
    EachHour,
    Never,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ObservationSource {
    pub product: ProductId,
    pub observation: ObservationId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProductId(pub &'static str);

#[derive(Clone)]
pub struct Observation {
    pub product: ProductId,
    pub id: ObservationId,
    pub wake: ObserveWake,
    pub tables: Arc<[TableId]>,
    pub predicate: Arc<ObservationPredicate>,
}

#[derive(Clone)]
pub enum Declaration {
    Stream {
        table: TableId,
        fields: Arc<[FieldId]>,
        filter: Filter,
    },
    Watch {
        table: TableId,
        fields: Arc<[FieldId]>,
        filter: Filter,
    },
    WakeOnRead {
        table: TableId,
        rule: ReadRule,
    },
    Observe(Observation),
    ObservationLookup {
        source: ObservationSource,
    },
    Lookup {
        table: TableId,
        fields: Arc<[FieldId]>,
        shape: LookupShape,
    },
    Historical {
        index: IndexId,
        fields: Arc<[ProductFieldId]>,
    },
    Reverse(Relation),
    Previous {
        table: TableId,
        fields: Arc<[FieldId]>,
    },
    Timer,
    Ticks,
    Provisional(ProvisionalRule),
    Ordered,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubscriptionError {
    UnknownTable {
        table: u16,
    },
    UnknownField {
        table: TableId,
        field: FieldId,
    },
    UnsubscribedWhere {
        table: TableId,
        field: FieldId,
    },
    EmptyRelation {
        relation: RelationId,
    },
    DisconnectedRelation {
        relation: RelationId,
        step: usize,
        expected: TableId,
        actual: TableId,
    },
    ConflictingRelation {
        relation: RelationId,
    },
    SecondaryTarget {
        relation: RelationId,
        table: TableId,
        actual: TableId,
    },
    EmptyPrefix {
        table: TableId,
    },
    DuplicateObservation {
        observation: ObservationId,
    },
    InvalidObservationSource {
        source: ObservationSource,
    },
    ConflictingObservationSource {
        source: ObservationSource,
    },
    InvalidWindowRule {
        declaration: DeclarationId,
    },
    InvalidWindowInput {
        rule: DeclarationId,
        input: DeclarationId,
    },
    Capacity {
        owner: &'static str,
        needed: usize,
    },
}

impl fmt::Display for SubscriptionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SubscriptionError {}

pub fn table_id(id: u16) -> Result<TableId, SubscriptionError> {
    TableId::try_from(id).map_err(|_| SubscriptionError::UnknownTable { table: id })
}

fn field_valid(table: TableId, field: FieldId) -> Result<(), SubscriptionError> {
    if field >= table.schema().len() {
        Err(SubscriptionError::UnknownField { table, field })
    } else {
        Ok(())
    }
}

impl Relation {
    pub fn validate(&self) -> Result<(), SubscriptionError> {
        if self.steps.is_empty() {
            return Err(SubscriptionError::EmptyRelation { relation: self.id });
        }
        for (index, step) in self.steps.iter().enumerate() {
            field_valid(step.source.table, step.source.field)?;
            if index > 0 && self.steps[index - 1].target != step.source.table {
                return Err(SubscriptionError::DisconnectedRelation {
                    relation: self.id,
                    step: index,
                    expected: self.steps[index - 1].target,
                    actual: step.source.table,
                });
            }
        }
        Ok(())
    }
}

impl Declaration {
    fn holds_window_field(&self, field: WindowField) -> bool {
        match field {
            WindowField::Table(field) => self
                .fields()
                .is_some_and(|(_, fields)| fields.contains(&field)),
            WindowField::Product(field) => {
                matches!(self, Self::Historical { fields, .. } if fields.contains(&field))
            }
        }
    }
    fn fields(&self) -> Option<(TableId, &[FieldId])> {
        match self {
            Self::Stream { table, fields, .. }
            | Self::Watch { table, fields, .. }
            | Self::Lookup { table, fields, .. }
            | Self::Previous { table, fields } => Some((*table, fields)),
            _ => None,
        }
    }
    fn relation(&self) -> Option<&Relation> {
        match self {
            Self::Reverse(relation)
            | Self::Lookup {
                shape: LookupShape::Secondary(relation),
                ..
            } => Some(relation),
            _ => None,
        }
    }
    fn previous(&self) -> bool {
        matches!(
            self,
            Self::Previous { .. }
                | Self::Lookup {
                    shape: LookupShape::Previous,
                    ..
                }
        )
    }
}

#[derive(Clone)]
pub struct Subscription {
    declarations: Arc<[Declaration]>,
}

impl Subscription {
    pub fn new(declarations: Arc<[Declaration]>) -> Result<Self, SubscriptionError> {
        let subscription = Self { declarations };
        subscription.validate()?;
        Ok(subscription)
    }
    pub fn declarations(&self) -> &[Declaration] {
        &self.declarations
    }
    pub fn validate(&self) -> Result<(), SubscriptionError> {
        for (index, declaration) in self.declarations.iter().enumerate() {
            if let Some((table, fields)) = declaration.fields() {
                for &field in fields {
                    field_valid(table, field)?;
                }
            }
            match declaration {
                Declaration::Provisional(rule) => {
                    let rule_id = DeclarationId(index);
                    if matches!(rule.bound, WindowBound::PrefixHours(0 | u64::MAX))
                        || self.declarations.iter().any(|item| matches!(item, Declaration::Ordered))
                        || rule.outputs.iter().any(|output| !output.keys.valid()) {
                        return Err(SubscriptionError::InvalidWindowRule { declaration: rule_id });
                    }
                    for input in rule.inputs.iter() {
                        if !input.keys.valid() || !self.declarations.get(input.declaration.0).is_some_and(|item|
                            !matches!(item, Declaration::Provisional(_) | Declaration::Ordered)
                            && input.fields.iter().all(|&field| item.holds_window_field(field))) {
                            return Err(SubscriptionError::InvalidWindowInput { rule: rule_id, input: input.declaration });
                        }
                    }
                }
                Declaration::Stream { table, filter: Filter::Where { field, .. }, .. }
                | Declaration::Watch { table, filter: Filter::Where { field, .. }, .. } => {
                    field_valid(*table, *field)?;
                    let reference = FieldRef { table: *table, field: *field };
                    if !self.declarations.iter().any(|item| {
                        item.fields().is_some_and(|(t, fields)| t == *table && fields.contains(field))
                        || item.relation().is_some_and(|relation| relation.steps.iter().any(|step| step.source == reference))
                    }) { return Err(SubscriptionError::UnsubscribedWhere { table: *table, field: *field }); }
                }
                Declaration::Lookup { table, shape: LookupShape::Prefix { components: 0 }, .. } => return Err(SubscriptionError::EmptyPrefix { table: *table }),
                Declaration::Lookup { table, shape: LookupShape::Secondary(relation), .. } => {
                    relation.validate()?;
                    let actual = relation.steps[0].source.table;
                    if *table != actual { return Err(SubscriptionError::SecondaryTarget { relation: relation.id, table: *table, actual }); }
                }
                Declaration::Observe(observation) => {
                    if self.declarations[..index].iter().any(|item| matches!(item, Declaration::Observe(other) if other.id == observation.id)) {
                        return Err(SubscriptionError::DuplicateObservation { observation: observation.id });
                    }
                }
                _ => {}
            }
            if let Some(relation) = declaration.relation() {
                relation.validate()?;
                for other in self.declarations[..index]
                    .iter()
                    .filter_map(Declaration::relation)
                {
                    if other.id == relation.id && other.steps != relation.steps {
                        return Err(SubscriptionError::ConflictingRelation {
                            relation: relation.id,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    pub fn union<'a>(
        families: &[Subscription],
        storage: &'a mut UnionStorage,
    ) -> Result<SubscriptionUnion<'a>, SubscriptionError> {
        Self::union_with_observations(families, &[], storage)
    }

    // Foreign never-wake predicates are evaluated locally over metadata, without co-grouping their producer.
    pub fn union_with_observations<'a>(
        families: &[Subscription],
        foreign: &[Observation],
        storage: &'a mut UnionStorage,
    ) -> Result<SubscriptionUnion<'a>, SubscriptionError> {
        storage.clear();
        for (family, subscription) in families.iter().enumerate() {
            subscription.validate()?;
            let family = FamilyId(family);
            for (local, declaration) in subscription.declarations().iter().enumerate() {
                if let Some((table, fields)) = declaration.fields() {
                    put(&mut storage.tables, table, "tables")?;
                    for &field in fields {
                        put(
                            &mut storage.fields,
                            HeldField {
                                field: FieldRef { table, field },
                                previous: declaration.previous(),
                            },
                            "fields",
                        )?;
                    }
                }
                match declaration {
                    Declaration::WakeOnRead { table, .. } => {
                        put(&mut storage.tables, *table, "tables")?
                    }
                    Declaration::Historical { index, fields } => {
                        for &field in fields.iter() {
                            put(
                                &mut storage.historical,
                                HistoricalField {
                                    index: *index,
                                    field,
                                },
                                "historical",
                            )?;
                        }
                    }
                    Declaration::Observe(observation) => {
                        let shared = share_observation(storage, observation)?;
                        put(
                            &mut storage.observation_bindings,
                            ObservationBinding {
                                family,
                                local: observation.id,
                                shared,
                                wake: observation.wake,
                            },
                            "observation bindings",
                        )?;
                    }
                    Declaration::Provisional(rule) => put(
                        &mut storage.windows,
                        WindowBinding {
                            family,
                            local: DeclarationId(local),
                            rule: rule.clone(),
                        },
                        "windows",
                    )?,
                    _ => {}
                }
                if let Some(relation) = declaration.relation() {
                    for step in relation.steps.iter() {
                        put(&mut storage.tables, step.source.table, "tables")?;
                        put(&mut storage.tables, step.target, "tables")?;
                        put(
                            &mut storage.fields,
                            HeldField {
                                field: step.source,
                                previous: false,
                            },
                            "fields",
                        )?;
                    }
                    let shared = match storage
                        .relations
                        .iter()
                        .position(|other| other.steps == relation.steps)
                    {
                        Some(index) => index,
                        None => {
                            let index = storage.relations.len();
                            put(
                                &mut storage.relations,
                                SharedRelation {
                                    steps: relation.steps.clone(),
                                },
                                "relations",
                            )?;
                            index
                        }
                    };
                    put(
                        &mut storage.relation_bindings,
                        RelationBinding {
                            family,
                            local: relation.id,
                            shared: SharedRelationId(shared),
                        },
                        "relation bindings",
                    )?;
                }
            }
        }
        for (family, subscription) in families.iter().enumerate() {
            for (local, declaration) in subscription.declarations().iter().enumerate() {
                if let Declaration::ObservationLookup { source } = declaration {
                    let mut definitions = families
                        .iter()
                        .flat_map(|family| family.declarations())
                        .filter_map(|declaration| match declaration {
                            Declaration::Observe(observation) => Some(observation),
                            _ => None,
                        })
                        .chain(foreign.iter())
                        .filter(|observation| {
                            observation.product == source.product
                                && observation.id == source.observation
                        });
                    let definition = definitions
                        .next()
                        .ok_or(SubscriptionError::InvalidObservationSource { source: *source })?;
                    let mut never = definition.wake == ObserveWake::Never;
                    for other in definitions {
                        if definition.tables != other.tables
                            || !Arc::ptr_eq(&definition.predicate, &other.predicate)
                        {
                            return Err(SubscriptionError::ConflictingObservationSource {
                                source: *source,
                            });
                        }
                        never |= other.wake == ObserveWake::Never;
                    }
                    if !never {
                        return Err(SubscriptionError::InvalidObservationSource {
                            source: *source,
                        });
                    }
                    let shared = share_observation(storage, definition)?;
                    put(
                        &mut storage.observation_lookups,
                        ObservationLookupBinding {
                            family: FamilyId(family),
                            local: DeclarationId(local),
                            shared,
                        },
                        "observation lookups",
                    )?;
                }
            }
        }
        storage.tables.sort_unstable();
        storage.tables.dedup();
        storage.fields.sort_unstable_by_key(|held| held.field);
        storage.fields.dedup_by(|later, earlier| {
            if later.field == earlier.field {
                earlier.previous |= later.previous;
                true
            } else {
                false
            }
        });
        storage.historical.sort_unstable();
        storage.historical.dedup();
        storage.relation_bindings.sort_unstable();
        storage.relation_bindings.dedup();
        Ok(SubscriptionUnion {
            tables: &storage.tables,
            fields: &storage.fields,
            historical: &storage.historical,
            relations: &storage.relations,
            relation_bindings: &storage.relation_bindings,
            observations: &storage.observations,
            observation_bindings: &storage.observation_bindings,
            observation_lookups: &storage.observation_lookups,
            windows: &storage.windows,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeldField {
    pub field: FieldRef,
    pub previous: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct HistoricalField {
    pub index: IndexId,
    pub field: ProductFieldId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FamilyId(pub usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SharedRelationId(pub usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SharedObservationId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RelationBinding {
    pub family: FamilyId,
    pub local: RelationId,
    pub shared: SharedRelationId,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ObservationBinding {
    pub family: FamilyId,
    pub local: ObservationId,
    pub shared: SharedObservationId,
    pub wake: ObserveWake,
}

impl ObservationBinding {
    pub fn observed(self, truth: crate::Truth<'_>) -> Option<crate::Event<'static>> {
        (truth == crate::Truth::Holds && self.wake == ObserveWake::EachHour).then_some(
            crate::Event::Observed {
                observation: self.local,
            },
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObservationLookupBinding {
    pub family: FamilyId,
    pub local: DeclarationId,
    pub shared: SharedObservationId,
}

pub struct WindowBinding {
    pub family: FamilyId,
    pub local: DeclarationId,
    pub rule: ProvisionalRule,
}

pub struct SharedRelation {
    pub steps: Arc<[RelationStep]>,
}
pub struct SharedObservation {
    pub tables: Arc<[TableId]>,
    pub predicate: Arc<ObservationPredicate>,
}

impl SharedObservation {
    pub fn evaluate<'a>(&self, metadata: &Metadata<'a>) -> crate::Truth<'a> {
        metadata
            .unavailable
            .map_or_else(|| (self.predicate)(metadata), crate::Truth::Unknown)
    }
}

fn share_observation(
    storage: &mut UnionStorage,
    observation: &Observation,
) -> Result<SharedObservationId, SubscriptionError> {
    if let Some(index) = storage.observations.iter().position(|other| {
        other.tables == observation.tables && Arc::ptr_eq(&other.predicate, &observation.predicate)
    }) {
        return Ok(SharedObservationId(index));
    }
    for &table in observation.tables.iter() {
        put(&mut storage.tables, table, "tables")?;
    }
    let index = storage.observations.len();
    put(
        &mut storage.observations,
        SharedObservation {
            tables: observation.tables.clone(),
            predicate: observation.predicate.clone(),
        },
        "observations",
    )?;
    Ok(SharedObservationId(index))
}

// Capacities count raw insertions before deduplication, including each relation/observation binding.
#[derive(Clone, Copy, Default)]
pub struct UnionCapacity {
    pub tables: usize,
    pub fields: usize,
    pub historical: usize,
    pub relations: usize,
    pub observations: usize,
    pub observation_lookups: usize,
    pub windows: usize,
}

pub struct UnionStorage {
    tables: Vec<TableId>,
    fields: Vec<HeldField>,
    historical: Vec<HistoricalField>,
    relations: Vec<SharedRelation>,
    observations: Vec<SharedObservation>,
    relation_bindings: Vec<RelationBinding>,
    observation_bindings: Vec<ObservationBinding>,
    observation_lookups: Vec<ObservationLookupBinding>,
    windows: Vec<WindowBinding>,
}

impl UnionCapacity {
    pub fn bytes(self) -> io::Result<usize> {
        [
            (self.tables, size_of::<TableId>()),
            (self.fields, size_of::<HeldField>()),
            (self.historical, size_of::<HistoricalField>()),
            (
                self.observation_lookups,
                size_of::<ObservationLookupBinding>(),
            ),
            (self.windows, size_of::<WindowBinding>()),
            (
                self.relations,
                size_of::<SharedRelation>() + size_of::<RelationBinding>(),
            ),
            (
                self.observations,
                size_of::<SharedObservation>() + size_of::<ObservationBinding>(),
            ),
        ]
        .into_iter()
        .try_fold(0usize, |sum, (n, width)| {
            n.checked_mul(width)
                .and_then(|bytes| sum.checked_add(bytes))
                .ok_or_else(|| io::ErrorKind::OutOfMemory.into())
        })
    }
}

impl UnionStorage {
    pub fn new(capacity: UnionCapacity, allowance: usize) -> io::Result<Self> {
        if capacity.bytes()? > allowance {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        fn reserve<T>(count: usize) -> io::Result<Vec<T>> {
            let mut values = Vec::new();
            values
                .try_reserve_exact(count)
                .map_err(|_| io::ErrorKind::OutOfMemory)?;
            Ok(values)
        }
        Ok(Self {
            tables: reserve(capacity.tables)?,
            fields: reserve(capacity.fields)?,
            historical: reserve(capacity.historical)?,
            relations: reserve(capacity.relations)?,
            observations: reserve(capacity.observations)?,
            relation_bindings: reserve(capacity.relations)?,
            observation_bindings: reserve(capacity.observations)?,
            observation_lookups: reserve(capacity.observation_lookups)?,
            windows: reserve(capacity.windows)?,
        })
    }
    fn clear(&mut self) {
        self.tables.clear();
        self.fields.clear();
        self.historical.clear();
        self.relations.clear();
        self.observations.clear();
        self.relation_bindings.clear();
        self.observation_bindings.clear();
        self.observation_lookups.clear();
        self.windows.clear();
    }
}

fn put<T>(values: &mut Vec<T>, value: T, owner: &'static str) -> Result<(), SubscriptionError> {
    if values.len() == values.capacity() {
        return Err(SubscriptionError::Capacity {
            owner,
            needed: values.len() + 1,
        });
    }
    values.push(value);
    Ok(())
}

pub struct SubscriptionUnion<'a> {
    pub tables: &'a [TableId],
    pub fields: &'a [HeldField],
    pub historical: &'a [HistoricalField],
    pub relations: &'a [SharedRelation],
    pub relation_bindings: &'a [RelationBinding],
    pub observations: &'a [SharedObservation],
    pub observation_bindings: &'a [ObservationBinding],
    pub observation_lookups: &'a [ObservationLookupBinding],
    pub windows: &'a [WindowBinding],
}
