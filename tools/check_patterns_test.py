"""Tests for tools/check-patterns.py: python tools/check_patterns_test.py"""

import contextlib
import importlib.util
import io
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SPEC = importlib.util.spec_from_file_location("check_patterns", os.path.join(ROOT, "tools", "check-patterns.py"))
cp = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(cp)


class Repository:
    """A Git repository of its own, standing in for the checkout while a test runs."""

    def __init__(self, files):
        self.folder = tempfile.TemporaryDirectory()
        self.root = Path(self.folder.name)
        self.git("init", "-q", "-b", "main")
        self.write(files)
        self.git("add", "-A")
        self.git("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "base", "--allow-empty")
        self.saved, cp.ROOT = cp.ROOT, self.root

    def git(self, *args):
        subprocess.run(["git", *args], cwd=self.root, check=True, capture_output=True)

    def write(self, files):
        for name, text in files.items():
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")

    def close(self):
        cp.ROOT = self.saved
        self.folder.cleanup()


def rust(files, name):
    repository = Repository(files)
    try:
        return cp.matches(name, cp.Revision(None))
    finally:
        repository.close()


def check(base, change, *, commit=False):
    repository = Repository(base)
    try:
        repository.git("checkout", "-q", "-b", "change")
        repository.write(change)
        head = None
        if commit:
            repository.git("add", "-A")
            repository.git("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "change")
            head = "HEAD"
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = cp.against("main", head)
        return code, out.getvalue()
    finally:
        repository.close()


LIB = "crates/a/src/lib.rs"


class Mask(unittest.TestCase):
    def test_comments_and_literal_contents_are_blanked_with_length_and_lines_kept(self):
        text = 'let a = "Box::leak(x)"; // Box::leak\n/* outer /* inner */ Box::leak */ let b = r#"q"#; let c = \'"\';\n'
        code = cp.mask(text)
        self.assertEqual(len(code), len(text))
        self.assertEqual(code.count("\n"), text.count("\n"))
        self.assertNotIn("leak", code)
        self.assertIn("let c", code)

    def test_a_lifetime_is_code(self):
        self.assertEqual(cp.mask("fn f<'a>(x: &'a str) {}"), "fn f<'a>(x: &'a str) {}")


class Cfg(unittest.TestCase):
    def test_only_a_predicate_false_outside_tests_is_test_only(self):
        self.assertIs(cp.cfg_value("test"), False)
        self.assertIs(cp.cfg_value('any(test, feature = "test-support")'), False)
        self.assertIs(cp.cfg_value("all(test, windows)"), False)
        self.assertIsNone(cp.cfg_value('any(test, feature = "export-types")'))
        self.assertIs(cp.cfg_value("not(test)"), True)
        self.assertIsNone(cp.cfg_value("windows"))


class TestCode(unittest.TestCase):
    def test_the_clock_counts_in_test_items_and_test_modules_only(self):
        files = {
            LIB: "fn live() { let _ = std::time::Instant::now(); }\n#[cfg(test)]\nmod tests;\n#[cfg(test)]\nmod inline {\n    fn f() { std::thread::sleep(D); }\n}\n#[test]\nfn t() { let _ = Instant::now(); }\n",
            "crates/a/src/tests.rs": "fn f() { std::thread::sleep(D); }\n",
        }
        self.assertEqual(rust(files, LIB)["test-clock"], [6, 9])
        self.assertEqual(rust(files, "crates/a/src/tests.rs")["test-clock"], [1])

    def test_a_module_is_test_code_through_a_path_attribute_an_inline_parent_or_a_test_parent(self):
        files = {
            LIB: '#[cfg(test)]\n#[path = "x/named_elsewhere.rs"]\nmod checks;\nmod live;\nmod outer {\n    #[cfg(test)]\n    mod live;\n}\n#[cfg(test)]\nmod kit;\n',
            "crates/a/src/x/named_elsewhere.rs": "fn f() { Box::leak(b); }\n",
            "crates/a/src/live.rs": "fn f() { Box::leak(b); }\n",
            "crates/a/src/outer/live.rs": "fn f() { Box::leak(b); }\n",
            "crates/a/src/kit.rs": "mod deep;\n",
            "crates/a/src/kit/deep.rs": "fn f() { Box::leak(b); }\n",
            "crates/a/tests/it.rs": "fn f() { Instant::now(); }\n",
        }
        leaks = {name: rust(files, name)["box-leak"] for name in files if name.endswith(".rs") and name != LIB and "/tests/" not in name and "kit.rs" not in name}
        self.assertEqual(leaks, {"crates/a/src/x/named_elsewhere.rs": [], "crates/a/src/live.rs": [1], "crates/a/src/outer/live.rs": [], "crates/a/src/kit/deep.rs": []})
        self.assertEqual(rust(files, "crates/a/tests/it.rs")["test-clock"], [1])

    def test_an_inner_test_cfg_covers_its_module_and_not_the_file(self):
        text = "mod tests {\n    #![cfg(test)]\n    fn f() { Box::leak(a); }\n}\nfn live() { Box::leak(b); }\n"
        self.assertEqual(rust({LIB: text}, LIB)["box-leak"], [5])
        files = {LIB: '#![cfg(feature = "test-support")]\nmod kit;\n', "crates/a/src/kit.rs": "fn f() { Box::leak(b); }\n"}
        self.assertEqual(rust(files, "crates/a/src/kit.rs")["box-leak"], [])

    def test_a_path_inside_an_inline_module_resolves_under_it(self):
        files = {
            LIB: 'mod outer {\n    #[cfg(test)]\n    #[path = "fixture.rs"]\n    mod cases;\n}\n',
            "crates/a/src/outer/fixture.rs": "fn f() { std::thread::sleep(D); }\n",
        }
        self.assertEqual(rust(files, "crates/a/src/outer/fixture.rs")["test-clock"], [1])

    def test_a_file_production_also_declares_is_not_test_code(self):
        files = {LIB: 'mod live;\n#[cfg(test)]\n#[path = "live.rs"]\nmod alias;\n', "crates/a/src/live.rs": "fn f() { Box::leak(b); }\n"}
        self.assertEqual(rust(files, "crates/a/src/live.rs")["box-leak"], [1])

    def test_a_file_path_loads_owns_its_directory(self):
        files = {
            LIB: '#[cfg(test)]\n#[path = "fixtures/suite.rs"]\nmod suite;\n',
            "crates/a/src/fixtures/suite.rs": "mod child;\n",
            "crates/a/src/fixtures/child.rs": "fn f() { std::thread::sleep(D); }\n",
        }
        self.assertEqual(rust(files, "crates/a/src/fixtures/child.rs")["test-clock"], [1])

    def test_a_path_quoted_in_a_comment_is_not_read(self):
        files = {LIB: '// #[path = "wrong.rs"]\n#[cfg(test)]\nmod child;\n', "crates/a/src/child.rs": "fn f() { std::thread::sleep(D); }\n"}
        self.assertEqual(rust(files, "crates/a/src/child.rs")["test-clock"], [1])

    def test_a_test_target_on_the_library_root_leaves_it_production_code(self):
        files = {"crates/a/Cargo.toml": '[package]\nname = "a"\n[[test]]\nname = "t"\npath = "src/lib.rs"\n', LIB: "fn f() { Box::leak(b); }\n"}
        self.assertEqual(rust(files, LIB)["box-leak"], [1])


    def test_js_and_powershell_tests_count_their_own_waits_and_other_files_are_not_read(self):
        files = {"ui/a.test.ts": "await new Promise(r => setTimeout(r, 5));\nsetInterval(f, 1);\n", "ui/a.ts": "setTimeout(f);\n", "tools/x.tests.ps1": "Start-Sleep -Milliseconds 5\n"}
        self.assertEqual(rust(files, "ui/a.test.ts"), {"test-clock": [1, 2]})
        self.assertEqual(rust(files, "ui/a.ts"), {})
        self.assertEqual(rust(files, "tools/x.tests.ps1"), {"test-clock": [1]})


class DebugHash(unittest.TestCase):
    def test_debug_output_fed_to_a_hash_or_digest_is_found(self):
        text = (
            'fn a(s: &mut H) { format!("{:?}", self.0).hash(s); }\n'
            'fn b() -> [u8; 32] { Sha256::digest(format!("{self:?}").as_bytes()).into() }\n'
            'fn c() { field(&mut hash, format!("{:#?}", x).as_bytes()); }\n'
            'fn d() { write_fmt(&mut hasher, format_args!("{value:?}")); }\n'
            'fn e(s: &mut H) { format!{"{:?}", self.0}.hash(s); }\n'
            'fn f() -> H { blake3::hash(format!("{x:?}").as_bytes()) }\n'
            'fn g(h: &mut H) { format!{"{:?}", Foo { x }}.hash(h); }\n'
            'fn h() -> H { blake3::hash(r.iter().map(|r| { format!("{r:?}") }).collect::<String>().as_bytes()) }\n'
            'fn i() -> Identity { Identity {\n    exe: digest(format!("{:?}", a).as_bytes()),\n    game: digest(format!("{:?}", b).as_bytes()),\n} }\n'
        )
        self.assertEqual(rust({LIB: text}, LIB)["debug-hash"], [1, 2, 3, 4, 5, 6, 7, 8, 10, 11])

    def test_debug_output_in_a_hashed_tuple_or_a_later_chained_update_is_found(self):
        text = (
            'fn a(h: &mut H) { ("weather", format!("{identity:?}")).hash(h); }\n'
            'fn b(hasher: &mut H) { hasher.update(&[1]).update(&key).update(format!("{encoded:?}").as_bytes()); }\n'
            'fn c() -> K { Sha256::new().chain_update(b"facts").chain_update(format!("{raw:?}").as_bytes()).finalize() }\n'
            'fn d() -> H { blake3::hash(name.to_str().map(str::to_owned).unwrap_or_else(|| format!("bytes:{:?}", name)).as_bytes()) }\n'
        )
        self.assertEqual(rust({LIB: text}, LIB)["debug-hash"], [1, 2, 3, 4])

    def test_display_output_messages_and_tests_are_not(self):
        text = (
            'fn a(s: &mut H) { format!("{}", self.0).hash(s); }\n'
            'fn b() { let e = format!("{x:?} {{:?}}"); }\n'
            'fn c() { match x { A => format!("{x:?}"), B => d.hash(s) } }\n'
            'fn shown(digest: u64) -> String { format!("{:?}", digest) }\n'
            'fn told(h: &mut H) { eprintln!{ "{}", format!("{x:?}") } x.hash(h); }\n'
            'fn key(asset: &str) -> String { format!("{asset}#{:?}", blake3::hash(pixels)) }\n'
            'fn digest(&self) -> Digest { eprint!("{}", format_args!("computing {self:?}")); make() }\n'
            'fn exe(p: &Path) -> R { hash_executable(File::open(p).map_err(|e| { format!("open {p:?}: {e}") })?) }\n'
            '#[cfg(test)]\nmod tests { fn d() { format!("{:?}", x).hash(s); } }\n'
        )
        self.assertEqual(rust({LIB: text}, LIB)["debug-hash"], [])


class TestSupportBranch(unittest.TestCase):
    def test_runtime_checks_and_negated_attributes_are_ratchet_instances(self):
        runtime = 'fn runtime() -> bool { cfg!(any(test, feature = "test-support")) }\n'
        production = '#[cfg(not(any(test, feature = "test-support")))]\nfn production() {}\n'
        code, out = check({}, {LIB: runtime + production})
        self.assertEqual(code, 1)
        self.assertIn(f"{LIB}: test-support-branch: 0 at the base, 2 now; new at line 1, 2", out)
        self.assertEqual(check({LIB: runtime + production}, {LIB: runtime + production + "fn added() {}\n"})[0], 0)

    def test_multiline_negations_and_other_macro_delimiters_count(self):
        text = (
            'fn live() -> bool { cfg!{feature = r#"test-support"#} }\n'
            '#[cfg(all(unix, not(\n    feature /* comment */ = "test-support",\n)))]\n'
            'fn production() {}\n'
        )
        self.assertEqual(rust({LIB: text}, LIB)["test-support-branch"], [1, 2])

    def test_a_fixture_match_arm_does_not_exempt_the_following_production_arm(self):
        text = (
            'fn live() -> bool {\n'
            '    match 1 {\n'
            '        #[cfg(feature = "test-support")]\n'
            '        1 => false,\n'
            '        #[cfg(not(feature = "test-support"))]\n'
            '        1 => true,\n'
            '        _ => false,\n'
            '    }\n'
            '}\n'
        )
        self.assertEqual(rust({LIB: text}, LIB)["test-support-branch"], [5])

    def test_fixture_items_and_unrelated_predicates_pass(self):
        gate = '#[cfg(any(test, feature = "test-support"))] '
        text = (
            f'{gate}fn fixture() {{}}\n'
            f'{gate}impl State {{}}\n'
            f'{gate}mod fixture {{}}\n'
            f'{gate}use fixture::build;\n'
            f'{gate}struct Fixture {{}}\n'
            f'struct State {{ {gate}counter: u8 }}\n'
            f'fn live() {{ {gate}fn local_fixture() {{}} }}\n'
            '#[cfg(all(not(windows), feature = "test-support"))] fn on_unix() {}\n'
            '#[cfg(not(target_os = "test-support"))] fn other_key() {}\n'
            'fn other_feature() -> bool { cfg!(feature = "test-support-extra") }\n'
            'fn comment() -> bool { cfg!(test /* feature = "test-support" */) }\n'
            '// cfg!(feature = "test-support")\n'
            'const EXAMPLE: &str = r#"#[cfg(not(feature = "test-support"))]"#;\n'
        )
        self.assertEqual(check({}, {LIB: text})[0], 0)

    def test_test_files_and_inline_test_modules_pass(self):
        text = ('fn fixture() -> bool { cfg!(feature = "test-support") }\n'
                '#[cfg(not(feature = "test-support"))] fn only_without_feature() {}\n')
        self.assertEqual(check({}, {
            LIB: ('#[cfg(test)] mod inline {\n' + text + '}\n'
                  '#[cfg(test)] mod checks;\n#[cfg(test)] mod unit_tests;\n'
                  '#[cfg(test)] mod test_support;\n#[cfg(test)] mod testing;\n'),
            'crates/a/src/checks.rs': text,
            'crates/a/src/unit_tests.rs': text,
            'crates/a/tests/support/fixture.rs': text,
            'crates/a/src/test_support.rs': 'mod builder;\n' + text,
            'crates/a/src/test_support/builder.rs': text,
            'crates/a/src/testing.rs': text,
            'tools/example.rs': text,
        })[0], 0)

    def test_a_production_module_named_contests_is_checked(self):
        name = "crates/a/src/contests.rs"
        code, out = check({LIB: "pub mod contests;\n"}, {
            name: 'pub fn live() -> bool { cfg!(feature = "test-support") }\n',
        })
        self.assertEqual(code, 1)
        self.assertIn(f"{name}: test-support-branch: 0 at the base, 1 now; new at line 1", out)

    def test_test_only_item_bodies_are_exempt(self):
        feature = 'cfg!(feature = "test-support")'
        text = (
            f'#[cfg(test)] impl<const N: usize> State<{{ N }}> {{ fn test() -> bool {{ {feature} }} }}\n'
            f'#[cfg(test)] pub(crate) trait Fixture {{ const ENABLED: bool = {feature}; }}\n'
            f'#[cfg(test)] enum Choice {{ First = {feature} as isize }}\n'
            f'#[cfg(test)] struct Sample {{ bytes: [u8; {feature} as usize] }}\n'
            f'fn live() -> bool {{ {feature} }}\n'
        )
        self.assertEqual(rust({LIB: text}, LIB)["test-support-branch"], [5])

    def test_any_test_only_attribute_exempts_the_whole_group(self):
        text = (
            '#![cfg(not(feature = "test-support"))]\n'
            '#[cfg(not(feature = "test-support"))]\n'
            '#[doc = "Fixture."]\n'
            '#[cfg(any(test, feature = "test-support"))]\n'
            'fn first() {}\n'
            '#[cfg(test)]\n'
            '#[cfg(not(feature = "test-support"))]\n'
            'fn second() {}\n'
            '#[cfg(not(feature = "test-support"))]\n'
            'fn live() {}\n'
            '#[cfg(not(feature = "test-support"))]\n'
            '#[cfg(test)]\n'
            'const FIXTURE: bool = true;\n'
        )
        self.assertEqual(rust({LIB: text}, LIB)["test-support-branch"], [1, 9])

    def test_comments_in_test_predicates_leave_inline_and_external_modules_exempt(self):
        feature = 'fn example() -> bool { cfg!(feature = "test-support") }\n'
        gate = '#[cfg(any(test /* units */, feature /* fixtures */ = "test-support"))] '
        name = 'crates/a/src/external.rs'
        base = {LIB: gate + 'mod inline {\n' + feature + '}\n' + gate + 'mod external;\n', name: feature}
        self.assertEqual(check({}, base)[0], 0)
        code, out = check(base, {LIB: gate + 'mod inline {\n' + feature + '}\nmod external;\n'})
        self.assertEqual(code, 1)
        self.assertIn(f"{name}: test-support-branch: 0 at the base, 1 now; new at line 1", out)

    def test_stringify_arguments_are_exempt_and_do_not_hide_live_gates(self):
        text = (
            '#[doc = stringify!(cfg!(feature = "test-support"))] fn example() {}\n'
            'const EXAMPLE: &str = stringify! {\n'
            '    #[cfg(not(feature = "test-support"))] fn only_without_feature() {}\n'
            '    #[cfg(test)] mod tests;\n'
            '    #![cfg(test)]\n'
            '};\n'
            'const FEATURE: &str = core::stringify![cfg!(feature = "test-support")];\n'
            'fn live() -> bool { cfg!(feature = "test-support") }\n'
        )
        self.assertEqual(rust({LIB: text}, LIB)["test-support-branch"], [8])


class Against(unittest.TestCase):
    def test_existing_instances_pass_and_a_new_one_fails_at_its_line(self):
        base = {LIB: "fn a() { Box::leak(x); }\n"}
        self.assertEqual(check(base, {LIB: "fn a() { Box::leak(x); }\nfn b() {}\n"})[0], 0)
        code, out = check(base, {LIB: "fn a() { Box::leak(x); }\nfn b() { Box::leak(y); }\n"})
        self.assertEqual(code, 1)
        self.assertIn(f"{LIB}: box-leak: 1 at the base, 2 now; new at line 2", out)

    def test_an_untracked_file_and_a_committed_head_are_both_checked(self):
        self.assertEqual(check({}, {"crates/a/src/new.rs": "fn b() { Box::leak(y); }\n"})[0], 1)
        self.assertEqual(check({LIB: ""}, {LIB: "fn b() { Box::leak(y); }\n"}, commit=True)[0], 1)

    def test_dropping_a_test_cfg_from_a_mod_declaration_checks_the_unchanged_child(self):
        base = {LIB: "#[cfg(test)]\nmod fixture;\n", "crates/a/src/fixture.rs": "fn f() { Box::leak(x); }\n"}
        code, out = check(base, {LIB: "mod fixture;\n"})
        self.assertEqual(code, 1)
        self.assertIn("crates/a/src/fixture.rs: box-leak: 0 at the base, 1 now", out)

    def test_declaring_an_unchanged_orphan_under_tests_checks_it(self):
        base = {LIB: "", "crates/a/src/orphan.rs": "fn f() { std::thread::sleep(D); }\n"}
        code, out = check(base, {LIB: "#[cfg(test)]\nmod orphan;\n"})
        self.assertEqual(code, 1)
        self.assertIn("crates/a/src/orphan.rs: test-clock", out)

    def test_a_manifest_target_that_makes_test_code_production_is_checked(self):
        manifest = '[package]\nname = "a"\n'
        base = {"crates/a/Cargo.toml": manifest, LIB: "#[cfg(test)]\nmod kit;\n", "crates/a/src/kit.rs": "fn f() { Box::leak(x); }\n"}
        code, out = check(base, {"crates/a/Cargo.toml": manifest + '[[bin]]\nname = "k"\npath = "src/kit.rs"\n'})
        self.assertEqual(code, 1)
        self.assertIn("crates/a/src/kit.rs: box-leak", out)

    def test_a_file_renamed_from_an_unread_tree_brings_no_allowance(self):
        repository = Repository({"vendor/x/fixture.rs": "fn f() { Box::leak(x); }\n" + "fn pad() {}\n" * 20})
        try:
            repository.git("checkout", "-q", "-b", "change")
            (repository.root / "crates/a/src").mkdir(parents=True)
            repository.git("mv", "vendor/x/fixture.rs", LIB)
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(cp.against("main", None), 1)
        finally:
            repository.close()

    def test_a_renamed_file_keeps_its_count(self):
        base = {LIB: "fn a() { Box::leak(x); }\n" + "fn pad() {}\n" * 20}
        repository = Repository(base)
        try:
            repository.git("mv", LIB, "crates/a/src/main.rs")
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                self.assertEqual(cp.against("main", None), 0)
        finally:
            repository.close()


class FamilyKitPatterns(unittest.TestCase):
    def test_local_contrast_ink_is_refused_in_families_and_projections(self):
        code, out = check({}, {
            "ui/src/families/bars/painter.ts": "function inkOnFill(fill: string) { return fill; }\nconst x = readableText(rgb);\n",
        })
        self.assertEqual(code, 1)
        self.assertIn("local-contrast-ink", out)
        self.assertIn("contrastInkForFill", out)
        self.assertEqual(check({}, {
            "ui/src/draw/kit.ts": "export function contrastInkForFill(fill: string) { return fill; }\n",
            "ui/src/families/bars/barsFamily.test.ts": "function inkOnFill() {}\n",
        })[0], 0)

    def test_local_tick_engine_is_refused_in_families_and_projections(self):
        code, out = check({}, {
            "ui/src/families/heat/lanesLayout.ts": "export function computeAxisTicks() { return []; }\n",
            "ui/src/families/time-chart/layout.ts": "export function niceChartTicks() { return []; }\n",
        })
        self.assertEqual(code, 1)
        self.assertIn("local-tick-engine", out)
        self.assertIn("computeTimeAxisTicks", out)
        self.assertEqual(check({}, {
            "ui/src/draw/kit.ts": "export function computeTimeAxisTicks() { return []; }\n",
        })[0], 0)

    def test_family_colour_literals_are_refused_in_families_and_projections(self):
        code, out = check({}, {
            "ui/src/families/table/cellPainters.ts": "const c = '#e5484d';\nconst g = `rgba(${r}, 0, 0, 0.5)`;\n",
            "ui/src/widgets/foo/fooProjection.ts": "const s = 'hsl(200, 50%, 50%)';\n",
        })
        self.assertEqual(code, 1)
        self.assertIn("family-colour-literal", out)
        self.assertIn("ui/src/charts/palette.ts", out)
        self.assertEqual(check({}, {
            "ui/src/charts/palette.ts": "export const RED = '#e5484d';\n",
            "ui/src/shell/appearance/color.ts": "export const f = () => `rgba(0, 0, 0, 1)`;\n",
            "ui/src/draw/kit.ts": "const c = '#111111';\n",
            "ui/src/families/table/tableFamily.test.ts": "const c = '#e5484d';\n",
        })[0], 0)

    def test_family_locale_string_is_refused_in_families_and_projections(self):
        code, out = check({}, {
            "ui/src/families/table/cellPainters.ts": "const s = count.toLocaleString('en-US');\nconst d = x.toFixed(1);\n",
            "ui/src/widgets/foo/fooProjection.ts": "import { usNumber } from '../../charts/numberFormat.ts';\nconst f = new Intl.NumberFormat('en-US');\n",
        })
        self.assertEqual(code, 1)
        self.assertIn("family-locale-string", out)
        self.assertIn("ui/src/families/valueFormat.ts", out)
        self.assertEqual(check({}, {
            "ui/src/families/valueFormat.ts": "const s = count.toLocaleString('en-US');\n",
            "ui/src/families/table/tableFamily.test.ts": "const s = count.toLocaleString('en-US');\n",
        })[0], 0)

    def test_family_measure_text_loop_is_refused_in_families_and_projections(self):
        code, out = check({}, {
            "ui/src/families/heat/matrixLaneView.ts": "for (const row of rows) { w = Math.max(w, ctx.measureText(row.label).width); }\n",
            "ui/src/families/time-chart/painter.ts": "items.map(item => ctx.measureText(item).width);\n",
        })
        self.assertEqual(code, 1)
        self.assertIn("family-measure-text-loop", out)
        self.assertIn("ui/src/charts/textWidth.ts", out)
        self.assertEqual(check({}, {
            "ui/src/families/table/painter.ts": "const measure = (text: string) => ctx.measureText(text).width;\n",
            "ui/src/charts/labels.ts": "for (const t of texts) { w = ctx.measureText(t).width; }\n",
            "ui/src/families/heat/heatFamily.test.ts": "for (const t of texts) { w = ctx.measureText(t).width; }\n",
        })[0], 0)

    def test_family_hand_percent_is_refused_in_families_and_projections(self):
        code, out = check({}, {
            "ui/src/families/bars/painter.ts": "const pct = `${Math.round(share * 100)}%`;\n",
            "ui/src/widgets/foo/fooProjection.ts": "const a = `${val.toFixed(1)}%`;\nconst b = Math.floor(share * 100) + '%';\n",
        })
        self.assertEqual(code, 1)
        self.assertIn("family-hand-percent", out)
        self.assertIn("formatShare", out)
        self.assertIn("formatProgress", out)
        self.assertIn("formatRecordedPercent", out)
        self.assertEqual(check({}, {
            "ui/src/families/valueFormat.ts": "const pct = `${Math.round(share * 100)}%`;\n",
            "ui/src/families/heat/matrixModel.ts": "if (Math.floor(at / width) === at % width) continue;\n",
            "ui/src/families/bars/barsFamily.test.ts": "const pct = `${Math.round(share * 100)}%`;\n",
        })[0], 0)

    def test_family_at_least_wording_is_refused_in_families_and_widgets(self):
        code, out = check({}, {
            "ui/src/families/valueFormat.ts": "const lead = value.atLeast ? `at least ${exact}` : exact;\n",
            "ui/src/widgets/foo/fooData.ts": "const note = '+ means this many';\nconst pre = '\\u2265';\n",
            "ui/src/widgets/foo/FooWidget.tsx": "export const x = () => <span>≥{count}</span>;\n",
        })
        self.assertEqual(code, 1)
        self.assertIn("family-at-least-wording", out)
        self.assertIn("ui/src/families/valueFormat.ts", out)
        self.assertIn("ui/src/widgets/foo/fooData.ts", out)
        self.assertIn("ui/src/widgets/foo/FooWidget.tsx", out)
        self.assertEqual(check({}, {
            "ui/src/families/value.ts": "export const p = (atLeast: boolean) => ({ atLeast });\n",
            "ui/src/widgets/foo/foo.test.ts": "const s = 'at least 1';\n",
            "ui/src/recorder/status.ts": "const s = 'At least one frame';\n",
            "ui/src/shell/AnalyticsNav.tsx": "const s = 'Choose at least one page';\n",
        })[0], 0)

    def test_family_private_art_cache_is_refused_outside_art_module(self):
        code, out = check({}, {
            "ui/src/families/table/cellPainters.ts": "function resolveArtState(url: string) { return cache.stateOfUrl(url); }\n",
            "ui/src/families/table/painter.ts": "export function collectVisibleCellIcons() { return []; }\n",
        })
        self.assertEqual(code, 1)
        self.assertIn("family-private-art-cache", out)
        self.assertIn("resolveCachedArt", out)
        self.assertIn("collectArtUrls", out)
        self.assertEqual(check({}, {
            "ui/src/families/art/react.ts": "const s = cache.stateOfUrl(url);\n",
            "ui/src/families/table/tableFamily.test.ts": "const s = cache.stateOfUrl(url);\n",
        })[0], 0)

    def test_widget_helpers_and_jsx_are_checked_and_false_positives_pass(self):
        code, out = check({}, {
            "ui/src/widgets/air-statistics/airStatisticsData.ts": "export const fmt = (n: number) => n.toLocaleString('en-US');\n",
            "ui/src/widgets/foo/FooWidget.tsx": "const inkOnFill = (f: string) => f;\nconst computeAxisTicks = () => [];\nconst c = '#e5484d';\nfor (const r of rows) { w = ctx.measureText(r).width; }\n",
        })
        self.assertEqual(code, 1)
        self.assertIn("ui/src/widgets/air-statistics/airStatisticsData.ts: family-locale-string", out)
        self.assertIn("local-contrast-ink", out)
        self.assertIn("local-tick-engine", out)
        self.assertIn("family-colour-literal", out)
        self.assertIn("family-measure-text-loop", out)
        self.assertEqual(check({}, {
            "ui/src/widgets/foo/FooWidget.tsx": (
                "const contrastInk = contrastInkForFill(fill, theme);\n"
                "const computedTicks = linearTicks(0, 100);\n"
                "const el = root.querySelector('#abc');\n"
                "const msg = 'do not use .toLocaleString( here';\n"
                "for (const r of rows) { h = ctx.measureText(r).actualBoundingBoxAscent; }\n"
            ),
            "ui/src/widgets/foo/foo.fixtures.ts": "const s = n.toLocaleString('en-US');\n",
        })[0], 0)


if __name__ == "__main__":
    unittest.main()
