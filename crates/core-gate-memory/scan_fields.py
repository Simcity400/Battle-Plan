import argparse
from collections import defaultdict
import json
from pathlib import Path
import re
import subprocess
import tomllib
from importlib.metadata import version

from tree_sitter import Language, Parser
import tree_sitter_rust


if version("tree-sitter") != "0.25.2":
    raise RuntimeError("Use tree-sitter==0.25.2 and tree-sitter-rust==0.24.2; isolate Python packages in captures.")
PARSER = Parser(Language(tree_sitter_rust.language()))


def nodes(node):
    yield node
    for child in node.named_children:
        yield from nodes(child)


def code(node, data):
    return data[node.start_byte:node.end_byte].decode()


def schema(root):
    registry = (root / "crates/replay-data/src/rows.rs").read_text(encoding="utf-8")
    rows = dict(re.findall(r"(\w+)\s*=\s*\d+\s*=>\s*\w+\((\w+Row)\)", registry))
    codecs, types = {}, {}
    for path in sorted((root / "crates/replay-data/src").rglob("*.rs")):
        data = path.read_bytes()
        for node in nodes(PARSER.parse(data).root_node):
            text = code(node, data)
            if node.type == "struct_item":
                name = code(node.child_by_field_name("name"), data)
                types[name] = {code(n.child_by_field_name("name"), data): code(n.child_by_field_name("type"), data)
                               for n in nodes(node) if n.type == "field_declaration"}
            if node.type != "macro_invocation" or not re.match(r"(?:typed_codec|row_codec)!", text):
                continue
            row = re.match(r"\w+!\(\s*(\w+)", text).group(1)
            fields = re.findall(r"(\d+)\s*=>\s*\(?\s*(\w+(?:\s*\[\s*\d+\s*\])?)\s*\)?\s*:", text)
            codecs[row] = [re.sub(r"\s+", "", name) for _, name in sorted(fields, key=lambda f: int(f[0]))]
    return {table: {"row": row, "fields": codecs.get(row, []), "types": types.get(row, {})}
            for table, row in rows.items()}


def location(path, node):
    return f"{path}:{node.start_point.row + 1}"


def production_nodes(node, data, function=None):
    if node.type == "function_item":
        function = (code(node.child_by_field_name("name"), data), code(node, data))
    if node.type in {"mod_item", "function_item"}:
        name = node.child_by_field_name("name")
        if name is not None and code(name, data) in {"tests", "test", "fixtures", "benchmarks"}:
            return
    yield node, function
    skip = False
    for child in node.named_children:
        if child.type == "attribute_item":
            skip |= bool(re.search(r"cfg\(\s*(?:test|any\(test,)", code(child, data)))
            continue
        if child.type in {"line_comment", "block_comment"}:
            continue
        if not skip:
            yield from production_nodes(child, data, function)
        skip = False


def call_name(node, data):
    called = node.child_by_field_name("function")
    if called.type == "generic_function":
        called = called.child_by_field_name("function")
    return re.sub(r"\s+", "", code(called, data))


def value_tree(data, tree):
    expanded = bytearray(data)
    for node in nodes(tree.root_node):
        if node.type != "macro_invocation":
            continue
        tokens = next((n for n in node.named_children if n.type == "token_tree"), None)
        if tokens is not None:
            bang = data.rfind(b"!", node.start_byte, tokens.start_byte)
            if bang >= 0:
                expanded[bang] = ord(" ")
                expanded[tokens.start_byte] = ord("(")
                expanded[tokens.end_byte - 1] = ord(")")
    return PARSER.parse(bytes(expanded))


def receiver_tables(node, receiver, data, row_tables, return_types=None):
    if not re.fullmatch(r"\w+", receiver):
        return None
    child = node
    parent = node.parent
    while parent is not None:
        if parent.type == "block":
            for binding in reversed(parent.named_children):
                if binding.end_byte > child.start_byte or binding.type != "let_declaration":
                    continue
                pattern = binding.child_by_field_name("pattern")
                if pattern is not None and code(pattern, data) == receiver:
                    ty = binding.child_by_field_name("type")
                    if ty is not None:
                        return set().union(*(row_tables[r] for r in re.findall(r"\b\w+Row\b", code(ty, data))))
                    value = binding.child_by_field_name("value")
                    while value is not None and value.type in {"try_expression", "reference_expression"}:
                        value = value.named_children[0]
                    if value is not None and value.type == "call_expression":
                        return (return_types or {}).get(call_name(value, data))
                    return None
        if parent.type in {"function_item", "closure_expression"}:
            params = parent.child_by_field_name("parameters")
            if params is not None:
                for param in params.named_children:
                    pattern = param.child_by_field_name("pattern") if param.type == "parameter" else param
                    if pattern is not None and code(pattern, data) == receiver:
                        ty = param.child_by_field_name("type")
                        return None if ty is None else set().union(*(row_tables[r] for r in re.findall(r"\b\w+Row\b", code(ty, data))))
        child, parent = parent, parent.parent
    return None


