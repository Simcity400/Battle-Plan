"""Tests for tools/check-colour-literals.py: python tools/check_colour_literals_test.py"""

import importlib.util
import os
import unittest
from pathlib import Path

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SPEC = importlib.util.spec_from_file_location("check_colour_literals", os.path.join(ROOT, "tools", "check-colour-literals.py"))
cc = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(cc)


class TypeScript(unittest.TestCase):
    def test_hex_and_colour_functions_in_strings_are_refused_at_their_lines(self):
        text = "ctx.fillStyle = '#e5484d';\nconst g = `rgba(${r}, 0, 0, 0.5)`;\nconst s = 'hsl(200, 50%, 50%)';\nconst o = \"oklch(0.7 0.1 80)\";\n"
        self.assertEqual(cc.literals("ui/src/families/x.ts", text), [1, 2, 3, 4])

    def test_a_dark_or_light_pair_and_a_fallback_are_both_refused(self):
        text = "const ink = dark ? '#d4d4d4' : '#64748b';\nconst surface = theme?.surface ?? '#111111';\n"
        self.assertEqual(cc.literals("ui/src/draw/kit.ts", text), [1, 2])

    def test_named_colours_are_refused_where_they_are_used_as_a_colour(self):
        text = "ctx.fillStyle = 'white';\ngrad.addColorStop(1, 'black');\nconst swatch = 'red';\n"
        self.assertEqual(cc.literals("ui/src/families/x.ts", text), [1, 2, 3])

    def test_words_that_are_not_colours_and_names_of_tones_pass(self):
        text = "type Tone = 'red' | 'grey';\nconst tone: Tone = 'red';\nconst kind = 'tan';\nctx.fillStyle = theme.red;\n"
        self.assertEqual(cc.literals("ui/src/families/x.ts", text), [])

    def test_comments_selectors_and_functions_named_like_colours_pass(self):
        text = (
            "// '#e5484d' was the old red\n/* rgba(0, 0, 0, 1) */\n"
            "const el = root.querySelector('#root');\nconst c = color(part.id);\nconst r = rgbaColor(theme.text, 0.2);\n"
        )
        self.assertEqual(cc.literals("ui/src/widgets/x/X.tsx", text), [])


class Stylesheets(unittest.TestCase):
    def test_hex_functions_and_named_colours_in_values_are_refused(self):
        text = ".a {\n  color: #e2e2e2;\n  box-shadow: 0 1px 2px rgb(0 0 0 / 5%);\n  background: color-mix(in srgb, var(--tone), white 14%);\n  fill: oklch(0.7 0.1 80);\n}\n"
        self.assertEqual(cc.literals("ui/src/x.css", text), [2, 3, 4, 5])

    def test_custom_properties_are_values_too(self):
        self.assertEqual(cc.literals("ui/src/x.css", ":root {\n  --edge: 0 0 #0000;\n  --ok: var(--text);\n}\n"), [2])

    def test_selectors_keywords_comments_and_quoted_text_pass(self):
        text = (
            "#abc {\n  color: var(--text);\n}\n/* color: #fff */\n.b { white-space: nowrap; background: transparent; border-color: currentColor; }\n"
            ".c { font-family: 'Black Ops'; transition: color var(--t); }\n@media (prefers-color-scheme: dark) { .d { color: var(--muted); } }\n"
        )
        self.assertEqual(cc.literals("ui/src/x.css", text), [])


class Scope(unittest.TestCase):
    def test_the_theme_and_palette_modules_tests_and_fixtures_are_not_checked(self):
        for name in [*cc.ALLOWED, "ui/src/families/x.test.ts", "ui/src/families/theme.fixture.ts", "ui/src/test-support/a.ts", "tools/x.ts"]:
            self.assertFalse(cc.checked(Path(name)), name)
        for name in ["ui/src/draw/kit.ts", "ui/src/widgets/x/X.tsx", "ui/src/styles.css"]:
            self.assertTrue(cc.checked(Path(name)), name)

    def test_the_message_names_the_palette_as_the_home_of_data_colours(self):
        self.assertIn("ui/src/charts/palette.ts", cc.HELP)
        self.assertIn("ui/src/charts/palette.ts", cc.ALLOWED)


if __name__ == "__main__":
    unittest.main()
