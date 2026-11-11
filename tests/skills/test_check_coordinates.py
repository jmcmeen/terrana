"""Eval for the check-coordinates skill: a dataset with one of each problem."""

from __future__ import annotations

import importlib.util
import json
import sys

import pytest
from conftest import SKILLS, write_csv

# Row ids follow file order, starting at 1.
MIXED = [
    ("ok1", 36.5, -82.5),  # 1
    ("ok2", 36.6, -82.4),  # 2
    ("paris", 48.8, 2.3),  # 3: valid, outside the expected box
    ("badlat", 95, -82),  # 4: out of range
    ("badlon", 36, 190),  # 5: out of range
    ("null_island", 0, 0),  # 6: (0, 0), and outside the box
    ("blank", None, None),  # 7: no point
]
BOX = "35,-84,37,-81"


def _load_script(skill: str, name: str):
    """A fresh copy of one of a skill's scripts, kept out of sys.modules. A script that
    imports _terrana puts its own folder first on sys.path; that's undone afterwards."""
    folder = SKILLS / skill / "scripts"
    spec = importlib.util.spec_from_file_location(f"{skill}.{name}", folder / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    saved_path, saved_modules = list(sys.path), dict(sys.modules)
    try:
        spec.loader.exec_module(module)
    finally:
        sys.path[:] = saved_path
        for key in set(sys.modules) - set(saved_modules):
            del sys.modules[key]
    return module


@pytest.fixture
def mixed(tmp_path):
    return write_csv(tmp_path / "mixed.csv", MIXED)


def test_flags_each_kind_of_problem(run_skill, mixed, tmp_path):
    code, summary = run_skill(
        "check-coordinates",
        "check_coordinates",
        "mixed.csv",
        "--bbox",
        BOX,
        cwd=tmp_path,
    )

    assert code == 2, summary
    assert summary["passed"] is False
    assert (summary["rows"], summary["with_point"]) == (7, 6)
    assert summary["counts"] == {
        "missing_point": 1,
        "out_of_range": 2,
        "null_island": 1,
        "outside_bbox": 2,
    }
    assert summary["examples"] == {
        "out_of_range": [4, 5],
        "null_island": [6],
        "outside_bbox": [3, 6],
    }
    # Row 6 is both at (0, 0) and outside the box: one flagged row, not two.
    assert summary["flagged_rows"] == 5
    assert summary["flagged_fraction"] == pytest.approx(5 / 7, abs=1e-6)
    assert summary["run_dir"].startswith("runs/check-coordinates/")
    report = json.loads((tmp_path / summary["run_dir"] / "report.json").read_text())
    assert report["row_ids"]["out_of_range"] == [4, 5]
    assert report["source"].endswith("mixed.csv")


def test_passes_under_the_threshold(run_skill, mixed, tmp_path):
    args = ["mixed.csv", "--bbox", BOX, "--max-flagged-fraction", "0.75"]
    code, summary = run_skill("check-coordinates", "check_coordinates", *args, cwd=tmp_path)
    assert code == 0, summary
    assert summary["passed"] is True


def test_clean_data_passes(run_skill, tmp_path):
    write_csv(tmp_path / "clean.csv", [("a", 36.5, -82.5), ("b", 35.6, -82.6)])
    code, summary = run_skill(
        "check-coordinates",
        "check_coordinates",
        "clean.csv",
        "--bbox",
        BOX,
        cwd=tmp_path,
    )
    assert code == 0, summary
    assert summary["flagged_rows"] == 0
    assert summary["examples"] == {}


def test_without_a_bbox_there_is_no_outside_check(run_skill, mixed, tmp_path):
    _, summary = run_skill("check-coordinates", "check_coordinates", "mixed.csv", cwd=tmp_path)
    assert "outside_bbox" not in summary["counts"]
    assert summary["flagged_rows"] == 4  # blank, two out of range, (0, 0)


def test_explicit_columns_are_passed_to_terrana(run_skill, tmp_path):
    (tmp_path / "cols.csv").write_text("site,north,east\na,36.5,-82.5\nb,91,-82.5\n")
    args = ["cols.csv", "--lat", "north", "--lon", "east"]
    code, summary = run_skill("check-coordinates", "check_coordinates", *args, cwd=tmp_path)
    assert code == 2, summary
    assert summary["lat_column"] == "north"
    assert summary["counts"]["out_of_range"] == 1


@pytest.mark.parametrize(
    ("setup", "expected"),
    [
        (lambda p: None, "no such file"),
        (lambda p: (p / "data.csv").write_text("a,b\n1,2\n"), "Error"),
    ],
    ids=["missing-file", "terrana-refuses"],
)
def test_a_terrana_failure_is_reported_as_json(run_skill, tmp_path, setup, expected):
    setup(tmp_path)
    code, summary = run_skill("check-coordinates", "check_coordinates", "data.csv", cwd=tmp_path)
    assert code == 1
    assert expected in summary["error"]


def test_a_missing_terrana_is_reported_as_json(run_skill, mixed, tmp_path):
    env = {"TERRANA_BIN": str(tmp_path / "no-such-terrana")}
    code, summary = run_skill(
        "check-coordinates", "check_coordinates", "mixed.csv", cwd=tmp_path, env=env
    )
    assert code == 1
    assert "can't run" in summary["error"]


@pytest.mark.parametrize(
    "args",
    [["--max-flagged-fraction", "5%"], ["--bbox", "35,-84,37"], ["--bbox", "37,-84,35,-81"]],
    ids=["bad-number", "short-bbox", "lats-reversed"],
)
def test_bad_arguments_exit_1_not_2(run_skill, mixed, tmp_path, args):
    """argparse's own exit code, 2, would read as "the check failed"."""
    code, summary = run_skill(
        "check-coordinates", "check_coordinates", "mixed.csv", *args, cwd=tmp_path
    )
    assert code == 1
    assert "error" in summary


def test_non_finite_coordinates_are_out_of_range(run_skill, tmp_path):
    # terrana gives these rows a point but sends the coordinates as null.
    write_csv(tmp_path / "nan.csv", [("a", 36.5, -82.5), ("b", "nan", -82), ("c", 36, "inf")])
    _, summary = run_skill("check-coordinates", "check_coordinates", "nan.csv", cwd=tmp_path)
    assert summary["examples"] == {"out_of_range": [2, 3]}


def test_coordinate_columns_with_spaces_in_their_names(run_skill, tmp_path):
    (tmp_path / "cols.csv").write_text("site,Lat Deg,Lon Deg\na,36.5,-82.5\nb,91,-82.5\n")
    args = ["cols.csv", "--lat", "Lat Deg", "--lon", "Lon Deg", "--bbox", BOX]
    code, summary = run_skill("check-coordinates", "check_coordinates", *args, cwd=tmp_path)
    assert code == 2, summary
    assert summary["counts"]["out_of_range"] == 1
    assert summary["counts"]["outside_bbox"] == 0


def test_a_bbox_across_the_antimeridian(run_skill, tmp_path):
    rows = [("fiji", -17.8, 178.0), ("samoa", -13.8, -172.0), ("perth", -31.9, 115.9)]
    write_csv(tmp_path / "pacific.csv", rows)
    args = ["pacific.csv", "--bbox", "-25,170,-10,-170"]
    _, summary = run_skill("check-coordinates", "check_coordinates", *args, cwd=tmp_path)
    assert summary["examples"] == {"outside_bbox": [3]}


def test_an_empty_dataset_fails():
    # Checked against a stand-in server: terrana can't load a header-only CSV yet (#76).
    class Empty:
        def get(self, path):
            return {"lat_column": "lat", "lon_column": "lon", "row_count": 0, "source": "x"}

        def rows(self, params):
            return iter([])

    module = _load_script("check-coordinates", "check_coordinates")
    report = module.check(Empty(), None, 0.0, 5)
    assert report["passed"] is False
    assert report["reason"] == "the dataset has no rows"


def test_runs_against_a_running_server(run_skill, running_server, mixed, tmp_path):
    url = running_server(mixed, token="s3cret")
    env = {"TERRANA_URL": url, "TERRANA_TOKEN": "s3cret"}
    code, summary = run_skill(
        "check-coordinates", "check_coordinates", "--bbox", BOX, cwd=tmp_path, env=env
    )
    assert code == 2, summary
    assert summary["flagged_rows"] == 5

    env["TERRANA_TOKEN"] = "wrong"
    code, summary = run_skill("check-coordinates", "check_coordinates", cwd=tmp_path, env=env)
    assert code == 1
    assert "HTTP 401" in summary["error"]


def test_a_file_alongside_terrana_url_is_refused(run_skill, running_server, mixed, tmp_path):
    """Checking the server's data while naming another file would report on the wrong one."""
    env = {"TERRANA_URL": running_server(mixed)}
    code, summary = run_skill(
        "check-coordinates", "check_coordinates", "other.csv", cwd=tmp_path, env=env
    )
    assert code == 1
    assert "TERRANA_URL is set" in summary["error"]


def test_the_report_names_the_servers_source(run_skill, running_server, mixed, tmp_path):
    env = {"TERRANA_URL": running_server(mixed)}
    _, summary = run_skill("check-coordinates", "check_coordinates", cwd=tmp_path, env=env)
    assert summary["source"].endswith("mixed.csv")


def test_rows_follow_paging(running_server, mixed):
    helper = _load_script("check-coordinates", "_terrana")
    helper.PAGE = 2
    srv = helper.Server(running_server(mixed))
    names = [row["name"] for row in srv.rows([("select", "name")])]
    assert names == ["ok1", "ok2", "paris", "badlat", "badlon", "null_island"]


def test_every_skill_has_a_skill_md_with_name_and_description():
    for skill in [p for p in SKILLS.iterdir() if p.is_dir()]:
        text = (skill / "SKILL.md").read_text()
        assert text.startswith("---\nname: terrana-"), skill.name
        assert "\ndescription: " in text.split("---")[1]
        assert (skill / "scripts").is_dir()


def test_skills_share_one_helper():
    """Every skill carries the same _terrana.py (a skill folder is installed alone)."""
    copies = {p.read_text() for p in SKILLS.glob("*/scripts/_terrana.py")}
    assert len(copies) == 1


def test_the_marketplace_installs_every_skill_as_one_plugin():
    """`/plugin install terrana --marketplace jmcmeen/terrana`: the plugin is the skills/
    folder, so Claude Code finds every skill there and an install copies only skills/."""
    market = json.loads((SKILLS.parent / ".claude-plugin" / "marketplace.json").read_text())
    assert market["name"] == "terrana"
    [plugin] = market["plugins"]
    assert (plugin["name"], plugin["source"], plugin["skills"]) == ("terrana", "./skills", ["./"])
    for skill in [p for p in SKILLS.iterdir() if p.is_dir()]:
        text = (skill / "SKILL.md").read_text()
        assert "<this skill>" not in text
        assert "${CLAUDE_SKILL_DIR}/scripts/" in text, skill.name