def use_paths(text):
    tokens = re.findall(r"\w+|::|[{},*]", text.split("use", 1)[-1])
    def group(index, prefix=()):
        paths = []
        while index < len(tokens) and tokens[index] != "}":
            path = list(prefix)
            while index < len(tokens) and tokens[index] not in {"{", "}", ",", "as"}:
                if tokens[index] != "::":
                    path.append(tokens[index])
                index += 1
            if index < len(tokens) and tokens[index] == "{":
                nested, index = group(index + 1, tuple(path))
                paths.extend(nested)
            else:
                alias = path[-1] if path else None
                if index < len(tokens) and tokens[index] == "as":
                    alias, index = tokens[index + 1], index + 2
                if path and path[-1] == "self":
                    path.pop()
                    alias = path[-1]
                if path:
                    paths.append((alias, tuple(path)))
            if index < len(tokens) and tokens[index] == ",":
                index += 1
        return paths, index + 1
    return group(0)[0]


class QueryReaders:
    def __init__(self, root):
        self.root, self.files, self.symbols = root, {}, defaultdict(list)
        self.crates = {p.parent.name.replace("-", "_"): p.parent / "src"
                       for p in sorted((root / "crates").glob("*/Cargo.toml"))}

    def module(self, path):
        parts = path.relative_to(self.root / "crates").parts
        tail = list(Path(*parts[2:]).with_suffix("").parts)
        if tail[-1] in {"lib", "main", "mod"}:
            tail.pop()
        return (parts[0].replace("-", "_"), *tail)

    def load(self, path):
        if path in self.files:
            return self.files[path]
        data = path.read_bytes()
        tree = value_tree(data, PARSER.parse(data))
        module = self.module(path)
        functions, imports = [], defaultdict(list)
        for node, _ in production_nodes(tree.root_node, data):
            if node.type not in {"function_item", "use_declaration"}:
                continue
            parents, owner = [], None
            parent = node.parent
            nested = False
            while parent is not None:
                if parent.type == "mod_item":
                    parents.append(code(parent.child_by_field_name("name"), data))
                elif parent.type == "impl_item":
                    owner = code(parent.child_by_field_name("type"), data).split("<", 1)[0]
                elif parent.type == "function_item":
                    nested = True
                parent = parent.parent
            namespace = (*module, *reversed(parents))
            if node.type == "use_declaration" and not nested:
                imports[namespace].extend(use_paths(code(node, data)))
            elif node.type == "function_item" and not nested:
                name = code(node.child_by_field_name("name"), data)
                symbol = (*namespace, *((owner,) if owner else ()), name)
                function = dict(node=node, name=name, owner=owner, module=namespace,
                                symbol=symbol, path=path, data=data)
                functions.append(function)
                self.symbols[symbol].append(function)
        self.files[path] = functions, imports
        return functions, imports

    def absolute(self, path, module):
        if not path:
            return module
        if path[0] == "crate":
            return (module[0], *path[1:])
        if path[0] == "self":
            return (*module, *path[1:])
        if path[0] == "super":
            return self.absolute(path[1:], module[:-1]) if len(path) > 1 and path[1] == "super" else (*module[:-1], *path[1:])
        return path if path[0] in self.crates else (*module, *path)

    def lookup(self, symbol, seen=None):
        seen = set() if seen is None else seen
        if symbol in seen or symbol[0] not in self.crates:
            return []
        seen.add(symbol)
        base = self.crates[symbol[0]]
        all_imports = []
        for count in range(len(symbol) - 1):
            parts = symbol[1:count + 1]
            stem = base.joinpath(*parts)
            paths = [stem.with_suffix(".rs"), stem / "mod.rs"] if parts else [base / "lib.rs"]
            for path in paths:
                if not path.is_file():
                    continue
                _, imports = self.load(path)
                all_imports.extend(imports.items())
        if symbol in self.symbols:
            return self.symbols[symbol]
        for namespace, uses in all_imports:
            if symbol[:len(namespace)] != namespace or len(symbol) <= len(namespace):
                continue
            rest = symbol[len(namespace):]
            for alias, target in uses:
                redirect = (namespace, alias, target)
                if alias == rest[0] and redirect not in seen:
                    seen.add(redirect)
                    found = self.lookup((*self.absolute(target, namespace), *rest[1:]), seen)
                    if found:
                        return found
        return self.symbols.get(symbol, [])

    def calls(self, function):
        data, module = function["data"], function["module"]
        _, imports = self.load(function["path"])
        local_imports = [use for node, _ in production_nodes(function["node"], data)
                         if node.type == "use_declaration" for use in use_paths(code(node, data))]
        for node, _ in production_nodes(function["node"], data):
            if node.type != "call_expression":
                continue
            called = call_name(node, data)
            if not re.fullmatch(r"\w+(?:::\w+)*", called):
                continue
            parts = tuple(called.split("::"))
            candidates = []
            if parts[0] == "Self" and function["owner"]:
                candidates.append((*module, function["owner"], *parts[1:]))
            for count in range(len(module), 0, -1):
                namespace = module[:count]
                for alias, target in (local_imports if namespace == module else []) + imports.get(namespace, []):
                    if alias == parts[0]:
                        candidates.append((*self.absolute(target, namespace), *parts[1:]))
                    elif alias == "*":
                        candidates.append((*self.absolute(target[:-1], namespace), *parts))
            candidates.append(self.absolute(parts, module))
            for candidate in candidates:
                found = self.lookup(candidate)
                if found:
                    for target in found:
                        yield node, target
                    break

    def expand(self, scopes):
        reached, pending, edges = {}, [], []
        for scope in scopes:
            base = self.root / scope["path"]
            for path in sorted(base.rglob("*.rs")) if base.is_dir() else [base]:
                functions, _ = self.load(path)
                for function in functions:
                    if "functions" in scope and function["name"] not in scope["functions"]:
                        continue
                    if scope.get("query") or function["name"] in scope.get("queries", []) or path.name in scope.get("query_files", []):
                        pending.append((function, {}))
        while pending:
            function, bindings = pending.pop()
            key = function["path"], function["node"].start_byte, tuple(sorted(bindings.items()))
            if key in reached:
                continue
            reached[key] = function, bindings
            for call, target in self.calls(function):
                generic = call.child_by_field_name("function")
                arguments = generic.child_by_field_name("type_arguments") if generic.type == "generic_function" else None
                arguments = [code(n, function["data"]) for n in arguments.named_children if n.type != "lifetime"] if arguments else []
                parameters = target["node"].child_by_field_name("type_parameters")
                parameters = [code(n.child_by_field_name("name"), target["data"]) for n in parameters.named_children
                              if n.type != "lifetime_parameter"] if parameters else []
                forwarded = {param: bindings.get(arg, arg) for param, arg in zip(parameters, arguments)}
                edges.append(dict(source=location(function["path"].relative_to(self.root).as_posix(), call),
                                  target=location(target["path"].relative_to(self.root).as_posix(), target["node"]),
                                  expression=call_name(call, function["data"]), type_arguments=forwarded))
                pending.append((target, forwarded))
        grouped, types = defaultdict(set), defaultdict(lambda: defaultdict(lambda: defaultdict(set)))
        for function, bindings in reached.values():
            path, line = function["path"], function["node"].start_point.row + 1
            grouped[path].add(line)
            for name, ty in bindings.items():
                types[path][line][name].update(re.findall(r"(?:TableId|tables)::(\w+)", ty))
        return ([dict(path=p.relative_to(self.root).as_posix(), query=True, function_lines=sorted(lines),
                      table_types={line: {name: sorted(tables) for name, tables in names.items()}
                                   for line, names in types[p].items()})
                 for p, lines in sorted(grouped.items())], edges)


