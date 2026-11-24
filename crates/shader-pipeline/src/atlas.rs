
use naga::{
    AddressSpace, BinaryOperator, Block, Expression, Function, Handle, MathFunction, Module,
    SampleLevel, Statement, TypeInner, valid::FunctionInfo, valid::ModuleInfo,
};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Val {
    Free,
    Uv(u8),
    Rect(u8),
    Product(u8),
    Mapped(u8),
}

impl Val {
    fn free(self) -> bool {
        matches!(self, Val::Free | Val::Uv(_))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Rectangle {
    pub binding: u32,
    pub offset: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct Contract {
    pub rectangle: Rectangle,
    pub uv: Option<u32>,
    pub flag: u32,
}

pub fn check(vertex: &Module, pixel: &Module, contract: Contract) -> Result<(), String> {
    let validate = |module: &Module| {
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(module)
        .map_err(|why| format!("a stage does not validate: {why:?}"))
    };
    let (vertex_info, pixel_info) = (validate(vertex)?, validate(pixel)?);
    let outputs = Stage::run(vertex, &vertex_info, contract, true, &HashMap::new())?;
    Stage::run(pixel, &pixel_info, contract, false, &outputs)?;
    Ok(())
}

fn refuse<T>(why: impl Into<String>) -> Result<T, String> {
    Err(why.into())
}

const OTHER_USE: &str = "the flag rectangle reaches an operation other than its mapping";
const UNFOLLOWED: &str = "the flag program uses a form the proof does not follow";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Var {
    Local(Option<Handle<Function>>, Handle<naga::LocalVariable>),
    Global(Handle<naga::GlobalVariable>),
}

#[derive(Clone, Debug)]
enum Abs {
    Value(Vec<Val>),
    Var {
        var: Var,
        at: usize,
        len: usize,
    },
    Uniform {
        binding: u32,
        offset: u32,
    },
    Resource(Option<naga::ResourceBinding>),
}

struct Stage<'a> {
    module: &'a Module,
    info: &'a ModuleInfo,
    contract: Contract,
    vertex: bool,
    inputs: &'a HashMap<u32, Vec<Val>>,
    vars: HashMap<Var, Vec<Val>>,
    called: bool,
}

type Values = HashMap<Handle<Expression>, Abs>;

impl<'a> Stage<'a> {
    fn run(
        module: &'a Module,
        info: &'a ModuleInfo,
        contract: Contract,
        vertex: bool,
        inputs: &'a HashMap<u32, Vec<Val>>,
    ) -> Result<HashMap<u32, Vec<Val>>, String> {
        let mut stage = Stage {
            module,
            info,
            contract,
            vertex,
            inputs,
            vars: HashMap::new(),
            called: false,
        };
        for (global, variable) in module.global_variables.iter() {
            if variable.space == AddressSpace::Private {
                let width = stage.width(&module.types[variable.ty].inner);
                stage
                    .vars
                    .insert(Var::Global(global), vec![Val::Free; width]);
            }
        }
        let output = stage.call(None)?;
        stage.outputs(&output)
    }

    fn entry(&self) -> Result<&'a naga::EntryPoint, String> {
        self.module
            .entry_points
            .first()
            .ok_or_else(|| "a stage has no entry point".to_owned())
    }

    fn function_info(&self, handle: Option<Handle<Function>>) -> &'a FunctionInfo {
        let info = self.info;
        match handle {
            Some(handle) => &info[handle],
            None => info.get_entry_point(0),
        }
    }

    fn width(&self, ty: &TypeInner) -> usize {
        match ty {
            TypeInner::Scalar(_) => 1,
            TypeInner::Vector { size, .. } => *size as usize,
            TypeInner::Matrix { columns, rows, .. } => *columns as usize * *rows as usize,
            TypeInner::Array {
                base,
                size: naga::ArraySize::Constant(count),
                ..
            } => count.get() as usize * self.width(&self.module.types[*base].inner),
            TypeInner::Struct { members, .. } => members
                .iter()
                .map(|member| self.width(&self.module.types[member.ty].inner))
                .sum(),
            _ => 0,
        }
    }

