from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from paradox_parser import parse
from extract import gui_define_table, parse_gui_file


def test_parse_gui_file_preserves_top_level_at_defines(tmp_path):
    gui = tmp_path / "sample.gui"
    gui.write_text(
        """
@prod_icon_pos_x = 358
@prod_sub_pos_x = 379
@ignored_non_numeric = UPPER_LEFT

guiTypes = {
    @nested_col = 42
    containerWindowType = {
        name = "root"
        position = { x = @prod_icon_pos_x y = @nested_col }
    }
}
""".strip(),
        encoding="latin-1",
    )

    windows, defines, stats = parse_gui_file(gui)

    assert defines == {
        "prod_icon_pos_x": 358,
        "prod_sub_pos_x": 379,
        "nested_col": 42,
    }
    assert "children" not in windows[0]
    assert windows[0]["position"]["x"] == "@prod_icon_pos_x"
    assert windows[0]["position"]["y"] == "@nested_col"
    assert stats["element_type_counts"]["containerWindowType"] == 1


def test_gui_define_table_does_not_flatten_conflicting_scoped_values():
    root = parse(
        """
guiTypes = {
    @shared_col = 10
    containerWindowType = {
        @shared_col = 20
        @unique_col = 30
    }
}
""".strip()
    )

    assert gui_define_table(root) == {"unique_col": 30}