def active_function(scope, name, node):
    return (("functions" not in scope or name in scope["functions"])
            and ("function_lines" not in scope or node.start_point.row + 1 in scope["function_lines"]))


def whole_row_sites(tree, data, rel, family, allowed, schemas, scope, instantiations):
    row_tables = defaultdict(set)
    for table, value in schemas.items():
        row_tables[value["row"]].add(table)
    functions = {}
    for node, _ in production_nodes(tree.root_node, data):
        if node.type != "function_item":
            continue
        name = code(node.child_by_field_name("name"), data)
        params = node.child_by_field_name("parameters")
        params = [code(p.child_by_field_name("pattern"), data) for p in params.named_children if p.type == "parameter"]
        generics = node.child_by_field_name("type_parameters")
        generics = [code(p.child_by_field_name("name"), data) for p in generics.named_children
                    if p.type != "lifetime_parameter"] if generics is not None else []
        calls = [n for n, _ in production_nodes(node, data) if n.type == "call_expression"]
        functions[node.start_byte] = dict(node=node, name=name, params=params, generics=generics, calls=calls,
                                          active=active_function(scope, name, node))
    local_names = {f["name"] for f in functions.values()}

    def tables_in(text, function, call):
        found = set(re.findall(r"(?:TableId|tables)::(\w+)", text)) & schemas.keys()
        bindings = scope.get("table_types", {}).get(function["node"].start_point.row + 1, {})
        for name in re.findall(r"\b\w+\b", text):
            found.update(bindings.get(name, []))
        for row in re.findall(r"\b\w+Row\b", text):
            found.update(row_tables[row])
        for alias in re.findall(r"\b\w+\b", text):
            prefix = data[function["node"].start_byte:call.start_byte].decode()
            values = re.findall(r"\blet\s+" + re.escape(alias) + r"\s*=\s*TableId::(\w+)", prefix)
            if values:
                found.add(values[-1])
        if re.fullmatch(r"\w+::ID", text):
            parent = function["node"].parent
            while parent is not None and parent.type != "impl_item":
                parent = parent.parent
            if parent is not None:
                impl_type = code(parent.child_by_field_name("type"), data).split("<", 1)[0]
                found.update(instantiations.get(impl_type, set()))
        return found

    module_names = {"columns"} | set(re.findall(r"\bcolumns\s+as\s+(\w+)", data.decode()))
    row_aliases = set()
    for imports in re.findall(r"columns::\s*\{([^}]+)\}", data.decode()):
        for name, alias in re.findall(r"\b(row|rows_at)(?:\s+as\s+(\w+))?", imports):
            row_aliases.add(alias or name)
    for name, alias in re.findall(r"\buse\s+[\w:]*columns::(row|rows_at)(?:\s+as\s+(\w+))?", data.decode()):
        row_aliases.add(alias or name)
    helpers = defaultdict(set)
    sites, unresolved, symbolic_sites = {}, {}, {}
    changed = True
    while changed:
        changed = False
        for function in functions.values():
            for call in function["calls"]:
                called = call_name(call, data)
                args = [code(a, data) for a in call.child_by_field_name("arguments").named_children]
                generic = call.child_by_field_name("function")
                type_args = generic.child_by_field_name("type_arguments") if generic.type == "generic_function" else None
                types = [code(t, data) for t in type_args.named_children if t.type != "lifetime"] if type_args is not None else []
                selected = set()
                operands = []
                decode_sources = set()
                if called in row_aliases or any(called.endswith(f"{name}::{method}") for name in module_names for method in ("row", "rows_at")):
                    operands = [("value", arg) for arg in args[1:2]]
                    decode_sources.add(location(rel, call))
                elif called.endswith("rows_from_wire"):
                    operands = [("value", called.rsplit(".", 1)[0])]
                    decode_sources.add(location(rel, call))
                elif called not in local_names and re.search(r"(?:\.|^)(?:complete|rows)$", called) and type_args is not None and (
                        "." in called or "tables::" in code(type_args, data)):
                    operands = [("type", ty) for ty in types]
                    decode_sources.add(location(rel, call))
                else:
                    for kind, index, source in helpers.get(re.split(r"::|\.", called)[-1], set()):
                        arguments = types if kind == "type" else args
                        if index < len(arguments):
                            operands.append((kind, arguments[index]))
                            decode_sources.add(source)
                if not operands and not selected:
                    continue
                symbolic = False
                for kind, operand in operands:
                    selected.update(tables_in(operand, function, call))
                    parameter = operand
                    if operand.endswith("::ID") and operand[:-4] in function["generics"]:
                        kind, parameter = "type", operand[:-4]
                    parameters = function["generics"] if kind == "type" else function["params"]
                    if parameter in parameters:
                        symbolic = True
                        for source in decode_sources:
                            item = (kind, parameters.index(parameter), source)
                            if item not in helpers[function["name"]]:
                                helpers[function["name"]].add(item)
                                changed = True
                if not selected and not symbolic:
                    parent = call.parent
                    while parent is not None and parent.type not in {"let_declaration", "function_item"}:
                        parent = parent.parent
                    if parent is not None and parent.type == "let_declaration":
                        ty = parent.child_by_field_name("type")
                        if ty is not None:
                            selected.update(tables_in(code(ty, data), function, call))
                item = dict(family=family, source=location(rel, call), expression=code(call, data),
                            function=function["name"], decode_sources=sorted(decode_sources),
                            query_read=scope.get("query", False) or function["name"] in scope.get("queries", []) or Path(rel).name in scope.get("query_files", []))
                if not function["active"]:
                    continue
                if symbolic and not selected:
                    symbolic_sites[item["source"], item["expression"]] = item
                if selected:
                    for table in sorted(selected & allowed):
                        sites[item["source"], item["expression"], table] = dict(item, table=table)
                elif not symbolic:
                    unresolved[item["source"], item["expression"]] = item
    reached = {source for site in sites.values() for source in site["decode_sources"]}
    for key, site in symbolic_sites.items():
        if not set(site["decode_sources"]) <= reached:
            unresolved[key] = site
    return list(sites.values()), list(unresolved.values())