    fn child(&self, ty: &TypeInner, index: u32) -> Option<(usize, TypeInner)> {
        let types = &self.module.types;
        Some(match ty {
            TypeInner::Struct { members, .. } => {
                let member = members.get(index as usize)?;
                let at = members[..index as usize]
                    .iter()
                    .map(|member| self.width(&types[member.ty].inner))
                    .sum();
                (at, types[member.ty].inner.clone())
            }
            TypeInner::Vector { scalar, .. } => (index as usize, TypeInner::Scalar(*scalar)),
            TypeInner::Matrix { rows, scalar, .. } => (
                index as usize * *rows as usize,
                TypeInner::Vector {
                    size: *rows,
                    scalar: *scalar,
                },
            ),
            TypeInner::Array { base, .. } => {
                let inner = types[*base].inner.clone();
                (index as usize * self.width(&inner), inner)
            }
            _ => return None,
        })
    }

    fn uniform_child(&self, ty: &TypeInner, index: u32) -> Option<(u32, TypeInner)> {
        let types = &self.module.types;
        Some(match ty {
            TypeInner::Struct { members, .. } => {
                let member = members.get(index as usize)?;
                (member.offset, types[member.ty].inner.clone())
            }
            TypeInner::Vector { scalar, .. } => {
                (index * u32::from(scalar.width), TypeInner::Scalar(*scalar))
            }
            TypeInner::Matrix { rows, scalar, .. } => {
                let stride = if *rows == naga::VectorSize::Tri {
                    4
                } else {
                    *rows as u32
                } * u32::from(scalar.width);
                (
                    index * stride,
                    TypeInner::Vector {
                        size: *rows,
                        scalar: *scalar,
                    },
                )
            }
            TypeInner::Array { base, stride, .. } => (index * stride, types[*base].inner.clone()),
            _ => return None,
        })
    }

    fn uniform_scalars(&self, ty: &TypeInner, offset: u32, out: &mut Vec<u32>) {
        if let TypeInner::Scalar(_) = ty {
            out.push(offset);
            return;
        }
        let count = match ty {
            TypeInner::Struct { members, .. } => members.len() as u32,
            TypeInner::Vector { size, .. } => *size as u32,
            TypeInner::Matrix { columns, .. } => *columns as u32,
            TypeInner::Array {
                size: naga::ArraySize::Constant(count),
                ..
            } => count.get(),
            _ => 0,
        };
        for index in 0..count {
            if let Some((at, child)) = self.uniform_child(ty, index) {
                self.uniform_scalars(&child, offset + at, out);
            }
        }
    }

    fn rectangle(&self, binding: u32, offset: u32) -> Val {
        let rectangle = self.contract.rectangle;
        if self.vertex
            && binding == rectangle.binding
            && (rectangle.offset..rectangle.offset + 16).contains(&offset)
        {
            Val::Rect(((offset - rectangle.offset) / 4) as u8)
        } else {
            Val::Free
        }
    }

    fn pointee(&self, info: &FunctionInfo, pointer: Handle<Expression>) -> Option<TypeInner> {
        match info[pointer].ty.inner_with(&self.module.types) {
            TypeInner::Pointer { base, .. } => Some(self.module.types[*base].inner.clone()),
            TypeInner::ValuePointer { size, scalar, .. } => Some(match size {
                Some(size) => TypeInner::Vector {
                    size: *size,
                    scalar: *scalar,
                },
                None => TypeInner::Scalar(*scalar),
            }),
            _ => None,
        }
    }

    fn call(&mut self, handle: Option<Handle<Function>>) -> Result<Vec<Val>, String> {
        let function = match handle {
            Some(handle) => &self.module.functions[handle],
            None => &self.entry()?.function,
        };
        let info = self.function_info(handle);
        for (local, variable) in function.local_variables.iter() {
            let value = match variable.init {
                None => vec![Val::Free; self.width(&self.module.types[variable.ty].inner)],
                Some(init) => match function.expressions[init] {
                    Expression::Literal(_) | Expression::Constant(_) | Expression::ZeroValue(_) => {
                        vec![Val::Free; self.width(&self.module.types[variable.ty].inner)]
                    }
                    _ => return refuse(UNFOLLOWED),
                },
            };
            self.vars.insert(Var::Local(handle, local), value);
        }
        let mut values = Values::new();
        let mut output = Vec::new();
        self.block(
            handle,
            function,
            info,
            &function.body,
            &mut values,
            &mut output,
        )?;
        Ok(output)
    }

