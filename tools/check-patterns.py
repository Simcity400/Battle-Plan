"""Refuse new instances of code patterns review keeps finding (Python 3.11+).

usage: python tools/check-patterns.py --against REF [--head REV]
       python tools/check-patterns.py --count [--head REV]

Rules, over first-party files (vendor/ and fuzz/ are not read):
- test-clock: test code that sleeps or reads the wall clock. Rust `thread::sleep`, `Instant::now`
  and `SystemTime::now` in test code; `setTimeout` or `setInterval` in a JS or TS `*.test.*`
  file; `Start-Sleep` or `::Sleep(` in a PowerShell `*.tests.ps1` file.
- box-leak: `Box::leak` in Rust code outside tests.
- debug-hash: a Rust hash or digest fed with `Debug` output, outside tests: a `format!`,
  `format_args!`, `write!` or `writeln!` with a `{:?}` placeholder inside the arguments of a
  hashing call (`hash*(`, `digest*(`, a hasher's `update` or `write`, or any call handed `&mut`
  a hasher, along a chain of a hasher's calls), heading a method chain that calls `hash`, alone
  or in a tuple or array, or writing to a hasher. An error message (`map_err`, `expect`, ...)
  is not hashed input. Debug output is not an identity.
- test-support-branch: `cfg!` checks of `test-support` and `#[cfg]` predicates that negate it,
  outside test and fixture code in `crates/`.

Test code is a `tests/` target, an item under a test-only `cfg` or `#[test]`, or a file the
crate's targets reach only through such items (`mod` declarations, `#[path]` included). Files
no target reaches are not test code.

Most of the tree predates these rules, so `--against REF` compares, per file that REV (default:
the working tree, untracked files included) changes or adds against its merge base with REF,
each rule's count with the same file at the merge base (at its old name when renamed). It fails
when any file has more, and names the matches on lines the change added. `--count` prints each
rule's total at REV. Comments and string contents are never read as Rust code. Exits 1 on any
failure.
"""
import argparse
import difflib
import re
import subprocess
import sys
import tomllib
from functools import cache
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[1]
RULES = ("test-clock", "box-leak", "debug-hash", "test-support-branch", "local-contrast-ink", "local-tick-engine", "family-colour-literal", "family-locale-string", "family-measure-text-loop", "family-hand-percent", "family-at-least-wording")
RULE_HINTS = {
    "local-contrast-ink": "use contrastInkForFill from ui/src/draw/kit.ts",
    "local-tick-engine": "use computeTimeAxisTicks from ui/src/draw/kit.ts or linearTicks from ui/src/charts/valueAxis.ts",
    "family-colour-literal": "use ui/src/charts/palette.ts, ui/src/shell/appearance/color.ts, or resolveKitTokens in ui/src/draw/kit.ts",
    "family-locale-string": "use formatScalar, formatCompactScalar, or formatValueState from ui/src/families/valueFormat.ts",
    "family-measure-text-loop": "use textWidth or measureCanvasText from ui/src/charts/textWidth.ts, or fitLabelWidth / fitLabel from ui/src/charts/labels.ts",
    "family-hand-percent": "use formatShare, formatProgress, or formatRecordedPercent from ui/src/families/valueFormat.ts",
    "family-at-least-wording": "show the figure with the partial mark and a plain missing reason; never print 'at least', '≥', or '+ means'",
}
JS_TEST = re.compile(r"\.test\.(?:ts|tsx|mts|js|mjs|cjs)$")
TS_FIXTURE = re.compile(r"\.fixtures?\.tsx?$")
FAMILY_OR_WIDGET_TS = re.compile(r"^ui/src/(?:families|widgets)/.+\.tsx?$")
TS_TOKEN = re.compile(r"""//|/\*|'(?:\\.|[^'\\\n])*'|"(?:\\.|[^"\\\n])*"|`(?:\\.|[^`\\])*`""", re.DOTALL)
TS_FN_RHS = r"(?:async\s+)?(?:function\b|\([^)]*\)\s*(?::[^=]+)?=>|[A-Za-z_$]\w*\s*=>)"
AT_LEAST_STRING = re.compile(r"\bat\s+least\b|\u2265|\\u2265|\+\s*means\b", re.IGNORECASE)
AT_LEAST_JSX = re.compile(r"\u2265|\+\s*means\b", re.IGNORECASE)
LOCAL_CONTRAST_INK = re.compile(
    rf"\b(?:function\s+(?:inkOn|contrastInk|relativeLuminance|readableText)\w*|(?:const|let)\s+(?:inkOn|contrastInk|relativeLuminance|readableText)\w*\s*=\s*{TS_FN_RHS}|\breadableText\s*\()"
)
LOCAL_TICK_ENGINE = re.compile(
    rf"\b(?:function\s+(?:compute\w*Ticks|nice\w*Ticks|linearTicks|timeAxisTicks|axisTicks|ticksForStep)\w*|(?:const|let)\s+(?:compute\w*Ticks|nice\w*Ticks|linearTicks|timeAxisTicks|axisTicks|ticksForStep)\w*\s*=\s*{TS_FN_RHS})"
)
FAMILY_COLOUR_LITERAL = re.compile(r"""['"`]#[0-9a-fA-F]{3,8}(?=['"`])|\b(?:rgba?|hsla?)\s*\(""")
SELECTOR_CALL_PREFIX = re.compile(r"\b(?:querySelector(?:All)?|closest|matches)\s*\(\s*$")
FAMILY_LOCALE_CODE = re.compile(r"""\.toLocaleString\s*\(|\bnew\s+Intl\.NumberFormat\s*\(""")
FAMILY_LOCALE_IMPORT = re.compile(r"""from\s+['"][^'"]*charts/numberFormat(?:\.ts)?['"]""")
FAMILY_HAND_PERCENT = re.compile(
    r"""\$\{[^}\n]*(?:\bMath\.(?:round|floor|trunc|ceil)\s*\(|\.toFixed\s*\()[^}\n]*\}\s*%|(?:\bMath\.(?:round|floor|trunc|ceil)\s*\(|\.toFixed\s*\()[^;\n]*\+\s*['"`]%"""
)
TS_LOOP_HEAD = re.compile(r"\b(?:for|while)\s*\(|\bdo\s*\{|\.(?:forEach|map|reduce|flatMap|filter|some|every)\s*\(")
TS_MEASURE_TEXT = re.compile(r"\.measureText\s*\(")
TS_MEASURE_BOX_METRIC = re.compile(r"\s*\.\s*(?:actual|font)BoundingBox(?:Ascent|Descent)\b")
PS_TEST = re.compile(r"\.tests\.ps1$", re.IGNORECASE)
JS_CLOCK = re.compile(r"\bset(?:Timeout|Interval)\s*\(")
PS_CLOCK = re.compile(r"\bStart-Sleep\b|::Sleep\s*\(", re.IGNORECASE)
RUST_CLOCK = re.compile(r"\bthread\s*::\s*sleep\b|\bInstant\s*::\s*now\b|\bSystemTime\s*::\s*now\b")
RUST_LEAK = re.compile(r"\bBox\s*(?:::\s*<[^;{}]*?>\s*)?::\s*leak\b")
FORMAT_MACRO = re.compile(r"\b(?:format|format_args|write|writeln)!\s*[(\[{]")
DEBUG_PLACEHOLDER = re.compile(r"(?<!\{)\{[^{}:]*:[^{}]*\?\}")
HASHER = r"\w*(?:hash|digest|sha\d*|blake\d*|xxh\w*|fnv|crc\d*)\w*"
HASH_CALL = re.compile(rf"(?<![\w.])(?:fn\s+)?(?:hash|digest)\w*\s*\(|\.\s*(?:hash|digest)\w*\s*\(|\b{HASHER}\s*\.\s*(?:update|write\w*|chain\w*)\s*\(|\b{HASHER}\s*::\s*new\s*\(", re.IGNORECASE)
HASHER_ARGUMENT = re.compile(rf"&\s*mut\s+(?:self\s*\.\s*)?{HASHER}\b", re.IGNORECASE)
WRITE_TO_HASHER = re.compile(rf"\b(?:write|writeln)!\s*\(\s*(?:&\s*mut\s+)?(?:self\s*\.\s*)?{HASHER}\s*,", re.IGNORECASE)
HASHER_METHOD = re.compile(r"update|write\w*|chain\w*")
DIAGNOSTIC_CALLS = {"map_err", "ok_or_else", "with_context", "context", "expect"}
CHAIN_LINK = re.compile(r"\s*(?:\?|\.\s*(\w+)\s*(?:::\s*<[^()]*?>\s*)?\()")
TOKEN = re.compile(r"""//|/\*|(?<![\w'])(?:b|c)?r(#*)"|(?<![\w'])[bc]?"|'(?:\\(?:x[0-9A-Fa-f]{2}|u\{[0-9A-Fa-f_]+\}|.)|[^\\'\n])'""")
BLOCK = re.compile(r"/\*|\*/")
STRING_END = re.compile(r'(?:[^"\\]|\\.)*"', re.DOTALL)
ATTRIBUTE = re.compile(r"#\s*(!?)\s*\[")
CFG_GATE = re.compile(r"\bcfg\s*!\s*([([{])|#\s*!?\s*\[\s*cfg\s*\(")
CFG_NOT = re.compile(r"\bnot\s*\(")
CFG_FEATURE = re.compile(r'\bfeature\s*=\s*(?:r#*)?"(\s*)"#*')
STRINGIFY = re.compile(r"\bstringify\s*!\s*[(\[{]")
TEST_ITEM = re.compile(
    r'(?:pub\s*(?:\([^)]*\)\s*)?)?'
    r'(?:(?:const|async|unsafe|safe|extern(?:\s*"\s*")?)\s+)*'
    r'(?:(?:fn|mod|trait|enum|struct)\s+(?:r#)?\w+|impl\b)'
)
MOD_DECLARATION = re.compile(r"\bmod\s+(?:r#)?(\w+)\s*([;{])")
PATH_START = re.compile(r'#\s*\[\s*path\s*=\s*"')