def scan(root, config, schemas):
    row_tables = defaultdict(set)
    for table, value in schemas.items():
        row_tables[value["row"]].add(table)
    facts, whole, ambiguous, scanned, unresolved, query_calls = [], [], [], [], [], []
    readers = QueryReaders(root)
    for family, declared_scopes in config["families"].items():
        query_scopes, edges = readers.expand(declared_scopes)
        query_calls.extend(dict(edge, family=family) for edge in edges)
        scopes = declared_scopes + query_scopes
        instantiations = defaultdict(set)
        for scope in scopes:
            base = root / scope["path"]
            for path in sorted(base.rglob("*.rs")) if base.is_dir() else [base]:
                for ty, table in re.findall(r"\b(\w+)<\s*tables::(\w+)\s*>", path.read_text(encoding="utf-8")):
                    instantiations[ty].add(table)
        for scope in scopes:
            base = root / scope["path"]
            paths = sorted(base.rglob("*.rs")) if base.is_dir() else [base]
            for path in paths:
                if any(part in {"tests", "fixtures", "benches"} for part in path.parts) or path.name in {"tests.rs", "fixtures.rs"} or path.name.endswith("_tests.rs"):
                    continue
                rel = path.relative_to(root).as_posix()
                data = path.read_bytes()
                tree = value_tree(data, PARSER.parse(data))
                sites, unknown = whole_row_sites(tree, data, rel, family, set(scope.get("tables", schemas)), schemas, scope, instantiations)
                whole.extend(sites)
                unresolved.extend(unknown)
                keep_nodes = list(nodes(tree.root_node))
                return_types = {}
                for function_node in (n for n in keep_nodes if n.type == "function_item"):
                    ty = function_node.child_by_field_name("return_type")
                    if ty is not None:
                        rows = re.findall(r"\b\w+Row\b", code(ty, data))
                        if rows:
                            return_types[code(function_node.child_by_field_name("name"), data)] = set().union(*(row_tables[r] for r in rows))
                methods = set()
                for call in (n for n in keep_nodes if n.type == "call_expression"):
                    called = call.child_by_field_name("function")
                    if called.type == "generic_function":
                        called = called.child_by_field_name("function")
                    methods.add((called.start_byte, called.end_byte))
                file_text = data.decode()
                file_tables = set(re.findall(r"(?:TableId|tables)::(\w+)", file_text)) & schemas.keys()
                for row in re.findall(r"\b\w+Row\b", file_text):
                    file_tables.update(row_tables[row])
                allowed = set(scope.get("tables", file_tables))
                scanned.append({"family": family, "file": rel, "tables": sorted(allowed), "functions": scope.get("function_lines", scope.get("functions", "all production"))})
                for node, function in production_nodes(tree.root_node, data):
                    if node.type not in {"call_expression", "field_expression", "match_arm", "tuple_expression", "macro_invocation"}:
                        continue
                    fn_name = function[0] if function else None
                    query_read = scope.get("query", False) or fn_name in scope.get("queries", []) or path.name in scope.get("query_files", [])
                    if "functions" in scope and fn_name not in scope["functions"]:
                        continue
                    if "function_lines" in scope:
                        parent = node
                        while parent is not None and parent.type != "function_item":
                            parent = parent.parent
                        if parent is None or parent.start_point.row + 1 not in scope["function_lines"]:
                            continue
                    fn_text = function[1] if function else file_text
                    tables = set(re.findall(r"(?:TableId|tables)::(\w+)", fn_text)) & allowed
                    for row in re.findall(r"\b\w+Row\b", fn_text):
                        tables.update(row_tables[row] & allowed)
                    tables = tables or allowed
                    text = code(node, data)
                    source = location(rel, node)
                    if node.type in {"tuple_expression", "macro_invocation"}:
                        for table, names in re.findall(r'TableId::(\w+)\s*,\s*&\[([^\]]*)\]', text, re.S):
                            if table in allowed:
                                for field in re.findall(r'"([\w\[\]]+)"', names):
                                    if field in schemas[table]["fields"]:
                                        facts.append(dict(family=family, table=table, field=field, source=source, kind="selection", expression=text, query_read=query_read))
                    if node.type == "match_arm":
                        pattern = code(node.child_by_field_name("pattern"), data)
                        selected = set(re.findall(r"TableId::(\w+)", pattern)) & allowed
                        value = node.child_by_field_name("value")
                        if value and re.match(r"\s*(?:\{\s*)?&\[", code(value, data)):
                            for table in selected:
                                for field in re.findall(r'"([\w\[\]]+)"', code(value, data)):
                                    if field in schemas[table]["fields"]:
                                        facts.append(dict(family=family, table=table, field=field, source=source, kind="selection", expression=text, query_read=query_read))
                    if node.type == "call_expression":
                        called = code(node.child_by_field_name("function"), data)
                        args = node.child_by_field_name("arguments")
                        arg_text = code(args, data)
                        if re.search(r"(?:field|field_mask|mask|named)(?:::[^()]*)?$", called):
                            selected = set(re.findall(r"TableId::(\w+)", arg_text)) & allowed
                            if not selected:
                                for var in re.findall(r"\b\w+\b", arg_text.split(",")[0]):
                                    selected.update(re.findall(r"\blet\s+" + re.escape(var) + r"\s*=\s*TableId::(\w+)", fn_text))
                            for table in selected & allowed:
                                names = re.findall(r'"([\w\[\]]+)"', arg_text)
                                if not names:
                                    names = re.findall(r'"([\w\[\]]+)"', fn_text)
                                for field in names:
                                    if field in schemas[table]["fields"]:
                                        facts.append(dict(family=family, table=table, field=field, source=source, kind="selection", expression=text, query_read=query_read))
                    if node.type != "field_expression":
                        continue
                    if (node.start_byte, node.end_byte) in methods:
                        continue
                    name_node = node.child_by_field_name("field")
                    field = code(name_node, data)
                    receiver = code(node.child_by_field_name("value"), data)
                    if receiver in {"self", "Self", "fields", "self.fields"}:
                        continue
                    candidates = {t for t in tables if field in schemas[t]["fields"] or any(n.startswith(field + "[") for n in schemas[t]["fields"])}
                    typed = receiver_tables(node, receiver, data, row_tables, return_types)
                    candidates = typed & allowed if typed is not None else candidates
                    if not candidates:
                        continue
                    for table in sorted(candidates):
                        fields = [n for n in schemas[table]["fields"] if n == field or n.startswith(field + "[")]
                        for name in fields:
                            item = dict(family=family, table=table, field=name, source=source, kind="member", expression=text, function=fn_name, query_read=query_read)
                            if len(candidates) > 1:
                                item["candidate_tables"] = sorted(candidates)
                                ambiguous.append(item)
                            else:
                                facts.append(item)
    def unique(items, keys):
        kept = {}
        for item in items:
            key = tuple(item.get(k) for k in keys)
            if key in kept:
                kept[key]["query_read"] = kept[key].get("query_read", False) or item.get("query_read", False)
            else:
                kept[key] = item
        return list(kept.values())
    return (unique(facts, ("family", "table", "field", "source", "expression", "kind")),
            unique(whole, ("family", "table", "source", "expression")),
            unique(ambiguous, ("family", "table", "field", "source", "expression", "kind")),
            scanned, unique(unresolved, ("family", "source", "expression")), query_calls)


