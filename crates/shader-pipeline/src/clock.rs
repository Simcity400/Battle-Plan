//! How a compiled stage's output depends on its constant buffers' scalars, read from the program
//! itself (the naga module DXC's SPIR-V translates to), so a caller can tell how long a draw keeps
//! changing on the clock alone: sprite draws upload the graphics clock `Time`
//! and the drawable's `AnimationTime` (the restart of its state effect) into buffer `b0`, and the
//! program decides what it does with them.
//!
//! The analysis over-approximates, per scalar component. It is flow-insensitive over variables
//! (every store to a local or private variable, the naga SPIR-V frontend's entry-point wrapper
//! globals included, joins its value) and interprocedural (arguments and results join over every
//! call). A value is either an exact affine combination of buffer scalars and a literal, or
//! opaque: the buffer bytes it may depend on directly, and the saturating functions (`clamp`,
//! `saturate`, `min`, `max`, `smoothstep`, `step`) of an affine combination with literal bounds it
//! may depend on through their result. A branch, a loop's exits and a switch add the deps of
//! their condition to everything stored, returned or discarded under them. What reaches the
//! stage's output (the entry point's result, a `discard`, a return taken under a condition, a
//! write outside the variables) is the [`Clock`]. A form it does not follow (atomics, image
//! stores, subgroup, ray or cooperative operations, a fixpoint that does not converge) makes every
//! buffer byte a direct input, which never settles.
use naga::{
    AddressSpace, BinaryOperator, Block, Expression, Function, Handle, Literal, MathFunction,
    Module, Scalar, Statement, TypeInner, UnaryOperator, VectorSize,
    valid::{FunctionInfo, ModuleInfo},
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// A 32-bit scalar of a group-0 buffer: its binding and byte offset.
pub type Slot = (u32, u32);

/// A saturating function of an affine combination of buffer scalars: constant once
/// `Σ k·slot + constant` has reached `high` going up, or `low` going down (passed it, when
/// `open_low`).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Bounded {
    pub terms: Vec<(Slot, f64)>,
    pub constant: f64,
    pub low: Option<f64>,
    pub high: Option<f64>,
    /// At `low` itself the output is not yet the one below it: `step(edge, x)` is `x >= edge`
    /// (GLSL.std.450 `Step`, naga's `step`), 1 at the edge and 0 only once `x` is below it.
    pub open_low: bool,
}

/// What a stage's output depends on among its group-0 buffers' bytes.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Clock {
    /// Byte ranges `(binding, start, end)` whose value reaches the output other than through a
    /// [`Bounded`] function.
    pub direct: Vec<(u32, u32, u32)>,
    pub bounded: Vec<Bounded>,
}

