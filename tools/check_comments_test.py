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