def compare(manifest, facts, schemas):
    declarations = {f["name"]: {t["name"]: t for t in f["tables"]} for f in manifest["families"]}
    missing = []
    for fact in facts:
        entry = declarations[fact["family"]].get(fact["table"], {})
        fields = entry.get("fields", [])
        named = schemas[fact["table"]]["fields"] if fields == "all" else fields
        if fact["field"] not in set(named) | set(entry.get("fallback", [])):
            missing.append(dict(fact, error="missing declaration"))
        elif fact["kind"] != "required recorded bit" and fact["field"] in entry.get("recorded_only", []):
            missing.append(dict(fact, error="value read declared as recorded bits only"))
        elif fact.get("query_read") and fact["field"] not in entry.get("query_sources", {}):
            missing.append(dict(fact, error="query value lacks a query_sources label"))
    return missing


def declaration_fields(entry, schemas):
    return (schemas[entry["name"]]["fields"] if entry["fields"] == "all" else entry["fields"]) + entry.get("fallback", [])


def audit_whole_rows(whole, unresolved, config):
    covered = {}
    for read in config.get("whole_reads", []):
        for source in read.get("sites", []):
            key = (read["family"], read["table"], source)
            if key in covered:
                raise ValueError(f"Duplicate whole-row decision: {key}")
            covered[key] = read
    exceptions = {(e["family"], e.get("table"), e["source"], e["expression"]): e for e in config.get("whole_exclusions", [])}
    used, found, missing, excluded = set(), set(), [], []
    for site in whole + unresolved:
        key = (site["family"], site.get("table"), site["source"])
        exact = (*key, site["expression"])
        if key in covered and site.get("table"):
            found.add(key)
        elif exact in exceptions:
            used.add(exact)
            excluded.append(dict(site, manual_check=exceptions[exact]["reason"]))
        else:
            missing.append(site)
    if covered.keys() - found or exceptions.keys() - used:
        raise ValueError(f"Stale whole-row decisions: {covered.keys() - found}; exclusions: {exceptions.keys() - used}")
    return missing, excluded


