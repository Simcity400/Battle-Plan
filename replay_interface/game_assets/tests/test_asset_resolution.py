"""Pure-unit tests for the game_assets asset-resolution core logic.

Covers the layered file-map resolver (override order + replace_path), texture
path normalization, filename safety, flag-name parsing, sprite-strip slicing,
mod-descriptor parsing/root resolution, and the tokenizer's backslash-escape
branch. These drive which asset file wins for every sprite the replay UI
renders -- an override-order regression would silently ship wrong art.

Deliberately does NOT duplicate the quoted-'#'/unterminated-string/anonymous-
block tokenizer tests: those already exist hermetically in
extractor/game_defs/tests/test_clausewitz_edges.py against this same module.
"""
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from paradox_parser import tokenize
from extract import (
    Layer,
    _norm_reldir,
    build_file_map_layers,
    normalize_texpath,
    parse_flag_filename,
    parse_mod_descriptor,
    resolve_mod_root,
    safe_basename,
    slice_and_save,
)


def _mk(root: Path, rel: str, content: str = "x") -> Path:
    p = root / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(content, encoding="utf-8")
    return p


def test_two_layer_override_mod_wins_and_displaces_cased_key(tmp_path):
    vanilla = tmp_path / "vanilla"
    mod = tmp_path / "mod"
    _mk(vanilla, "interface/Topbar.gfx", "vanilla")
    _mk(vanilla, "interface/other.gfx", "vanilla-other")
    _mk(mod, "interface/topbar.GFX", "mod")

    layers = [Layer("vanilla", vanilla), Layer("mod", mod)]
    eff = build_file_map_layers(layers, "interface", (".gfx",))

    assert "interface/Topbar.gfx" not in eff
    assert eff["interface/topbar.GFX"] == mod / "interface/topbar.GFX"
    assert eff["interface/other.gfx"] == vanilla / "interface/other.gfx"
    assert len(eff) == 2


def test_replace_path_drops_only_immediate_parent_dir(tmp_path):
    vanilla = tmp_path / "vanilla"
    mod = tmp_path / "mod"
    _mk(vanilla, "common/units/infantry.txt")
    _mk(vanilla, "common/units/names_divisions/eng.txt")
    _mk(mod, "common/units/modded.txt")

    layers = [Layer("vanilla", vanilla),
              Layer("mod", mod, replace_paths=["common/units"])]
    eff = build_file_map_layers(layers, "common", (".txt",))

    assert "common/units/infantry.txt" not in eff
    assert "common/units/names_divisions/eng.txt" in eff
    assert eff["common/units/modded.txt"] == mod / "common/units/modded.txt"


def test_replace_path_matches_case_insensitively(tmp_path):
    vanilla = tmp_path / "vanilla"
    mod = tmp_path / "mod"
    _mk(vanilla, "common/UNITS/old.txt")
    _mk(mod, "common/decisions/new.txt")

    layers = [Layer("vanilla", vanilla),
              Layer("mod", mod, replace_paths=["common/units"])]
    eff = build_file_map_layers(layers, "common", (".txt",))
    assert not any(k.lower() == "common/units/old.txt" for k in eff)
    assert "common/decisions/new.txt" in eff


def test_normalize_texpath_absolute_path_defense():
    assert normalize_texpath("C:/abs/path/gfx/x.dds") == "gfx/x.dds"
    assert normalize_texpath("/abs/no/gfxroot") == ""
    assert normalize_texpath("gfx\\interface\\a.dds") == "gfx/interface/a.dds"
    assert normalize_texpath("gfx//interface//a.dds") == "gfx/interface/a.dds"
    assert normalize_texpath(None) == ""


def test_safe_basename_strips_traversal():
    assert safe_basename("gfx/flags/GER.tga") == "GER.tga"
    assert safe_basename("..\\..\\evil") == "evil"
    assert safe_basename("a/../b") == "b"
    assert safe_basename("..") == "_"
    assert safe_basename("a..b") == "a_b"
    assert safe_basename("weird name!") == "weird_name_"
    assert safe_basename("") == "_unnamed"


