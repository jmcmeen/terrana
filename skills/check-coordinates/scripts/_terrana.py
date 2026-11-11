"""Talk to a terrana server for a skill script and read its JSON results.

Every skill carries its own copy of this file (a skill folder is installed on its
own), and the copies must stay identical. Scripts use only this module and the
standard library: they drive terrana through its HTTP API, never by importing it.

Where the server comes from is configuration:

- ``TERRANA_URL`` set: use that running server, sending ``TERRANA_TOKEN`` (if set) as
  the bearer token. The data file is whatever that server was started on.
- Otherwise: start ``terrana serve`` on the script's data file, on a free loopback
  port, for the length of the script. ``TERRANA_BIN`` names the ``terrana`` executable
  (default: the one on the ``PATH``).
"""

from __future__ import annotations

import contextlib
import json
import os
import re
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Iterator
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

PAGE = 100_000  # terrana's per-page cap on `limit`
_ANSI = re.compile(r"\x1b\[[0-9;]*m")
_LISTENING = re.compile(r"listening on (\S+)")


class TerranaError(RuntimeError):
    """terrana failed: it couldn't start, or a request returned an error."""


def _timeout() -> float:
    """Seconds to wait for startup or a request: ``TERRANA_TIMEOUT`` (default 300)."""
    try:
        return float(os.environ.get("TERRANA_TIMEOUT", "300"))
    except ValueError:
        return 300.0


class Server:
    """A terrana server's base URL and token, with JSON requests against it."""

    def __init__(self, url: str, token: str | None = None):
        self.url = url.rstrip("/")
        self.token = token

    def _open(self, path: str, params: dict | list | None = None):
        query = urllib.parse.urlencode(params or {}, doseq=True)
        req = urllib.request.Request(f"{self.url}{path}{'?' + query if query else ''}")
        if self.token:
            req.add_header("Authorization", f"Bearer {self.token}")
        try:
            return urllib.request.urlopen(req, timeout=_timeout())
        except urllib.error.HTTPError as e:
            body = e.read().decode(errors="replace")
            try:
                message = json.loads(body)["error"]
            except (ValueError, KeyError, TypeError):
                message = body.strip() or e.reason
            raise TerranaError(f"{path}: {message} (HTTP {e.code})") from None
        except urllib.error.URLError as e:
            raise TerranaError(f"no terrana server at {self.url}: {e.reason}") from None
        except OSError as e:  # a timeout or reset before the response arrives
            raise TerranaError(f"{path}: {e}") from None

    def _read(self, path: str, params: dict | list | None = None) -> tuple[Any, str]:
        """``GET`` and decode the JSON body; returns it and the ``Link`` header."""
        with self._open(path, params) as resp:
            try:
                return json.load(resp), resp.headers.get("Link", "")
            except (OSError, ValueError) as e:  # cut off, timed out or not JSON
                raise TerranaError(f"{path}: unreadable answer: {e}") from None

    def get(self, path: str, params: dict | list | None = None) -> Any:
        """``GET path?params`` and return the decoded JSON."""
        return self._read(path, params)[0]

    def rows(self, params: dict | list) -> Iterator[dict]:
        """Every row ``/query?params`` matches, following the ``Link`` header's pages."""
        params = [*_pairs(params), ("limit", str(PAGE))]
        path = "/query"
        while True:
            page, link = self._read(path, params)
            yield from page
            match = re.search(r'<\?([^>]*)>;\s*rel="next"', link)
            if not match:
                return
            params = urllib.parse.parse_qsl(match.group(1), keep_blank_values=True)


def _pairs(params: dict | list) -> list[tuple[str, str]]:
    items = params.items() if isinstance(params, dict) else params
    return [(k, str(v)) for k, v in items]


@contextlib.contextmanager
def server(data_file: str | None, *serve_args: str) -> Iterator[Server]:
    """The server to use: ``TERRANA_URL``'s, or one started on ``data_file``.

    ``serve_args`` are extra ``terrana serve`` options (``--lat``, ``--lon``, ...),
    used only when starting a server.

    Raises:
        TerranaError: If no server is configured and ``data_file`` is missing, a file
            or options are given alongside ``TERRANA_URL``, terrana can't be run, or
            the server doesn't start (with terrana's own message).
    """
    url = os.environ.get("TERRANA_URL")
    if url and (data_file or serve_args):
        # Checking the server's data while reporting on the named file would answer
        # for the wrong dataset.
        raise TerranaError(
            "TERRANA_URL is set, so a data file or column options can't be used: "
            "leave them out to use that server's data, or unset TERRANA_URL"
        )
    if url:
        yield Server(url, os.environ.get("TERRANA_TOKEN") or None)
        return
    if not data_file:
        raise TerranaError("give a data file, or set TERRANA_URL to a running server")
    if not Path(data_file).is_file():
        raise TerranaError(f"no such file: {data_file}")
    # A token in the environment is for a remote server; this one is local and open.
    env = {k: v for k, v in os.environ.items() if k != "TERRANA_TOKEN"}
    with tempfile.TemporaryFile("w+") as log:
        # No shell: the command is always "terrana serve" plus the script's arguments,
        # and TERRANA_BIN only chooses which executable that name runs.
        try:
            proc = subprocess.Popen(
                [
                    "terrana",
                    "serve",
                    "--port",
                    "0",
                    "--bind",
                    "127.0.0.1",
                    *serve_args,
                    "--",
                    data_file,
                ],
                executable=os.environ.get("TERRANA_BIN") or None,
                stdout=log,
                stderr=subprocess.STDOUT,
                stdin=subprocess.DEVNULL,
                env=env,
            )
        except OSError as e:
            where = os.environ.get("TERRANA_BIN") or "terrana on the PATH"
            raise TerranaError(f"can't run {where}: {e.strerror or e}") from None
        try:
            yield Server(f"http://{_wait_listening(proc, log)}")
        finally:
            proc.terminate()
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait()


def _wait_listening(proc: subprocess.Popen, log) -> str:
    """The address the server logs once it's listening; raises if it exits first."""
    deadline = time.monotonic() + _timeout()
    while time.monotonic() < deadline:
        log.seek(0)
        text = _ANSI.sub("", log.read())
        match = _LISTENING.search(text)
        if match:
            return match.group(1)
        if proc.poll() is not None:
            errors = [line for line in text.splitlines() if line.startswith("Error")]
            raise TerranaError(errors[-1] if errors else f"terrana exited ({proc.returncode})")
        time.sleep(0.1)
    raise TerranaError(f"terrana didn't start within {_timeout():g} s (TERRANA_TIMEOUT)")


def run_dir(skill: str, root: str | Path = ".") -> Path:
    """A new folder for this run's files: ``runs/<skill>/<UTC timestamp>/``."""
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    path = Path(root) / "runs" / skill / stamp
    path.mkdir(parents=True)
    return path


def write_json(path: Path, data: Any) -> None:
    path.write_text(json.dumps(data, indent=2) + "\n")