def render(result, manifest, schemas, manifest_text):
    entries_at = {}
    family, section = None, None
    for number, line in enumerate(manifest_text.splitlines(), 1):
        if line == "[[families]]":
            section = "family"
        elif line == "[[families.tables]]":
            section = "table"
        elif line.startswith("[[shared"):
            section = None
        elif line.startswith("name = "):
            name = json.loads(line.split("=", 1)[1])
            if section == "family":
                family = name
            elif section == "table":
                entries_at[family, name] = number
    lines = ["# Independent field-access audit", "",
             f"Source commit: `{result['provenance']['commit']}`. Working tree clean: {str(result['provenance']['working_tree_clean']).lower()}.", "",
             f"Code-derived family/table/field reads found: {result['fields_found']}. Invalid value/bit declarations: {len(result['missing'])}. Unresolved invalid candidates: {len(result['ambiguous_missing'])}. Uncovered whole-row sites: {len(result['whole_missing'])}.", "",
             "Reads and whole-row decode sites are enumerated from Rust syntax and row-codec schemas before the manifest is opened. Local helpers forwarding a TableId or a table type parameter are followed to their decode sites. Query entry points are followed transitively through statically resolved function and associated-function calls, including imports and re-exports; reached readers are scanned without configured table filters. Every discovered whole-row site requires an explicit recorded-state decision or an exact cited exemption. A value read declared as bits-only is an error. Query reads are included and labelled. The configuration supplies scopes and path/type decisions, never expected field lists. All file:line evidence is validated against this checkout.", "",
             "## Limits and manual checks", ""]
    lines += ["- " + value for value in result["limitations"]]
    for check in result["manual_checks"]:
        lines += ["", f"- {check['case']} ({check['source']}): {check['decision']}"]
    lines += ["", f"Whole-row table sites found: {len(result['whole_rows'])}. Generic projection sites requiring exact review: {len(result['whole_unresolved'])}. Whole-row path decisions: {len(result['whole_row_audit'])}. Exact whole-row exemptions: {len(result['whole_excluded'])}. Query fields: {result['query_fields_found']}. Exact value candidates rejected after manual type/path checks: {len(result['excluded_candidates'])}. Their evidence and reasons, the mechanical whole-row inventory, every scanned scope, and ambiguous candidates are in scan.json.", ""]
    accesses = defaultdict(lambda: defaultdict(set))
    query_gates = {}
    for fact in result["accesses"] + result["ambiguous"]:
        accesses[fact["family"]][(fact["table"], fact["field"])].add(fact["source"])
        if fact.get("query_source"):
            query_gates[fact["family"], fact["table"], fact["field"]] = fact["query_source"]
    for family in manifest["families"]:
        if family["name"] not in accesses:
            continue
        entries = {e["name"]: e for e in family["tables"]}
        lines += [f"## {family['name']}", "", "| Table.field | Code scan evidence | Manifest charge | Recorded-state serving gate |", "|---|---|---|---|"]
        for (table, field), sources in sorted(accesses[family["name"]].items()):
            entry = entries.get(table, {})
            declared = field in declaration_fields(entry, schemas) if entry else False
            charge = "MISSING" if not declared else ("recorded bits" if field in entry.get("recorded_only", []) else "values + recorded bits")
            if field in entry.get("fallback", []):
                charge += ", fallback only"
            if field in entry.get("query_sources", {}):
                charge += ", query read: " + entry["query_sources"][field]
            elif (family["name"], table, field) in query_gates:
                charge += ", query gate: " + query_gates[family["name"], table, field]
            at = entries_at.get((family["name"], table), "missing")
            lines.append(f"| {table}.{field} | {'; '.join(sorted(sources))} | manifest.toml:{at}: {charge} | {entry.get('recorded_sources', {}).get(field, '')} |")
    lines += ["", "## Whole-value declarations", "", "| Family | Table | Every schema field found by scan | Reason |", "|---|---|---|---|"]
    for family in manifest["families"]:
        if family["name"] not in accesses:
            continue
        for entry in family["tables"]:
            if entry["fields"] == "all":
                absent = [f for f in schemas[entry["name"]]["fields"] if (entry["name"], f) not in accesses[family["name"]]]
                lines.append(f"| {family['name']} | {entry['name']} | {'yes' if not absent else ', '.join(absent)} | {entry.get('all_reason', '')} |")
    return "\n".join(lines) + "\n"


