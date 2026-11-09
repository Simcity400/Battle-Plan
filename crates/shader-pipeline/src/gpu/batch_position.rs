use naga::{
    AddressSpace, BinaryOperator, Binding, BuiltIn, Expression, Handle, Literal, Module, Statement,
    TypeInner,
};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Node {
    Literal(u32),
    Buffer(u32, u32),
    Input(u32, usize),
    Add(usize, usize),
    Mul(usize, usize),
    Neg(usize),
    Dot([(usize, usize); 4], u8),
}

#[derive(Clone, Debug, PartialEq)]
enum Value {
    Unknown,
    Scalar(usize),
    Parts(Vec<Value>),
    Pointer(bool, usize, Vec<usize>),
}

pub(super) struct Plan {
    nodes: Vec<Node>,
    output: [usize; 4],
}

struct Compiler<'a> {
    module: &'a Module,
    nodes: Vec<Node>,
    intern: HashMap<Node, usize>,
    globals: Vec<Value>,
}

#[derive(Clone)]
struct Frame {
    local: Vec<Value>,
    expressions: Vec<Option<Value>>,
    arguments: Vec<Value>,
    result: Option<Value>,
}

impl Compiler<'_> {
    fn node(&mut self, node: Node) -> Value {
        let index = *self.intern.entry(node).or_insert_with(|| {
            self.nodes.push(node);
            self.nodes.len() - 1
        });
        Value::Scalar(index)
    }

    fn constant(&mut self, value: f32) -> Value {
        self.node(Node::Literal(value.to_bits()))
    }

    fn shape(&mut self, ty: Handle<naga::Type>, source: Option<(u32, u32)>) -> Value {
        let scalar = |this: &mut Self, offset| match source {
            Some((binding, base)) => this.node(Node::Buffer(binding, base + offset)),
            None => this.constant(0.0),
        };
        match &self.module.types[ty].inner {
            TypeInner::Scalar(value)
                if value.width == 4 && value.kind == naga::ScalarKind::Float =>
            {
                scalar(self, 0)
            }
            TypeInner::Vector {
                size,
                scalar: value,
            } if value.width == 4 && value.kind == naga::ScalarKind::Float => {
                Value::Parts((0..*size as u32).map(|i| scalar(self, i * 4)).collect())
            }
            TypeInner::Matrix {
                columns,
                rows,
                scalar: value,
            } if value.width == 4 && value.kind == naga::ScalarKind::Float => {
                let stride = if *rows == naga::VectorSize::Bi { 8 } else { 16 };
                Value::Parts(
                    (0..*columns as u32)
                        .map(|column| {
                            Value::Parts(
                                (0..*rows as u32)
                                    .map(|row| scalar(self, column * stride + row * 4))
                                    .collect(),
                            )
                        })
                        .collect(),
                )
            }
            TypeInner::Struct { members, .. } => {
                let members = members.clone();
                Value::Parts(
                    members
                        .iter()
                        .map(|member| {
                            self.shape(
                                member.ty,
                                source.map(|(binding, offset)| (binding, offset + member.offset)),
                            )
                        })
                        .collect(),
                )
            }
            TypeInner::Array {
                base,
                size: naga::ArraySize::Constant(size),
                stride,
            } => {
                let (base, count, stride) = (*base, size.get(), *stride);
                Value::Parts(
                    (0..count)
                        .map(|i| {
                            self.shape(
                                base,
                                source.map(|(binding, offset)| (binding, offset + i * stride)),
                            )
                        })
                        .collect(),
                )
            }
            _ => Value::Unknown,
        }
    }

    fn input(&mut self, ty: Handle<naga::Type>, binding: Option<&Binding>) -> Value {
        match binding {
            Some(Binding::Location { location, .. }) => {
                let count = match self.module.types[ty].inner {
                    TypeInner::Scalar(value) if value.kind == naga::ScalarKind::Float => 1,
                    TypeInner::Vector { size, scalar }
                        if scalar.kind == naga::ScalarKind::Float =>
                    {
                        size as usize
                    }
                    _ => return Value::Unknown,
                };
                let values: Vec<_> = (0..count)
                    .map(|component| self.node(Node::Input(*location, component)))
                    .collect();
                if count == 1 {
                    values.into_iter().next().unwrap()
                } else {
                    Value::Parts(values)
                }
            }
            None => match &self.module.types[ty].inner {
                TypeInner::Struct { members, .. } => {
                    let members = members.clone();
                    Value::Parts(
                        members
                            .iter()
                            .map(|member| self.input(member.ty, member.binding.as_ref()))
                            .collect(),
                    )
                }
                _ => Value::Unknown,
            },
            _ => Value::Unknown,
        }
    }

    fn literal(&mut self, value: Literal) -> Value {
        match value {
            Literal::F32(value) => self.constant(value),
            Literal::I32(value) => self.constant(value as f32),
            Literal::U32(value) => self.constant(value as f32),
            _ => Value::Unknown,
        }
    }

    fn compose(&self, ty: Handle<naga::Type>, values: Vec<Value>) -> Value {
        if matches!(self.module.types[ty].inner, TypeInner::Vector { .. }) {
            Value::Parts(
                values
                    .into_iter()
                    .flat_map(|value| match value {
                        Value::Parts(values) => values,
                        value => vec![value],
                    })
                    .collect(),
            )
        } else {
            Value::Parts(values)
        }
    }

    fn global_expression(&mut self, expression: Handle<Expression>) -> Value {
        match self.module.global_expressions[expression].clone() {
            Expression::Literal(value) => self.literal(value),
            Expression::ZeroValue(ty) => self.shape(ty, None),
            Expression::Constant(handle) => {
                self.global_expression(self.module.constants[handle].init)
            }
            Expression::Compose { ty, components } => {
                let values = components
                    .into_iter()
                    .map(|value| self.global_expression(value))
                    .collect();
                self.compose(ty, values)
            }
            Expression::Splat { size, value } => {
                Value::Parts(vec![self.global_expression(value); size as usize])
            }
            _ => Value::Unknown,
        }
    }

    fn read(&self, pointer: Value, frame: &Frame) -> Value {
        let Value::Pointer(global, index, path) = pointer else {
            return Value::Unknown;
        };
        let mut value = if global {
            self.globals.get(index)
        } else {
            frame.local.get(index)
        };
        for index in path {
            value = match value {
                Some(Value::Parts(parts)) => parts.get(index),
                _ => None,
            };
        }
        value.cloned().unwrap_or(Value::Unknown)
    }

    fn write(&mut self, pointer: Value, value: Value, frame: &mut Frame) {
        let Value::Pointer(global, index, path) = pointer else {
            frame.local.fill(Value::Unknown);
            self.globals.fill(Value::Unknown);
            return;
        };
        let mut destination = if global {
            self.globals.get_mut(index)
        } else {
            frame.local.get_mut(index)
        };
        for index in path {
            destination = match destination {
                Some(Value::Parts(parts)) => parts.get_mut(index),
                _ => None,
            };
        }
        if let Some(destination) = destination {
            *destination = value;
        } else {
            frame.local.fill(Value::Unknown);
            self.globals.fill(Value::Unknown);
        }
    }

    fn access(value: Value, index: usize) -> Value {
        match value {
            Value::Pointer(global, root, mut path) => {
                path.push(index);
                Value::Pointer(global, root, path)
            }
            Value::Parts(parts) => parts.get(index).cloned().unwrap_or(Value::Unknown),
            _ => Value::Unknown,
        }
    }

    fn unary(&mut self, value: Value) -> Value {
        match value {
            Value::Scalar(value) => self.node(Node::Neg(value)),
            Value::Parts(values) => {
                Value::Parts(values.into_iter().map(|value| self.unary(value)).collect())
            }
            _ => Value::Unknown,
        }
    }

    fn binary(&mut self, op: BinaryOperator, left: Value, right: Value) -> Value {
        match (op, left, right) {
            (BinaryOperator::Add, Value::Scalar(a), Value::Scalar(b)) => self.node(Node::Add(a, b)),
            (BinaryOperator::Subtract, Value::Scalar(a), Value::Scalar(b)) => {
                let Value::Scalar(b) = self.node(Node::Neg(b)) else {
                    unreachable!()
                };
                self.node(Node::Add(a, b))
            }
            (BinaryOperator::Multiply, Value::Scalar(a), Value::Scalar(b)) => {
                self.node(Node::Mul(a, b))
            }
            (BinaryOperator::Multiply, Value::Parts(a), Value::Parts(b))
                if a.iter().all(|v| matches!(v, Value::Parts(_)))
                    && b.iter().all(|v| matches!(v, Value::Parts(_))) =>
            {
                Value::Parts(
                    b.into_iter()
                        .map(|column| {
                            self.binary(BinaryOperator::Multiply, Value::Parts(a.clone()), column)
                        })
                        .collect(),
                )
            }
            (BinaryOperator::Multiply, Value::Parts(a), Value::Parts(b))
                if a.first()
                    .is_some_and(|value| matches!(value, Value::Parts(_)))
                    && b.iter().all(|value| matches!(value, Value::Scalar(_))) =>
            {
                let rows = match a.first() {
                    Some(Value::Parts(values)) => values.len(),
                    _ => 0,
                };
                if a.len() != b.len() {
                    return Value::Unknown;
                }
                Value::Parts(
                    (0..rows)
                        .map(|row| {
                            let mut pairs = [(0, 0); 4];
                            for (index, (column, factor)) in a.iter().zip(&b).enumerate() {
                                let Value::Scalar(left) = Self::access(column.clone(), row) else {
                                    return Value::Unknown;
                                };
                                let Value::Scalar(right) = factor else {
                                    return Value::Unknown;
                                };
                                pairs[index] = (left, *right);
                            }
                            self.node(Node::Dot(pairs, a.len() as u8))
                        })
                        .collect(),
                )
            }
            (BinaryOperator::Multiply, Value::Parts(a), Value::Parts(b))
                if a.iter().all(|value| matches!(value, Value::Scalar(_)))
                    && b.iter().all(|value| matches!(value, Value::Parts(_))) =>
            {
                Value::Parts(
                    b.into_iter()
                        .map(|column| {
                            let Value::Parts(column) = column else {
                                return Value::Unknown;
                            };
                            if column.len() != a.len() {
                                return Value::Unknown;
                            }
                            let mut pairs = [(0, 0); 4];
                            for (index, (left, right)) in a.iter().zip(column).enumerate() {
                                let (Value::Scalar(left), Value::Scalar(right)) = (left, right)
                                else {
                                    return Value::Unknown;
                                };
                                pairs[index] = (*left, right);
                            }
                            self.node(Node::Dot(pairs, a.len() as u8))
                        })
                        .collect(),
                )
            }
            (BinaryOperator::Multiply, Value::Parts(a), Value::Parts(b))
                if a.iter().chain(&b).any(|v| matches!(v, Value::Parts(_))) =>
            {
                Value::Unknown
            }
            (
                op @ (BinaryOperator::Add | BinaryOperator::Subtract | BinaryOperator::Multiply),
                Value::Parts(a),
                Value::Parts(b),
            ) if a.len() == b.len() => Value::Parts(
                a.into_iter()
                    .zip(b)
                    .map(|(a, b)| self.binary(op, a, b))
                    .collect(),
            ),
            (BinaryOperator::Multiply, Value::Parts(a), b @ Value::Scalar(_)) => Value::Parts(
                a.into_iter()
                    .map(|a| self.binary(BinaryOperator::Multiply, a, b.clone()))
                    .collect(),
            ),
            (BinaryOperator::Multiply, a @ Value::Scalar(_), Value::Parts(b)) => Value::Parts(
                b.into_iter()
                    .map(|b| self.binary(BinaryOperator::Multiply, a.clone(), b))
                    .collect(),
            ),
            _ => Value::Unknown,
        }
    }

    fn expression(
        &mut self,
        function: &naga::Function,
        handle: Handle<Expression>,
        frame: &mut Frame,
    ) -> Value {
        if let Some(value) = &frame.expressions[handle.index()] {
            return value.clone();
        }
        let value = match function.expressions[handle].clone() {
            Expression::Literal(value) => self.literal(value),
            Expression::Constant(handle) => {
                self.global_expression(self.module.constants[handle].init)
            }
            Expression::ZeroValue(ty) => self.shape(ty, None),
            Expression::FunctionArgument(index) => frame.arguments[index as usize].clone(),
            Expression::GlobalVariable(handle) => Value::Pointer(true, handle.index(), Vec::new()),
            Expression::LocalVariable(handle) => Value::Pointer(false, handle.index(), Vec::new()),
            Expression::Load { pointer } => {
                let pointer = self.expression(function, pointer, frame);
                self.read(pointer, frame)
            }
            Expression::AccessIndex { base, index } => {
                Self::access(self.expression(function, base, frame), index as usize)
            }
            Expression::Access { base, index } => {
                let index = self.expression(function, index, frame);
                let index = match index {
                    Value::Scalar(index) => match self.nodes[index] {
                        Node::Literal(value) => Some(f32::from_bits(value) as usize),
                        _ => None,
                    },
                    _ => None,
                };
                index.map_or(Value::Unknown, |index| {
                    Self::access(self.expression(function, base, frame), index)
                })
            }
            Expression::Compose { ty, components } => {
                let values = components
                    .into_iter()
                    .map(|value| self.expression(function, value, frame))
                    .collect();
                self.compose(ty, values)
            }
            Expression::Splat { size, value } => {
                Value::Parts(vec![self.expression(function, value, frame); size as usize])
            }
            Expression::Swizzle {
                size,
                vector,
                pattern,
            } => {
                let value = self.expression(function, vector, frame);
                Value::Parts(
                    pattern[..size as usize]
                        .iter()
                        .map(|part| Self::access(value.clone(), *part as usize))
                        .collect(),
                )
            }
            Expression::Unary {
                op: naga::UnaryOperator::Negate,
                expr,
            } => {
                let value = self.expression(function, expr, frame);
                self.unary(value)
            }
            Expression::Binary { op, left, right } => {
                let left = self.expression(function, left, frame);
                let right = self.expression(function, right, frame);
                self.binary(op, left, right)
            }
            Expression::Math {
                fun: naga::MathFunction::Transpose,
                arg,
                ..
            } => match self.expression(function, arg, frame) {
                Value::Parts(columns)
                    if columns
                        .iter()
                        .all(|column| matches!(column, Value::Parts(_))) =>
                {
                    let rows = match columns.first() {
                        Some(Value::Parts(rows)) => rows.len(),
                        _ => 0,
                    };
                    Value::Parts(
                        (0..rows)
                            .map(|row| {
                                Value::Parts(
                                    columns
                                        .iter()
                                        .cloned()
                                        .map(|column| Self::access(column, row))
                                        .collect(),
                                )
                            })
                            .collect(),
                    )
                }
                _ => Value::Unknown,
            },
            Expression::Math {
                fun: naga::MathFunction::Dot,
                arg,
                arg1: Some(other),
                ..
            } => {
                let left = self.expression(function, arg, frame);
                let right = self.expression(function, other, frame);
                match (left, right) {
                    (Value::Parts(left), Value::Parts(right))
                        if left.len() == right.len() && left.len() <= 4 =>
                    {
                        let count = left.len();
                        let mut pairs = [(0, 0); 4];
                        let mut known = true;
                        for (index, (a, b)) in left.into_iter().zip(right).enumerate() {
                            if let (Value::Scalar(a), Value::Scalar(b)) = (a, b) {
                                pairs[index] = (a, b);
                            } else {
                                known = false;
                            }
                        }
                        if known {
                            self.node(Node::Dot(pairs, count as u8))
                        } else {
                            Value::Unknown
                        }
                    }
                    _ => Value::Unknown,
                }
            }
            Expression::Select { accept, reject, .. } => {
                let a = self.expression(function, accept, frame);
                let b = self.expression(function, reject, frame);
                Self::join(a, b)
            }
            _ => Value::Unknown,
        };
        frame.expressions[handle.index()] = Some(value.clone());
        value
    }

    fn join(a: Value, b: Value) -> Value {
        if a == b {
            return a;
        }
        match (a, b) {
            (Value::Parts(a), Value::Parts(b)) if a.len() == b.len() => Value::Parts(
                a.into_iter()
                    .zip(b)
                    .map(|(a, b)| Self::join(a, b))
                    .collect(),
            ),
            _ => Value::Unknown,
        }
    }

    fn block(&mut self, function: &naga::Function, body: &naga::Block, frame: &mut Frame) {
        for statement in body {
            if frame.result.is_some() {
                break;
            }
            match statement {
                Statement::Emit(range) => {
                    for handle in range.clone() {
                        self.expression(function, handle, frame);
                    }
                }
                Statement::Store { pointer, value } => {
                    let pointer = self.expression(function, *pointer, frame);
                    let value = self.expression(function, *value, frame);
                    self.write(pointer, value, frame);
                }
                Statement::Call {
                    function: target,
                    arguments,
                    result,
                } => {
                    let arguments: Vec<_> = arguments
                        .iter()
                        .map(|argument| self.expression(function, *argument, frame))
                        .collect();
                    let value = if arguments
                        .iter()
                        .any(|value| matches!(value, Value::Pointer(false, ..)))
                    {
                        frame.local.fill(Value::Unknown);
                        self.globals.fill(Value::Unknown);
                        Value::Unknown
                    } else {
                        self.function(&self.module.functions[*target], arguments)
                    };
                    if let Some(result) = result {
                        frame.expressions[result.index()] = Some(value);
                    }
                }
                Statement::Return { value } => {
                    frame.result = Some(value.map_or(Value::Parts(Vec::new()), |value| {
                        self.expression(function, value, frame)
                    }))
                }
                Statement::Block(body) => self.block(function, body, frame),
                Statement::If { accept, reject, .. } => {
                    let globals = self.globals.clone();
                    let mut a = frame.clone();
                    self.block(function, accept, &mut a);
                    let a_globals = std::mem::replace(&mut self.globals, globals);
                    let mut b = frame.clone();
                    self.block(function, reject, &mut b);
                    self.globals = a_globals
                        .into_iter()
                        .zip(std::mem::take(&mut self.globals))
                        .map(|(a, b)| Self::join(a, b))
                        .collect();
                    frame.local = a
                        .local
                        .into_iter()
                        .zip(b.local)
                        .map(|(a, b)| Self::join(a, b))
                        .collect();
                    if a.result.is_some() || b.result.is_some() {
                        frame.result = Some(Self::join(
                            a.result.unwrap_or(Value::Unknown),
                            b.result.unwrap_or(Value::Unknown),
                        ));
                    }
                }
                _ => {
                    frame.local.fill(Value::Unknown);
                    self.globals.fill(Value::Unknown);
                    frame.result = Some(Value::Unknown);
                }
            }
        }
    }

    fn function(&mut self, function: &naga::Function, arguments: Vec<Value>) -> Value {
        let mut frame = Frame {
            local: function
                .local_variables
                .iter()
                .map(|(_, local)| self.shape(local.ty, None))
                .collect(),
            expressions: vec![None; function.expressions.len()],
            arguments,
            result: None,
        };
        for (handle, local) in function.local_variables.iter() {
            if let Some(init) = local.init {
                frame.local[handle.index()] = self.expression(function, init, &mut frame);
            }
        }
        self.block(function, &function.body, &mut frame);
        frame.result.unwrap_or(Value::Unknown)
    }
}