def mask(text):
    """The Rust text with comments and the insides of string and char literals blanked, newlines
    and length kept, so what is left is code."""
    out, at, length = [], 0, len(text)
    blank = lambda start, end: re.sub(r"[^\n]", " ", text[start:end])
    while (found := TOKEN.search(text, at)) is not None:
        start, token = found.start(), found.group()
        out.append(text[at:start])
        if token == "//":
            end = text.find("\n", start)
            end = length if end < 0 else end
            out.append(blank(start, end))
        elif token == "/*":
            depth, end = 0, length
            for edge in BLOCK.finditer(text, start):
                depth += 1 if edge.group() == "/*" else -1
                if not depth:
                    end = edge.end()
                    break
            out.append(blank(start, end))
        elif token.endswith('"'):
            hashes = found.group(1)
            if hashes is not None:
                close = text.find('"' + hashes, found.end())
                inner = length if close < 0 else close
            else:
                rest = STRING_END.match(text, found.end())
                inner = rest.end() - 1 if rest else length
            end = min(length, inner + 1 + len(hashes or ""))
            out += [token, blank(found.end(), inner), text[inner:end]]
        else:
            end = found.end()
            out.append("'" + " " * (end - start - 2) + "'")
        at = end
    out.append(text[at:])
    return "".join(out)


