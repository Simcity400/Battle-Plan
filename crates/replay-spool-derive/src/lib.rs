use proc_macro::{Delimiter, Spacing, TokenStream, TokenTree};

#[proc_macro_derive(Spooled, attributes(spool))]
pub fn derive(input: TokenStream) -> TokenStream {
    let code = match expand(input) {
        Ok(code) => code,
        Err(message) => format!("::core::compile_error!({message:?});"),
    };
    code.parse()
        .unwrap_or_else(|_| "::core::compile_error!(\"Spooled made no code\");".parse().unwrap())
}

const PATH: &str = "::replay_spool";

struct Field {
    name: String,
    ty: String,
    skip: bool,
    cfg: String,
}

impl Field {
    fn bound(&self) -> String {
        format!("__{}", self.name.trim_start_matches("r#"))
    }
}

enum Shape {
    Fields(Vec<Field>),
    Unit,
}

struct Variant {
    name: String,
    shape: Shape,
    cfg: String,
}

fn is_punct(token: &TokenTree, ch: char) -> bool {
    matches!(token, TokenTree::Punct(punct) if punct.as_char() == ch)
}

fn is_ident(token: &TokenTree, name: &str) -> bool {
    matches!(token, TokenTree::Ident(ident) if ident.to_string() == name)
}

fn text(tokens: &[TokenTree]) -> String {
    tokens.iter().cloned().collect::<TokenStream>().to_string()
}

fn split(tokens: &[TokenTree]) -> Vec<Vec<TokenTree>> {
    let mut parts = vec![Vec::new()];
    let mut depth = 0_i32;
    let mut arrow = false;
    for token in tokens {
        if let TokenTree::Punct(punct) = token {
            match punct.as_char() {
                '<' => depth += 1,
                '>' if !arrow => depth -= 1,
                ',' if depth == 0 => {
                    parts.push(Vec::new());
                    arrow = false;
                    continue;
                }
                _ => {}
            }
            arrow = punct.as_char() == '-' && punct.spacing() == Spacing::Joint;
        } else {
            arrow = false;
        }
        parts.last_mut().expect("a part is open").push(token.clone());
    }
    if parts.last().is_some_and(Vec::is_empty) {
        parts.pop();
    }
    parts
}

fn attributes(tokens: &[TokenTree], at: &mut usize) -> Result<(bool, String), String> {
    let (mut skip, mut cfg) = (false, String::new());
    while *at + 1 < tokens.len() && is_punct(&tokens[*at], '#') {
        let TokenTree::Group(group) = &tokens[*at + 1] else {
            return Err("an attribute without brackets".into());
        };
        let inner: Vec<TokenTree> = group.stream().into_iter().collect();
        if inner.first().is_some_and(|first| is_ident(first, "spool")) {
            match inner.get(1) {
                Some(TokenTree::Group(arguments))
                    if arguments.stream().to_string().trim() == "skip" =>
                {
                    skip = true;
                }
                _ => return Err("spool takes only skip".into()),
            }
        } else if inner.first().is_some_and(|first| is_ident(first, "cfg")) {
            cfg.push_str(&format!("#[{}] ", group.stream()));
        }
        *at += 2;
    }
    Ok((skip, cfg))
}

fn visibility(tokens: &[TokenTree], at: &mut usize) {
    if *at < tokens.len() && is_ident(&tokens[*at], "pub") {
        *at += 1;
        if let Some(TokenTree::Group(group)) = tokens.get(*at)
            && group.delimiter() == Delimiter::Parenthesis
        {
            *at += 1;
        }
    }
}

fn fields(group: &proc_macro::Group) -> Result<Shape, String> {
    let tokens: Vec<TokenTree> = group.stream().into_iter().collect();
    let named = group.delimiter() == Delimiter::Brace;
    let mut fields = Vec::new();
    for (index, part) in split(&tokens).into_iter().enumerate() {
        let mut at = 0;
        let (skip, cfg) = attributes(&part, &mut at)?;
        visibility(&part, &mut at);
        let name = if named {
            let Some(TokenTree::Ident(name)) = part.get(at) else {
                return Err("a field without a name".into());
            };
            if !part.get(at + 1).is_some_and(|token| is_punct(token, ':')) {
                return Err("a field without a type".into());
            }
            at += 2;
            name.to_string()
        } else {
            index.to_string()
        };
        fields.push(Field {
            name,
            ty: text(&part[at..]),
            skip,
            cfg,
        });
    }
    Ok(Shape::Fields(fields))
}

