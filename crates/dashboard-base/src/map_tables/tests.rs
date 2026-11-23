use super::*;
use proc_macro2::{TokenStream, TokenTree};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use syn::visit::{self, Visit};

#[test]
fn map_selection_keeps_every_authored_trigger_operand_and_omits_product_only_inputs() {
    let selected = selected_tables();
    assert!(selected.contains(&TableId::Campaign));
    for &table in crate::triggers::RECORDED_TABLES {
        assert!(
            selected.contains(&table),
            "authored trigger input {table:?} is required"
        );
    }
    for table in [
        TableId::BattleSounds,
        TableId::Clock,
        TableId::DivisionMovement,
        TableId::DivisionHistory,
        TableId::DivisionPanelStats,
        TableId::DivisionLandInputs,
        TableId::DivisionModelInputs,
        TableId::MapEvents,
        TableId::NuclearStrikes,
        TableId::TaskForceMapMotion,
        TableId::OperativeMissions,
        TableId::StrategicOperatives,
        TableId::AgencyOperationBuffers,
        TableId::Operations,
        TableId::CountryStabilityInputs,
        TableId::MovementStateTerms,
        TableId::CoreSupplyAccess,
        TableId::CountryUnitSpeedEntries,
        TableId::RiverRelations,
        TableId::DivisionTransportSpeeds,
        TableId::ConvoyClients,
        TableId::CountryOilFactors,
        TableId::RegionalConvoyEfficiency,
        TableId::ResourceExchanges,
        TableId::ResourceOrigins,
        TableId::ResourceRights,
        TableId::StateTemporaryResources,
        TableId::StrategicRegionWeights,
        TableId::TradeRoutes,
        TableId::LandUnitOrder,
        TableId::StateBuildingInputs,
    ] {
        assert!(selected.contains(&table));
    }
    assert!(selected.contains(&TableId::DivisionHistory));
    assert!(selected.contains(&TableId::DivisionLandInputs));
    assert!(selected.len() < TableId::ALL.len());
}

#[derive(Default)]
struct Census {
    reads: BTreeMap<String, BTreeSet<String>>,
    files: BTreeSet<PathBuf>,
}

fn cfg_value(meta: &syn::Meta) -> Option<bool> {
    match meta {
        syn::Meta::Path(path) if path.is_ident("test") => Some(false),
        syn::Meta::NameValue(value) if value.path.is_ident("feature") => {
            matches!(&value.value, syn::Expr::Lit(lit)
                if matches!(&lit.lit, syn::Lit::Str(s) if s.value() == "test-support"))
            .then_some(false)
        }
        syn::Meta::List(list) => {
            let values = list
                .parse_args_with(
                    syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
                )
                .unwrap();
            let values: Vec<_> = values.iter().map(cfg_value).collect();
            if list.path.is_ident("not") && values.len() == 1 {
                values[0].map(|value| !value)
            } else if list.path.is_ident("any") {
                if values.contains(&Some(true)) {
                    Some(true)
                } else if values.iter().all(|value| *value == Some(false)) {
                    Some(false)
                } else {
                    None
                }
            } else if list.path.is_ident("all") {
                if values.contains(&Some(false)) {
                    Some(false)
                } else if values.iter().all(|value| *value == Some(true)) {
                    Some(true)
                } else {
                    None
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

fn test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("test")
            || attr.path().is_ident("cfg")
                && attr
                    .parse_args::<syn::Meta>()
                    .is_ok_and(|meta| cfg_value(&meta) == Some(false))
    })
}

fn attributes(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::Const(x) => &x.attrs,
        syn::Item::Enum(x) => &x.attrs,
        syn::Item::ExternCrate(x) => &x.attrs,
        syn::Item::Fn(x) => &x.attrs,
        syn::Item::ForeignMod(x) => &x.attrs,
        syn::Item::Impl(x) => &x.attrs,
        syn::Item::Macro(x) => &x.attrs,
        syn::Item::Mod(x) => &x.attrs,
        syn::Item::Static(x) => &x.attrs,
        syn::Item::Struct(x) => &x.attrs,
        syn::Item::Trait(x) => &x.attrs,
        syn::Item::TraitAlias(x) => &x.attrs,
        syn::Item::Type(x) => &x.attrs,
        syn::Item::Union(x) => &x.attrs,
        syn::Item::Use(x) => &x.attrs,
        _ => &[],
    }
}

struct Scan<'a> {
    names: &'a BTreeSet<String>,
    census: &'a mut Census,
    path: PathBuf,
    module_dir: PathBuf,
    function: String,
    ids: bool,
    aliases: BTreeMap<String, String>,
}