    fn value(&self, values: &Values, expression: Handle<Expression>) -> Result<Vec<Val>, String> {
        match values.get(&expression) {
            Some(Abs::Value(value)) => Ok(value.clone()),
            _ => refuse(UNFOLLOWED),
        }
    }

    fn evaluate(
        &self,
        handle: Option<Handle<Function>>,
        function: &Function,
        info: &FunctionInfo,
        values: &mut Values,
        expression: Handle<Expression>,
    ) -> Result<(), String> {
        if values.contains_key(&expression) {
            return Ok(());
        }
        let value = self.expression(handle, function, info, values, expression)?;
        values.insert(expression, value);
        Ok(())
    }

    fn block(
        &mut self,
        handle: Option<Handle<Function>>,
        function: &Function,
        info: &FunctionInfo,
        block: &Block,
        values: &mut Values,
        output: &mut Vec<Val>,
    ) -> Result<(), String> {
        for statement in block.iter() {
            match statement {
                Statement::Emit(range) => {
                    for expression in range.clone() {
                        self.evaluate(handle, function, info, values, expression)?;
                    }
                }
                Statement::Store { pointer, value } => {
                    self.evaluate(handle, function, info, values, *pointer)?;
                    self.evaluate(handle, function, info, values, *value)?;
                    let value = self.value(values, *value)?;
                    let Some(Abs::Var { var, at, len }) = values.get(pointer).cloned() else {
                        return refuse("the flag program stores outside its variables");
                    };
                    let slot = self.vars.get_mut(&var).ok_or(UNFOLLOWED)?;
                    if value.len() != len || slot.len() < at + len {
                        return refuse(UNFOLLOWED);
                    }
                    slot[at..at + len].copy_from_slice(&value);
                }
                Statement::Call {
                    function: callee,
                    arguments,
                    result: None,
                } if handle.is_none() && arguments.is_empty() && !self.called => {
                    self.called = true;
                    self.call(Some(*callee))?;
                }
                Statement::Return { value } => {
                    if let Some(value) = value {
                        self.evaluate(handle, function, info, values, *value)?;
                        *output = self.value(values, *value)?;
                    }
                    return Ok(());
                }
                _ => return refuse(UNFOLLOWED),
            }
        }
        Ok(())
    }

