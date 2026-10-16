"""Report executable addresses, native offsets and decompiler names in comments, strings and docs.

usage: python tools/check-citations.py [--summary] [--code] [--revision REV] [paths...]

Default paths: crates, tools, docs, ui/src, fuzz and the top-level Markdown files.

- `address`: an executable address (`0x14xxxxxxx`, a bare `14xxxxxxx`, `Name@14xxxxxxx`) in a
  comment, string literal or doc. One used as a code value is counted (`code-address`) and
  reported only with `--code`, unless it is the value of a constant (`const NAME: T = 0x...;`)
  or the image base.
- `offset`: native memory-layout notation. In a comment or doc: a field offset (`+0x2A8`,
  `record + 0x30`, `army+0x340`, `+D0`, `this - 0xB8`), a decimal offset placed as one
  (`at +8`, `header +8`, `(+12)`, `+20/+24`, `+0..`), one attached to a name (`order+8`,
  `following+818`), a dereference (`[0x30]`), a register operand (`[rsp+0x28]`), an
  `offset 0x30`, a slot (`slot 0x2E`, `vslot 0xD8`), a bare code address in either case
  (`F3A90`, `f3a90`), and any hexadecimal cell under an `Offset` column of a table. In a string
  literal outside an assembly block: `+0x2A8`, `offset 0x30`, `slot 0x2E` and register
  operands such as `[rax + 0x28]`, `[rsp + 8]` or `[RBP-0x29]`. Code that needs an offset
  names it with a constant, and text names the constant or describes the field.
- `decompiler`: decompiler names (`FUN_`, `DAT_`, `LAB_`, `PTR_`, `sub_`, `loc_`, `unk_`, `off_`,
  `byte_`, `word_`, `dword_`, `qword_`, `stru_`, `vtable_` with a hex suffix) anywhere, and
  numbered virtual-function slots (`vf22`) in comments and docs.
- `native-name`: native `CClass::Method` names in text.

Not reported, deliberately:
- key chords and code points: `Shift+F10`, `U+E000`;
- arithmetic: `+1`, `+2`, `+3` (also attached, as in `index+1`), a signed number in prose
  without an offset context (`+20`, `hours +2 to +4`), `+5%`, `(-1, +1)`, a call argument
  (`zigzag(+99)`), and a `+` displacement, spaced or attached, whose left operand is a number,
  constant or parenthesis or which another operator follows (`UINT_MAX + 0x4A2 - 1`,
  `UINT_MAX+5`, `(raw + 0x4000) >> 15`, `(count+8) >> 3`);
- sizes, strides, counts and indices (`stride 0x20`, `0x30-byte entries`, `index 0x2E`);
- values: sentinels (`0xFFFFFFFF`), masks, bit patterns, colours, save and lexer tokens,
  modifier and enum numbers, file-format bytes, negative literals (`-0x8000`);
- synthetic addresses in test fixtures written without an offset sign (`one province at
  0x4000`);
- register and calling-convention names (`RBX`, `RDX`);
- assembly: string literals inside `asm!`, `global_asm!` and `naked_asm!` blocks, whose
  operands (`[rsp + 0x28]`) are code;
- decimal or bare numbers in strings, which are displayed values and test data;
- decimal byte positions in a table of one of our own buffers (a WGSL uniform register map);
- code: values in code (test data and named constants) and whole lines of scripts outside
  their `#` comments.
Exits 1 when anything is reported.
"""
import argparse
import os
import re
import subprocess
import sys

ADDRESS = re.compile(r'(?<![0-9A-Za-z_])(?:0x)?14[0-9A-Fa-f]{7}(?![0-9A-Za-z_])')
CONST_DEF = re.compile(r'^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const|static)\s+[A-Z][A-Z0-9_]*\s*:')
DECOMPILER = re.compile(
    r'\b(?:FUN|DAT|LAB|PTR|vtable)_[0-9A-Fa-f]{6,}\b|\bsub_14[0-9A-Fa-f]{7}\b'
    r'|\b(?:sub|loc|unk|off|byte|word|dword|qword|stru|nullsub|j_sub)_[0-9A-Fa-f]{3,}\b')