impl Clock {
    /// The first duration after the restart `since` (a scalar that stays fixed) from which the
    /// output no longer changes as `time` grows; `None` when it keeps changing; `Some(0.0)` for
    /// an output that does not read `time`. Only `k·(time - since) + c` saturating at the bound
    /// it moves towards settles: any other use of `time` keeps changing, or changes at a moment
    /// that is not a duration since the restart. A bound reached going down that is still open
    /// there ([`Bounded::open_low`]) settles only past it, at the next `f32` duration, so a
    /// caller comparing `elapsed < settles` keeps presenting frames at the boundary itself.
    pub fn settles(&self, time: Slot, since: Slot) -> Option<f32> {
        let covers = |&(binding, start, end): &(u32, u32, u32)| {
            binding == time.0 && start < time.1 + 4 && time.1 < end
        };
        if self.direct.iter().any(covers) {
            return None;
        }
        let mut settles = 0.0_f32;
        for bounded in &self.bounded {
            let Some(&(_, k)) = bounded.terms.iter().find(|(slot, _)| *slot == time) else {
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
            let bound = if k > 0.0 { bounded.high } else { bounded.low }?;
            let mut at = ((bound - bounded.constant) / k) as f32;
            if k < 0.0 && bounded.open_low {
                at = at.next_up();
            }
            settles = settles.max(at);
        }
        Some(settles)
    }
}

/// The [`Clock`] of the module's first entry point.
pub fn analyse(module: &Module, info: &ModuleInfo) -> Clock {
    let mut analysis = Analysis {
        module,
        info,
        vars: HashMap::new(),
        args: HashMap::new(),
        results: HashMap::new(),
        contexts: HashMap::new(),
        loops: HashMap::new(),
        output: Deps::default(),
        changed: false,
        unsupported: false,
    };
    // Each pass only grows the joined states, which range over finite sets of spans and bounded
    // functions of the module's own expressions; the bound only guards that argument.
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

/// Every group-0 buffer's whole byte range.
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
    /// Adds `other`; whether that grew it.
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

#[derive(Clone, Debug, PartialEq)]
enum Val {
    /// `Σ k·slot + constant`, exactly.
    Affine(BTreeMap<Slot, f64>, f64),
    Opaque(Deps),
}

impl Val {
    /// A value no buffer scalar reaches (an input, a texel, an integer).
    fn free() -> Self {
        Val::Opaque(Deps::default())
    }

    fn literal(value: f64) -> Self {
        Val::Affine(BTreeMap::new(), value)
    }

    fn deps(&self) -> Deps {
        match self {
            Val::Affine(terms, _) => Deps {
                direct: terms
                    .keys()
                    .map(|&(binding, offset)| (binding, offset, offset + 4))
                    .collect(),
                bounded: Vec::new(),
            },
            Val::Opaque(deps) => deps.clone(),
        }
    }

    fn constant(&self) -> Option<f64> {
        match self {
            Val::Affine(terms, value) if terms.is_empty() => Some(*value),
            _ => None,
        }
    }

    fn scaled(&self, factor: f64) -> Option<Self> {
        match self {
            Val::Affine(terms, value) => Some(Val::Affine(
                terms.iter().map(|(slot, k)| (*slot, k * factor)).collect(),
                value * factor,
            )),
            Val::Opaque(_) => None,
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

/// Joins `new` into a slot; whether the slot changed.
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
        (Val::Affine(x, c), Val::Affine(y, d)) => {
            let mut terms = x.clone();
            for (slot, k) in y {
                *terms.entry(*slot).or_default() += sign * k;
            }
            terms.retain(|_, k| *k != 0.0);
            Val::Affine(terms, c + sign * d)
        }
        _ => opaque([a, b]),
    }
}

fn product(x: &Val, y: &Val) -> Val {
    match (x.constant(), y.constant()) {
        (Some(c), _) => y.scaled(c),
        (_, Some(c)) => x.scaled(c),
        _ => None,
    }
    .unwrap_or_else(|| opaque([x, y]))
}

/// `value` saturating at `low`/`high` (either may be absent) when it is affine in some scalar and
/// each bound given is a literal; otherwise the operands join. `open_low`: [`Bounded::open_low`].
fn bounded(value: &Val, low: Option<&Val>, high: Option<&Val>, open_low: bool) -> Val {
    let literal = |bound: Option<&Val>| match bound {
        None => Some(None),
        Some(bound) => bound.constant().map(Some),
    };
    match (value, literal(low), literal(high)) {
        (Val::Affine(terms, constant), Some(low), Some(high)) if !terms.is_empty() => {
            Val::Opaque(Deps {
                direct: BTreeSet::new(),
                bounded: vec![Bounded {
                    terms: terms.iter().map(|(slot, k)| (*slot, *k)).collect(),
                    constant: *constant,
                    low,
                    high,
                    open_low,
                }],
            })
        }
        _ => opaque([Some(value), low, high].into_iter().flatten()),
    }
}

/// A type as the analysis walks it: a module type, or a column or component of one.
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

/// Scalars a value of `shape` holds.
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

/// Bytes a buffer member of `shape` spans (matrix columns at their vector alignment).
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

/// The shape one index into `shape` selects (an element, a column, a component).
fn element<'m>(module: &'m Module, shape: Shape<'m>) -> Shape<'m> {
    match shape.normal() {
        Shape::Of(TypeInner::Array { base, .. }) => Shape::of(module, *base),
        Shape::Of(TypeInner::Matrix { rows, scalar, .. }) => Shape::Vector(*rows, *scalar),
        Shape::Vector(_, scalar) => Shape::Scalar(scalar),
        shape => shape,
    }
}

/// The scalars before member `index` of a struct shape, and the member's.
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

/// Each scalar of a buffer value of `shape` at `offset`.
fn buffer_values(module: &Module, binding: u32, offset: u32, shape: Shape, out: &mut Vec<Val>) {
    match shape.normal() {
        Shape::Scalar(scalar) if scalar.width == 4 => {
            out.push(Val::Affine([((binding, offset), 1.0)].into(), 0.0));
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
        // A matrix, or a scalar of another width: every scalar depends on the whole span.
        shape => {
            let whole = Val::Opaque(Deps {
                direct: [(binding, offset, offset + size(shape))].into(),
                bounded: Vec::new(),
            });
            out.extend(std::iter::repeat_n(whole, comps(module, shape)));
        }
    }
}

/// A variable a store or load reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Var {
    Local(Option<Handle<Function>>, Handle<naga::LocalVariable>),
    Global(Handle<naga::GlobalVariable>),
}

/// Where a pointer points.
enum Place<'m> {
    /// A buffer's bytes from `offset`, of `shape`; `whole` once an index the analysis cannot
    /// read, or a matrix, hides the exact scalars (the span, and the index's deps).
    Buffer {
        binding: u32,
        offset: u32,
        shape: Shape<'m>,
        whole: Option<((u32, u32, u32), Deps)>,
    },
    /// Scalars `start..start + len` of a variable; `dynamic` when an index picks among them.
    Var {
        var: Var,
        start: usize,
        len: usize,
        dynamic: Option<Deps>,
    },
    /// Anything else (storage, handles): a write there is output.
    Other,
}

struct Analysis<'m> {
    module: &'m Module,
    info: &'m ModuleInfo,
    vars: HashMap<Var, Vec<Option<Val>>>,
    args: HashMap<Handle<Function>, Vec<Vec<Option<Val>>>>,
    results: HashMap<Handle<Function>, Vec<Option<Val>>>,
    /// The joined control context every call of a function runs under.
    contexts: HashMap<Handle<Function>, Deps>,
    /// Each loop's exit conditions, by its body's address.
    loops: HashMap<*const Block, Deps>,
    output: Deps,
    changed: bool,
    unsupported: bool,
}

/// One function's pass: its expression values, each computed once.
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