def closing(code, start, opening="(", close=")"):
    """The index after the bracket that closes the one at `start`."""
    depth = 0
    for index in range(start, len(code)):
        if code[index] == opening:
            depth += 1
        elif code[index] == close:
            depth -= 1
            if not depth:
                return index + 1
    return len(code)


def delimiter_end(code, start):
    opening = code[start]
    return closing(code, start, opening, {"(": ")", "[": "]", "{": "}"}[opening])


def mask_stringify(code):
    """Blank the token trees that stringify turns into text, preserving offsets and lines."""
    chunks, at = [], 0
    for macro in STRINGIFY.finditer(code):
        if macro.start() < at:
            continue
        end = delimiter_end(code, macro.end() - 1)
        chunks += [code[at:macro.end()], re.sub(r"[^\n]", " ", code[macro.end():end - 1])]
        at = end - 1
    chunks.append(code[at:])
    return "".join(chunks)


def cfg_value(predicate):
    """Whether a `cfg` predicate holds in a build that is not a test and has no `test-support`
    feature: True, False, or None when other configuration decides it."""
    predicate = predicate.strip()
    if predicate == "test" or re.fullmatch(r'feature\s*=\s*"test-support"', predicate):
        return False
    call = re.fullmatch(r"(all|any|not)\s*\((.*)\)", predicate, re.DOTALL)
    if not call:
        return None
    parts, depth, begun = [], 0, 0
    body = call.group(2)
    for index, char in enumerate(body):
        depth += char == "("
        depth -= char == ")"
        if char == "," and not depth:
            parts.append(body[begun:index])
            begun = index + 1
    parts = [part for part in [*parts, body[begun:]] if part.strip()]
    values = [cfg_value(part) for part in parts]
    if call.group(1) == "not":
        return None if values[0] is None else not values[0]
    if call.group(1) == "all":
        return False if False in values else None if None in values else True
    return True if True in values else None if None in values else False


def item_end(code, at, *, skip_generics=False):
    """The end of the item starting at `at`, after its attributes: its `;` or closing brace."""
    depth, angles = 0, 0
    index = at
    while index < len(code):
        char = code[index]
        if char in "([":
            depth += 1
        elif char in ")]":
            depth -= 1
        elif skip_generics and not depth and char == "<":
            angles += 1
        elif angles and not depth and char == ">" and code[index - 1] != "-":
            angles -= 1
        elif not depth and char == ";":
            return index + 1
        elif not depth and char == "{":
            end = closing(code, index, "{", "}")
            if not angles:
                return end
            index = end
            continue
        index += 1
    return len(code)


def test_support_features(text, code, start, end):
    return (feature for feature in CFG_FEATURE.finditer(code, start, end)
            if text[feature.start(1):feature.end(1)] == "test-support")


def cfg_attribute_text(text, code, start, end):
    """Read the attribute with comments and unrelated literal contents still masked."""
    parts, at = [], start
    for feature in test_support_features(text, code, start, end):
        parts += [code[at:feature.start()], 'feature = "test-support"']
        at = feature.end()
    parts.append(code[at:end])
    return "".join(parts)