impl Plan {
    pub fn new(module: &Module) -> Option<Self> {
        let entry = module.entry_points.first()?;
        let mut compiler = Compiler {
            module,
            nodes: Vec::new(),
            intern: HashMap::new(),
            globals: Vec::new(),
        };
        for (_, global) in module.global_variables.iter() {
            let value = match (global.space, global.binding.as_ref()) {
                (AddressSpace::Uniform, Some(binding)) if binding.group == 0 => {
                    compiler.shape(global.ty, Some((binding.binding, 0)))
                }
                (AddressSpace::Private, _) => match global.init {
                    Some(value) => compiler.global_expression(value),
                    None => compiler.shape(global.ty, None),
                },
                _ => Value::Unknown,
            };
            compiler.globals.push(value);
        }
        let arguments = entry
            .function
            .arguments
            .iter()
            .map(|argument| compiler.input(argument.ty, argument.binding.as_ref()))
            .collect();
        let result = compiler.function(&entry.function, arguments);
        let binding = entry.function.result.as_ref()?;
        let value = match &module.types[binding.ty].inner {
            TypeInner::Struct { members, .. } => {
                let at = members.iter().position(|member| {
                    matches!(
                        member.binding,
                        Some(Binding::BuiltIn(BuiltIn::Position { .. }))
                    )
                })?;
                Compiler::access(result, at)
            }
            _ if matches!(
                binding.binding,
                Some(Binding::BuiltIn(BuiltIn::Position { .. }))
            ) =>
            {
                result
            }
            _ => return None,
        };
        let Value::Parts(parts) = value else {
            return None;
        };
        if parts.len() != 4 {
            return None;
        }
        let mut output = [0; 4];
        for (target, value) in output.iter_mut().zip(parts) {
            let Value::Scalar(value) = value else {
                return None;
            };
            *target = value;
        }
        let mut needed = vec![false; compiler.nodes.len()];
        let mut stack = output.to_vec();
        while let Some(index) = stack.pop() {
            if std::mem::replace(&mut needed[index], true) {
                continue;
            }
            match compiler.nodes[index] {
                Node::Add(a, b) | Node::Mul(a, b) => stack.extend([a, b]),
                Node::Neg(a) => stack.push(a),
                Node::Dot(pairs, count) => {
                    for &(a, b) in &pairs[..count as usize] {
                        stack.extend([a, b]);
                    }
                }
                _ => {}
            }
        }
        let mut nodes = Vec::new();
        let mut mapped = vec![0; compiler.nodes.len()];
        for (index, node) in compiler.nodes.into_iter().enumerate() {
            if !needed[index] {
                continue;
            }
            mapped[index] = nodes.len();
            nodes.push(match node {
                Node::Add(a, b) => Node::Add(mapped[a], mapped[b]),
                Node::Mul(a, b) => Node::Mul(mapped[a], mapped[b]),
                Node::Neg(a) => Node::Neg(mapped[a]),
                Node::Dot(mut pairs, count) => {
                    for (a, b) in &mut pairs[..count as usize] {
                        *a = mapped[*a];
                        *b = mapped[*b];
                    }
                    Node::Dot(pairs, count)
                }
                node => node,
            });
        }
        Some(Self {
            nodes,
            output: output.map(|index| mapped[index]),
        })
    }

