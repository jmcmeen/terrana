"""The skills eval harness: run a skill's scripts the way an agent does.

Each skill script runs as its own process, with ``TERRANA_BIN`` pointing at the
``terrana`` built from this checkout (``make skills-test`` builds it) and a temporary
folder as its working directory, and its exit code and JSON summary are returned.
Fixtures write small datasets with known problems, so each skill gets a pass/fail eval.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
SKILLS = ROOT / "skills"


def terrana_bin() -> str:
    """``TERRANA_BIN``, or the debug build in this checkout."""
    path = os.environ.get("TERRANA_BIN") or str(ROOT / "target" / "debug" / "terrana")
    if not Path(path).is_file():
        pytest.fail(f"no terrana executable at {path}: run `cargo build` (or make skills-test)")
    return path


@pytest.fixture
def run_skill():
    """``run_skill(skill, script, *args, cwd=dir, env={...})`` -> (exit code, JSON)."""

    def run(skill: str, script: str, *args: str, cwd: Path, env: dict | None = None):
        # A server or token from the caller's own environment mustn't leak in.
        local = {
            "TERRANA_BIN": terrana_bin(),
            "TERRANA_URL": None,
            "TERRANA_TOKEN": None,
        }
        full_env = {**os.environ, **local, **(env or {})}
        for key, value in list(full_env.items()):
            if value is None:
                del full_env[key]
        proc = subprocess.run(
            [sys.executable, str(SKILLS / skill / "scripts" / f"{script}.py"), *args],
            cwd=cwd,
            env=full_env,
            capture_output=True,
            text=True,
            check=False,
            timeout=300,
        )
        try:
            return proc.returncode, json.loads(proc.stdout)
        except ValueError:
            raise AssertionError(f"no JSON from {script}: {proc.stdout}\n{proc.stderr}") from None

    return run


@pytest.fixture
def running_server(tmp_path):
    """``running_server(data_file, token=None)`` -> base URL of a ``terrana serve``
    started outside the skill, as a remote one would be. Stopped after the test."""
    procs = []

    def start(data_file: Path, token: str | None = None) -> str:
        env = {k: v for k, v in os.environ.items() if k != "TERRANA_TOKEN"}
        if token:
            env["TERRANA_TOKEN"] = token
        log = (tmp_path / f"server{len(procs)}.log").open("w+")
        proc = subprocess.Popen(
            [terrana_bin(), "serve", "--port", "0", str(data_file)],
            stdout=log,
            stderr=subprocess.STDOUT,
            env=env,
        )
        procs.append(proc)
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            log.seek(0)
            match = re.search(r"listening on (\S+)", re.sub(r"\x1b\[[0-9;]*m", "", log.read()))
            if match:
                return f"http://{match.group(1)}"
            assert proc.poll() is None, "terrana exited before listening"
            time.sleep(0.1)
        raise AssertionError("terrana didn't start")

    yield start
    for proc in procs:
        proc.terminate()
        proc.wait(timeout=10)


def write_csv(path: Path, rows: list[tuple]) -> Path:
    """A ``name,lat,lon`` CSV; ``None`` coordinates are left blank."""
    lines = ["name,lat,lon"]
    lines += [",".join("" if v is None else str(v) for v in row) for row in rows]
    path.write_text("\n".join(lines) + "\n")
    return path