def test_spans(text, code, *, branch_rule=False):
    """Test-only spans and whether the whole file is test code. The branch rule groups outer
    attributes and extends spans past them only when it recognizes the item boundary."""
    spans, whole = [], False
    group_start, previous_end, previous_inner = 0, 0, None
    for found in ATTRIBUTE.finditer(code):
        end = closing(code, found.end() - 1, "[", "]")
        if branch_rule:
            if found.start() < previous_end:
                continue
            if found.group(1) != previous_inner or found.start() != skip_space(code, previous_end):
                group_start = found.start()
            previous_inner = found.group(1)
            previous_end = end
            inside = cfg_attribute_text(text, code, found.end(), end - 1).strip()
        else:
            group_start = found.start()
            inside = text[found.end():end - 1].strip()
        cfg = re.fullmatch(r"cfg\s*\((.*)\)", inside, re.DOTALL)
        test_only = (cfg and cfg_value(cfg.group(1)) is False) or re.fullmatch(r"(?:\w+\s*::\s*)*test", inside)
        if not test_only:
            continue
        if found.group(1):
            if cfg:
                block = enclosing_block(code, found.start())
                if block is None:
                    whole = True
                else:
                    spans.append(block)
            continue
        at = end
        while (more := ATTRIBUTE.match(code, skip_space(code, at))) and not more.group(1):
            at = closing(code, more.end() - 1, "[", "]")
        if not branch_rule or TEST_ITEM.match(code, skip_space(code, at)):
            at = item_end(code, at, skip_generics=branch_rule)
        spans.append((group_start, at))
    return spans, whole


def enclosing_block(code, offset):
    """The (start, end) span of the innermost braces around `offset`, or None at the top level."""
    depth = 0
    for index in range(offset - 1, -1, -1):
        if code[index] == "}":
            depth += 1
        elif code[index] == "{":
            if not depth:
                return index, closing(code, index, "{", "}")
            depth -= 1
    return None


def path_attribute(text, code, start, end):
    """The file a `#[path = ".."]` attribute between `start` and `end` names, or None. The
    attribute is found in code, so a comment that quotes one is not read."""
    found = PATH_START.search(code, start, end)
    if found is None:
        return None
    rest = STRING_END.match(text, found.end())
    return text[found.end():rest.end() - 1] if rest else None


def attribute_start(code, offset):
    """Where the attributes and visibility of the item starting at `offset` begin."""
    return max(code.rfind(mark, 0, offset) for mark in ";{}") + 1


def skip_space(code, at):
    while at < len(code) and code[at].isspace():
        at += 1
    return at


def within(spans, offset):
    return any(start <= offset < end for start, end in spans)


