"""Report executable addresses and decompiler names in comments, strings and docs.

usage: python tools/check-citations.py [--summary] [--code] [--revision REV] [paths...]

Default paths: crates, tools, docs, ui/src, fuzz and the top-level Markdown files.
An executable address (`0x14xxxxxxx`, a bare `14xxxxxxx`, `Name@14xxxxxxx`) in a
comment, string literal or doc is reported. One used as a code value is counted
(`code-address`) and reported only with `--code`, unless it is the value of a
constant (`const NAME: T = 0x...;`) or the image base. Decompiler names (`FUN_`,
`DAT_`, `LAB_`, `PTR_`, `sub_`, `vtable_`) are always reported, and native
`CClass::Method` names when they appear in text. Abbreviated addresses are not
recognised. Exits 1 when anything is reported.
"""
import argparse
import os
import re
import subprocess
import sys

ADDRESS = re.compile(r'(?<![0-9A-Za-z_])(?:0x)?14[0-9A-Fa-f]{7}(?![0-9A-Za-z_])')
CONST_DEF = re.compile(r'^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const|static)\s+[A-Z][A-Z0-9_]*\s*:')
DECOMPILER = re.compile(r'\b(?:FUN|DAT|LAB|PTR|vtable)_[0-9A-Fa-f]{6,}\b|\bsub_14[0-9A-Fa-f]{7}\b')
NATIVE_METHOD = re.compile(r'\bC[A-Z][A-Za-z0-9]+::[A-Z~][A-Za-z0-9_]*')
IMAGE_BASE = {'0x140000000', '140000000'}
EXTENSIONS = {'.rs', '.c', '.h', '.cpp', '.md', '.py', '.ts', '.tsx', '.mjs', '.js',
              '.ps1', '.toml', '.json', '.wgsl', '.hlsl', '.txt', '.tsv'}
DEFAULT_PATHS = ['crates', 'tools', 'docs', 'ui/src', 'fuzz']
SELF = 'tools/check-citations.py'


PREFILTER = r'14[0-9A-Fa-f]{7}|(FUN|DAT|LAB|PTR|vtable)_[0-9A-Fa-f]|sub_14|C[A-Z][A-Za-z0-9]+::'


def candidate_lines(revision, paths):
    """(file, line number, text) for every line the cheap prefilter matches."""
    command = ['git', 'grep', '-n', '-I', '-E', PREFILTER]
    if revision:
        command.append(revision)
    top_level = [':(glob)*.md'] if paths == DEFAULT_PATHS else []
    command += ['--', *paths, *top_level, f':!{SELF}']
    out = subprocess.run(command, capture_output=True).stdout.decode('utf-8', 'replace')
    for entry in out.splitlines():
        if revision:
            entry = entry[len(revision) + 1:]
        name, number, text = entry.split(':', 2)
        if os.path.splitext(name)[1] in EXTENSIONS:
            yield name, int(number), text


CODE_EXTENSIONS = {'.rs', '.c', '.h', '.cpp', '.ts', '.tsx', '.mjs', '.js', '.wgsl', '.hlsl'}


def text_spans(line, extension):
    """The parts of a code line that are comments or string literals."""
    if extension not in CODE_EXTENSIONS:
        return [(0, len(line))]
    spans, quote, start, index = [], None, 0, 0
    while index < len(line):
        char = line[index]
        if quote:
            if char == '\\':
                index += 1
            elif char == quote:
                spans.append((start, index + 1))
                quote = None
        elif line.startswith('//', index) or line.startswith('/*', index) or (
                line.lstrip().startswith('*') and index == len(line) - len(line.lstrip())):
            spans.append((index, len(line)))
            break
        elif char == '"' or (char == "'" and extension != '.rs'):
            quote, start = char, index
        index += 1
    if quote:
        spans.append((start, len(line)))
    return spans


def scan_line(line, extension):
    """(kind, text) findings: `text` addresses sit in comments, strings or docs;
    `code` addresses are values the code uses."""
    spans = text_spans(line, extension)
    in_text = lambda position: any(a <= position < b for a, b in spans)
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
    found += [('native-name', m.group(0)) for m in NATIVE_METHOD.finditer(line)
              if in_text(m.start())]
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
    for name, number, line in candidate_lines(args.revision, args.paths):
        for kind, text in scan_line(line, os.path.splitext(name)[1]):
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