def test_parse_flag_filename_ideology_suffix():
    assert parse_flag_filename("GER_fascism") == ("GER", "fascism")
    assert parse_flag_filename("SPR_republic") == ("SPR_republic", None)
    assert parse_flag_filename("GER") == ("GER", None)
    assert parse_flag_filename("RAJ_UK_communism") == ("RAJ_UK", "communism")


class _StubImage:
    """Minimal size/crop/save duck-type for slice_and_save."""

    def __init__(self, w, h):
        self.size = (w, h)
        self.saved = []

    def crop(self, box):
        left, upper, right, lower = box
        return _StubImage(right - left, lower - upper)

    def save(self, path):
        Path(path).write_bytes(b"png")


def test_slice_and_save_width_not_divisible_flags_anomaly(tmp_path):
    im = _StubImage(10, 4)
    names, anomaly = slice_and_save(im, 3, tmp_path / "spr", "GFX_spr")
    assert names == ["spr.png"]
    assert anomaly == {
        "sprite": "GFX_spr", "issue": "width_not_divisible_by_frames",
        "width": 10, "no_of_frames": 3,
    }
    assert (tmp_path / "spr.png").is_file()


def test_slice_and_save_divisible_yields_frames_no_anomaly(tmp_path):
    im = _StubImage(9, 4)
    names, anomaly = slice_and_save(im, 3, tmp_path / "spr", "GFX_spr")
    assert anomaly is None
    assert names == ["spr_f0.png", "spr_f1.png", "spr_f2.png"]
    for n in names:
        assert (tmp_path / n).is_file()


def test_slice_and_save_single_frame(tmp_path):
    im = _StubImage(8, 8)
    names, anomaly = slice_and_save(im, 1, tmp_path / "one", "GFX_one")
    assert anomaly is None
    assert names == ["one.png"]


def test_parse_mod_descriptor_round_trip(tmp_path):
    mod_file = tmp_path / "amsb.mod"
    mod_file.write_text(
        'version = "1.0"\n'
        'tags = { "Gameplay" "Balance" }\n'
        'name = "A Mod"\n'
        'path = "mod/amsb"\n'
        'replace_path = "common\\\\units"\n'
        'replace_path = "interface//sub/"\n'
        'supported_version = "1.14.*"\n',
        encoding="latin-1",
    )
    d = parse_mod_descriptor(mod_file)
    assert d["name"] == "A Mod"
    assert d["path"] == "mod/amsb"
    assert d["version"] == "1.0"
    assert d["supported_version"] == "1.14.*"
    assert d["replace_path"] == ["common/units", "interface/sub"]
    assert d["tags"] == ["Gameplay", "Balance"]
    assert d["archive"] is None


def test_norm_reldir():
    assert _norm_reldir("common\\units\\") == "common/units"
    assert _norm_reldir("//interface//sub//") == "interface/sub"
    assert _norm_reldir("./common/./x") == "common/x"
    assert _norm_reldir("") == ""


def test_resolve_mod_root_absolute_and_relative(tmp_path):
    userdata = tmp_path / "userdata"
    absroot = tmp_path / "absmod"
    absroot.mkdir()
    (userdata / "mod" / "relmod").mkdir(parents=True)

    assert resolve_mod_root({"path": str(absroot)}, userdata) == absroot.resolve()
    rel = resolve_mod_root({"path": "mod/relmod"}, userdata)
    assert rel == (userdata / "mod" / "relmod").resolve()
    assert resolve_mod_root({"path": None, "archive": "x.zip"}, userdata) is None
    assert resolve_mod_root({"path": "mod/absent"}, userdata) is None


def test_tokenize_keeps_escaped_char_in_quoted_string():
    toks = tokenize('name = "a\\"b"')
    assert ("str", 'a"b') in toks
    toks = tokenize('name = "a\\\\b"')
    assert ("str", "a\\b") in toks