class Revision:
    """Files at a Git revision, or in the working tree when `rev` is None."""

    def __init__(self, rev):
        self.rev = rev
        self.batch = None

    @cache
    def text(self, name):
        if self.rev is None:
            path = ROOT / name
            return path.read_text(encoding="utf-8", errors="replace") if path.is_file() else None
        if self.batch is None:
            self.batch = subprocess.Popen(["git", "cat-file", "--batch"], cwd=ROOT, stdin=subprocess.PIPE, stdout=subprocess.PIPE)
        self.batch.stdin.write(f"{self.rev}:{name}\n".encode())
        self.batch.stdin.flush()
        header = self.batch.stdout.readline().split()
        if len(header) != 3 or header[1] != b"blob":
            return None
        data = self.batch.stdout.read(int(header[2]))
        self.batch.stdout.read(1)
        return data.decode("utf-8", errors="replace")

    @cache
    def code(self, name, branch_rule=False):
        if branch_rule:
            code = self.code(name, False)
            return None if code is None else mask_stringify(code)
        text = self.text(name)
        return None if text is None else mask(text)

    @cache
    def spans(self, name, branch_rule=False):
        code = self.code(name, branch_rule)
        return ([], False) if code is None else test_spans(self.text(name), code, branch_rule=branch_rule)

    @cache
    def files(self, crate):
        """The crate's files at the revision, as repository paths."""
        command = ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", crate] if self.rev is None else ["git", "ls-tree", "-r", "-z", "--name-only", self.rev, "--", crate]
        listed = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, encoding="utf-8").stdout.split("\0")
        return frozenset(name for name in listed if name and (self.rev is not None or (ROOT / name).is_file()))

    @cache
    def tests_only(self, crate, branch_rule=False):
        """{Rust file: whether only tests compile it} for each file the crate's targets reach
        through `mod` declarations. A file several declarations reach is test-only when all are."""
        files = self.files(crate)
        roots = {}

        def root(name, test):
            roots[name] = test and roots.get(name, True)

        try:
            manifest = tomllib.loads(self.text(f"{crate}/Cargo.toml") or "")
        except tomllib.TOMLDecodeError:
            manifest = {}
        for kind in ("lib", "bin", "test", "bench", "example"):
            for target in (lambda value: value if isinstance(value, list) else [value])(manifest.get(kind, [])):
                if isinstance(target, dict) and isinstance(target.get("path"), str):
                    root(str(PurePosixPath(*_normal(PurePosixPath(crate, target["path"]).parts))), kind == "test")
        for name in files:
            inner = name[len(crate) + 1:].split("/")
            if not name.endswith(".rs"):
                continue
            if inner in (["src", "lib.rs"], ["src", "main.rs"], ["build.rs"]) or (inner[:2] == ["src", "bin"] and (len(inner) == 3 or inner[3:] == ["main.rs"])):
                root(name, False)
            elif inner[0] in ("tests", "benches", "examples") and (len(inner) == 2 or inner[2:] == ["main.rs"]):
                root(name, inner[0] == "tests")
        state = {name: test for name, test in roots.items() if name in files}
        owner = {name: True for name in state}
        pending = list(state)
        while pending:
            parent = pending.pop()
            for child, declared_test, by_path in self.children(parent, owner[parent], branch_rule):
                if child not in files:
                    continue
                test = state[parent] or self.spans(parent, branch_rule)[1] or declared_test
                owner[child] = owner.get(child, False) or by_path or child.endswith("/mod.rs")
                if child not in state or (state[child] and not test):
                    state[child] = test
                    pending.append(child)
        return state

    def children(self, name, owns_directory, branch_rule=False):
        """[(child file, whether its declaration is test-only, whether `#[path]` names it)] for
        each `mod x;` in the file. A crate root, a `mod.rs` and a file `#[path]` loads own their
        directory; another file's children live in the folder of its name."""
        text, code = self.text(name), self.code(name, branch_rule)
        if code is None:
            return []
        path = PurePosixPath(name)
        here = path.parent if owns_directory else path.parent / path.stem
        spans = self.spans(name, branch_rule)[0]
        inline = [(found.group(1), found.start(), closing(code, found.end() - 1, "{", "}")) for found in MOD_DECLARATION.finditer(code) if found.group(2) == "{"]
        out = []
        for found in MOD_DECLARATION.finditer(code):
            if found.group(2) != ";":
                continue
            nesting = [module for module, start, end in inline if start < found.start() < end]
            given = path_attribute(text, code, attribute_start(code, found.start()), found.start())
            test = within(spans, found.start())
            if given is not None:
                folder = path.parent if not nesting else here.joinpath(*nesting)
                out.append((str(PurePosixPath(*_normal((folder / given).parts))), test, True))
            else:
                folder = here.joinpath(*nesting)
                out += [(f"{folder}/{found.group(1)}.rs", test, False), (f"{folder}/{found.group(1)}/mod.rs", test, False)]
        return out

    @cache
    def test_file(self, name, branch_rule=False):
        """Whether only tests compile the Rust file: anything under a crate's `tests/`, or a file
        its crate's targets reach only through test-only items. A file no target reaches is not
        a test file."""
        parts = PurePosixPath(name).parts
        if len(parts) < 3 or parts[0] != "crates":
            return False
        if parts[2] == "tests":
            return True
        return self.tests_only("/".join(parts[:2]), branch_rule).get(name, False) or self.spans(name, branch_rule)[1]

    def close(self):
        if self.batch is not None:
            self.batch.communicate()
            self.batch = None


def _normal(parts):
    out = []
    for part in parts:
        if part == "..":
            out.pop()
        elif part != ".":
            out.append(part)
    return out


def line_of(text, offset):
    return text.count("\n", 0, offset) + 1


def enclosing_parenthesis(code, offset, brackets="("):
    """The index of the innermost unclosed `(` (or other of `brackets`) before `offset`, or None
    when a brace or another bracket encloses it first."""
    depth = 0
    for index in range(offset - 1, -1, -1):
        if code[index] in ")]}":
            depth += 1
        elif code[index] in "([{":
            if not depth:
                return index if code[index] in brackets else None
            depth -= 1
    return None


def called(code, opening):
    """The name of the function or method whose arguments open at `opening`, or ''."""
    end = opening
    while end > 0 and code[end - 1].isspace():
        end -= 1
    start = end
    while start > 0 and (code[start - 1].isalnum() or code[start - 1] == "_"):
        start -= 1
    return code[start:end]


def hashes_on(code, at):
    """Whether a method chain going on from `at` calls `hash`, as in `format!(..).hash(state)`."""
    while (link := CHAIN_LINK.match(code, at)) is not None:
        if link.group(1) is None:
            at = link.end()
            continue
        if link.group(1).lower().startswith("hash"):
            return True
        at = closing(code, link.end() - 1)
    return False


def hashed_regions(code):
    """The (start, end) spans whose contents reach a hash: the arguments of a hashing call, and
    of any call handed `&mut` a hasher."""
    regions = []
    for call in HASH_CALL.finditer(code):
        if call.group().lower().startswith("fn"):
            continue
        end = closing(code, call.end() - 1)
        regions.append((call.end() - 1, end))
        while (link := CHAIN_LINK.match(code, end)) is not None and link.group(1) is not None and HASHER_METHOD.fullmatch(link.group(1)):
            end = closing(code, link.end() - 1)
            regions.append((link.end() - 1, end))
    for argument in HASHER_ARGUMENT.finditer(code):
        opening = enclosing_parenthesis(code, argument.start())
        if opening is not None:
            regions.append((opening, closing(code, opening)))
    return regions