    fn expression(
        &self,
        handle: Option<Handle<Function>>,
        function: &Function,
        info: &FunctionInfo,
        values: &mut Values,
        expression: Handle<Expression>,
    ) -> Result<Abs, String> {
        let width = self.width(info[expression].ty.inner_with(&self.module.types));
        let free = |width: usize| Abs::Value(vec![Val::Free; width]);
        let operand = |values: &mut Values, operand: Handle<Expression>| {
            self.evaluate(handle, function, info, values, operand)?;
            Ok::<_, String>(values[&operand].clone())
        };
        let operands_free = |values: &mut Values, operands: &[Handle<Expression>]| {
            for &handle in operands {
                if let Abs::Value(value) = operand(values, handle)?
                    && !value.iter().all(|value| value.free())
                {
                    return refuse(OTHER_USE);
                }
            }
            Ok::<_, String>(())
        };
        let value = |values: &mut Values, handle: Handle<Expression>| match operand(values, handle)?
        {
            Abs::Value(value) => Ok(value),
            _ => refuse(UNFOLLOWED),
        };
        Ok(match &function.expressions[expression] {
            Expression::Literal(_) | Expression::Constant(_) | Expression::ZeroValue(_) => {
                free(width)
            }
            Expression::Compose { components, .. } => {
                let mut composed = Vec::new();
                for component in components {
                    composed.extend(value(values, *component)?);
                }
                Abs::Value(composed)
            }
            Expression::Splat { size, value: inner } => {
                Abs::Value(value(values, *inner)?.repeat(*size as usize))
            }
            Expression::Swizzle {
                size,
                vector,
                pattern,
            } => {
                let vector = value(values, *vector)?;
                Abs::Value(
                    pattern[..*size as usize]
                        .iter()
                        .map(|component| vector[*component as usize])
                        .collect(),
                )
            }
            Expression::AccessIndex { base, index } => match operand(values, *base)? {
                Abs::Value(base_value) => {
                    let ty = info[*base].ty.inner_with(&self.module.types);
                    let (at, child) = self.child(ty, *index).ok_or(UNFOLLOWED)?;
                    Abs::Value(base_value[at..at + self.width(&child)].to_vec())
                }
                Abs::Var { var, at, .. } => {
                    let ty = self.pointee(info, *base).ok_or(UNFOLLOWED)?;
                    let (offset, child) = self.child(&ty, *index).ok_or(UNFOLLOWED)?;
                    Abs::Var {
                        var,
                        at: at + offset,
                        len: self.width(&child),
                    }
                }
                Abs::Uniform { binding, offset } => {
                    let ty = self.pointee(info, *base).ok_or(UNFOLLOWED)?;
                    let (at, _) = self.uniform_child(&ty, *index).ok_or(UNFOLLOWED)?;
                    Abs::Uniform {
                        binding,
                        offset: offset + at,
                    }
                }
                Abs::Resource(_) => return refuse(UNFOLLOWED),
            },
            Expression::FunctionArgument(index) if handle.is_none() => {
                Abs::Value(self.argument(*index)?)
            }
            Expression::GlobalVariable(global) => {
                let variable = &self.module.global_variables[*global];
                match variable.space {
                    AddressSpace::Uniform => Abs::Uniform {
                        binding: variable.binding.as_ref().map_or(u32::MAX, |b| b.binding),
                        offset: 0,
                    },
                    AddressSpace::Handle => Abs::Resource(variable.binding.clone()),
                    AddressSpace::Private => Abs::Var {
                        var: Var::Global(*global),
                        at: 0,
                        len: self.width(&self.module.types[variable.ty].inner),
                    },
                    _ => return refuse(UNFOLLOWED),
                }
            }
            Expression::LocalVariable(local) => Abs::Var {
                var: Var::Local(handle, *local),
                at: 0,
                len: self.width(&self.module.types[function.local_variables[*local].ty].inner),
            },
            Expression::Load { pointer } => match operand(values, *pointer)? {
                Abs::Var { var, at, len } => {
                    let held = self.vars.get(&var).ok_or(UNFOLLOWED)?;
                    Abs::Value(held.get(at..at + len).ok_or(UNFOLLOWED)?.to_vec())
                }
                Abs::Uniform { binding, offset } => {
                    let ty = self.pointee(info, *pointer).ok_or(UNFOLLOWED)?;
                    let mut scalars = Vec::new();
                    self.uniform_scalars(&ty, offset, &mut scalars);
                    Abs::Value(
                        scalars
                            .iter()
                            .map(|at| self.rectangle(binding, *at))
                            .collect(),
                    )
                }
                _ => return refuse(UNFOLLOWED),
            },
            Expression::ImageSample {
                image,
                sampler,
                gather,
                coordinate,
                array_index,
                offset,
                level,
                depth_ref,
                clamp_to_edge,
            } => {
                let flag = matches!(operand(values, *image)?, Abs::Resource(Some(binding))
                    if binding.group == 1 && binding.binding == self.contract.flag);
                operand(values, *sampler)?;
                let coordinates = value(values, *coordinate)?;
                let ordinary = gather.is_none()
                    && array_index.is_none()
                    && offset.is_none()
                    && depth_ref.is_none()
                    && !clamp_to_edge
                    && matches!(level, SampleLevel::Auto);
                if flag {
                    if !ordinary || coordinates != [Val::Mapped(0), Val::Mapped(1)] {
                        return refuse(
                            "the flag program samples the flag other than at its mapped coordinate",
                        );
                    }
                } else {
                    operands_free(values, &[*coordinate])?;
                }
                let mut others: Vec<Handle<Expression>> = Vec::new();
                others.extend(array_index.iter().chain(offset).chain(depth_ref));
                match level {
                    SampleLevel::Auto | SampleLevel::Zero => {}
                    SampleLevel::Exact(value) | SampleLevel::Bias(value) => others.push(*value),
                    SampleLevel::Gradient { x, y } => others.extend([*x, *y]),
                }
                operands_free(values, &others)?;
                free(width)
            }
            Expression::ImageLoad {
                image,
                coordinate,
                array_index,
                sample,
                level,
            } => {
                self.not_flag(operand(values, *image)?)?;
                let mut operands = vec![*coordinate];
                operands.extend(array_index.iter().chain(sample).chain(level));
                operands_free(values, &operands)?;
                free(width)
            }
            Expression::ImageQuery { image, .. } => {
                self.not_flag(operand(values, *image)?)?;
                free(width)
            }
            Expression::Binary { op, left, right } => {
                let (left, right) = (value(values, *left)?, value(values, *right)?);
                binary(*op, &left, &right, width)?
            }
            Expression::Math {
                fun: MathFunction::Fma,
                arg,
                arg1: Some(arg1),
                arg2: Some(arg2),
                ..
            } => {
                let (a, b, c) = (
                    value(values, *arg)?,
                    value(values, *arg1)?,
                    value(values, *arg2)?,
                );
                match binary(BinaryOperator::Multiply, &a, &b, width)? {
                    Abs::Value(product) => binary(BinaryOperator::Add, &product, &c, width)?,
                    other => other,
                }
            }
            Expression::Math {
                arg,
                arg1,
                arg2,
                arg3,
                ..
            } => {
                let args: Vec<_> = std::iter::once(*arg)
                    .chain(*arg1)
                    .chain(*arg2)
                    .chain(*arg3)
                    .collect();
                operands_free(values, &args)?;
                free(width)
            }
            Expression::Unary { expr, .. }
            | Expression::Derivative { expr, .. }
            | Expression::As { expr, .. } => {
                operands_free(values, &[*expr])?;
                free(width)
            }
            Expression::Relational { argument, .. } => {
                operands_free(values, &[*argument])?;
                free(width)
            }
            _ => return refuse(UNFOLLOWED),
        })
    }

