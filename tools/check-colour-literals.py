"""Refuse colour literals in the UI outside the theme and palette modules (Python 3.11+).

usage: python tools/check-colour-literals.py [FILE ...]

Every colour the UI draws comes from the user's theme or from the shared data palette, so a
custom theme reaches every widget. A colour literal is a hex colour, a colour function such as
`rgb(`, `rgba(`, `hsl(` or `oklch(`, or a named CSS colour used as a value. It is refused in any
TypeScript file or stylesheet under ui/src, except test files, test fixtures and the modules in
ALLOWED. With no FILE, every such file is read. Comments are never read. Exits 1 on any literal.
"""
import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
UI = "ui/src"
ALLOWED = {
    "ui/src/shell/appearance/themes.ts": "the built-in themes and what is derived from a theme's roles",
    "ui/src/shell/appearance/color.ts": "colour arithmetic",
    "ui/src/shell/appearance/sidebarIcons.ts": "the default sidebar icon colours a user can change",
    "ui/src/shell/appearance/colorPicker.css": "the colour space the colour picker shows",
    "ui/src/charts/palette.ts": "every data colour, read against the active theme's surface",
}
HELP = (
    "take chrome colours from the theme (readFamilyTheme, resolveKitTokens, or a CSS variable such as var(--text)); "
    "put a data colour in ui/src/charts/palette.ts, the one home of data colours, and read it with the theme's surface"
)
NAMED = frozenset("""
aliceblue antiquewhite aqua aquamarine azure beige bisque black blanchedalmond blue blueviolet brown burlywood
cadetblue chartreuse chocolate coral cornflowerblue cornsilk crimson cyan darkblue darkcyan darkgoldenrod darkgray
darkgreen darkgrey darkkhaki darkmagenta darkolivegreen darkorange darkorchid darkred darksalmon darkseagreen
darkslateblue darkslategray darkslategrey darkturquoise darkviolet deeppink deepskyblue dimgray dimgrey dodgerblue
firebrick floralwhite forestgreen fuchsia gainsboro ghostwhite gold goldenrod gray green greenyellow grey honeydew
hotpink indianred indigo ivory khaki lavender lavenderblush lawngreen lemonchiffon lightblue lightcoral lightcyan
lightgoldenrodyellow lightgray lightgreen lightgrey lightpink lightsalmon lightseagreen lightskyblue lightslategray
lightslategrey lightsteelblue lightyellow lime limegreen linen magenta maroon mediumaquamarine mediumblue
mediumorchid mediumpurple mediumseagreen mediumslateblue mediumspringgreen mediumturquoise mediumvioletred
midnightblue mintcream mistyrose moccasin navajowhite navy oldlace olive olivedrab orange orangered orchid
palegoldenrod palegreen paleturquoise palevioletred papayawhip peachpuff peru pink plum powderblue purple
rebeccapurple red rosybrown royalblue saddlebrown salmon sandybrown seagreen seashell sienna silver skyblue
slateblue slategray slategrey snow springgreen steelblue tan teal thistle tomato turquoise violet wheat white
whitesmoke yellow yellowgreen
""".split())
HEX = re.compile(r"(?<![\w&#])#(?:[0-9a-fA-F]{3,4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})(?![\w-])")
FUNCTION = re.compile(r"(?<![\w$.-])(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)\s*\(", re.IGNORECASE)
WORD = re.compile(r"(?<![\w$-])[a-zA-Z]+(?![\w-])")
TS_STYLE_VALUE = re.compile(
    r"(?:[\w$]*(?:colou?r|background|fill|stroke|outline|border|shadow|swatch|tint|ink)[\w$]*\s*(?::|=(?!=))|\baddColorStop\s*\()",
    re.IGNORECASE,
)
STATEMENT_BREAK = re.compile(r"[;,}]")
COMPARED = re.compile(r"[=!]==?\s*$")
COMPARED_AFTER = re.compile(r"\s*[=!]==?")
CLOSED_ATTRIBUTE = re.compile(r"(?<=\s)[\w-]+=(?:\"[^\"\n]*\"|'[^'\n]*')")