def debug_hashes(text, code, spans):
    """The offsets of `Debug` format macros outside tests whose output reaches a hash."""
    found, regions = [], None
    for macro in FORMAT_MACRO.finditer(code):
        opening = macro.end() - 1
        if within(spans, macro.start()) or not debug_format(text, code, opening):
            continue
        inner = enclosing_parenthesis(code, macro.start(), "([{")
        while inner is not None and code[inner] == "{":
            inner = enclosing_parenthesis(code, inner, "([{")
        if inner is not None and called(code, inner) in DIAGNOSTIC_CALLS:
            continue
        if regions is None:
            regions = hashed_regions(code)
        if within(regions, macro.start()) or WRITE_TO_HASHER.match(code, macro.start()) or chained_hash(code, macro.start(), opening):
            found.append(macro.start())
    return found


def chained_hash(code, start, opening):
    """Whether the macro's value, or a tuple, array or call holding it, heads a chain that calls
    `hash`, as in `(key, format!(..)).hash(state)`."""
    end = closing(code, opening, code[opening], {"(": ")", "[": "]", "{": "}"}[code[opening]])
    while True:
        if hashes_on(code, end):
            return True
        outer = enclosing_parenthesis(code, start, "([")
        if outer is None:
            return False
        start, end = outer, closing(code, outer, code[outer], ")" if code[outer] == "(" else "]")


def debug_format(text, code, opening):
    """Whether the format string of the macro whose arguments open at `opening` has a Debug
    placeholder."""
    pair = {"(": ")", "[": "]", "{": "}"}[code[opening]]
    literal = code.find('"', opening, closing(code, opening, code[opening], pair))
    if literal < 0:
        return False
    rest = STRING_END.match(text, literal + 1)
    body = text[literal + 1:rest.end() - 1] if rest else text[literal + 1:]
    return bool(DEBUG_PLACEHOLDER.search(body.replace("{{", "")))


def test_support_branches(text, code, spans):
    """Offsets of runtime checks and negated feature gates in production code."""
    for gate in CFG_GATE.finditer(code):
        if within(spans, gate.start()):
            continue
        start = gate.end()
        end = delimiter_end(code, start - 1) - 1
        if gate.group(1):
            if any(test_support_features(text, code, start, end)):
                yield gate.start()
            continue
        for negation in CFG_NOT.finditer(code, start, end):
            negation_end = closing(code, negation.end() - 1) - 1
            if any(test_support_features(text, code, negation.end(), negation_end)):
                yield gate.start()
                break


def rust_matches(name, revision):
    """{rule: [line, ...]} for a Rust file at a revision."""
    text, code = revision.text(name), revision.code(name, False)
    spans, _ = revision.spans(name, False)
    test = revision.test_file(name, False)
    found = {rule: [] for rule in RULES}
    for match in RUST_CLOCK.finditer(code):
        if test or within(spans, match.start()):
            found["test-clock"].append(line_of(text, match.start()))
    if not test:
        for match in RUST_LEAK.finditer(code):
            if not within(spans, match.start()):
                found["box-leak"].append(line_of(text, match.start()))
        found["debug-hash"] = [line_of(text, offset) for offset in debug_hashes(text, code, spans)]
    if name.startswith("crates/") and "test-support" in text and not revision.test_file(name, True):
        branch_code = revision.code(name, True)
        branch_spans, _ = revision.spans(name, True)
        found["test-support-branch"] = [line_of(text, offset) for offset in test_support_branches(text, branch_code, branch_spans)]
    return found


def is_family_or_widget(name):
    if JS_TEST.search(name) or TS_FIXTURE.search(name):
        return False
    return bool(FAMILY_OR_WIDGET_TS.match(name))


def at_least_wording_lines(text):
    lines = []

    def scan_template(body, base_offset):
        at, length = 0, len(body)
        while at < length:
            interp = body.find("${", at)
            chunk = body[at:] if interp < 0 else body[at:interp]
            for hit in AT_LEAST_STRING.finditer(chunk):
                lines.append(line_of(text, base_offset + at + hit.start()))
            if interp < 0:
                break
            end = closing(body, interp + 1, "{", "}")
            lines.extend(at_least_wording_lines_in_segment(text, body[interp + 2:end - 1], base_offset + interp + 2))
            at = end

    def at_least_wording_lines_in_segment(full_text, segment, base_offset):
        found_lines = []
        at, length = 0, len(segment)
        while (tok := TS_TOKEN.search(segment, at)) is not None:
            start, token = tok.start(), tok.group()
            plain = segment[at:start]
            for hit in AT_LEAST_JSX.finditer(plain):
                found_lines.append(line_of(full_text, base_offset + at + hit.start()))
            if token == "//":
                end = segment.find("\n", start)
                end = length if end < 0 else end
            elif token == "/*":
                close = segment.find("*/", start + 2)
                end = length if close < 0 else close + 2
            elif token.startswith("`"):
                end = tok.end()
                scan_template(token[1:-1], base_offset + start + 1)
            else:
                end = tok.end()
                for hit in AT_LEAST_STRING.finditer(token[1:-1]):
                    found_lines.append(line_of(full_text, base_offset + start + 1 + hit.start()))
            at = end
        for hit in AT_LEAST_JSX.finditer(segment[at:]):
            found_lines.append(line_of(full_text, base_offset + at + hit.start()))
        return found_lines

    lines.extend(at_least_wording_lines_in_segment(text, text, 0))
    return sorted(lines)