    fn not_flag(&self, image: Abs) -> Result<(), String> {
        match image {
            Abs::Resource(Some(binding))
                if binding.group == 1 && binding.binding == self.contract.flag =>
            {
                refuse("the flag program reads the flag's page other than by sampling")
            }
            _ => Ok(()),
        }
    }

    fn argument(&self, index: u32) -> Result<Vec<Val>, String> {
        let argument = self
            .entry()?
            .function
            .arguments
            .get(index as usize)
            .ok_or(UNFOLLOWED)?;
        let ty = &self.module.types[argument.ty].inner;
        match (&argument.binding, ty) {
            (Some(binding), _) => Ok(self.bound(binding, ty)),
            (None, TypeInner::Struct { members, .. }) => {
                let mut value = Vec::new();
                for member in members {
                    let inner = &self.module.types[member.ty].inner;
                    match &member.binding {
                        Some(binding) => value.extend(self.bound(binding, inner)),
                        None => value.extend(vec![Val::Free; self.width(inner)]),
                    }
                }
                Ok(value)
            }
            _ => Ok(vec![Val::Free; self.width(ty)]),
        }
    }

    fn bound(&self, binding: &naga::Binding, ty: &TypeInner) -> Vec<Val> {
        let width = self.width(ty);
        match binding {
            naga::Binding::Location { location, .. } if self.vertex => {
                if Some(*location) == self.contract.uv {
                    (0..width).map(|axis| Val::Uv(axis as u8)).collect()
                } else {
                    vec![Val::Free; width]
                }
            }
            naga::Binding::Location { location, .. } => {
                let mut value = self
                    .inputs
                    .get(location)
                    .cloned()
                    .unwrap_or_else(|| vec![Val::Free; width]);
                value.resize(width, Val::Free);
                value
            }
            naga::Binding::BuiltIn(_) => vec![Val::Free; width],
        }
    }

    fn outputs(&self, output: &[Val]) -> Result<HashMap<u32, Vec<Val>>, String> {
        let entry = self.entry()?;
        let mut outputs = HashMap::new();
        let Some(result) = &entry.function.result else {
            return Ok(outputs);
        };
        let ty = &self.module.types[result.ty].inner;
        let mut bound = Vec::new();
        match (&result.binding, ty) {
            (Some(binding), _) => bound.push((binding.clone(), 0, self.width(ty))),
            (None, TypeInner::Struct { members, .. }) => {
                let mut at = 0;
                for member in members {
                    let width = self.width(&self.module.types[member.ty].inner);
                    if let Some(binding) = &member.binding {
                        bound.push((binding.clone(), at, width));
                    }
                    at += width;
                }
            }
            _ => {}
        }
        for (binding, at, width) in bound {
            let value = output.get(at..at + width).ok_or(UNFOLLOWED)?.to_vec();
            let located = match binding {
                naga::Binding::Location { location, .. } if self.vertex => Some(location),
                _ => None,
            };
            let carried = value
                .iter()
                .all(|value| value.free() || matches!(value, Val::Mapped(_)));
            match located {
                Some(location) if carried => {
                    outputs.insert(location, value);
                }
                _ if value.iter().all(|value| value.free()) => {}
                _ => {
                    return refuse(
                        "the flag rectangle reaches the program's output other than as its mapped coordinate",
                    );
                }
            }
        }
        Ok(outputs)
    }
}