VIRTUAL_SLOT = re.compile(r'\bvf[0-9]{1,3}\b')
NATIVE_METHOD = re.compile(r'\bC[A-Z][A-Za-z0-9]+::[A-Z~][A-Za-z0-9_]*')
IMAGE_BASE = {'0x140000000', '140000000'}

HEX = r'[0-9A-Fa-f]'
PLUS_HEX = re.compile(r'\+ ?0x' + HEX + r'[0-9A-Fa-f_]*')
OFFSET_WORD = re.compile(r'\b(?:offsets?|displacement)(?: of)? `?0x' + HEX, re.IGNORECASE)
SLOT = re.compile(r'\b(?:v|vtable |virtual |virtual-function )?slots? `?0x' + HEX + r'+',
                  re.IGNORECASE)
REGISTER_OPERAND = re.compile(r'\[ ?r[a-z0-9]{1,3} ?[+-] ?(?:0x[0-9A-Fa-f_]+|[0-9A-Fa-f_]*[0-9]'
                              r'[0-9A-Fa-f_]*)(?![\w])', re.IGNORECASE)
OFFSETS = [
    # `+0x2A8`, `record + 0x30`, `+0x10..0x20`.
    PLUS_HEX,
    # `this - 0xB8`: a short negative displacement.
    re.compile(r'(?<=[\w)`] )- ?0x' + HEX + r'{1,4}(?![0-9A-Za-z_])'),
    # `+D0`, `+2A0`, `+AC8`: hexadecimal without its prefix.
    re.compile(r'(?<![\w)\]+])\+(?=' + HEX + r'{0,4}[A-Fa-f])(?=' + HEX + r'{0,4}[0-9])'
               + HEX + r'{2,5}(?![\w])'),
    # `+8/+C`: a one-letter offset in a list of offsets.
    re.compile(r'(?<=/)\+[A-F]{1,3}(?![\w])|(?<![\w)\]+])\+[A-F]{1,3}(?=/\+)'),
    # `[0x30]`.
    re.compile(r'\[ ?0x' + HEX + r'+ ?\]'),
    # `offset 0x30`, `displacement of 0x30`.
    OFFSET_WORD,
    # `slot 0x2E`, `vslot 0xD8`, `vtable slot 0x20`.
    SLOT,
    # `[rax + 0x28]`, `[RBP-0x29]`: a register operand in prose.
    REGISTER_OPERAND,
    # `F3A90`, `f3a90`: a bare code address, a letter after its first digit.
    re.compile(r'(?<![\w#&+-])(?=' + HEX + r'*[0-9])(?=' + HEX + r'+[A-Fa-f])(?!D3D11\b)'
               r'(?![0Ff]+\b)' + HEX + r'{5,7}(?![\w])'),
]
# `header +8`, `contribution +38`, `(+12)`, `+20/+24`, `+C`: a decimal or one-letter displacement
# standing alone. Arithmetic is told apart by value (`+1` to `+3`), units (`+5%`, `+2 days`) and
# call arguments (`zigzag(+99)`); other arithmetic in prose is written without a sign.
DECIMAL_OFFSET = re.compile(
    r'(?<![\w)\]+\-])(?<!\w\()\+(?:([0-9]+)(?![\w]|[.,][0-9])'
    r'(?!\s*(?:%|percent|days?|hours?|weeks?|months?|years?)\b|\s*%)|([A-F])(?![\w]))')