def provenance(root):
    def git(*args):
        return subprocess.check_output(["git", "-C", str(root), *args], text=True).strip()
    status = git("status", "--porcelain", "--untracked-files=all")
    return dict(commit=git("rev-parse", "HEAD"), working_tree_clean=not status, status=status)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--audit", type=Path, default=Path(__file__).with_name("field_audit.json"))
    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--require-clean", action="store_true")
    args = parser.parse_args()
    source_state = provenance(args.root)
    if args.require_clean and not source_state["working_tree_clean"]:
        raise ValueError("Commit all source changes before generating the final proof")
    config = json.loads(args.audit.read_text(encoding="utf-8"))
    schemas = schema(args.root)
    facts, whole, ambiguous, scanned, unresolved, query_calls = scan(args.root, config, schemas)
    whole_missing, whole_excluded = audit_whole_rows(whole, unresolved, config)
    for read in config.get("whole_reads", []):
        table = schemas[read["table"]]
        fields = table["fields"]
        if read.get("through"):
            fields = fields[:fields.index(read["through"]) + 1]
        for field in fields:
            ty = table["types"].get(field.split("[")[0])
            if ty is None:
                raise ValueError(f"No decoded type for {read['table']}.{field}")
            if not ty.startswith("Recorded<"):
                facts.append(dict(family=read["family"], table=read["table"], field=field, source=read["source"],
                                  kind="required recorded bit", expression=read["reason"], served_source=read["served_source"],
                                  query_source=read.get("query_source")))
    excluded = []
    exceptions = {(e["family"], e["table"], e["field"], e["source"], e["expression"]): e for e in config.get("exclusions", [])}
    used = set()
    def apply_exclusions(items):
        kept = []
        for item in items:
            key = tuple(item[k] for k in ("family", "table", "field", "source", "expression"))
            if key in exceptions:
                if item["kind"] == "required recorded bit":
                    raise ValueError("Recorded bits require a whole-row path decision, not a value exclusion: " + str(key))
                excluded.append(dict(item, manual_check=exceptions[key]["reason"]))
                used.add(key)
            else:
                kept.append(item)
        return kept
    facts = apply_exclusions(facts)
    ambiguous = apply_exclusions(ambiguous)
    if exceptions.keys() - used:
        raise ValueError("Stale manual scan exclusions: " + str(exceptions.keys() - used))
    manifest_text = (args.manifest or args.root / "crates/core-gate-memory/manifest.toml").read_text(encoding="utf-8")
    manifest = tomllib.loads(manifest_text)
    source_errors, evidence = [], {}
    for family in manifest["families"]:
        if family["name"] not in config["families"]:
            continue
        for entry in family["tables"]:
            if not schemas[entry["name"]]["fields"]:
                raise ValueError("Unparsed codec schema: " + entry["name"])
            names = declaration_fields(entry, schemas)
            if len(names) != len(set(names)) or set(names) - set(schemas[entry["name"]]["fields"]):
                raise ValueError(f"Invalid field declaration: {family['name']} {entry['name']}")
            bits = entry.get("recorded_only", [])
            if len(bits) != len(set(bits)) or set(bits) - set(names) or set(bits) != entry.get("recorded_sources", {}).keys():
                raise ValueError(f"Invalid recorded-bit declaration: {family['name']} {entry['name']}")
            if set(entry.get("query_sources", {})) - (set(names) - set(bits)):
                raise ValueError(f"Invalid query-value declaration: {family['name']} {entry['name']}")
            for source in list(entry.get("sources", {}).values()) + list(entry.get("recorded_sources", {}).values()) + list(entry.get("query_sources", {}).values()):
                file, line = source.rsplit(":", 1)
                path = args.root / file
                content = path.read_text(encoding="utf-8").splitlines() if path.is_file() else []
                if not 0 < int(line) <= len(content):
                    source_errors.append(source)
                else:
                    evidence[source] = content[int(line) - 1].strip()
    missing = compare(manifest, facts, schemas)
    uncertain_missing = compare(manifest, ambiguous, schemas)
    result = dict(fields_found=len({(r["family"], r["table"], r["field"]) for r in facts + ambiguous}), accesses=facts,
                  missing=missing, ambiguous_missing=uncertain_missing, ambiguous=ambiguous, whole_rows=whole,
                  scopes=scanned, limitations=config.get("limitations", []))
    result["whole_missing"] = whole_missing
    result["whole_excluded"] = whole_excluded
    result["whole_unresolved"] = unresolved
    result["query_fields_found"] = len({(r["family"], r["table"], r["field"]) for r in facts + ambiguous if r.get("query_read")})
    result["excluded_candidates"] = excluded
    result["whole_row_audit"] = config.get("whole_reads", [])
    result["manual_checks"] = config.get("manual_checks", [])
    result["source_errors"] = source_errors
    result["source_evidence"] = evidence
    result["query_calls"] = query_calls
    result["provenance"] = source_state
    if provenance(args.root) != source_state:
        raise ValueError("Source changed while generating the proof")
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "scan.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    (args.out / "schema.json").write_text(json.dumps(schemas, indent=2), encoding="utf-8")
    (args.out / "field-proof.md").write_text(render(result, manifest, schemas, manifest_text), encoding="utf-8")
    print(json.dumps({k: len(v) if isinstance(v, (list, dict)) else v for k, v in result.items() if k not in {"accesses", "ambiguous", "whole_rows", "scopes"}}, indent=2))
    for item in sorted(missing + uncertain_missing, key=lambda r: (r["family"], r["table"], r["field"], r["source"])):
        print(f"{item['source']}: ERROR {item['family']} {item['table']}.{item['field']}: {item['error']}")
    for site in whole_missing:
        print(f"{site['source']}: ERROR uncovered whole-row decode: {site['family']} {site.get('table', 'unresolved table')}")
    return bool(missing or uncertain_missing or source_errors or whole_missing)


if __name__ == "__main__":
    raise SystemExit(main())