def mask_ts_comments(text, *, blank_strings=False):
    out, at, length = [], 0, len(text)
    blank = lambda start, end: re.sub(r"[^\n]", " ", text[start:end])
    while (found := TS_TOKEN.search(text, at)) is not None:
        start, token = found.start(), found.group()
        out.append(text[at:start])
        if token == "//":
            end = text.find("\n", start)
            end = length if end < 0 else end
            out.append(blank(start, end))
        elif token == "/*":
            close = text.find("*/", start + 2)
            end = length if close < 0 else close + 2
            out.append(blank(start, end))
        else:
            end = found.end()
            if blank_strings and len(token) >= 2:
                out.append(token[0] + blank(start + 1, end - 1) + token[-1])
            else:
                out.append(token)
        at = end
    out.append(text[at:])
    return "".join(out)


def ts_loop_spans(code):
    spans = []
    for head in TS_LOOP_HEAD.finditer(code):
        token = head.group()
        if token.startswith("."):
            end = closing(code, head.end() - 1, "(", ")")
            spans.append((head.end() - 1, end))
        elif token.endswith("{"):
            end = closing(code, head.end() - 1, "{", "}")
            spans.append((head.end() - 1, end))
        else:
            after = skip_space(code, closing(code, head.end() - 1, "(", ")"))
            if after < len(code) and code[after] == "{":
                spans.append((after, closing(code, after, "{", "}")))
            else:
                semi = code.find(";", after)
                nl = code.find("\n", after)
                end = min(p for p in (semi + 1 if semi >= 0 else len(code), nl if nl >= 0 else len(code)))
                spans.append((after, end))
    return spans


def family_ts_matches(name, text):
    code_with_strings = mask_ts_comments(text, blank_strings=False)
    code = mask_ts_comments(text, blank_strings=True)
    found = {}
    contrast_lines = [line_of(text, m.start()) for m in LOCAL_CONTRAST_INK.finditer(code)]
    if contrast_lines:
        found["local-contrast-ink"] = contrast_lines
    tick_lines = [line_of(text, m.start()) for m in LOCAL_TICK_ENGINE.finditer(code)]
    if tick_lines:
        found["local-tick-engine"] = tick_lines
    colour_lines = [
        line_of(text, m.start())
        for m in FAMILY_COLOUR_LITERAL.finditer(code_with_strings)
        if not (m.group().startswith(("'", '"', "`")) and SELECTOR_CALL_PREFIX.search(code_with_strings, max(0, m.start() - 40), m.start()))
    ]
    if colour_lines:
        found["family-colour-literal"] = colour_lines
    if name != "ui/src/families/valueFormat.ts":
        locale_offsets = [m.start() for m in FAMILY_LOCALE_CODE.finditer(code)] + [m.start() for m in FAMILY_LOCALE_IMPORT.finditer(code_with_strings)]
        locale_lines = [line_of(text, offset) for offset in sorted(locale_offsets)]
        if locale_lines:
            found["family-locale-string"] = locale_lines
        percent_lines = [line_of(text, m.start()) for m in FAMILY_HAND_PERCENT.finditer(code_with_strings)]
        if percent_lines:
            found["family-hand-percent"] = percent_lines
    if ".measureText" in code:
        loops = ts_loop_spans(code)
        measure_lines = [
            line_of(text, m.start())
            for m in TS_MEASURE_TEXT.finditer(code)
            if within(loops, m.start()) and not TS_MEASURE_BOX_METRIC.match(code, closing(code, m.end() - 1, "(", ")"))
        ]
        if measure_lines:
            found["family-measure-text-loop"] = measure_lines
    at_least_lines = at_least_wording_lines(text)
    if at_least_lines:
        found["family-at-least-wording"] = at_least_lines
    return found


def matches(name, revision):
    """{rule: [line, ...]} for a file at a revision; empty when no rule reads the file."""
    text = revision.text(name)
    if text is None:
        return {}
    if name.endswith(".rs"):
        return rust_matches(name, revision)
    if is_family_or_widget(name):
        return family_ts_matches(name, text)
    pattern = JS_CLOCK if JS_TEST.search(name) else PS_CLOCK if PS_TEST.search(name) else None
    if pattern is None:
        return {}
    return {"test-clock": [line_of(text, found.start()) for found in pattern.finditer(text)]}