struct Generics {
    declared: Vec<String>,
    named: Vec<String>,
}

fn generics(tokens: &[TokenTree]) -> Result<Generics, String> {
    let mut generics = Generics {
        declared: Vec::new(),
        named: Vec::new(),
    };
    for part in split(tokens) {
        let mut at = 0;
        attributes(&part, &mut at)?;
        let part = &part[at..];
        let mut end = part.len();
        let mut depth = 0_i32;
        for (index, token) in part.iter().enumerate() {
            if let TokenTree::Punct(punct) = token {
                match punct.as_char() {
                    '<' => depth += 1,
                    '>' => depth -= 1,
                    '=' if depth == 0 => {
                        end = index;
                        break;
                    }
                    _ => {}
                }
            }
        }
        let declared = &part[..end];
        let named = match declared {
            [TokenTree::Punct(tick), TokenTree::Ident(name), ..] if tick.as_char() == '\'' => {
                format!("'{name}")
            }
            [konst, TokenTree::Ident(name), ..] if is_ident(konst, "const") => name.to_string(),
            [TokenTree::Ident(name), ..] => name.to_string(),
            _ => return Err("a generic parameter it cannot read".into()),
        };
        generics.declared.push(text(declared));
        generics.named.push(named);
    }
    Ok(generics)
}