    /// The context under a condition; inside loops, it is each loop's exit condition too.
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
                    // The entry point's result is the stage's output.
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
                        // Whether it returns here at all decides what it goes on to write.
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

    /// The type of an expression's value.
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

    /// The type a pointer expression points at.
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

    /// A literal integer an index expression is.
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

    /// The place `index` (or an index the analysis cannot read, with its `deps`) selects inside
    /// `base`'s.
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
                    // A matrix column, or an index read at run time: the whole span.
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
                    // A dynamic index anywhere reads and writes the whole variable.
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

    /// Every scalar of `expression` depending on everything `operands` do.
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
                    UnaryOperator::Negate => values
                        .iter()
                        .map(|value| value.scaled(-1.0).unwrap_or_else(|| value.clone()))
                        .collect(),
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
                            BinaryOperator::Subtract => add(x, y, -1.0),
                            BinaryOperator::Multiply => product(x, y),
                            BinaryOperator::Divide => match y.constant() {
                                Some(c) if c != 0.0 => {
                                    x.scaled(1.0 / c).unwrap_or_else(|| opaque([x, y]))
                                }
                                _ => opaque([x, y]),
                            },
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
                            (Some(a), Some(b)) => bounded(
                                &x,
                                Some(&Val::literal(a.min(b))),
                                Some(&Val::literal(a.max(b))),
                                false,
                            ),
                            _ => opaque([&from, &to, &x]),
                        }
                    }),
                    // `x >= edge`: final at the edge going up, only past it going down.
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
            Expression::As { expr, kind, .. } => {
                let values = self.value(frame, *expr);
                let float = |shape: Shape| match shape.normal() {
                    Shape::Scalar(scalar) | Shape::Vector(_, scalar) => {
                        scalar.kind == naga::ScalarKind::Float
                    }
                    Shape::Of(_) => false,
                };
                if *kind == naga::ScalarKind::Float && float(self.shape(frame, *expr)) {
                    values
                } else {
                    values.iter().map(|value| opaque([value])).collect()
                }
            }
            Expression::ImageSample {
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
                self.joined(frame, expression, &operands)
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

/// A module-scope constant's scalars (literals, compositions, zeros, splats).
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
        }
    }

    /// A shade saturating on the time since the restart settles when it saturates; the time
    /// read directly, a moment that is not a duration, or a bound it moves away from never do.
    #[test]
    fn only_a_saturating_duration_settles() {
        let clock = |direct: Vec<(u32, u32, u32)>, bounded: Vec<Bounded>| Clock { direct, bounded };
        assert_eq!(clock(vec![], vec![]).settles(TIME, SINCE), Some(0.0));
        // `AnimationTime` alone is fixed.
        assert_eq!(
            clock(vec![(0, 100, 104)], vec![]).settles(TIME, SINCE),
            Some(0.0)
        );
        assert_eq!(clock(vec![(0, 96, 100)], vec![]).settles(TIME, SINCE), None);
        // A matrix or array span covering it.
        assert_eq!(clock(vec![(0, 64, 128)], vec![]).settles(TIME, SINCE), None);
        // Another binding (the other stage's) is another buffer.
        assert_eq!(
            clock(vec![(1, 96, 100)], vec![]).settles(TIME, SINCE),
            Some(0.0)
        );
        let over = saturating(4.0, 0.0, Some(0.0), Some(1.0));
        let down = saturating(16.0, 0.0, Some(0.0), Some(1.0));
        assert_eq!(
            clock(vec![], vec![over.clone()]).settles(TIME, SINCE),
            Some(0.25)
        );
        assert_eq!(
            clock(vec![], vec![over, down]).settles(TIME, SINCE),
            Some(0.25)
        );
        // Going down to its low bound: `clamp(1 - 2d, 0, 1)` at 0.5 s.
        let falling = saturating(-2.0, 1.0, Some(0.0), Some(1.0));
        assert_eq!(clock(vec![], vec![falling]).settles(TIME, SINCE), Some(0.5));
        // `min(-d, 0)` falls forever.
        let unbounded = saturating(-1.0, 0.0, None, Some(0.0));
        assert_eq!(clock(vec![], vec![unbounded]).settles(TIME, SINCE), None);
        // `saturate(Time * 4)`: a moment on the clock, not a duration since the restart.
        let moment = Bounded {
            terms: vec![(TIME, 4.0)],
            constant: 0.0,
            low: Some(0.0),
            high: Some(1.0),
            open_low: false,
        };
        assert_eq!(clock(vec![], vec![moment]).settles(TIME, SINCE), None);
        // Already past its bound at the restart: settled at once.
        let past = saturating(1.0, 2.0, Some(0.0), Some(1.0));
        assert_eq!(clock(vec![], vec![past]).settles(TIME, SINCE), Some(0.0));
    }

    /// `step(edge, x)` is `x >= edge`. Going up it is final at the edge; going down it is still
    /// 1 at the edge and changes only past it, so it settles at the next duration: at the
    /// boundary itself `elapsed < settles` still holds.
    #[test]
    fn a_descending_step_settles_only_past_its_edge() {
        let step = |k: f64, constant: f64| Clock {
            direct: Vec::new(),
            bounded: vec![Bounded {
                open_low: true,
                ..saturating(k, constant, Some(0.0), Some(0.0))
            }],
        };
        // `step(0, AnimationTime - Time)`: 1 at the restart, 0 after it.
        let drop = step(-1.0, 0.0).settles(TIME, SINCE).unwrap();
        assert!(0.0 < drop);
        assert_eq!(drop, 0.0_f32.next_up());
        // `step(0, 0.5 - (Time - AnimationTime))`: 1 up to and at half a second.
        let late = step(-1.0, 0.5).settles(TIME, SINCE).unwrap();
        assert!(0.5 < late);
        assert_eq!(late, 0.5_f32.next_up());
        // `step(0, Time - AnimationTime - 0.5)`: going up, final at the edge.
        assert_eq!(step(1.0, -0.5).settles(TIME, SINCE), Some(0.5));
        // Already below its edge at the restart: settled at once.
        assert_eq!(step(-1.0, -1.0).settles(TIME, SINCE), Some(0.0));
    }
}