ARITHMETIC_VALUES = {'1', '2', '3'}
# `x + 0x4000` is arithmetic when its left operand is a number, a constant or a parenthesis, or
# when another operator follows it: `UINT_MAX + 0x4A2 - 1`, `(raw + 0x4000) >> 15`.
ARITHMETIC_LEFT = re.compile(r'(?:\b[0-9A-Z_]+|\))\s$')
ARITHMETIC_RIGHT = re.compile(r'^\)?\s*(?:>>|<<|[-*/%&|^])\s')
# String literals outside assembly blocks: only the prefixed forms; decimal and bare numbers there
# are displayed values and test data.
STRING_OFFSETS = [PLUS_HEX, OFFSET_WORD, SLOT, REGISTER_OPERAND]
REGISTER_BEFORE = re.compile(r'\[ ?r[a-z0-9]{1,3} ?$', re.IGNORECASE)
# `order+8`, `following+818`: a displacement attached to the name of what it is inside, decimal
# or bare hexadecimal (`+0x` is matched above).
ATTACHED = re.compile(r'\b(?!(?:Shift|Ctrl|Alt|Cmd|Meta|U)\+)[A-Za-z_][A-Za-z0-9_]*\+('
                      + HEX + r'*[0-9]' + HEX + r'*)(?!\w|\.[0-9])')
# The macros whose string arguments are assembly templates.
ASSEMBLY_MACRO = re.compile(r'\b(?:asm|global_asm|naked_asm)!\s*[(\[{]')
# A table header cell that names an offset column.
OFFSET_HEADER = re.compile(r'^\s*`?offsets?`?\s*$', re.IGNORECASE)
TABLE_HEX = re.compile(r'0x' + HEX + r'+|(?=' + HEX + r'*[A-Fa-f])' + HEX + r'*[0-9]' + HEX + r'*')

EXTENSIONS = {'.rs', '.c', '.h', '.cpp', '.md', '.py', '.ts', '.tsx', '.mjs', '.js',
              '.ps1', '.toml', '.json', '.wgsl', '.hlsl', '.txt', '.tsv'}
DOC_EXTENSIONS = {'.md', '.txt'}
HASH_COMMENTS = {'.py', '.ps1', '.toml'}
CODE_EXTENSIONS = {'.rs', '.c', '.h', '.cpp', '.ts', '.tsx', '.mjs', '.js', '.wgsl', '.hlsl'}
DEFAULT_PATHS = ['crates', 'tools', 'docs', 'ui/src', 'fuzz']
SELF = 'tools/check-citations.py'

PREFILTER = (r'14[0-9A-Fa-f]{7}|_[0-9A-Fa-f]{3}|C[A-Z][A-Za-z0-9]+::|\bvf[0-9]'
             r'|[+-] ?0x|\+[0-9A-Fa-f]|\[ ?0x|\[ ?[Rr][A-Za-z0-9]{1,3} ?[+-]'
             r'|[Oo]ffset|[Dd]isplacement|[Ss]lot'
             r'|[0-9A-Fa-f]{5}')


def candidate_files(revision, paths):
    """Every file under `paths` with a line the cheap prefilter matches."""
    command = ['git', 'grep', '-l', '-I', '-E', PREFILTER]
    if revision:
        command.append(revision)
    top_level = [':(glob)*.md'] if paths == DEFAULT_PATHS else []
    command += ['--', *paths, *top_level, f':!{SELF}']
    out = subprocess.run(command, capture_output=True).stdout.decode('utf-8', 'replace')
    for name in out.splitlines():
        if revision:
            name = name[len(revision) + 1:]
        if os.path.splitext(name)[1] in EXTENSIONS:
            yield name


def read_files(revision, names):
    """(name, lines) of each file, from the working tree or, in one batch, from `revision`."""
    if not revision:
        for name in names:
            with open(name, 'rb') as handle:
                yield name, handle.read().decode('utf-8', 'replace').split('\n')
        return
    request = ''.join(f'{revision}:{name}\n' for name in names).encode()
    out = subprocess.run(['git', 'cat-file', '--batch'], input=request, capture_output=True).stdout
    at = 0
    for name in names:
        end = out.index(b'\n', at)
        size = int(out[at:end].split()[2])
        yield name, out[end + 1:end + 1 + size].decode('utf-8', 'replace').split('\n')
        at = end + 1 + size + 1