    pub fn bounds(
        &self,
        constants: &[(u32, Vec<u8>)],
        inputs: &[(u32, usize, usize)],
        vertices: &[f32],
        stride: usize,
        size: [u32; 2],
        values: &mut Vec<(f64, f64)>,
    ) -> Option<[f32; 4]> {
        let round = |low: f64, high: f64| -> Option<(f64, f64)> {
            if !low.is_finite()
                || !high.is_finite()
                || low.abs().max(high.abs()) > f64::from(f32::MAX)
            {
                return None;
            }
            let low = f64::from((low as f32).next_down());
            let high = f64::from((high as f32).next_up());
            Some((
                low.min(if low.abs() < f64::from(f32::MIN_POSITIVE) {
                    0.0
                } else {
                    low
                }),
                high.max(if high.abs() < f64::from(f32::MIN_POSITIVE) {
                    0.0
                } else {
                    high
                }),
            ))
        };
        let product = |a: (f64, f64), b: (f64, f64)| {
            let products = [a.0 * b.0, a.0 * b.1, a.1 * b.0, a.1 * b.1];
            (
                products.into_iter().fold(f64::INFINITY, f64::min),
                products.into_iter().fold(f64::NEG_INFINITY, f64::max),
            )
        };
        values.clear();
        for node in &self.nodes {
            let mut value = match *node {
                Node::Literal(value) => {
                    let value = f64::from(f32::from_bits(value));
                    (value, value)
                }
                Node::Input(location, component) => {
                    let (_, start, count) = inputs.iter().find(|(key, _, _)| *key == location)?;
                    if component >= *count {
                        return None;
                    }
                    let mut low = f64::INFINITY;
                    let mut high = f64::NEG_INFINITY;
                    for vertex in vertices.chunks_exact(stride) {
                        let value = f64::from(*vertex.get(start + component)?);
                        if !value.is_finite() {
                            return None;
                        }
                        low = low.min(value);
                        high = high.max(value);
                    }
                    (low, high)
                }
                Node::Buffer(binding, offset) => {
                    let bytes = &constants.iter().find(|(slot, _)| *slot == binding)?.1;
                    let at = offset as usize;
                    let value =
                        f64::from(f32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
                    (value, value)
                }
                Node::Neg(a) => (-values[a].1, -values[a].0),
                Node::Add(a, b) => round(values[a].0 + values[b].0, values[a].1 + values[b].1)?,
                Node::Mul(a, b) => {
                    let value = product(values[a], values[b]);
                    round(value.0, value.1)?
                }
                Node::Dot(pairs, count) => {
                    let mut low = 0.0;
                    let mut high = 0.0;
                    let mut magnitude = 0.0;
                    for &(a, b) in &pairs[..count as usize] {
                        let value = product(values[a], values[b]);
                        low += value.0;
                        high += value.1;
                        magnitude += value.0.abs().max(value.1.abs());
                    }
                    let operations = f64::from(count) * 2.0;
                    let unit = f64::from(f32::EPSILON) * 0.5;
                    let error = operations * unit / (1.0 - operations * unit) * magnitude
                        + operations * f64::from(f32::MIN_POSITIVE);
                    round(low - error, high + error)?
                }
            };
            if value.0 > 0.0 && value.0 < f64::from(f32::MIN_POSITIVE) {
                value.0 = 0.0;
            }
            if value.1 < 0.0 && value.1 > -f64::from(f32::MIN_POSITIVE) {
                value.1 = 0.0;
            }
            if !value.0.is_finite() || !value.1.is_finite() {
                return None;
            }
            values.push(value);
        }
        let [x, y, _, w] = self.output.map(|index| values[index]);
        if w.0 <= 0.0 {
            return None;
        }
        let divide = |value: (f64, f64)| {
            let candidates = [value.0 / w.0, value.0 / w.1, value.1 / w.0, value.1 / w.1];
            (
                candidates.into_iter().fold(f64::INFINITY, f64::min),
                candidates.into_iter().fold(f64::NEG_INFINITY, f64::max),
            )
        };
        let (x, y) = (divide(x), divide(y));
        Some([
            ((x.0 + 1.0) * 0.5 * f64::from(size[0])).floor() as f32 - 1.0,
            ((1.0 - y.1) * 0.5 * f64::from(size[1])).floor() as f32 - 1.0,
            ((x.1 + 1.0) * 0.5 * f64::from(size[0])).ceil() as f32 + 1.0,
            ((1.0 - y.0) * 0.5 * f64::from(size[1])).ceil() as f32 + 1.0,
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_products_use_columns_and_unknown_vectors_stay_unknown() {
        let module = Module::default();
        let mut compiler = Compiler {
            module: &module,
            nodes: Vec::new(),
            intern: HashMap::new(),
            globals: Vec::new(),
        };
        let matrix = |compiler: &mut Compiler<'_>, columns: [[f32; 2]; 2]| {
            Value::Parts(
                columns
                    .into_iter()
                    .map(|column| {
                        Value::Parts(column.into_iter().map(|v| compiler.constant(v)).collect())
                    })
                    .collect(),
            )
        };
        let left = matrix(&mut compiler, [[1.0, 2.0], [3.0, 4.0]]);
        let right = matrix(&mut compiler, [[5.0, 6.0], [7.0, 8.0]]);
        let product = compiler.binary(BinaryOperator::Multiply, left.clone(), right);
        let mut values = Vec::<f32>::new();
        for node in &compiler.nodes {
            values.push(match *node {
                Node::Literal(v) => f32::from_bits(v),
                Node::Dot(pairs, count) => pairs[..count as usize]
                    .iter()
                    .map(|(a, b)| values[*a] * values[*b])
                    .sum(),
                _ => panic!("unexpected matrix expression"),
            });
        }
        let Value::Parts(columns) = product else {
            panic!("matrix product missing")
        };
        let actual: Vec<Vec<f32>> = columns
            .into_iter()
            .map(|column| {
                let Value::Parts(parts) = column else {
                    panic!("matrix column missing")
                };
                parts
                    .into_iter()
                    .map(|v| {
                        let Value::Scalar(i) = v else {
                            panic!("matrix element missing")
                        };
                        values[i]
                    })
                    .collect()
            })
            .collect();
        assert_eq!(actual, vec![vec![23.0, 34.0], vec![31.0, 46.0]]);
        let one = compiler.constant(1.0);
        assert_eq!(
            compiler.binary(
                BinaryOperator::Multiply,
                left,
                Value::Parts(vec![one, Value::Unknown])
            ),
            Value::Unknown
        );
    }
}