impl Scan<'_> {
    fn record(&mut self, name: &str) {
        let name = self.aliases.get(name).map_or(name, String::as_str);
        if self.names.contains(name) {
            self.census
                .reads
                .entry(name.into())
                .or_default()
                .insert(format!("{}::{}", self.path.display(), self.function));
        }
    }

    fn tokens(&mut self, tokens: TokenStream) {
        for token in tokens {
            match token {
                TokenTree::Ident(ident) => self.record(&ident.to_string()),
                TokenTree::Group(group) => self.tokens(group.stream()),
                _ => {}
            }
        }
    }

    fn imports(&mut self, tree: &syn::UseTree, schemas: bool) {
        match tree {
            syn::UseTree::Path(path) => {
                let name = path.ident.to_string();
                let schemas = schemas
                    || name == "tables"
                    || self.aliases.get(&name).is_some_and(|name| name == "tables");
                self.imports(&path.tree, schemas);
            }
            syn::UseTree::Group(group) => {
                for tree in &group.items {
                    self.imports(tree, schemas);
                }
            }
            syn::UseTree::Name(name) if schemas && self.names.contains(&name.ident.to_string()) => {
                self.aliases
                    .insert(name.ident.to_string(), name.ident.to_string());
            }
            syn::UseTree::Rename(rename) => {
                let name = rename.ident.to_string();
                if schemas && self.names.contains(&name)
                    || matches!(name.as_str(), "tables" | "TableId" | "TableRows")
                {
                    self.aliases.insert(rename.rename.to_string(), name);
                }
            }
            _ => {}
        }
    }

    fn item_imports(&mut self, items: &[syn::Item]) {
        for item in items {
            if let syn::Item::Use(item) = item
                && !test_only(&item.attrs)
            {
                self.imports(&item.tree, false);
            }
        }
    }

    fn file(&mut self, path: PathBuf, module_dir: PathBuf) {
        if !self.census.files.insert(path.clone()) {
            return;
        }
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let file =
            syn::parse_file(&source).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let old_path = std::mem::replace(&mut self.path, path);
        let old_dir = std::mem::replace(&mut self.module_dir, module_dir);
        let old_aliases = self.aliases.clone();
        self.visit_file(&file);
        self.path = old_path;
        self.module_dir = old_dir;
        self.aliases = old_aliases;
    }
}

impl<'ast> Visit<'ast> for Scan<'_> {
    fn visit_file(&mut self, file: &'ast syn::File) {
        self.item_imports(&file.items);
        visit::visit_file(self, file);
    }

    fn visit_item(&mut self, item: &'ast syn::Item) {
        if !test_only(attributes(item)) {
            visit::visit_item(self, item);
        }
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        self.imports(&item.tree, false);
    }

    fn visit_item_mod(&mut self, module: &'ast syn::ItemMod) {
        if let Some((_, items)) = &module.content {
            let old = self.module_dir.clone();
            self.module_dir.push(module.ident.to_string());
            self.item_imports(items);
            for item in items {
                self.visit_item(item);
            }
            self.module_dir = old;
        } else {
            let explicit = module.attrs.iter().find_map(|attr| match &attr.meta {
                syn::Meta::NameValue(value) if value.path.is_ident("path") => match &value.value {
                    syn::Expr::Lit(lit) => match &lit.lit {
                        syn::Lit::Str(path) => Some(path.value()),
                        _ => None,
                    },
                    _ => None,
                },
                _ => None,
            });
            let path = if let Some(explicit) = explicit {
                self.path.parent().unwrap().join(explicit)
            } else {
                let file = self.module_dir.join(format!("{}.rs", module.ident));
                let directory = self
                    .module_dir
                    .join(module.ident.to_string())
                    .join("mod.rs");
                assert_ne!(
                    file.is_file(),
                    directory.is_file(),
                    "resolve module {} in {}",
                    module.ident,
                    self.path.display()
                );
                if file.is_file() { file } else { directory }
            };
            let dir = if path.file_name().unwrap() == "mod.rs" {
                path.parent().unwrap().to_path_buf()
            } else {
                path.with_extension("")
            };
            self.file(path, dir);
        }
    }

    fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
        let old = std::mem::replace(&mut self.function, function.sig.ident.to_string());
        visit::visit_item_fn(self, function);
        self.function = old;
    }

    fn visit_impl_item_fn(&mut self, function: &'ast syn::ImplItemFn) {
        if test_only(&function.attrs) {
            return;
        }
        let old = std::mem::replace(&mut self.function, function.sig.ident.to_string());
        visit::visit_impl_item_fn(self, function);
        self.function = old;
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        let segments: Vec<_> = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect();
        for pair in segments.windows(2) {
            let namespace = self.aliases.get(&pair[0]).unwrap_or(&pair[0]).as_str();
            if namespace == "tables" || self.ids && matches!(namespace, "TableId" | "TableRows") {
                self.record(&pair[1]);
            }
        }
        if segments.len() == 1
            && let Some(name) = self.aliases.get(&segments[0]).cloned()
        {
            self.record(&name);
        }
        visit::visit_path(self, path);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        assert!(
            !mac.path.is_ident("include"),
            "source census must follow include! in {}::{}",
            self.path.display(),
            self.function
        );
        self.tokens(mac.tokens.clone());
    }
}