def mask_ts(text):
    """The code with comments blanked, and the spans of its string and template literals."""
    out = list(text)
    strings = []
    length = len(text)
    index = 0
    while index < length:
        if text.startswith("//", index):
            end = text.find("\n", index)
            end = length if end < 0 else end
            out[index:end] = " " * (end - index)
            index = end
        elif text.startswith("/*", index):
            end = text.find("*/", index + 2)
            end = length if end < 0 else end + 2
            out[index:end] = [ch if ch == "\n" else " " for ch in text[index:end]]
            index = end
        elif text[index] in "'\"`":
            quote, end = text[index], index + 1
            while end < length and text[end] != quote and not (quote != "`" and text[end] == "\n"):
                end += 2 if text[end] == "\\" else 1
            strings.append((index + 1, min(end, length)))
            index = end + 1
        else:
            index += 1
    return "".join(out), strings


def mask_css(text):
    """The stylesheet with comments and quoted strings blanked."""
    text = re.sub(r"/\*.*?\*/", lambda m: re.sub(r"[^\n]", " ", m.group()), text, flags=re.DOTALL)
    return re.sub(r"(['\"])(?:\\.|(?!\1).)*\1", lambda m: " " * len(m.group()), text)


def line_of(text, offset):
    return text.count("\n", 0, offset) + 1


def statement_before(code, end):
    before = code[max(0, end - 200):end]
    breaks = sorted([m.end() for m in STATEMENT_BREAK.finditer(before)] + [m.end() for m in CLOSED_ATTRIBUTE.finditer(before)])
    stop = re.search(r"addColorStop\s*\([^()]*$", before)
    return before[stop.start():] if stop else before[breaks[-1]:] if breaks else before


def ts_literals(text):
    code, strings = mask_ts(text)
    found = []
    for start, end in strings:
        found += [m.start() for m in HEX.finditer(code, start, end)]
        found += [m.start() for m in FUNCTION.finditer(code, start, end)]
        content = code[start:end].strip().lower()
        compared = COMPARED.search(code[max(0, start - 80):start - 1]) or COMPARED_AFTER.match(code, end + 1)
        if content in NAMED and not compared and TS_STYLE_VALUE.search(statement_before(code, start - 1)):
            found.append(start)
    return sorted({line_of(text, offset) for offset in found})


def css_values(code):
    depth = 0
    index = 0
    length = len(code)
    while index < length:
        ch = code[index]
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth = max(0, depth - 1)
        elif ch == ":" and depth > 0:
            start = index + 1
            end = start
            nested = 0
            while end < length and not (nested == 0 and code[end] in ";}"):
                if code[end] == "(":
                    nested += 1
                elif code[end] == ")":
                    nested = max(0, nested - 1)
                elif code[end] == "{":
                    break
                end += 1
            if end < length and code[end] == "{":
                index = end
                continue
            yield start, end
            index = end
            continue
        index += 1


def css_literals(text):
    code = mask_css(text)
    found = []
    for start, end in css_values(code):
        found += [m.start() for m in HEX.finditer(code, start, end)]
        found += [m.start() for m in FUNCTION.finditer(code, start, end)]
        found += [m.start() for m in WORD.finditer(code, start, end) if m.group().lower() in NAMED]
    return sorted({line_of(text, offset) for offset in found})


def checked(path):
    name = path.as_posix()
    if not name.startswith(UI + "/") or name in ALLOWED:
        return False
    if re.search(r"\.(?:test|fixtures?)\.tsx?$", name) or "/test-support/" in name:
        return False
    return path.suffix in (".ts", ".tsx", ".css")


def literals(name, text):
    return css_literals(text) if name.endswith(".css") else ts_literals(text)


def main():
    parser = argparse.ArgumentParser(description="Refuse colour literals in the UI outside the theme and palette modules.")
    parser.add_argument("files", nargs="*")
    names = parser.parse_args().files
    paths = [Path(name) for name in names] if names else sorted(
        path.relative_to(ROOT) for path in (ROOT / UI).rglob("*") if path.is_file() and "node_modules" not in path.parts
    )
    failures = []
    for path in paths:
        if not checked(path) or not (ROOT / path).is_file():
            continue
        text = (ROOT / path).read_text(encoding="utf-8")
        failures += [f"{path.as_posix()}:{line}" for line in literals(path.as_posix(), text)]
    for failure in failures:
        print(f"{failure}: colour literal: {HELP}")
    if failures:
        print(f"{len(failures)} colour literal(s). Allowed only in: " + "; ".join(f"{name} ({why})" for name, why in ALLOWED.items()))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
