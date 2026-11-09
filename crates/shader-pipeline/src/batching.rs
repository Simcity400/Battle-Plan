use naga::{AddressSpace, Binding, Block, BuiltIn, Module, Statement, TypeInner};

pub(crate) fn independent(module: &Module) -> bool {
    if module.entry_points.len() != 1 {
        return false;
    }
    module.global_variables.iter().all(|(_, variable)| {
        matches!(
            variable.space,
            AddressSpace::Private | AddressSpace::Uniform | AddressSpace::Handle
        )
    }) && module.entry_points.iter().all(|entry| {
        entry
            .function
            .arguments
            .iter()
            .all(|argument| input(module, argument.ty, argument.binding.as_ref()))
            && block(&entry.function.body)
    }) && module
        .functions
        .iter()
        .all(|(_, function)| block(&function.body))
}

fn input(module: &Module, ty: naga::Handle<naga::Type>, binding: Option<&Binding>) -> bool {
    match binding {
        Some(Binding::BuiltIn(builtin)) => matches!(
            builtin,
            BuiltIn::Position { .. }
                | BuiltIn::ViewIndex
                | BuiltIn::BaseInstance
                | BuiltIn::BaseVertex
                | BuiltIn::InstanceIndex
                | BuiltIn::FrontFacing
                | BuiltIn::PointCoord
                | BuiltIn::SampleIndex
                | BuiltIn::SampleMask
                | BuiltIn::Barycentric { .. }
        ),
        Some(Binding::Location { .. }) => true,
        None => match &module.types[ty].inner {
            TypeInner::Struct { members, .. } => members
                .iter()
                .all(|member| input(module, member.ty, member.binding.as_ref())),
            _ => false,
        },
    }
}

fn block(body: &Block) -> bool {
    body.iter().all(|statement| match statement {
        Statement::Emit(_)
        | Statement::Break
        | Statement::Continue
        | Statement::Return { .. }
        | Statement::Kill
        | Statement::Store { .. }
        | Statement::Call { .. } => true,
        Statement::Block(body) => block(body),
        Statement::If { accept, reject, .. } => block(accept) && block(reject),
        Statement::Switch { cases, .. } => cases.iter().all(|case| block(&case.body)),
        Statement::Loop {
            body, continuing, ..
        } => block(body) && block(continuing),
        _ => false,
    })
}