fn expand(input: TokenStream) -> Result<String, String> {
    let tokens: Vec<TokenTree> = input.into_iter().collect();
    let mut at = 0;
    attributes(&tokens, &mut at)?;
    visibility(&tokens, &mut at);
    let kind = match tokens.get(at) {
        Some(token) if is_ident(token, "struct") => "struct",
        Some(token) if is_ident(token, "enum") => "enum",
        _ => return Err("Spooled derives for structs and enums".into()),
    };
    let Some(TokenTree::Ident(name)) = tokens.get(at + 1) else {
        return Err("a type without a name".into());
    };
    let name = name.to_string();
    at += 2;
    let mut parameters = Generics {
        declared: Vec::new(),
        named: Vec::new(),
    };
    if tokens.get(at).is_some_and(|token| is_punct(token, '<')) {
        let start = at + 1;
        let mut depth = 1;
        let mut arrow = false;
        at += 1;
        while at < tokens.len() {
            if let TokenTree::Punct(punct) = &tokens[at] {
                match punct.as_char() {
                    '<' => depth += 1,
                    '>' if !arrow => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                arrow = punct.as_char() == '-' && punct.spacing() == Spacing::Joint;
            } else {
                arrow = false;
            }
            at += 1;
        }
        parameters = generics(&tokens[start..at])?;
        at += 1;
    }
    let mut predicates = Vec::new();
    let mut body = None;
    let mut clause: Option<Vec<TokenTree>> = None;
    while at < tokens.len() {
        let token = &tokens[at];
        let opens = match token {
            TokenTree::Group(group) => match group.delimiter() {
                Delimiter::Brace => true,
                Delimiter::Parenthesis => clause.is_none(),
                _ => false,
            },
            _ => false,
        };
        match token {
            TokenTree::Group(group) if body.is_none() && opens => {
                if let Some(taken) = clause.take() {
                    predicates.push(taken);
                }
                body = Some(group.clone());
            }
            _ if is_ident(token, "where") => clause = Some(Vec::new()),
            _ if is_punct(token, ';') => {
                if let Some(taken) = clause.take() {
                    predicates.push(taken);
                }
            }
            _ => {
                if let Some(clause) = clause.as_mut() {
                    clause.push(token.clone());
                }
            }
        }
        at += 1;
    }
    if let Some(taken) = clause.take() {
        predicates.push(taken);
    }
    let mut bounds: Vec<String> = predicates
        .iter()
        .flat_map(|predicate| split(predicate))
        .map(|predicate| text(&predicate))
        .filter(|predicate| !predicate.trim().is_empty())
        .collect();

    let variants: Vec<Variant> = if kind == "struct" {
        vec![Variant {
            name: String::new(),
            shape: match &body {
                Some(group) => fields(group)?,
                None => Shape::Unit,
            },
            cfg: String::new(),
        }]
    } else {
        let Some(group) = &body else {
            return Err("an enum without variants".into());
        };
        let tokens: Vec<TokenTree> = group.stream().into_iter().collect();
        let mut variants = Vec::new();
        for part in split(&tokens) {
            let mut at = 0;
            let (_, cfg) = attributes(&part, &mut at)?;
            let Some(TokenTree::Ident(variant)) = part.get(at) else {
                return Err("a variant without a name".into());
            };
            let shape = match part.get(at + 1) {
                Some(TokenTree::Group(group)) => fields(group)?,
                _ => Shape::Unit,
            };
            variants.push(Variant {
                name: variant.to_string(),
                shape,
                cfg,
            });
        }
        variants
    };
    let (spool, unspool) = if kind == "struct" {
        struct_code(&variants[0].shape)
    } else {
        enum_code(&variants)
    };
    if !parameters.named.is_empty() {
        for variant in &variants {
            let Shape::Fields(fields) = &variant.shape else {
                continue;
            };
            for field in fields.iter().filter(|field| !field.skip && field.cfg.is_empty()) {
                bounds.push(format!("{}: {PATH}::Spooled", field.ty));
            }
        }
    }
    let declared = if parameters.declared.is_empty() {
        String::new()
    } else {
        format!("<{}>", parameters.declared.join(", "))
    };
    let named = if parameters.named.is_empty() {
        String::new()
    } else {
        format!("<{}>", parameters.named.join(", "))
    };
    let clause = if bounds.is_empty() {
        String::new()
    } else {
        format!("where {}", bounds.join(", "))
    };
    Ok(format!(
        "#[automatically_derived]
        impl{declared} {PATH}::Spooled for {name}{named} {clause} {{
            fn spool(&self, out: &mut {PATH}::Writer<'_>) -> ::std::io::Result<()> {{ {spool} }}
            fn unspool(input: &mut {PATH}::Reader<'_>) -> ::std::io::Result<Self> {{ {unspool} }}
        }}"
    ))
}

fn read(field: &Field) -> String {
    let value = if field.skip {
        "::core::default::Default::default()".to_string()
    } else {
        format!("{PATH}::Spooled::unspool(input)?")
    };
    format!("{}{}: {value},", field.cfg, field.name)
}

fn written(fields: &[Field], at: impl Fn(&Field) -> String) -> String {
    fields
        .iter()
        .filter(|field| !field.skip)
        .map(|field| format!("{}{PATH}::Spooled::spool({}, out)?;", field.cfg, at(field)))
        .collect()
}

fn struct_code(shape: &Shape) -> (String, String) {
    match shape {
        Shape::Fields(fields) => {
            let spool = written(fields, |field| format!("&self.{}", field.name));
            let unspool: String = fields.iter().map(read).collect();
            (
                format!("{spool} ::std::result::Result::Ok(())"),
                format!("::std::result::Result::Ok(Self {{ {unspool} }})"),
            )
        }
        Shape::Unit => (
            "let _ = out; ::std::result::Result::Ok(())".into(),
            "let _ = input; ::std::result::Result::Ok(Self)".into(),
        ),
    }
}

fn enum_code(variants: &[Variant]) -> (String, String) {
    if variants.is_empty() {
        return (
            "let _ = out; match *self {}".into(),
            format!("let _ = input; ::std::result::Result::Err({PATH}::invalid(\"An empty enum\"))"),
        );
    }
    let mut spool = String::from("match self {");
    let mut unspool = format!("match {PATH}::Reader::tag(input)? {{");
    for (index, variant) in variants.iter().enumerate() {
        let (name, cfg) = (&variant.name, &variant.cfg);
        match &variant.shape {
            Shape::Fields(fields) => {
                let bound: String = fields
                    .iter()
                    .map(|field| {
                        if field.skip {
                            format!("{}{}: _,", field.cfg, field.name)
                        } else {
                            format!("{}{}: {},", field.cfg, field.name, field.bound())
                        }
                    })
                    .collect();
                let written = written(fields, Field::bound);
                let read: String = fields.iter().map(read).collect();
                spool.push_str(&format!(
                    "{cfg}Self::{name} {{ {bound} }} => {{ {PATH}::Writer::tag(out, {index})?; {written} }}"
                ));
                unspool.push_str(&format!(
                    "{cfg}{index} => ::std::result::Result::Ok(Self::{name} {{ {read} }}),"
                ));
            }
            Shape::Unit => {
                spool.push_str(&format!(
                    "{cfg}Self::{name} => {{ {PATH}::Writer::tag(out, {index})?; }}"
                ));
                unspool.push_str(&format!(
                    "{cfg}{index} => ::std::result::Result::Ok(Self::{name}),"
                ));
            }
        }
    }
    spool.push_str("} ::std::result::Result::Ok(())");
    unspool.push_str(&format!(
        "_ => ::std::result::Result::Err({PATH}::invalid(\"A spooled variant it does not know\")), }}"
    ));
    (spool, unspool)
}