CHAR_LITERAL = re.compile(r"'(?:\\(?:x[0-9A-Fa-f]{2}|u\{[0-9A-Fa-f]+\}|.)|[^\\'\n])'")
RAW_STRING = re.compile(r'b?r(#*)"')
OPENERS, CLOSERS = '([{', ')]}'


def text_spans(lines, extension):
    """Per line, the (start, end, kind) parts that are comments (and docs), string literals or
    assembly templates (`asm`), tracking block comments, multi-line strings and assembly macro
    invocations across lines."""
    if extension not in CODE_EXTENSIONS:
        if extension in HASH_COMMENTS:
            return [[(0, len(line), 'line')] + ([(line.index('#'), len(line), 'comment')]
                                                  if '#' in line else []) for line in lines]
        kind = 'comment' if extension in DOC_EXTENSIONS else 'string'
        return [[(0, len(line), kind)] for line in lines]
    rust = extension == '.rs'
    quotes = '"' if extension in ('.rs', '.wgsl', '.hlsl') else '"\'`'
    result = []
    state, depth, closing, string_kind = 'code', 0, '', 'string'
    assembly = 0  # bracket depth inside an assembly macro invocation, zero outside one
    for line in lines:
        spans, index, start = [], 0, 0
        while index < len(line):
            if state == 'block':
                end = line.find('*/', index)
                nested = line.find('/*', index) if rust else -1
                if nested != -1 and (end == -1 or nested < end):
                    depth, index = depth + 1, nested + 2
                    continue
                if end == -1:
                    break
                depth, index = depth - 1, end + 2
                if depth == 0:
                    spans.append((start, index, 'comment'))
                    state = 'code'
                continue
            if state == 'string':
                char = line[index]
                if char == '\\':
                    index += 2
                    continue
                index += 1
                if char == closing:
                    spans.append((start, index, string_kind))
                    state = 'code'
                continue
            if state == 'raw':
                end = line.find(closing, index)
                if end == -1:
                    break
                index = end + len(closing)
                spans.append((start, index, string_kind))
                state = 'code'
                continue
            if line.startswith('//', index):
                spans.append((index, len(line), 'comment'))
                index = len(line)
                break
            if line.startswith('/*', index):
                state, depth, start, index = 'block', 1, index, index + 2
                continue
            char = line[index]
            previous = line[index - 1] if index else ' '
            if rust and not assembly:
                macro = ASSEMBLY_MACRO.match(line, index)
                if macro and not (previous.isalnum() or previous == '_'):
                    assembly, index = 1, macro.end()
                    continue
            if assembly and char in OPENERS:
                assembly += 1
            elif assembly and char in CLOSERS:
                assembly -= 1
            kind = 'asm' if assembly else 'string'
            if rust and char in 'br' and not (previous.isalnum() or previous == '_'):
                raw = RAW_STRING.match(line, index)
                if raw:
                    state, closing, start, string_kind = 'raw', '"' + raw.group(1), index, kind
                    index = raw.end()
                    continue
            if rust and char == "'":
                literal = CHAR_LITERAL.match(line, index)
                index = literal.end() if literal else index + 1
                continue
            if char in quotes:
                state, closing, start, string_kind = 'string', char, index, kind
                index += 1
                continue
            index += 1
        if state != 'code':
            spans.append((start, len(line), 'comment' if state == 'block' else string_kind))
        result.append(spans)
    return result


