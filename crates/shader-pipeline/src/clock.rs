use naga::{
    AddressSpace, BinaryOperator, Block, Expression, Function, Handle, Literal, MathFunction,
    Module, Scalar, Statement, TypeInner, UnaryOperator, VectorSize,
    valid::{FunctionInfo, ModuleInfo},
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub type Slot = (u32, u32);

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Bounded {
    pub terms: Vec<(Slot, f64)>,
    pub constant: f64,
    pub low: Option<f64>,
    pub high: Option<f64>,
    pub open_low: bool,
    pub slack: f64,
    pub per: Vec<(Slot, f64)>,
    pub reach: f64,
    pub peak: f64,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Clock {
    pub direct: Vec<(u32, u32, u32)>,
    pub bounded: Vec<Bounded>,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Settling {
    pub after: f32,
    pub per_clock: f32,
    pub limit: f32,
}

impl Settling {
    pub const NOW: Self = Self::after(0.0);

    pub const fn after(seconds: f32) -> Self {
        Self {
            after: seconds,
            per_clock: 0.0,
            limit: f32::INFINITY,
        }
    }

    pub fn reached(&self, elapsed: f32, clock: f32) -> bool {
        clock <= self.limit && elapsed >= self.after + self.per_clock * clock
    }

    pub fn and(self, other: Self) -> Self {
        Self {
            after: self.after.max(other.after),
            per_clock: self.per_clock.max(other.per_clock),
            limit: self.limit.min(other.limit),
        }
    }
}

fn up(value: f64) -> f32 {
    let rounded = value as f32;
    if f64::from(rounded) < value {
        rounded.next_up()
    } else {
        rounded
    }
}

fn down(value: f64) -> f32 {
    let rounded = value as f32;
    if f64::from(rounded) > value {
        rounded.next_down()
    } else {
        rounded
    }
}

impl Clock {
    pub fn settles(&self, time: Slot, since: Slot) -> Option<Settling> {
        let covers = |&(binding, start, end): &(u32, u32, u32)| {
            binding == time.0 && start < time.1 + 4 && time.1 < end
        };
        if self.direct.iter().any(covers) {
            return None;
        }
        let mut settling = Settling::NOW;
        for bounded in &self.bounded {
            let Some(&(_, k)) = bounded.terms.iter().find(|(slot, _)| *slot == time) else {
                if bounded.per.iter().any(|&(slot, _)| slot == time) {
                    return None;
                }
                continue;
            };
            let exact = bounded.terms.len() == 2
                && bounded
                    .terms
                    .iter()
                    .any(|&(slot, j)| slot == since && j == -k);
            if !exact || k == 0.0 {
                return None;
            }
            if bounded
                .per
                .iter()
                .any(|&(slot, _)| slot != time && slot != since)
            {
                return None;
            }
            let bound = if k > 0.0 { bounded.high } else { bounded.low }?;
            let after = (bound - bounded.constant) / k + bounded.slack / k.abs();
            let per_clock: f64 = bounded.per.iter().map(|&(_, per)| per).sum::<f64>() / k.abs();
            if !(after.is_finite() && per_clock.is_finite()) {
                return None;
            }
            let mut at = up(after);
            if k < 0.0 && bounded.open_low {
                at = at.next_up();
            }
            let limit = if bounded.peak > 0.0 {
                (LARGEST - bounded.reach) / bounded.peak
            } else {
                f64::INFINITY
            };
            if !(limit >= 0.0) {
                return None;
            }
            settling = settling.and(Settling {
                after: at.max(0.0),
                per_clock: up(per_clock),
                limit: down(limit),
            });
        }
        Some(settling)
    }
}

pub fn analyse(module: &Module, info: &ModuleInfo) -> Clock {
    analyse_with_zeros(module, info, &[], None)
}

pub fn reads_with_zeros(
    wgsl: &str,
    inputs: &[Slot],
    zeros: &[Slot],
    zero_channel: Option<Slot>,
) -> Option<bool> {
    let module = naga::front::wgsl::parse_str(wgsl).ok()?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .ok()?;
    let clock = analyse_with_zeros(&module, &info, zeros, zero_channel);
    Some(inputs.iter().any(|&(binding, byte)| {
        clock
            .direct
            .iter()
            .any(|&(b, start, end)| b == binding && start <= byte && byte < end)
            || clock.bounded.iter().any(|bound| {
                bound
                    .terms
                    .iter()
                    .chain(&bound.per)
                    .any(|&(slot, _)| slot == (binding, byte))
            })
    }))
}

fn analyse_with_zeros(
    module: &Module,
    info: &ModuleInfo,
    zeros: &[Slot],
    zero_channel: Option<Slot>,
) -> Clock {
    let mut analysis = Analysis {
        module,
        info,
        zeros,
        zero_channel,
        vars: HashMap::new(),
        args: HashMap::new(),
        results: HashMap::new(),
        contexts: HashMap::new(),
        loops: HashMap::new(),
        output: Deps::default(),
        changed: false,
        unsupported: false,
    };
    const PASSES: usize = 64;
    let mut converged = false;
    for _ in 0..PASSES {
        analysis.changed = false;
        analysis.function(None);
        for (handle, _) in module.functions.iter() {
            analysis.function(Some(handle));
        }
        if analysis.unsupported {
            break;
        }
        if !analysis.changed {
            converged = true;
            break;
        }
    }
    if !converged {
        return Clock {
            direct: buffers(module),
            bounded: Vec::new(),
        };
    }
    Clock {
        direct: analysis.output.direct.into_iter().collect(),
        bounded: analysis.output.bounded,
    }
}

fn buffers(module: &Module) -> Vec<(u32, u32, u32)> {
    module
        .global_variables
        .iter()
        .filter_map(|(_, variable)| {
            let binding = variable.binding.as_ref()?;
            (binding.group == 0).then(|| {
                let end = size(Shape::of(module, variable.ty));
                (binding.binding, 0, end)
            })
        })
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Deps {
    direct: BTreeSet<(u32, u32, u32)>,
    bounded: Vec<Bounded>,
}

impl Deps {
    fn union(&mut self, other: &Deps) -> bool {
        let before = (self.direct.len(), self.bounded.len());
        self.direct.extend(other.direct.iter().copied());
        for bounded in &other.bounded {
            if !self.bounded.contains(bounded) {
                self.bounded.push(bounded.clone());
            }
        }
        before != (self.direct.len(), self.bounded.len())
    }

    fn is_empty(&self) -> bool {
        self.direct.is_empty() && self.bounded.is_empty()
    }
}

const ROUNDED: f64 = 1.0 / (1u64 << 24) as f64;
const DIVIDED: f64 = 5.0 / (1u64 << 24) as f64;
const FLUSH: f64 = f32::MIN_POSITIVE as f64;
const LARGEST: f64 = f32::MAX as f64;

#[derive(Clone, Debug, Default, PartialEq)]
struct Rounding {
    slack: f64,
    per: BTreeMap<Slot, f64>,
    constants: f64,
    terms: f64,
}

impl Rounding {
    fn plus(&self, other: &Rounding) -> Rounding {
        let mut per = self.per.clone();
        for (slot, bound) in &other.per {
            *per.entry(*slot).or_default() += bound;
        }
        Rounding {
            slack: self.slack + other.slack,
            per,
            constants: self.constants.max(other.constants),
            terms: self.terms.max(other.terms),
        }
    }

    fn times(&self, factor: f64) -> Rounding {
        Rounding {
            slack: self.slack * factor,
            per: self
                .per
                .iter()
                .map(|(slot, bound)| (*slot, bound * factor))
                .collect(),
            constants: self.constants,
            terms: self.terms,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Affine {
    terms: BTreeMap<Slot, f64>,
    constant: f64,
    error: Rounding,
}

impl Affine {
    fn rounded(mut self, error: Rounding, unit: f64) -> Val {
        let mut bound = error.times(1.0 + unit);
        bound.slack += unit * self.constant.abs() + FLUSH;
        for (slot, k) in &self.terms {
            *bound.per.entry(*slot).or_default() += unit * k.abs();
        }
        bound.constants = bound.constants.max(self.constant.abs() + bound.slack);
        bound.terms = bound.terms.max(self.terms.values().map(|k| k.abs()).sum());
        let held = bound.constants <= LARGEST
            && bound.terms <= LARGEST
            && bound.per.values().all(|bound| bound.is_finite());
        self.error = bound;
        let value = Val::Affine(self);
        if held { value } else { opaque([&value]) }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Val {
    Affine(Affine),
    Opaque(Deps),
}

impl Val {
    fn free() -> Self {
        Val::Opaque(Deps::default())
    }

    fn literal(value: f64) -> Self {
        if !(value.abs() <= LARGEST) {
            return Val::free();
        }
        let slack = if value != 0.0 && value.abs() < FLUSH {
            FLUSH
        } else {
            0.0
        };
        Val::Affine(Affine {
            terms: BTreeMap::new(),
            constant: value,
            error: Rounding {
                slack,
                constants: value.abs() + slack,
                ..Rounding::default()
            },
        })
    }

    fn slot(slot: Slot) -> Self {
        Val::Affine(Affine {
            terms: [(slot, 1.0)].into(),
            constant: 0.0,
            error: Rounding {
                slack: FLUSH,
                constants: FLUSH,
                terms: 1.0,
                ..Rounding::default()
            },
        })
    }

    fn deps(&self) -> Deps {
        match self {
            Val::Affine(affine) => Deps {
                direct: affine
                    .terms
                    .keys()
                    .chain(affine.error.per.keys())
                    .map(|&(binding, offset)| (binding, offset, offset + 4))
                    .collect(),
                bounded: Vec::new(),
            },
            Val::Opaque(deps) => deps.clone(),
        }
    }

    fn constant(&self) -> Option<(f64, f64)> {
        match self {
            Val::Affine(affine) if affine.terms.is_empty() && affine.error.per.is_empty() => {
                Some((affine.constant, affine.error.slack))
            }
            _ => None,
        }
    }

    fn negated(&self) -> Self {
        match self {
            Val::Affine(affine) => Val::Affine(Affine {
                terms: affine.terms.iter().map(|(slot, k)| (*slot, -k)).collect(),
                constant: -affine.constant,
                error: affine.error.clone(),
            }),
            Val::Opaque(_) => self.clone(),
        }
    }
}

fn opaque<'a>(values: impl IntoIterator<Item = &'a Val>) -> Val {
    let mut deps = Deps::default();
    for value in values {
        deps.union(&value.deps());
    }
    Val::Opaque(deps)
}

fn join_into(slot: &mut Option<Val>, new: &Val) -> bool {
    let joined = match slot {
        None => new.clone(),
        Some(old) if old == new => return false,
        Some(old) => opaque([&*old, new]),
    };
    if slot.as_ref() == Some(&joined) {
        return false;
    }
    *slot = Some(joined);
    true
}

fn under(value: &Val, context: &Deps) -> Val {
    if context.is_empty() {
        value.clone()
    } else {
        opaque([value, &Val::Opaque(context.clone())])
    }
}

fn add(a: &Val, b: &Val, sign: f64) -> Val {
    match (a, b) {
        (Val::Affine(x), Val::Affine(y)) => {
            let mut terms = x.terms.clone();
            for (slot, k) in &y.terms {
                *terms.entry(*slot).or_default() += sign * k;
            }
            terms.retain(|_, k| *k != 0.0);
            Affine {
                terms,
                constant: x.constant + sign * y.constant,
                error: Rounding::default(),
            }
            .rounded(x.error.plus(&y.error), ROUNDED)
        }
        _ => opaque([a, b]),
    }
}

fn product(x: &Val, y: &Val) -> Val {
    let (value, (c, e)) = match (x.constant(), y.constant()) {
        (Some(c), _) => (y, c),
        (_, Some(c)) => (x, c),
        _ => return opaque([x, y]),
    };
    let Val::Affine(value) = value else {
        return opaque([x, y]);
    };
    let mut error = value.error.times(c.abs() + e);
    error.slack += value.constant.abs() * e;
    for (slot, k) in &value.terms {
        *error.per.entry(*slot).or_default() += k.abs() * e;
    }
    Affine {
        terms: value.terms.iter().map(|(slot, k)| (*slot, k * c)).collect(),
        constant: value.constant * c,
        error: Rounding::default(),
    }
    .rounded(error, ROUNDED)
}

fn quotient(x: &Val, y: &Val) -> Val {
    let (Val::Affine(value), Some((c, e))) = (x, y.constant()) else {
        return opaque([x, y]);
    };
    let room = c.abs() - e;
    if !(room > 0.0) {
        return opaque([x, y]);
    }
    let mut error = value.error.clone();
    error.slack += (value.constant / c).abs() * e;
    for (slot, k) in &value.terms {
        *error.per.entry(*slot).or_default() += (k / c).abs() * e;
    }
    Affine {
        terms: value.terms.iter().map(|(slot, k)| (*slot, k / c)).collect(),
        constant: value.constant / c,
        error: Rounding::default(),
    }
    .rounded(error.times(1.0 / room), DIVIDED)
}

fn bounded(value: &Val, low: Option<&Val>, high: Option<&Val>, open_low: bool) -> Val {
    let constant = |bound: Option<&Val>| match bound {
        None => Some(None),
        Some(bound) => bound.constant().map(Some),
    };
    match (value, constant(low), constant(high)) {
        (Val::Affine(affine), Some(low), Some(high)) if !affine.terms.is_empty() => {
            let bound_slack = [low, high]
                .into_iter()
                .flatten()
                .map(|(_, slack)| slack)
                .fold(0.0, f64::max);
            Val::Opaque(Deps {
                direct: BTreeSet::new(),
                bounded: vec![Bounded {
                    terms: affine.terms.iter().map(|(slot, k)| (*slot, *k)).collect(),
                    constant: affine.constant,
                    low: low.map(|(value, _)| value),
                    high: high.map(|(value, _)| value),
                    open_low,
                    slack: affine.error.slack + bound_slack,
                    per: affine
                        .error
                        .per
                        .iter()
                        .map(|(slot, per)| (*slot, *per))
                        .collect(),
                    reach: affine.error.constants,
                    peak: affine.error.terms,
                }],
            })
        }
        _ => opaque([Some(value), low, high].into_iter().flatten()),
    }
}

#[derive(Clone, Copy)]
enum Shape<'m> {
    Of(&'m TypeInner),
    Vector(VectorSize, Scalar),
    Scalar(Scalar),
}

impl<'m> Shape<'m> {
    fn of(module: &'m Module, ty: Handle<naga::Type>) -> Self {
        Shape::Of(&module.types[ty].inner).normal()
    }

    fn normal(self) -> Self {
        match self {
            Shape::Of(TypeInner::Scalar(scalar) | TypeInner::Atomic(scalar)) => {
                Shape::Scalar(*scalar)
            }
            Shape::Of(TypeInner::Vector { size, scalar }) => Shape::Vector(*size, *scalar),
            Shape::Of(TypeInner::ValuePointer {
                size: Some(size),
                scalar,
                ..
            }) => Shape::Vector(*size, *scalar),
            Shape::Of(TypeInner::ValuePointer {
                size: None, scalar, ..
            }) => Shape::Scalar(*scalar),
            shape => shape,
        }
    }
}

fn comps(module: &Module, shape: Shape) -> usize {
    match shape.normal() {
        Shape::Scalar(_) => 1,
        Shape::Vector(size, _) => size as usize,
        Shape::Of(TypeInner::Matrix { columns, rows, .. }) => *columns as usize * *rows as usize,
        Shape::Of(TypeInner::Array {
            base,
            size: naga::ArraySize::Constant(count),
            ..
        }) => count.get() as usize * comps(module, Shape::of(module, *base)),
        Shape::Of(TypeInner::Struct { members, .. }) => members
            .iter()
            .map(|member| comps(module, Shape::of(module, member.ty)))
            .sum(),
        Shape::Of(_) => 0,
    }
}

fn size(shape: Shape) -> u32 {
    match shape.normal() {
        Shape::Scalar(scalar) => u32::from(scalar.width),
        Shape::Vector(size, scalar) => size as u32 * u32::from(scalar.width),
        Shape::Of(TypeInner::Matrix {
            columns,
            rows,
            scalar,
        }) => {
            let column = if *rows == VectorSize::Bi { 2 } else { 4 };
            *columns as u32 * column * u32::from(scalar.width)
        }
        Shape::Of(TypeInner::Array {
            size: naga::ArraySize::Constant(count),
            stride,
            ..
        }) => count.get() * stride,
        Shape::Of(TypeInner::Struct { span, .. }) => *span,
        Shape::Of(_) => 0,
    }
}

fn element<'m>(module: &'m Module, shape: Shape<'m>) -> Shape<'m> {
    match shape.normal() {
        Shape::Of(TypeInner::Array { base, .. }) => Shape::of(module, *base),
        Shape::Of(TypeInner::Matrix { rows, scalar, .. }) => Shape::Vector(*rows, *scalar),
        Shape::Vector(_, scalar) => Shape::Scalar(scalar),
        shape => shape,
    }
}

fn member(module: &Module, shape: Shape, index: u32) -> Option<(usize, usize)> {
    let Shape::Of(TypeInner::Struct { members, .. }) = shape.normal() else {
        return None;
    };
    let before = members
        .get(..index as usize)?
        .iter()
        .map(|member| comps(module, Shape::of(module, member.ty)))
        .sum();
    let own = comps(module, Shape::of(module, members.get(index as usize)?.ty));
    Some((before, own))
}

fn buffer_values(module: &Module, binding: u32, offset: u32, shape: Shape, out: &mut Vec<Val>) {
    match shape.normal() {
        Shape::Scalar(scalar) if scalar.width == 4 => {
            out.push(Val::slot((binding, offset)));
        }
        Shape::Vector(size, scalar) => {
            for index in 0..size as u32 {
                let at = offset + index * u32::from(scalar.width);
                buffer_values(module, binding, at, Shape::Scalar(scalar), out);
            }
        }
        Shape::Of(TypeInner::Array {
            base,
            size: naga::ArraySize::Constant(count),
            stride,
        }) => {
            for index in 0..count.get() {
                let at = offset + index * stride;
                buffer_values(module, binding, at, Shape::of(module, *base), out);
            }
        }
        Shape::Of(TypeInner::Struct { members, .. }) => {
            for member in members {
                let at = offset + member.offset;
                buffer_values(module, binding, at, Shape::of(module, member.ty), out);
            }
        }
        shape => {
            let whole = Val::Opaque(Deps {
                direct: [(binding, offset, offset + size(shape))].into(),
                bounded: Vec::new(),
            });
            out.extend(std::iter::repeat_n(whole, comps(module, shape)));
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Var {
    Local(Option<Handle<Function>>, Handle<naga::LocalVariable>),
    Global(Handle<naga::GlobalVariable>),
}

enum Place<'m> {
    Buffer {
        binding: u32,
        offset: u32,
        shape: Shape<'m>,
        whole: Option<((u32, u32, u32), Deps)>,
    },
    Var {
        var: Var,
        start: usize,
        len: usize,
        dynamic: Option<Deps>,
    },
    Other,
}

struct Analysis<'m> {
    module: &'m Module,
    info: &'m ModuleInfo,
    zeros: &'m [Slot],
    zero_channel: Option<Slot>,
    vars: HashMap<Var, Vec<Option<Val>>>,
    args: HashMap<Handle<Function>, Vec<Vec<Option<Val>>>>,
    results: HashMap<Handle<Function>, Vec<Option<Val>>>,
    contexts: HashMap<Handle<Function>, Deps>,
    loops: HashMap<*const Block, Deps>,
    output: Deps,
    changed: bool,
    unsupported: bool,
}

struct Frame<'m> {
    id: Option<Handle<Function>>,
    function: &'m Function,
    info: &'m FunctionInfo,
    values: HashMap<Handle<Expression>, Vec<Val>>,
}

impl<'m> Analysis<'m> {
    fn function(&mut self, id: Option<Handle<Function>>) {
        let (function, info) = match id {
            None => (
                &self.module.entry_points[0].function,
                self.info.get_entry_point(0),
            ),
            Some(handle) => (&self.module.functions[handle], &self.info[handle]),
        };
        let mut frame = Frame {
            id,
            function,
            info,
            values: HashMap::new(),
        };
        for (handle, local) in function.local_variables.iter() {
            if let Some(init) = local.init {
                let values = self.value(&mut frame, init);
                for (index, value) in values.iter().enumerate() {
                    self.store(Var::Local(id, handle), index, value, &Deps::default());
                }
            }
        }
        let context = id
            .and_then(|handle| self.contexts.get(&handle).cloned())
            .unwrap_or_default();
        self.block(&mut frame, &function.body, &context, &[]);
    }

    fn grow(&mut self, deps: &Deps) {
        self.changed |= self.output.union(deps);
    }

    fn block(
        &mut self,
        frame: &mut Frame<'m>,
        block: &'m Block,
        context: &Deps,
        loops: &[*const Block],
    ) {
        for statement in block.iter() {
            self.statement(frame, statement, context, loops);
        }
    }

    fn condition(
        &mut self,
        frame: &mut Frame<'m>,
        expression: Handle<Expression>,
        context: &Deps,
        loops: &[*const Block],
    ) -> Deps {
        let deps = opaque(&self.value(frame, expression)).deps();
        for body in loops {
            self.changed |= self.loops.entry(*body).or_default().union(&deps);
        }
        let mut inner = context.clone();
        inner.union(&deps);
        inner
    }

    fn statement(
        &mut self,
        frame: &mut Frame<'m>,
        statement: &'m Statement,
        context: &Deps,
        loops: &[*const Block],
    ) {
        match statement {
            Statement::Emit(range) => {
                for handle in range.clone() {
                    self.value(frame, handle);
                }
            }
            Statement::Block(block) => self.block(frame, block, context, loops),
            Statement::If {
                condition,
                accept,
                reject,
            } => {
                let inner = self.condition(frame, *condition, context, loops);
                self.block(frame, accept, &inner, loops);
                self.block(frame, reject, &inner, loops);
            }
            Statement::Switch { selector, cases } => {
                let inner = self.condition(frame, *selector, context, loops);
                for case in cases {
                    self.block(frame, &case.body, &inner, loops);
                }
            }
            Statement::Loop {
                body,
                continuing,
                break_if,
            } => {
                let key: *const Block = body;
                let mut inner = context.clone();
                inner.union(&self.loops.get(&key).cloned().unwrap_or_default());
                let mut within = loops.to_vec();
                within.push(key);
                self.block(frame, body, &inner, &within);
                self.block(frame, continuing, &inner, &within);
                if let Some(condition) = break_if {
                    self.condition(frame, *condition, &inner, &within);
                }
            }
            Statement::Break
            | Statement::Continue
            | Statement::ControlBarrier(_)
            | Statement::MemoryBarrier(_) => {}
            Statement::Return { value } => {
                let values = value
                    .map(|value| self.value(frame, value))
                    .unwrap_or_default();
                match frame.id {
                    None => {
                        let mut deps = opaque(&values).deps();
                        deps.union(context);
                        self.grow(&deps);
                    }
                    Some(id) => {
                        let result = self.results.entry(id).or_default();
                        if result.len() < values.len() {
                            result.resize(values.len(), None);
                        }
                        let mut changed = false;
                        for (slot, value) in result.iter_mut().zip(&values) {
                            changed |= join_into(slot, &under(value, context));
                        }
                        self.changed |= changed;
                        self.grow(context);
                    }
                }
            }
            Statement::Kill => self.grow(context),
            Statement::Store { pointer, value } => {
                let values = self.value(frame, *value);
                match self.place(frame, *pointer) {
                    Place::Var {
                        var,
                        start,
                        len,
                        dynamic: None,
                    } => {
                        for (index, value) in values.iter().take(len).enumerate() {
                            self.store(var, start + index, value, context);
                        }
                    }
                    Place::Var {
                        var,
                        dynamic: Some(index),
                        ..
                    } => {
                        let mut deps = opaque(&values).deps();
                        deps.union(&index);
                        let joined = Val::Opaque(deps);
                        for component in 0..self.var_len(var) {
                            self.store(var, component, &joined, context);
                        }
                    }
                    Place::Buffer { .. } | Place::Other => {
                        let mut deps = opaque(&values).deps();
                        deps.union(context);
                        self.grow(&deps);
                    }
                }
            }
            Statement::Call {
                function,
                arguments,
                result,
            } => {
                let values: Vec<Vec<Val>> = arguments
                    .iter()
                    .map(|argument| self.value(frame, *argument))
                    .collect();
                let args = self.args.entry(*function).or_default();
                if args.len() < values.len() {
                    args.resize(values.len(), Vec::new());
                }
                let mut changed = false;
                for (slots, values) in args.iter_mut().zip(&values) {
                    if slots.len() < values.len() {
                        slots.resize(values.len(), None);
                    }
                    for (slot, value) in slots.iter_mut().zip(values) {
                        changed |= join_into(slot, value);
                    }
                }
                changed |= self.contexts.entry(*function).or_default().union(context);
                self.changed |= changed;
                if let Some(result) = result {
                    self.value(frame, *result);
                }
            }
            _ => self.unsupported = true,
        }
    }

    fn var_len(&self, var: Var) -> usize {
        let module = self.module;
        let ty = match var {
            Var::Local(None, local) => module.entry_points[0].function.local_variables[local].ty,
            Var::Local(Some(function), local) => {
                module.functions[function].local_variables[local].ty
            }
            Var::Global(global) => module.global_variables[global].ty,
        };
        comps(module, Shape::of(module, ty))
    }

    fn store(&mut self, var: Var, component: usize, value: &Val, context: &Deps) {
        let count = self.var_len(var);
        let slots = self.vars.entry(var).or_insert_with(|| vec![None; count]);
        if let Some(slot) = slots.get_mut(component) {
            self.changed |= join_into(slot, &under(value, context));
        }
    }

    fn load(&self, var: Var, start: usize, len: usize, dynamic: Option<&Deps>) -> Vec<Val> {
        let slots = self.vars.get(&var);
        let at = |component: usize| {
            slots
                .and_then(|slots| slots.get(component).cloned().flatten())
                .unwrap_or_else(Val::free)
        };
        match dynamic {
            None => (start..start + len).map(at).collect(),
            Some(index) => {
                let all: Vec<Val> = (0..self.var_len(var)).map(at).collect();
                let mut deps = opaque(&all).deps();
                deps.union(index);
                vec![Val::Opaque(deps); len]
            }
        }
    }

    fn shape(&self, frame: &Frame<'m>, expression: Handle<Expression>) -> Shape<'m> {
        let module: &'m Module = self.module;
        let info: &'m FunctionInfo = frame.info;
        match &info[expression].ty {
            naga::proc::TypeResolution::Handle(handle) => Shape::of(module, *handle),
            naga::proc::TypeResolution::Value(inner) => Shape::Of(inner).normal(),
        }
    }

    fn is_pointer(&self, frame: &Frame<'m>, expression: Handle<Expression>) -> bool {
        matches!(
            self.shape(frame, expression),
            Shape::Of(TypeInner::Pointer { .. } | TypeInner::ValuePointer { .. })
        )
    }

    fn pointee(&self, frame: &Frame<'m>, pointer: Handle<Expression>) -> Shape<'m> {
        let module: &'m Module = self.module;
        let info: &'m FunctionInfo = frame.info;
        let inner = match &info[pointer].ty {
            naga::proc::TypeResolution::Handle(handle) => &module.types[*handle].inner,
            naga::proc::TypeResolution::Value(inner) => inner,
        };
        match inner {
            TypeInner::Pointer { base, .. } => Shape::of(module, *base),
            TypeInner::ValuePointer {
                size: Some(size),
                scalar,
                ..
            } => Shape::Vector(*size, *scalar),
            TypeInner::ValuePointer {
                size: None, scalar, ..
            } => Shape::Scalar(*scalar),
            inner => Shape::Of(inner).normal(),
        }
    }

    fn literal_index(&self, frame: &Frame<'m>, expression: Handle<Expression>) -> Option<u32> {
        let module = self.module;
        let mut expression = &frame.function.expressions[expression];
        loop {
            match expression {
                Expression::Literal(Literal::U32(value)) => return Some(*value),
                Expression::Literal(Literal::I32(value)) => return u32::try_from(*value).ok(),
                Expression::Literal(Literal::AbstractInt(value)) => {
                    return u32::try_from(*value).ok();
                }
                Expression::Constant(constant) => {
                    expression = &module.global_expressions[module.constants[*constant].init];
                }
                _ => return None,
            }
        }
    }

    fn place(&mut self, frame: &mut Frame<'m>, pointer: Handle<Expression>) -> Place<'m> {
        let module: &'m Module = self.module;
        let function: &'m Function = frame.function;
        match &function.expressions[pointer] {
            Expression::GlobalVariable(global) => {
                let variable = &module.global_variables[*global];
                match (variable.space, &variable.binding) {
                    (AddressSpace::Uniform, Some(binding)) if binding.group == 0 => Place::Buffer {
                        binding: binding.binding,
                        offset: 0,
                        shape: Shape::of(module, variable.ty),
                        whole: None,
                    },
                    (AddressSpace::Private | AddressSpace::WorkGroup, _) => Place::Var {
                        var: Var::Global(*global),
                        start: 0,
                        len: comps(module, Shape::of(module, variable.ty)),
                        dynamic: None,
                    },
                    _ => Place::Other,
                }
            }
            Expression::LocalVariable(local) => Place::Var {
                var: Var::Local(frame.id, *local),
                start: 0,
                len: comps(
                    module,
                    Shape::of(module, function.local_variables[*local].ty),
                ),
                dynamic: None,
            },
            Expression::AccessIndex { base, index } => self.step(frame, *base, Some(*index), None),
            Expression::Access { base, index } => match self.literal_index(frame, *index) {
                Some(literal) => self.step(frame, *base, Some(literal), None),
                None => {
                    let deps = opaque(&self.value(frame, *index)).deps();
                    self.step(frame, *base, None, Some(deps))
                }
            },
            _ => Place::Other,
        }
    }

    fn step(
        &mut self,
        frame: &mut Frame<'m>,
        base: Handle<Expression>,
        index: Option<u32>,
        deps: Option<Deps>,
    ) -> Place<'m> {
        let module = self.module;
        match self.place(frame, base) {
            Place::Buffer {
                binding,
                offset,
                shape,
                whole,
            } => {
                let known = |whole: Option<((u32, u32, u32), Deps)>, deps: Option<Deps>| {
                    let (span, mut known) = whole.unwrap_or_else(|| {
                        ((binding, offset, offset + size(shape)), Deps::default())
                    });
                    if let Some(deps) = deps {
                        known.union(&deps);
                    }
                    Some((span, known))
                };
                let (shape, offset, whole) = match (shape.normal(), whole, index) {
                    (_, Some(whole), _) => {
                        (element(module, shape), offset, known(Some(whole), deps))
                    }
                    (Shape::Of(TypeInner::Struct { members, .. }), None, Some(index)) => {
                        match members.get(index as usize) {
                            Some(member) => {
                                (Shape::of(module, member.ty), offset + member.offset, None)
                            }
                            None => (shape, offset, known(None, deps)),
                        }
                    }
                    (Shape::Of(TypeInner::Array { base, stride, .. }), None, Some(index)) => {
                        (Shape::of(module, *base), offset + index * stride, None)
                    }
                    (Shape::Vector(_, scalar), None, Some(index)) => (
                        Shape::Scalar(scalar),
                        offset + index * u32::from(scalar.width),
                        None,
                    ),
                    (_, None, _) => (element(module, shape), offset, known(None, deps)),
                };
                Place::Buffer {
                    binding,
                    offset,
                    shape,
                    whole,
                }
            }
            Place::Var {
                var,
                start,
                len,
                dynamic,
            } => {
                let shape = self.pointee(frame, base);
                let (skip, count) = match index.and_then(|index| member(module, shape, index)) {
                    Some(member) => member,
                    None => {
                        let count = comps(module, element(module, shape));
                        (index.unwrap_or(0) as usize * count, count)
                    }
                };
                let dynamic = match (dynamic, deps) {
                    (Some(mut known), Some(deps)) => {
                        known.union(&deps);
                        Some(known)
                    }
                    (known, deps) => known.or(deps),
                };
                Place::Var {
                    var,
                    start: if dynamic.is_some() {
                        0
                    } else {
                        start + skip.min(len)
                    },
                    len: count,
                    dynamic,
                }
            }
            Place::Other => Place::Other,
        }
    }

    fn value(&mut self, frame: &mut Frame<'m>, expression: Handle<Expression>) -> Vec<Val> {
        if let Some(values) = frame.values.get(&expression) {
            return values.clone();
        }
        let values = self.compute(frame, expression);
        frame.values.insert(expression, values.clone());
        values
    }

    fn count(&self, frame: &Frame<'m>, expression: Handle<Expression>) -> usize {
        comps(self.module, self.shape(frame, expression))
    }

    fn nonnegative_bounded(&self, frame: &Frame<'m>, expr: Handle<Expression>) -> bool {
        let expressions = &frame.function.expressions;
        let nonnegative = |v| matches!(expressions[v], Expression::Literal(Literal::F32(n)) if n.is_finite() && n >= 0.0);
        match expressions[expr] {
            Expression::Math {
                fun: MathFunction::Saturate,
                ..
            } => true,
            Expression::Math {
                fun: MathFunction::Clamp,
                arg1: Some(lo),
                arg2: Some(hi),
                ..
            } => nonnegative(lo) && nonnegative(hi),
            Expression::Math {
                fun: MathFunction::Min,
                arg,
                arg1: Some(hi),
                ..
            } if nonnegative(hi) => {
                matches!(expressions[arg], Expression::Math { fun: MathFunction::Max, arg, arg1: Some(lo), .. }
                    if nonnegative(arg) || nonnegative(lo))
            }
            _ => false,
        }
    }

    fn unit_wave(frame: &Frame<'m>, expr: Handle<Expression>) -> bool {
        let expressions = &frame.function.expressions;
        let constant =
            |expr, n| matches!(expressions[expr], Expression::Literal(Literal::F32(v)) if v == n);
        let scaled = |a, b| {
            if !constant(b, 0.5) {
                return false;
            }
            let Expression::Binary {
                op: BinaryOperator::Add,
                left,
                right,
            } = expressions[a]
            else {
                return false;
            };
            let wave = |e| {
                matches!(
                    expressions[e],
                    Expression::Math {
                        fun: MathFunction::Sin | MathFunction::Cos,
                        ..
                    }
                )
            };
            (wave(left) && constant(right, 1.0)) || (wave(right) && constant(left, 1.0))
        };
        matches!(expressions[expr], Expression::Binary { op: BinaryOperator::Multiply, left, right }
            if scaled(left, right) || scaled(right, left))
    }

    fn joined(
        &mut self,
        frame: &mut Frame<'m>,
        expression: Handle<Expression>,
        operands: &[Handle<Expression>],
    ) -> Vec<Val> {
        let mut deps = Deps::default();
        for operand in operands {
            deps.union(&opaque(&self.value(frame, *operand)).deps());
        }
        vec![Val::Opaque(deps); self.count(frame, expression)]
    }

    fn compute(&mut self, frame: &mut Frame<'m>, expression: Handle<Expression>) -> Vec<Val> {
        let module: &'m Module = self.module;
        let function: &'m Function = frame.function;
        match &function.expressions[expression] {
            Expression::Literal(literal) => vec![literal_value(literal)],
            Expression::Constant(constant) => {
                global_value(module, module.constants[*constant].init)
            }
            Expression::ZeroValue(ty) => {
                vec![Val::literal(0.0); comps(module, Shape::of(module, *ty))]
            }
            Expression::Compose { components, .. } => components
                .iter()
                .flat_map(|component| self.value(frame, *component))
                .collect(),
            Expression::Splat { size, value } => {
                let value = self.value(frame, *value);
                (0..*size as usize)
                    .map(|index| {
                        value
                            .get(index % value.len().max(1))
                            .cloned()
                            .unwrap_or_else(Val::free)
                    })
                    .collect()
            }
            Expression::Swizzle {
                size,
                vector,
                pattern,
            } => {
                let vector = self.value(frame, *vector);
                pattern[..*size as usize]
                    .iter()
                    .map(|component| {
                        vector
                            .get(*component as usize)
                            .cloned()
                            .unwrap_or_else(Val::free)
                    })
                    .collect()
            }
            Expression::FunctionArgument(index) => {
                let count = self.count(frame, expression);
                let known = frame
                    .id
                    .and_then(|id| self.args.get(&id))
                    .and_then(|args| args.get(*index as usize));
                (0..count)
                    .map(|component| {
                        known
                            .and_then(|known| known.get(component).cloned().flatten())
                            .unwrap_or_else(Val::free)
                    })
                    .collect()
            }
            Expression::GlobalVariable(_) | Expression::LocalVariable(_) => Vec::new(),
            Expression::Load { pointer } => {
                let count = comps(module, self.pointee(frame, *pointer));
                match self.place(frame, *pointer) {
                    Place::Buffer {
                        binding,
                        offset,
                        shape,
                        whole: None,
                    } => {
                        let mut out = Vec::new();
                        buffer_values(module, binding, offset, shape, &mut out);
                        if !self.zeros.is_empty() {
                            for value in &mut out {
                                if let Val::Affine(affine) = value
                                    && affine.terms.keys().any(|slot| self.zeros.contains(slot))
                                {
                                    *value = Val::literal(0.0);
                                }
                            }
                        }
                        out
                    }
                    Place::Buffer {
                        whole: Some((span, mut deps)),
                        ..
                    } => {
                        deps.direct.insert(span);
                        vec![Val::Opaque(deps); count]
                    }
                    Place::Var {
                        var,
                        start,
                        len,
                        dynamic,
                    } => self.load(var, start, len, dynamic.as_ref()),
                    Place::Other => vec![Val::free(); count],
                }
            }
            Expression::AccessIndex { base, index } => {
                if self.is_pointer(frame, *base) {
                    return Vec::new();
                }
                let values = self.value(frame, *base);
                let shape = self.shape(frame, *base);
                let (skip, count) = member(module, shape, *index).unwrap_or_else(|| {
                    let count = comps(module, element(module, shape));
                    (*index as usize * count, count)
                });
                values.iter().skip(skip).take(count).cloned().collect()
            }
            Expression::Access { base, index } => {
                if self.is_pointer(frame, *base) {
                    return Vec::new();
                }
                self.joined(frame, expression, &[*base, *index])
            }
            Expression::Unary { op, expr } => {
                let values = self.value(frame, *expr);
                match op {
                    UnaryOperator::Negate => values.iter().map(Val::negated).collect(),
                    _ => values.iter().map(|value| opaque([value])).collect(),
                }
            }
            Expression::Binary { op, left, right } => {
                let matrix = |shape: Shape| matches!(shape, Shape::Of(TypeInner::Matrix { .. }));
                if matrix(self.shape(frame, *left)) || matrix(self.shape(frame, *right)) {
                    return self.joined(frame, expression, &[*left, *right]);
                }
                let (a, b) = (self.value(frame, *left), self.value(frame, *right));
                let count = self.count(frame, expression);
                (0..count)
                    .map(|index| {
                        let x = &a[index % a.len().max(1)];
                        let y = &b[index % b.len().max(1)];
                        match op {
                            BinaryOperator::Add => add(x, y, 1.0),
                            BinaryOperator::Subtract if self.zero_channel.is_some()
                                && x.constant() == Some((1.0, 0.0)) && y.constant() == Some((0.0, 0.0)) => Val::literal(1.0),
                            BinaryOperator::Subtract => add(x, y, -1.0),
                            BinaryOperator::Multiply
                                if !self.zeros.is_empty()
                                    && ((x.constant() == Some((0.0, 0.0)) && self.nonnegative_bounded(frame, *right))
                                        || (y.constant() == Some((0.0, 0.0)) && self.nonnegative_bounded(frame, *left))) =>
                            {
                                Val::literal(0.0)
                            }
                            BinaryOperator::Multiply => product(x, y),
                            BinaryOperator::Divide => quotient(x, y),
                            _ => opaque([x, y]),
                        }
                    })
                    .collect()
            }
            Expression::Math {
                fun,
                arg,
                arg1,
                arg2,
                arg3,
            } => {
                let args: Vec<Vec<Val>> = [Some(*arg), *arg1, *arg2, *arg3]
                    .into_iter()
                    .flatten()
                    .map(|operand| self.value(frame, operand))
                    .collect();
                let count = self.count(frame, expression);
                let at = |operand: usize, index: usize| {
                    let values = &args[operand];
                    values
                        .get(index % values.len().max(1))
                        .cloned()
                        .unwrap_or_else(Val::free)
                };
                let each = |f: &dyn Fn(usize) -> Val| (0..count).map(f).collect::<Vec<_>>();
                let (zero, one) = (Val::literal(0.0), Val::literal(1.0));
                match fun {
                    MathFunction::Mix if self.zero_channel.is_some() => each(&|i| {
                        let (a, b, t) = (at(0, i), at(1, i), at(2, i));
                        if b.constant() == Some((1.0, 0.0)) && a.constant().is_some_and(|(v, e)| e == 0.0 && (0.0..=1.0).contains(&v))
                            && (t.constant() == Some((1.0, 0.0))
                                || (a.constant() == b.constant() && arg2.is_some_and(|e| Self::unit_wave(frame, e))))
                        {
                            b
                        } else {
                            opaque([&a, &b, &t])
                        }
                    }),
                    MathFunction::Saturate => {
                        each(&|i| bounded(&at(0, i), Some(&zero), Some(&one), false))
                    }
                    MathFunction::Clamp => {
                        each(&|i| bounded(&at(0, i), Some(&at(1, i)), Some(&at(2, i)), false))
                    }
                    MathFunction::Min | MathFunction::Max => each(&|i| {
                        let (x, y) = (at(0, i), at(1, i));
                        let (value, bound) = if y.constant().is_some() {
                            (x, y)
                        } else {
                            (y, x)
                        };
                        if *fun == MathFunction::Min {
                            bounded(&value, None, Some(&bound), false)
                        } else {
                            bounded(&value, Some(&bound), None, false)
                        }
                    }),
                    MathFunction::SmoothStep => each(&|i| {
                        let (from, to, x) = (at(0, i), at(1, i), at(2, i));
                        match (from.constant(), to.constant()) {
                            (Some((a, _)), Some((b, _))) => {
                                let (low, high) = if a <= b { (&from, &to) } else { (&to, &from) };
                                bounded(&x, Some(low), Some(high), false)
                            }
                            _ => opaque([&from, &to, &x]),
                        }
                    }),
                    MathFunction::Step => each(&|i| {
                        let (edge, x) = (at(0, i), at(1, i));
                        bounded(&x, Some(&edge), Some(&edge), true)
                    }),
                    MathFunction::Fma => {
                        each(&|i| add(&product(&at(0, i), &at(1, i)), &at(2, i), 1.0))
                    }
                    _ => vec![opaque(args.iter().flatten()); count],
                }
            }
            Expression::As {
                expr,
                kind,
                convert,
            } => {
                let values = self.value(frame, *expr);
                let float = |shape: Shape| match shape.normal() {
                    Shape::Scalar(scalar) | Shape::Vector(_, scalar) => {
                        scalar.kind == naga::ScalarKind::Float && scalar.width >= 4
                    }
                    Shape::Of(_) => false,
                };
                if *kind == naga::ScalarKind::Float
                    && convert.is_none_or(|width| width >= 4)
                    && float(self.shape(frame, *expr))
                {
                    values
                } else {
                    values.iter().map(|value| opaque([value])).collect()
                }
            }
            Expression::ImageSample {
                image,
                gather,
                coordinate,
                array_index,
                offset,
                level,
                depth_ref,
                ..
            } => {
                let mut operands = vec![*coordinate];
                operands.extend(array_index.iter().chain(offset).chain(depth_ref).copied());
                match level {
                    naga::SampleLevel::Exact(value) | naga::SampleLevel::Bias(value) => {
                        operands.push(*value)
                    }
                    naga::SampleLevel::Gradient { x, y } => operands.extend([*x, *y]),
                    naga::SampleLevel::Auto | naga::SampleLevel::Zero => {}
                }
                let mut values = self.joined(frame, expression, &operands);
                if let Some((binding, component)) = self.zero_channel
                    && gather.is_none() && depth_ref.is_none()
                    && let Expression::GlobalVariable(global) = frame.function.expressions[*image]
                    && module.global_variables[global].binding.as_ref().is_some_and(|r| r.group == 1 && r.binding == binding)
                    && let Some(value) = values.get_mut(component as usize)
                {
                    *value = Val::literal(0.0);
                }
                values
            }
            Expression::ImageLoad {
                coordinate,
                array_index,
                sample,
                level,
                ..
            } => {
                let mut operands = vec![*coordinate];
                operands.extend(array_index.iter().chain(sample).chain(level).copied());
                self.joined(frame, expression, &operands)
            }
            Expression::ImageQuery { query, .. } => match query {
                naga::ImageQuery::Size { level: Some(level) } => {
                    self.joined(frame, expression, &[*level])
                }
                _ => self.joined(frame, expression, &[]),
            },
            Expression::Select {
                condition,
                accept,
                reject,
            } => self.joined(frame, expression, &[*condition, *accept, *reject]),
            Expression::Derivative { expr, .. } => self.joined(frame, expression, &[*expr]),
            Expression::Relational { argument, .. } => self.joined(frame, expression, &[*argument]),
            Expression::CallResult(callee) => {
                let count = self.count(frame, expression);
                let known = self.results.get(callee);
                (0..count)
                    .map(|component| {
                        known
                            .and_then(|known| known.get(component).cloned().flatten())
                            .unwrap_or_else(Val::free)
                    })
                    .collect()
            }
            Expression::Override(_) | Expression::ArrayLength(_) => {
                self.joined(frame, expression, &[])
            }
            _ => {
                self.unsupported = true;
                self.joined(frame, expression, &[])
            }
        }
    }
}

fn literal_value(literal: &Literal) -> Val {
    match *literal {
        Literal::F32(value) => Val::literal(f64::from(value)),
        Literal::F64(value) | Literal::AbstractFloat(value) => Val::literal(value),
        _ => Val::free(),
    }
}

fn global_value(module: &Module, expression: Handle<Expression>) -> Vec<Val> {
    match &module.global_expressions[expression] {
        Expression::Literal(literal) => vec![literal_value(literal)],
        Expression::Constant(constant) => global_value(module, module.constants[*constant].init),
        Expression::ZeroValue(ty) => vec![Val::literal(0.0); comps(module, Shape::of(module, *ty))],
        Expression::Compose { components, .. } => components
            .iter()
            .flat_map(|component| global_value(module, *component))
            .collect(),
        Expression::Splat { size, value } => {
            let value = global_value(module, *value);
            (0..*size as usize)
                .map(|index| {
                    value
                        .get(index % value.len().max(1))
                        .cloned()
                        .unwrap_or_else(Val::free)
                })
                .collect()
        }
        _ => vec![Val::free()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIME: Slot = (0, 96);
    const SINCE: Slot = (0, 100);

    fn saturating(k: f64, constant: f64, low: Option<f64>, high: Option<f64>) -> Bounded {
        Bounded {
            terms: vec![(TIME, k), (SINCE, -k)],
            constant,
            low,
            high,
            open_low: false,
            slack: 0.0,
            per: Vec::new(),
            reach: 0.0,
            peak: 0.0,
        }
    }

    fn after(seconds: f32) -> Option<Settling> {
        Some(Settling::after(seconds))
    }

    #[test]
    fn only_a_saturating_duration_settles() {
        let clock = |direct: Vec<(u32, u32, u32)>, bounded: Vec<Bounded>| Clock { direct, bounded };
        assert_eq!(clock(vec![], vec![]).settles(TIME, SINCE), after(0.0));
        assert_eq!(
            clock(vec![(0, 100, 104)], vec![]).settles(TIME, SINCE),
            after(0.0)
        );
        assert_eq!(clock(vec![(0, 96, 100)], vec![]).settles(TIME, SINCE), None);
        assert_eq!(clock(vec![(0, 64, 128)], vec![]).settles(TIME, SINCE), None);
        assert_eq!(
            clock(vec![(1, 96, 100)], vec![]).settles(TIME, SINCE),
            after(0.0)
        );
        let over = saturating(4.0, 0.0, Some(0.0), Some(1.0));
        let down = saturating(16.0, 0.0, Some(0.0), Some(1.0));
        assert_eq!(
            clock(vec![], vec![over.clone()]).settles(TIME, SINCE),
            after(0.25)
        );
        assert_eq!(
            clock(vec![], vec![over, down]).settles(TIME, SINCE),
            after(0.25)
        );
        let falling = saturating(-2.0, 1.0, Some(0.0), Some(1.0));
        assert_eq!(
            clock(vec![], vec![falling]).settles(TIME, SINCE),
            after(0.5)
        );
        let unbounded = saturating(-1.0, 0.0, None, Some(0.0));
        assert_eq!(clock(vec![], vec![unbounded]).settles(TIME, SINCE), None);
        let moment = Bounded {
            terms: vec![(TIME, 4.0)],
            ..saturating(0.0, 0.0, Some(0.0), Some(1.0))
        };
        assert_eq!(clock(vec![], vec![moment]).settles(TIME, SINCE), None);
        let past = saturating(1.0, 2.0, Some(0.0), Some(1.0));
        assert_eq!(clock(vec![], vec![past]).settles(TIME, SINCE), after(0.0));
    }

    #[test]
    fn a_zero_weight_suppresses_only_the_clock_it_multiplies() {
        let reads = |output, zeros: &[Slot]| {
            let wgsl = format!(
                "@group(0) @binding(3) var<uniform> inputs: vec4<f32>;
                 @fragment fn main() -> @location(0) f32 {{
                     let pulse = min(max(sin(inputs.x), 0.0), 1.0) * inputs.y;
                     return {output};
                 }}"
            );
            reads_with_zeros(&wgsl, &[(3, 0)], zeros, None)
        };
        assert_eq!(reads("pulse", &[]), Some(true));
        assert_eq!(reads("pulse", &[(3, 4)]), Some(false));
        assert_eq!(reads("pulse + inputs.x", &[(3, 4)]), Some(true));
        assert_eq!(reads("pulse", &[(3, 8)]), Some(true));
        let region = |output, zero_channel| {
            let wgsl = format!(
                "@group(0) @binding(3) var<uniform> inputs: vec4<f32>;
                 @group(1) @binding(7) var styles: texture_2d<f32>;
                 @group(2) @binding(7) var linear: sampler;
                 @fragment fn main(@location(0) uv: vec2<f32>) -> @location(0) f32 {{
                     let strength = textureSample(styles, linear, uv).b;
                     let wave = (sin(inputs.x) + 1.0) * 0.5;
                     let pulse = mix(mix(0.27, 1.0, 1.0 - strength), 1.0, wave);
                     return {output};
                 }}"
            );
            reads_with_zeros(&wgsl, &[(3, 0)], &[], zero_channel)
        };
        assert_eq!(region("pulse", None), Some(true));
        assert_eq!(region("pulse", Some((7, 2))), Some(false));
        assert_eq!(region("pulse", Some((7, 1))), Some(true));
        assert_eq!(region("pulse + inputs.x", Some((7, 2))), Some(true));
    }

    #[test]
    fn a_descending_step_settles_only_past_its_edge() {
        let step = |k: f64, constant: f64| Clock {
            direct: Vec::new(),
            bounded: vec![Bounded {
                open_low: true,
                ..saturating(k, constant, Some(0.0), Some(0.0))
            }],
        };
        let drop = step(-1.0, 0.0).settles(TIME, SINCE).unwrap().after;
        assert!(0.0 < drop);
        assert_eq!(drop, 0.0_f32.next_up());
        let late = step(-1.0, 0.5).settles(TIME, SINCE).unwrap().after;
        assert!(0.5 < late);
        assert_eq!(late, 0.5_f32.next_up());
        assert_eq!(step(1.0, -0.5).settles(TIME, SINCE), after(0.5));
        assert_eq!(step(-1.0, -1.0).settles(TIME, SINCE), after(0.0));
    }

    fn saturated(value: &Val) -> Clock {
        let Val::Opaque(deps) = bounded(
            value,
            Some(&Val::literal(0.0)),
            Some(&Val::literal(1.0)),
            false,
        ) else {
            panic!("a saturating function is opaque");
        };
        Clock {
            direct: deps.direct.into_iter().collect(),
            bounded: deps.bounded,
        }
    }

    fn elapsed() -> Val {
        add(&Val::slot(TIME), &Val::slot(SINCE), -1.0)
    }

    #[test]
    fn rounding_that_cancellation_hides_delays_settling() {
        let big = Val::literal(33_554_432.0);
        let value = add(&add(&elapsed(), &big, 1.0), &big, -1.0);
        let settling = saturated(&value).settles(TIME, SINCE).unwrap();
        let f32_value =
            |elapsed: f32| ((elapsed + 33_554_432.0_f32) - 33_554_432.0).clamp(0.0, 1.0);
        assert_eq!(f32_value(1.25), 0.0);
        assert!(settling.after > 2.0, "{settling:?}");
        for step in 0..1000 {
            let elapsed = settling.after + step as f32 * 0.01;
            assert_eq!(f32_value(elapsed), 1.0, "{elapsed}");
        }
        assert!(!settling.reached(1.25, 1000.0));
    }

    #[test]
    fn a_scaled_duration_settles_where_it_saturates() {
        let value = product(&elapsed(), &Val::literal(4.0));
        let settling = saturated(&value).settles(TIME, SINCE).unwrap();
        assert_eq!(settling.after, 0.25);
        assert!(0.0 < settling.per_clock && settling.per_clock < 1e-6);
        let shifted = quotient(
            &add(&elapsed(), &Val::literal(1.0), -1.0),
            &Val::literal(2.0),
        );
        let settling = saturated(&shifted).settles(TIME, SINCE).unwrap();
        assert!(
            3.0 <= settling.after && settling.after < 3.0001,
            "{settling:?}"
        );
        let other = add(&elapsed(), &Val::slot((0, 104)), 1.0);
        let other = add(&other, &Val::slot((0, 104)), -1.0);
        assert_eq!(saturated(&other).settles(TIME, SINCE), None);
        let scale = add(
            &add(&Val::slot(TIME), &Val::literal(1.0), 1.0),
            &Val::slot(TIME),
            -1.0,
        );
        assert!(matches!(product(&elapsed(), &scale), Val::Opaque(_)));
        let since = add(
            &add(&Val::slot(TIME), &Val::slot(SINCE), 1.0),
            &Val::slot(TIME),
            -1.0,
        );
        assert_eq!(saturated(&since).settles(TIME, SINCE), None);
    }

    #[test]
    fn flushed_subnormals_delay_settling() {
        let tiny = Val::literal(f64::from(f32::MIN_POSITIVE));
        let undone = product(&product(&elapsed(), &tiny), &Val::literal(2f64.powi(126)));
        let doubled = product(&undone, &Val::literal(2.0));
        let settling = saturated(&doubled).settles(TIME, SINCE).unwrap();
        assert!(settling.after >= 1.0, "{settling:?}");
        assert!(matches!(Val::literal(1e39), Val::Opaque(_)));
        let huge = product(&elapsed(), &Val::literal(1e38));
        let settling = saturated(&huge).settles(TIME, SINCE).unwrap();
        assert!(settling.limit < 10.0, "{settling:?}");
        assert!(!settling.reached(1.0, 100.0));
    }

    #[test]
    fn settling_is_reached_past_its_duration_and_the_clocks_rounding() {
        let settling = Settling {
            after: 1.0,
            per_clock: 1e-3,
            limit: 1000.0,
        };
        assert!(!settling.reached(5.0, 2000.0));
        assert!(!settling.reached(1.0, 100.0));
        assert!(settling.reached(1.2, 100.0));
        assert!(!settling.reached(1.2, 300.0));
        assert!(Settling::NOW.reached(0.0, 1e9));
        assert_eq!(
            Settling::after(2.0).and(settling),
            Settling {
                after: 2.0,
                per_clock: 1e-3,
                limit: 1000.0,
            }
        );
    }
}