fn names() -> BTreeSet<String> {
    TableId::ALL
        .into_iter()
        .map(|table| format!("{table:?}"))
        .collect()
}

// quarantine: #3 dashboard rework
#[test]
#[ignore = "quarantined under #3"]
fn production_selection_covers_map_and_shared_recorded_input_readers() {
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")), PathBuf::from);
    let root = manifest.parent().unwrap();
    let names = names();
    let mut census = Census::default();
    let mut crates: Vec<_> = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    crates.sort();
    for directory in crates {
        let name = directory.file_name().unwrap().to_str().unwrap();
        if !(name.starts_with("viewer") || matches!(name, "dashboard-effects" | "dashboard-text" | "replay-derive")) {
            continue;
        }
        let src = directory.join("src");
        let entry = if src.join("lib.rs").is_file() {
            src.join("lib.rs")
        } else {
            src.join("main.rs")
        };
        Scan {
            names: &names,
            census: &mut census,
            path: PathBuf::new(),
            module_dir: PathBuf::new(),
            function: "<module>".into(),
            ids: name.starts_with("viewer"),
            aliases: BTreeMap::new(),
        }
        .file(entry, src);
    }
    let selected: BTreeSet<_> = selected_tables()
        .iter()
        .map(|table| format!("{table:?}"))
        .collect();
    let missing: Vec<_> = census
        .reads
        .iter()
        .filter(|(name, _)| !selected.contains(*name))
        .map(|(name, sites)| {
            format!(
                "{name}: {}",
                sites.iter().cloned().collect::<Vec<_>>().join(", ")
            )
        })
        .collect();
    assert!(
        missing.is_empty(),
        "map selection omits recorded inputs:\n{}",
        missing.join("\n")
    );
    assert!(!census.files.is_empty());
}

#[test]
fn source_census_follows_aliases_macros_and_production_after_test_modules() {
    let source = r#"
        use replay_data::rows::{tables::{AirGroundCrews as Crews, WarOwnedRelations}, TableId as T};
        #[cfg(any(test, feature = "test-support"))]
        mod tests { fn ignored() { rows!(Scientists); } }
        fn later() {
            read::<Crews>();
            rows!(CountryGameScoreInputs);
            let _ = T::WarOwnedRelations;
            read::<WarOwnedRelations>();
            let _: content_reader_military::intelligence::OperationTokens;
            let _ = "tables::Decisions"; // tables::TimedIdeas
        }
    "#;
    let names = names();
    let mut census = Census::default();
    Scan {
        names: &names,
        census: &mut census,
        path: "synthetic.rs".into(),
        module_dir: PathBuf::new(),
        function: "<module>".into(),
        ids: true,
        aliases: BTreeMap::new(),
    }
    .visit_file(&syn::parse_file(source).unwrap());
    assert_eq!(
        census.reads,
        [
            "AirGroundCrews",
            "CountryGameScoreInputs",
            "WarOwnedRelations"
        ]
        .map(|name| (
            name.to_owned(),
            BTreeSet::from(["synthetic.rs::later".into()])
        ))
        .into()
    );
}