def offsets(text, kind='comment'):
    """Offset notation in a comment, or the prefixed forms in a string literal outside an
    assembly block."""
    if kind in ('line', 'asm'):
        return []
    patterns = OFFSETS if kind == 'comment' else STRING_OFFSETS
    found = []
    for pattern in patterns:
        for match in pattern.finditer(text):
            before = text[:match.start()]
            if pattern is PLUS_HEX and REGISTER_BEFORE.search(before):
                continue  # reported once, as the register operand
            if pattern is PLUS_HEX and match.group(0).startswith('+ ') and (
                    ARITHMETIC_LEFT.search(before)
                    or ARITHMETIC_RIGHT.match(text[match.end():])):
                continue
            found.append(match.group(0))
    if kind == 'comment':
        for match in DECIMAL_OFFSET.finditer(text):
            if (match.group(1) or match.group(2)) not in ARITHMETIC_VALUES:
                found.append(match.group(0))
        for match in ATTACHED.finditer(text):
            name = match.group(0)[:-len(match.group(1)) - 1]
            if (match.group(1) in ARITHMETIC_VALUES or re.fullmatch(r'[0-9A-Z_]+', name)
                    or ARITHMETIC_RIGHT.match(text[match.end():])):
                continue  # arithmetic, as for a spaced `+`
            found.append(match.group(0))
    return found


def table_cells(text):
    """The cells of a Markdown table row (`| a | b |`) in a doc line or comment, else None."""
    body = re.sub(r'^\s*(?://[/!]?|#|\*)?\s*', '', text).strip()
    if not body.startswith('|'):
        return None
    return [cell.strip() for cell in body.strip('|').split('|')]


def scan_line(line, spans, table):
    """(kind, text) findings: `text` findings sit in comments, strings or docs; `code`
    addresses are values the code uses. `table` carries the offset columns of the table the
    previous lines opened."""
    in_text = lambda position: any(a <= position < b for a, b, _ in spans)
    found = []
    for match in ADDRESS.finditer(line):
        if match.group(0) in IMAGE_BASE:
            continue
        if not in_text(match.start()):
            if CONST_DEF.match(line):
                continue
            found.append(('code-address', match.group(0)))
        else:
            found.append(('address', match.group(0)))
    found += [('decompiler', m.group(0)) for m in DECOMPILER.finditer(line)]
    cells = None
    for a, b, kind in spans:
        text = line[a:b]
        found += [('native-name', m.group(0)) for m in NATIVE_METHOD.finditer(text)]
        if kind == 'comment':
            found += [('decompiler', m.group(0)) for m in VIRTUAL_SLOT.finditer(text)]
            cells = cells or table_cells(text)
        found += [('offset', m) for m in offsets(text, kind)]
    # A table whose header names an offset column reports every hexadecimal cell under it.
    if cells is None:
        table.clear()
    elif all(re.fullmatch(r':?-+:?', cell) for cell in cells if cell):
        pass
    elif not table.get('open'):
        table['open'] = True
        table['columns'] = {n for n, cell in enumerate(cells) if OFFSET_HEADER.match(cell)}
    else:
        for n in table['columns']:
            if n < len(cells) and TABLE_HEX.fullmatch(cells[n].strip('`')):
                found.append(('offset', cells[n]))
    return found


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--revision')
    parser.add_argument('--summary', action='store_true', help='print counts per file only')
    parser.add_argument('--code', action='store_true',
                        help='also fail on addresses used as code values outside named constants')
    parser.add_argument('paths', nargs='*', default=DEFAULT_PATHS)
    args = parser.parse_args()
    total = 0
    kinds = {}
    per_file = {}
    for name, lines in read_files(args.revision, list(candidate_files(args.revision, args.paths))):
        table = {}
        for number, (line, spans) in enumerate(
                zip(lines, text_spans(lines, os.path.splitext(name)[1])), 1):
            for kind, text in scan_line(line, spans, table):
                kinds[kind] = kinds.get(kind, 0) + 1
                if kind == 'code-address' and not args.code:
                    continue
                total += 1
                per_file[name] = per_file.get(name, 0) + 1
                if not args.summary:
                    print(f'{name}:{number}: {kind} {text}')
    if args.summary:
        for name, count in sorted(per_file.items()):
            print(f'{count:5} {name}')
    print(f'{total} finding(s); by kind {kinds}', file=sys.stderr)
    return 1 if total else 0


if __name__ == '__main__':
    sys.exit(main())
