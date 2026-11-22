use std::collections::BTreeSet;
use syn::{
    Expr, ExprCall, Item, ItemFn,
    visit::{self, Visit},
};

fn path(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}
fn production(attrs: &[syn::Attribute]) -> bool {
    !attrs.iter().any(|a| {
        a.path().is_ident("cfg")
            && a.parse_args::<syn::Path>()
                .is_ok_and(|p| p.is_ident("test"))
    })
}
#[derive(Default)]
struct Discovery {
    in_run: bool,
    audited: bool,
    types: BTreeSet<String>,
    routes: BTreeSet<String>,
    errors: Vec<String>,
}
impl Discovery {
    fn parsed(&mut self, expressions: &[Expr]) {
        for expression in expressions {
            self.visit_expr(expression);
        }
    }
    fn reject_hook_import(&mut self, tree: &syn::UseTree) {
        match tree {
            syn::UseTree::Path(p) => self.reject_hook_import(&p.tree),
            syn::UseTree::Group(g) => {
                for item in &g.items {
                    self.reject_hook_import(item);
                }
            }
            syn::UseTree::Name(n) if n.ident == "Hooks" => self
                .errors
                .push("runtime hook type imports require canonical constructor paths".into()),
            syn::UseTree::Rename(n) if n.ident == "Hooks" => self
                .errors
                .push("runtime hook constructor aliases are unsupported".into()),
            _ => (),
        }
    }
}
impl<'ast> Visit<'ast> for Discovery {
    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        let syntax = item.mac.tokens.to_string();
        let fingerprint = syntax.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        });
        self.routes.insert(format!(
            "macro-definition:{}:{fingerprint:016x}",
            item.ident
                .as_ref()
                .map_or("<anonymous>".into(), ToString::to_string)
        ));
    }
    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        self.reject_hook_import(&item.tree);
    }
    fn visit_item_type(&mut self, item: &'ast syn::ItemType) {
        if let syn::Type::Path(p) = item.ty.as_ref()
            && p.path.segments.last().is_some_and(|s| s.ident == "Hooks")
        {
            self.errors
                .push("runtime hook type aliases are unsupported".into());
        }
        visit::visit_item_type(self, item);
    }
    fn visit_item(&mut self, item: &'ast Item) {
        let attrs = match item {
            Item::Fn(i) => &i.attrs,
            Item::Mod(i) => &i.attrs,
            _ => return visit::visit_item(self, item),
        };
        if production(attrs) {
            visit::visit_item(self, item);
        }
    }
    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        let previous = self.in_run;
        self.in_run = function.sig.ident == "run";
        let audited = self.audited;
        self.audited = false;
        visit::visit_item_fn(self, function);
        self.audited = audited;
        self.in_run = previous;
    }
    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        let Expr::Path(callee) = call.func.as_ref() else {
            let immediate_closure = matches!(call.func.as_ref(), Expr::Paren(p) if matches!(p.expr.as_ref(), Expr::Closure(_)));
            if self.in_run && !immediate_closure {
                self.errors
                    .push("indirect runtime call requires explicit inventory review".into());
            }
            return visit::visit_expr_call(self, call);
        };
        let name = path(&callee.path);
        if name == "FamilyHooks::construct" {
            if !self.in_run || self.audited || call.args.len() != 2 {
                self.errors
                    .push("unsupported FamilyHooks::construct form".into());
                return;
            }
            let Some(Expr::Closure(closure)) = call.args.last() else {
                self.errors
                    .push("runtime hook factory must be a direct audited closure".into());
                return;
            };
            let before = self.types.len();
            self.audited = true;
            self.visit_expr(&closure.body);
            self.audited = false;
            if self.types.len() != before + 1 {
                self.errors.push(
                    "audited closure must construct one distinct concrete runtime hook type".into(),
                );
            }
            self.visit_expr(&call.args[0]);
            return;
        }
        if name.ends_with("::Hooks::new") {
            if !self.in_run
                || !self.audited
                || !name
                    .split("::")
                    .next()
                    .is_some_and(|root| root == "crate" || root.starts_with("collector_"))
            {
                self.errors.push(format!(
                    "{name} bypasses the direct registered runtime constructor"
                ));
            } else if !self.types.insert(name.trim_end_matches("::new").to_owned()) {
                self.errors
                    .push(format!("duplicate runtime constructor {name}"));
            }
        } else if self.in_run && name == "Hook::new" && !self.audited {
            if !self.types.insert("collector_hooks::hook::Hook".into()) {
                self.errors
                    .push("duplicate essential hourly constructor".into());
            }
        } else if self.in_run {
            self.routes.insert(format!("call:{name}"));
        }
        visit::visit_expr_path(self, callee);
        for argument in &call.args {
            self.visit_expr(argument);
        }
    }
    fn visit_expr_path(&mut self, expression: &'ast syn::ExprPath) {
        let name = path(&expression.path);
        if name.ends_with("::Hooks::new") {
            self.errors
                .push(format!("unregistered constructor reference {name}"));
        }
        visit::visit_expr_path(self, expression);
    }
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if self.in_run {
            self.routes.insert(format!("method:{}", call.method));
        }
        visit::visit_expr_method_call(self, call);
    }
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if !self.in_run {
            return;
        }
        use syn::parse::Parser;
        let name = path(&mac.path);
        self.routes.insert(format!("macro:{name}"));
        let expressions = if name == "collector_status::status::status" {
            syn::parse2::<StatusValues>(mac.tokens.clone()).map(|v| v.0)
        } else {
            syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated
                .parse2(mac.tokens.clone())
                .map(|v| v.into_iter().collect())
        };
        match expressions {
            Ok(expressions) => self.parsed(&expressions),
            Err(error) => self
                .errors
                .push(format!("unsupported runtime macro {name}: {error}")),
        }
    }
}
struct StatusValues(Vec<Expr>);
impl syn::parse::Parse for StatusValues {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let content;
        syn::braced!(content in input);
        let mut values = Vec::new();
        while !content.is_empty() {
            let _: syn::LitStr = content.parse()?;
            let _: syn::Token![:] = content.parse()?;
            if content.peek(syn::token::Brace) {
                values.extend(content.parse::<Self>()?.0);
            } else {
                values.push(content.parse()?);
            }
            if content.is_empty() {
                break;
            }
            let _: syn::Token![,] = content.parse()?;
        }
        Ok(Self(values))
    }
}
fn discover(source: &str) -> Result<Discovery, String> {
    let file = syn::parse_file(source).map_err(|e| e.to_string())?;
    let mut found = Discovery::default();
    found.visit_file(&file);
    if !found.errors.is_empty() {
        return Err(found.errors.join("\n"));
    }
    let reviewed: BTreeSet<_> = include_str!("hook_constructor_routes.txt")
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect();
    let unreviewed: Vec<_> = found.routes.difference(&reviewed).collect();
    if !unreviewed.is_empty() {
        return Err(format!(
            "unreviewed runtime call routes (classify explicitly; never auto-accept hook factories):\n{}",
            unreviewed
                .into_iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    Ok(found)
}

pub(super) fn registered_types(source: &str) -> Result<BTreeSet<String>, String> {
    discover(source).map(|found| found.types)
}

#[test]
fn quiescence_refusal_retains_every_constructed_family_owner() {
    use proc_macro2::TokenTree;
    #[derive(Default)]
    struct Owners {
        constructed: BTreeSet<String>,
        retained: BTreeSet<String>,
    }
    fn identifiers(tokens: proc_macro2::TokenStream, out: &mut BTreeSet<String>) {
        for token in tokens {
            match token {
                TokenTree::Ident(ident) => {
                    out.insert(ident.to_string());
                }
                TokenTree::Group(group) => identifiers(group.stream(), out),
                _ => {}
            }
        }
    }
    fn forgotten(tokens: proc_macro2::TokenStream, out: &mut BTreeSet<String>) {
        let mut tokens = tokens.into_iter();
        while let Some(token) = tokens.next() {
            match token {
                TokenTree::Ident(ident) if ident == "forget" => {
                    if let Some(TokenTree::Group(group)) = tokens.next() {
                        identifiers(group.stream(), out);
                    }
                }
                TokenTree::Group(group) => forgotten(group.stream(), out),
                _ => {}
            }
        }
    }
    impl<'ast> Visit<'ast> for Owners {
        fn visit_local(&mut self, local: &'ast syn::Local) {
            if let Some(init) = &local.init {
                let expr = match &*init.expr {
                    Expr::Try(expr) => &*expr.expr,
                    expr => expr,
                };
                if let Expr::Call(call) = expr {
                    if matches!(&*call.func, Expr::Path(callee) if path(&callee.path) == "FamilyHooks::construct")
                    {
                        let syn::Pat::Ident(binding) = &local.pat else {
                            panic!("hook owner must be named");
                        };
                        self.constructed.insert(binding.ident.to_string());
                    }
                }
            }
            visit::visit_local(self, local);
        }
        fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
            if item
                .ident
                .as_ref()
                .is_some_and(|ident| ident == "retain_detach")
            {
                forgotten(item.mac.tokens.clone(), &mut self.retained);
            }
        }
    }
    let file = syn::parse_file(include_str!("runtime.rs")).unwrap();
    let run = file
        .items
        .iter()
        .find_map(|item| match item {
            Item::Fn(function) if function.sig.ident == "run" => Some(function),
            _ => None,
        })
        .unwrap();
    let mut owners = Owners::default();
    owners.visit_block(&run.block);
    assert!(!owners.constructed.is_empty());
    assert!(
        owners.constructed.is_subset(&owners.retained),
        "unretained native hook owners: {:?}",
        owners
            .constructed
            .difference(&owners.retained)
            .collect::<Vec<_>>()
    );
}

#[test]
fn constructor_values_cannot_hide_behind_an_approved_read_route() {
    let canonical = "fn run() { FamilyHooks::construct(base, || unsafe { BODY crate::portrait_changes::Hooks::new(&memory, base) }); }";
    assert!(discover(&canonical.replace("BODY", "let _value = read(&memory, base);")).is_ok());
    for body in [
        "let read = crate::unregistered::Hooks::new; let _hidden = read(&memory, base);",
        "format!(\"{:?}\", { let read = crate::unregistered::Hooks::new; read(&memory, base) });",
    ] {
        let error = discover(&canonical.replace("BODY", body))
            .err()
            .expect("constructor value escaped through approved read route");
        assert!(
            error.contains("unregistered constructor reference crate::unregistered::Hooks::new"),
            "{error}"
        );
    }
}

#[test]
fn unsupported_alias_and_helper_construction_fail_closed() {
    let canonical = "fn run() { let hooks = FamilyHooks::construct(base, || unsafe { crate::extra::Hooks::new(memory, base) }); }";
    assert!(discover(canonical).is_ok());
    for invalid in [
        format!(
            "use crate::extra::Hooks as NewHooks; {canonical} fn helper() {{ NewHooks::new(memory, base); }}"
        ),
        canonical.replace(
            "let hooks =",
            "let missed = NewHooks::new(memory, base); let hooks =",
        ),
        canonical.replace(
            "let hooks =",
            "let missed = crate::extra::prepare(memory, base); let hooks =",
        ),
        canonical.replace(
            "crate::extra::Hooks::new(memory, base)",
            "make_hooks(memory, base)",
        ),
        "fn run() { make_hooks(); } fn make_hooks() { crate::extra::Hooks::new(memory, base); }"
            .into(),
        canonical.replace(
            "let hooks =",
            "format!(\"{:?}\", crate::extra::prepare(memory, base)); let hooks =",
        ),
    ] {
        assert!(
            discover(&invalid).is_err(),
            "unsupported construction escaped the gate: {invalid}"
        );
    }
}