def git(*args):
    result = subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True, encoding="utf-8")
    if result.returncode:
        sys.exit(f"git {' '.join(args)} failed: {result.stderr.strip()}")
    return result.stdout


def read(name):
    """Whether a rule reads the file: first-party Rust, family/widget TS, JS and TS tests, PowerShell tests."""
    if name.startswith(("vendor/", "fuzz/")):
        return False
    return name.endswith(".rs") or is_family_or_widget(name) or bool(JS_TEST.search(name) or PS_TEST.search(name))


def changes(base, head):
    """[(old name or None, new name)] for files `head` changes or adds against `base`."""
    fields = git("diff", "--name-status", "-M", "-z", base, *([head] if head else []), "--").split("\0")
    changed, index = [], 0
    while index < len(fields) - 1:
        status = fields[index]
        if status.startswith(("R", "C")):
            old, new = fields[index + 1], fields[index + 2]
            index += 3
        else:
            old = new = fields[index + 1]
            index += 2
        if not status.startswith("D"):
            changed.append((old if not status.startswith("A") else None, new))
    if head is None:
        changed += [(None, name) for name in git("ls-files", "-z", "--others", "--exclude-standard").split("\0") if name]
    return [(old, new) for old, new in changed if read(new)]


def added_lines(before, after):
    """The 1-based lines of `after` that are not carried over from `before`."""
    old, new = (before or "").splitlines(), after.splitlines()
    added = set()
    for tag, _, _, start, end in difflib.SequenceMatcher(None, old, new, autojunk=False).get_opcodes():
        if tag in ("replace", "insert"):
            added.update(range(start + 1, end + 1))
    return added


def against(ref, head):
    found = subprocess.run(["git", "merge-base", head or "HEAD", ref], cwd=ROOT, capture_output=True, text=True)
    if found.returncode:
        sys.exit(f"No merge base with {ref}: fetch it, or pass another --against.")
    base, now = Revision(found.stdout.strip()), Revision(head)
    try:
        failures = compare(base, now)
    finally:
        base.close()
        now.close()
    for failure in failures:
        print(failure)
    if failures:
        print(f"\n{len(failures)} file(s) add instances of a refused pattern; see `python tools/check-patterns.py --help`.")
        return 1
    print("No new instances of refused patterns.")
    return 0


def compare(base, now):
    """A line for each file `now` gives more instances of a rule than `base`."""
    failures = []
    changed = changes(base.rev, now.rev)
    for old, new in changed + reclassified(base, now, changed):
        after = matches(new, now)
        before = matches(old, base) if old and read(old) else {}
        for rule, lines in after.items():
            if len(lines) > len(before.get(rule, ())):
                fresh = sorted(set(lines) & added_lines(base.text(old) if old else "", now.text(new))) or lines
                hint = f" ({RULE_HINTS[rule]})" if rule in RULE_HINTS else ""
                failures.append(f"{new}: {rule}: {len(before.get(rule, ()))} at the base, {len(lines)} now; new at line {', '.join(map(str, fresh))}{hint}")
    return failures


def reclassified(base, now, changed):
    """[(name, name)] for Rust files outside `changed` that the change moves into or out of test
    code, as when a parent's `#[cfg(test)]` on its `mod` declaration goes."""
    names = {new for _, new in changed}
    manifests = git("diff", "--name-only", "-z", base.rev, *([now.rev] if now.rev else []), "--", "crates/*/Cargo.toml").split("\0")
    crates = {"/".join(name.split("/")[:2]) for name in [*names, *manifests] if name.startswith("crates/") and name.endswith((".rs", "/Cargo.toml"))}
    moved = []
    for crate in sorted(crates):
        reached = {name for revision in (base, now) for branch_rule in (False, True)
                   for name in revision.tests_only(crate, branch_rule)}
        for name in sorted(reached - names):
            if base.text(name) is not None and any(base.test_file(name, branch_rule) != now.test_file(name, branch_rule)
                                                   for branch_rule in (False, True)):
                moved.append((name, name))
    return moved


def count(head):
    revision = Revision(head)
    listing = git("ls-files", "-z", "--cached", "--others", "--exclude-standard") if head is None else git("ls-tree", "-r", "--name-only", "-z", head)
    names = [name for name in listing.split("\0") if name and read(name)]
    totals = {rule: 0 for rule in RULES}
    try:
        for name in names:
            for rule, lines in matches(name, revision).items():
                totals[rule] += len(lines)
    finally:
        revision.close()
    for rule in RULES:
        print(f"{rule}: {totals[rule]}")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--against", metavar="REF", help="refuse files with more instances than at the merge base with REF")
    mode.add_argument("--count", action="store_true", help="print each rule's total")
    parser.add_argument("--head", metavar="REV", help="check a revision instead of the working tree")
    args = parser.parse_args()
    return count(args.head) if args.count else against(args.against, args.head)


if __name__ == "__main__":
    sys.exit(main())