fn binary(op: BinaryOperator, left: &[Val], right: &[Val], width: usize) -> Result<Abs, String> {
    if left.iter().chain(right).all(|value| value.free()) {
        return Ok(Abs::Value(vec![Val::Free; width]));
    }
    let componentwise = left.len() == right.len() || left.len() == 1 || right.len() == 1;
    if !componentwise || width != left.len().max(right.len()) {
        return refuse(OTHER_USE);
    }
    let at = |side: &[Val], index: usize| {
        if side.len() == 1 {
            side[0]
        } else {
            side[index]
        }
    };
    let mut value = Vec::with_capacity(width);
    for index in 0..width {
        let (a, b) = (at(left, index), at(right, index));
        value.push(match (op, a, b) {
            (_, a, b) if a.free() && b.free() => Val::Free,
            (BinaryOperator::Multiply, Val::Uv(axis), Val::Rect(size))
            | (BinaryOperator::Multiply, Val::Rect(size), Val::Uv(axis))
                if size == axis + 2 =>
            {
                Val::Product(axis)
            }
            (BinaryOperator::Add, Val::Product(axis), Val::Rect(offset))
            | (BinaryOperator::Add, Val::Rect(offset), Val::Product(axis))
                if offset == axis =>
            {
                Val::Mapped(axis)
            }
            _ => return refuse(OTHER_USE),
        });
    }
    Ok(Abs::Value(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertex_with(declarations: &str, body: &str, entry: &str) -> Module {
        let source = format!(
            r#"
struct C {{ m: mat4x4<f32>, flag: vec4<f32>, tail: array<vec4<f32>, 2> }}
struct Out {{ @builtin(position) position: vec4<f32>, @location(0) coords: vec4<f32> }}
@group(0) @binding(0) var<uniform> c: C;
var<private> position_in: vec3<f32>;
var<private> uv_in: vec2<f32>;
var<private> position_out: vec4<f32>;
var<private> coords_out: vec4<f32>;
{declarations}
fn body() {{
    let uv = uv_in;
    let flag = c.flag;
    position_out = c.m * vec4<f32>(position_in, 1.0);
    {body}
}}
@vertex
fn main(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>) -> Out {{
    position_in = position;
    uv_in = uv;
    {entry}
}}
"#
        );
        naga::front::wgsl::parse_str(&source).unwrap()
    }

    const CALL: &str = "body();\n    return Out(position_out, coords_out);";

    fn vertex(mapping: &str) -> Module {
        vertex_with("", &format!("coords_out = vec4<f32>({mapping}, uv);"), CALL)
    }

    fn pixel(flag: &str, mask: &str, colour: &str) -> Module {
        let source = format!(
            r#"
@group(1) @binding(1) var flag_texture: texture_2d<f32>;
@group(2) @binding(1) var flag_sampler: sampler;
@group(1) @binding(3) var mask_texture: texture_2d<f32>;
@group(2) @binding(3) var mask_sampler: sampler;
@fragment
fn main(@location(0) coords: vec4<f32>) -> @location(0) vec4<f32> {{
    let sampled = textureSample(flag_texture, flag_sampler, {flag});
    let masked = textureSample(mask_texture, mask_sampler, {mask});
    return {colour};
}}
"#
        );
        naga::front::wgsl::parse_str(&source).unwrap()
    }

    fn contract() -> Contract {
        Contract {
            rectangle: Rectangle {
                binding: 0,
                offset: 64,
            },
            uv: Some(1),
            flag: 1,
        }
    }

    const MAPPED: &str = "flag.xy + uv * flag.zw";
    const COLOUR: &str = "vec4<f32>(sampled.xyz, masked.w)";

    fn checked(mapping: &str, flag: &str, mask: &str, colour: &str) -> Result<(), String> {
        check(&vertex(mapping), &pixel(flag, mask, colour), contract())
    }

    fn checked_vertex(vertex: Module) -> Result<(), String> {
        check(
            &vertex,
            &pixel("coords.xy", "coords.zw", COLOUR),
            contract(),
        )
    }

    #[test]
    fn the_atlas_mapping_alone_draws_from_the_flag_s_own_page() {
        for mapping in [
            MAPPED,
            "uv * flag.zw + flag.xy",
            "fma(uv, flag.zw, flag.xy)",
        ] {
            checked(mapping, "coords.xy", "coords.zw", COLOUR).unwrap();
        }
        checked("uv", "vec2<f32>(0.5)", "coords.zw", COLOUR).unwrap_err();
        checked("uv", "coords.xy", "coords.zw", COLOUR).unwrap_err();
    }

    #[test]
    fn any_other_use_of_the_rectangle_is_refused() {
        for mapping in [
            "flag.xy + uv * flag.zw * (flag.x + flag.z)",
            "flag.zw * 0.75",
            "uv * flag.zw",
            "flag.xy + uv * flag.wz",
            "flag.xy + uv * 2.0 * flag.zw",
            "select(uv, flag.xy + uv * flag.zw, flag.x > 0.0)",
            "select(uv, flag.xy + uv * flag.zw, uv.x > 0.5)",
        ] {
            let why = checked(mapping, "coords.xy", "coords.zw", COLOUR).unwrap_err();
            assert!(
                why.contains("rectangle") || why.contains("flag"),
                "{mapping}: {why}"
            );
        }
    }

    #[test]
    fn dynamic_indexing_anywhere_in_a_uniform_is_refused() {
        let rows = naga::front::wgsl::parse_str(
            r#"
struct C { rows: array<vec4<f32>, 6> }
struct Out { @builtin(position) position: vec4<f32>, @location(0) coords: vec4<f32> }
@group(0) @binding(0) var<uniform> c: C;
@vertex
fn main(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>) -> Out {
    let flag = c.rows[4];
    let i = u32(uv.x);
    let j = u32(uv.y);
    let tint = c.rows[i][j];
    return Out(vec4<f32>(position, tint), vec4<f32>(flag.xy + uv * flag.zw, uv));
}
"#,
        )
        .unwrap();
        assert!(checked_vertex(rows).is_err());
        let tail = vertex_with(
            "",
            "let i = u32(uv.x);\n    coords_out = vec4<f32>(flag.xy + uv * flag.zw, c.tail[i].xy);",
            CALL,
        );
        assert!(checked_vertex(tail).is_err());
    }

    #[test]
    fn branches_and_stores_out_of_order_are_refused() {
        let branched = vertex_with(
            "var<private> coordinate: vec2<f32> = vec2<f32>(0.25);",
            "if uv.x > 0.5 { coordinate = flag.xy + uv * flag.zw; }\n    coords_out = vec4<f32>(coordinate, uv);",
            CALL,
        );
        assert!(checked_vertex(branched).is_err());
        let early = vertex_with(
            "",
            &format!("coords_out = vec4<f32>({MAPPED}, uv);"),
            "let early = coords_out;\n    body();\n    return Out(position_out, early);",
        );
        assert!(checked_vertex(early).is_err());
        let stored = vertex_with("", &format!("coords_out = vec4<f32>({MAPPED}, uv);"), CALL);
        checked_vertex(stored).unwrap();
    }

    #[test]
    fn the_mapped_coordinate_may_only_sample_the_flag() {
        for (flag, mask, colour) in [
            ("coords.xy", "coords.zw", "vec4<f32>(coords.xy, sampled.zw)"),
            ("coords.xy", "coords.xy", COLOUR),
            ("coords.yx", "coords.zw", COLOUR),
            ("coords.xy + vec2<f32>(0.1)", "coords.zw", COLOUR),
            ("coords.zw", "coords.zw", COLOUR),
        ] {
            assert!(
                checked(MAPPED, flag, mask, colour).is_err(),
                "{flag} {mask} {colour}"
            );
        }
    }
}
