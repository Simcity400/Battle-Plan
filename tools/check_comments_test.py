import importlib.util
import io
from pathlib import Path
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location("check_comments", Path(__file__).with_name("check-comments.py"))
lint = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(lint)


def bodies(name, source):
    return [source[start:end] for start, end in lint.comments(name, source)]


class Comments(unittest.TestCase):
    def test_source_tokens_distinguish_literals_and_nested_comments(self):
        rust = b'fn text<\'a>() { let _ = r###"/* raw */ // text"###; }\n/* outer /* inner */ end */\n/// Public docs\n'
        tsx = b'const pattern = /[//]/; const s = `// template ${0 /* expression */}`;\nconst v = <p>// text{/* child */}</p>;'
        self.assertEqual(bodies("file.rs", rust), [b"/* outer /* inner */ end */", b"/// Public docs\n"])
        self.assertEqual(bodies("file.tsx", tsx), [b"/* expression */", b"/* child */"])
        self.assertEqual(bodies("index.html", b"<!-- page --><style>/* css */</style><script>// js\n</script>"),
                         [b"<!-- page -->", b"/* css */", b"// js"])

    def test_workflow_scripts_respect_shell_and_literals(self):
        workflow = b"""jobs:
  build:
    runs-on: windows-latest
    defaults:
      run:
        shell: bash
    steps:
      - run: | # YAML
          cat <<'EOF'
          # literal
          EOF
          echo '${{ env.EXAMPLE }}' # Bash
      - shell: powershell
        run: |
          Write-Output '# literal'
          <# PowerShell #>
"""
        self.assertEqual(bodies(".github/workflows/test.yml", workflow), [b"# YAML", b"# Bash", b"<# PowerShell #>"])

    def test_ratchet_accepts_legacy_removals_and_refuses_new_or_duplicate_comments(self):
        name = "code.rs"
        before = b"// kept\nfn before() {}\n// removed\n"
        self.assertEqual(list(lint.added_comments(name, before, b"fn after() {}\n// kept\n")), [])
        self.assertEqual(list(lint.added_comments(name, before, b"// replacement\n// kept\n// kept\n")), [1, 3])
        self.assertEqual(list(lint.added_comments(name, b"", b"fn after() {} // copied\n")), [1])
        quarantined = b"// quarantine: #3 why\ntest.skip('name', () => {});\n"
        self.assertEqual(list(lint.added_comments("ui/src/a.test.ts", b"", quarantined)), [])
        self.assertEqual(list(lint.added_comments("tools/a.test.mjs", b"", quarantined)), [])
        self.assertEqual(list(lint.added_comments("ui/src/a.ts", b"", quarantined)), [1])
        self.assertEqual(list(lint.added_comments("ui/src/a.test.ts", b"", b"// quarantine: why\n")), [1])
        self.assertEqual(list(lint.added_comments("ui/src/a.test.ts", b"", b"// quarantine: #3 why\ntest('name', () => {});\n")), [1])
        looped = b"// quarantine: #3 why\r\nfor (const x of [1]) test.skip(`n ${x}`, () => {});\r\n"
        self.assertEqual(list(lint.added_comments("ui/src/a.test.ts", b"", looped)), [])

    def test_safety_contracts_are_exempt_only_at_unsafe_code(self):
        rust = b"""fn f(p: *const u8) -> u8 {
    // SAFETY: kept.
    // Continued.
    let a = unsafe { *p };
    // SAFETY: kept.
    #[allow(unused_unsafe)]
    unsafe { g() };
    if a > 0
        // SAFETY: kept.
        && let Some(b) = unsafe { p.as_ref() }
    {
        h(a,
            // SAFETY: kept.
            unsafe { *p },
        );
    }
    macro_rules! read {
        () => {{
            // SAFETY: kept.
            unsafe { *p }
        }};
    }
    // SAFETY: refused, the code is safe.
    let c = a;

    // Refused, not part of the run above.
    a // SAFETY: refused, trailing.
}
// SAFETY: kept.
unsafe impl Send for S {}
// SAFETY: refused, not an unsafe impl.
impl S { fn i() { unsafe { g() } } }
// SAFETY: refused, a function's contract belongs in its docs.
unsafe fn j() {}
// Safety: refused, not the marker.
unsafe impl Sync for S {}
/// Kept summary.
///
/// # Safety
/// Kept contract.
#[inline]
pub unsafe fn k() {}
/// # Safety
/// Refused, the function is safe.
pub fn l() {}
/// Refused, no Safety section.
pub unsafe fn m() {}
/// # Safety
/// Kept.
pub unsafe trait T {}
"""
        self.assertEqual([body for body in bodies("file.rs", rust)], [
            b"// SAFETY: refused, the code is safe.", b"// Refused, not part of the run above.",
            b"// SAFETY: refused, trailing.", b"// SAFETY: refused, not an unsafe impl.",
            b"// SAFETY: refused, a function's contract belongs in its docs.", b"// Safety: refused, not the marker.",
            b"/// # Safety\n", b"/// Refused, the function is safe.\n", b"/// Refused, no Safety section.\n",
        ])

    def test_safety_contracts_must_touch_their_code_and_stay_inside_their_statement(self):
        rust = b"""fn f(p: *const u8) {
    // SAFETY: refused, detached by a blank line.

    // SAFETY: kept.
    unsafe { g() };
    macro_rules! m {
        () => {{
            // SAFETY: refused, the statement it precedes is safe.
            if ready { h() } else { i() }
            unsafe { g() }
            // SAFETY: kept.
            if ready { unsafe { g() } }
        }};
    }
}
/// Refused: the heading is only an example.
/// ```text
/// # Safety
/// ```
pub unsafe fn j() {}
macro_rules! emit {
    () => {
        /// # Safety
        /// Kept: an unsafe function the macro emits.
        pub unsafe fn k() {}
    };
}
"""
        self.assertEqual(bodies("file.rs", rust), [
            b"// SAFETY: refused, detached by a blank line.", b"// SAFETY: refused, the statement it precedes is safe.",
            b"/// Refused: the heading is only an example.\n", b"/// ```text\n", b"/// # Safety\n",
            b"/// ```\n",
        ])

    def test_macro_items_attribute_lines_and_markdown_code_are_classified_exactly(self):
        rust = b"""macro_rules! m {
    ($p:expr) => {
        // SAFETY: refused, a safe function's body is not this comment's code.
        pub fn safe() { unsafe { g() } }
        /// # Safety
        /// Refused: a type alias is not an unsafe item.
        type Callback = unsafe fn();
        // SAFETY: kept, the unsafe call is in this statement.
        let x = S { v: 1 }.combine(unsafe { *$p });
        // SAFETY: kept.
        unsafe impl Send for S {}
    };
}
/// # Safety
/// Kept, with its attribute on the item's line.
#[inline] pub unsafe fn fetch() {}
/// Text.
///
///     # Safety
/// Refused: the heading is indented code.
pub unsafe fn indented() {}
/// ````text
/// ```
/// # Safety
/// ````
/// Refused: a shorter fence does not close the block.
pub unsafe fn fenced() {}
/// Refused: the contract sits above an empty heading.
///
/// # Safety
pub unsafe fn empty() {}
"""
        self.assertEqual(bodies("file.rs", rust), [
            b"// SAFETY: refused, a safe function's body is not this comment's code.",
            b"/// # Safety\n", b"/// Refused: a type alias is not an unsafe item.\n",
            b"/// Text.\n", b"///\n", b"///     # Safety\n", b"/// Refused: the heading is indented code.\n",
            b"/// ````text\n", b"/// ```\n", b"/// # Safety\n", b"/// ````\n",
            b"/// Refused: a shorter fence does not close the block.\n",
            b"/// Refused: the contract sits above an empty heading.\n", b"///\n", b"/// # Safety\n",
        ])

    def test_function_pointer_types_unsafe_attributes_and_foreign_functions(self):
        rust = b"""fn f(callback: unsafe fn()) {
    // SAFETY: refused, storing a function pointer is safe.
    let alias: unsafe fn() = callback;
}
// SAFETY: kept, the exported symbol is unique in this image.
#[unsafe(no_mangle)]
extern "C" fn exported() {}
/// # Safety
/// Kept: the caller's contract, with the symbol's contract between it and the function.
// SAFETY: kept, the exported symbol is unique in this image.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn first(reason: u32) -> i32 { 1 }
// SAFETY: kept, the exported symbol is unique in this image.
#[unsafe(no_mangle)]
/// # Safety
/// Kept: the caller's contract after the symbol's.
pub unsafe extern "system" fn second(reason: u32) -> i32 { 1 }
unsafe extern "C" {
    /// # Safety
    /// Kept: a foreign function is unsafe to call.
    fn copy(destination: *mut u8, len: usize);
    /// # Safety
    /// Refused: declared safe.
    pub(crate) safe fn version() -> u32;
}
macro_rules! entry {
    ($name:ident) => {
        /// # Safety
        /// Kept: the macro emits an unsafe callback.
        unsafe extern "system-unwind" fn $name() {}
    };
}
macro_rules! imports {
    () => {
        // SAFETY: kept, the macro declares foreign functions.
        unsafe extern "C" {
            /// # Safety
            /// Kept: a foreign function the macro declares is unsafe to call.
            #[link_name = "native_entry"]
            fn native();
            /// # Safety
            /// Refused: declared safe.
            safe fn version() -> u32;
        }
    };
}
macro_rules! visible {
    ($vis:vis) => {
        /// # Safety
        /// Kept: a visibility parameter before an unsafe function.
        $vis unsafe fn fetch() {}
        // SAFETY: refused, a visibility parameter before a safe function.
        $vis fn safe() { unsafe { g() } }
    };
}
macro_rules! detours {
    ($($name:ident),*) => {
        $(
            /// # Safety
            /// Kept: each repetition emits an unsafe callback.
            unsafe extern "system" fn $name() {}
            // SAFETY: refused, a safe function's body is not this comment's code.
            fn safe() { unsafe { g() } }
        )*
    };
}
fn g(cells: &[*const u8]) -> Vec<u8> {
    cells
        .iter()
        // SAFETY: kept, every cell points at a live byte.
        .map(|p| unsafe { **p })
        .collect()
}
fn checked(p: *const u32) {
    assert_eq!(
        // SAFETY: kept, `p` points at a live word.
        Ok::<_, &'static str>(unsafe { *p }),
        Ok(1),
    );
    assert_eq!(
        // SAFETY: refused, the empty vector is safe; the unsafe call is the next argument.
        Vec::<Vec<u8>>::new(),
        unsafe { make_vec() },
    );
}
macro_rules! fetch {
    ($p:expr) => {{
        // SAFETY: kept, `$p` points at a live byte.
        let x: Result<u8, E> = unsafe { read($p) };
    }};
}
macro_rules! row {
    ($row:expr, $limit:expr, $p:expr) => {
        Row {
            // SAFETY: refused, a comparison is safe; the unsafe read is the next field.
            in_range: $row < $limit,
            value: unsafe { *$p },
        }
    };
}
macro_rules! pop {
    ($head:expr) => {{
        // SAFETY: kept, every linked base holds a header that push wrote.
        let Node { next, bytes, .. } = unsafe { $head.read() };
    }};
}
fn total(cells: &[*const u32]) -> std::io::Result<u32> {
    cells
        .iter()
        // SAFETY: kept, every cell points at a live word.
        .try_fold::<_, _, std::io::Result<u32>>(0, |sum, &p| Ok(sum + unsafe { *p }))
}
"""
        self.assertEqual(bodies("file.rs", rust), [
            b"// SAFETY: refused, storing a function pointer is safe.", b"/// # Safety\n", b"/// Refused: declared safe.\n",
            b"/// # Safety\n", b"/// Refused: declared safe.\n",
            b"// SAFETY: refused, a visibility parameter before a safe function.",
            b"// SAFETY: refused, a safe function's body is not this comment's code.",
            b"// SAFETY: refused, the empty vector is safe; the unsafe call is the next argument.",
            b"// SAFETY: refused, a comparison is safe; the unsafe read is the next field.",
        ])

    def test_git_status_keeps_rename_baselines_without_granting_them_to_copies(self):
        with patch.object(lint, "paths", side_effect=[
            ["R100", "old.rs", "new.rs", "C100", "old.rs", "copy.rs", "M", "edit.rs", "A", "add.rs"],
            ["untracked.rs"],
        ]):
            self.assertEqual(dict(lint.changes("BASE")), {
                "new.rs": "old.rs", "copy.rs": None, "edit.rs": "edit.rs", "add.rs": None, "untracked.rs": None,
            })

    def test_counts_only_comment_lines_that_can_disappear(self):
        source = b"/* first */ /* second */\nconst x = 1; /* inline\n * then standalone */\n// last\n"
        self.assertEqual(lint.comment_lines(source, lint.comments("code.ts", source)), (4, 3))

    def test_list_escapes_comments_on_an_ascii_output_stream(self):
        with io.BytesIO() as output, io.TextIOWrapper(output, encoding="ascii", newline="\n") as stdout:
            with (patch("sys.stdout", stdout), patch.object(Path, "is_file", return_value=True),
                  patch.object(lint, "paths", return_value=["fixture.rs"]),
                  patch.object(lint, "read_source", return_value="// →".encode("utf-8"))):
                lint.inventory(True, [])
            stdout.flush()
            self.assertEqual(output.getvalue(), b"fixture.rs:1: '// \\u2192'\n")


if __name__ == "__main__":
    unittest.main()
