# Agent skills

An **agent skill** lets an AI agent (Claude Code, for example) drive terrana for a
task, such as checking a dataset's coordinates before it's used. Skills live here,
one folder per skill.

| Skill | What it does |
|---|---|
| `check-coordinates` | Counts rows with no point, coordinates out of range, points at (0, 0) and points outside an expected area, and fails when the flagged share is above a threshold. Also the minimal example of the convention. |

## Layout

```
skills/<name>/
├── SKILL.md          # what the skill is for, when to use it, how to run it
└── scripts/
    ├── _terrana.py   # starts or reaches a terrana server, reads its JSON (shared, identical)
    └── <step>.py     # one script per step
```

`SKILL.md` starts with front matter giving the skill's `name` (prefixed `terrana-`),
a `description` that says when an agent should use it, and `allowed-tools` for its
scripts. The body tells the agent
which scripts to run, in what order, and what to report.

## Rules for skill scripts

An agent's decisions rest on objective scripts, not its own judgement:

- **Drive terrana through its server.** terrana's command line is `terrana serve`, so
  scripts query its HTTP API, which answers in JSON; they never import terrana.
  `_terrana.py` starts `terrana serve` on the script's data file, on a free loopback
  port, for the length of the script (`TERRANA_BIN` names the executable; default the
  one on the `PATH`). With `TERRANA_URL` set it uses that running server instead,
  sending `TERRANA_TOKEN` as the bearer token, so the same skill works against a
  remote server.
- **Decide by thresholds on objective measurements**, given as script options with
  documented defaults. Anything a threshold can't settle goes to a person; the agent
  reports it as it is.
- **Print a JSON summary** on standard output, so the agent reads results, not prose.
- **Exit codes mean something:** `0` the step ran and its checks passed, `1` terrana
  failed (the summary is `{"error": ...}` with terrana's own message), `2` the step
  ran but a check failed (the summary says which).
- **Paths are relative to the working directory**, and each run writes its files into
  its own folder, `runs/<skill>/<timestamp>/`.
- **Standard library only**, Python 3.9 or newer. A script needs Python and `terrana`
  (the binary, or `pip install terrana`).
- **Each skill folder is self-contained**, so it can be installed on its own: it
  carries its own copy of `_terrana.py`, and the copies stay identical.

## Installing a skill

In Claude Code, install them all as the `terrana` plugin from this repository's
marketplace (`.claude-plugin/marketplace.json`); a skill then runs as
`/terrana:<name>`, for example `/terrana:check-coordinates`:

```
/plugin install terrana --marketplace jmcmeen/terrana
```

On Claude Code older than 2.1.275, add the marketplace first:
`/plugin marketplace add jmcmeen/terrana`, then `/plugin install terrana@terrana`.

For any agent, copy or link a skill's folder into the agent's skills directory
(Claude Code: `.claude/skills/` in a project or `~/.claude/skills/` for every
project):

```console
$ ln -s /path/to/terrana/skills/check-coordinates ~/.claude/skills/terrana-check-coordinates
```

`SKILL.md` refers to its scripts as `${CLAUDE_SKILL_DIR}/scripts/...`, which Claude Code
fills in either way.

## Evaluating a skill

Every skill has a pass/fail eval in `tests/skills/`. The harness runs the skill's
scripts the way an agent does, as separate processes in a temporary folder with a
dataset that has known problems, and checks their exit codes, JSON and the files they
wrote. `make skills-test` builds terrana and runs them; CI runs them on every PR, and
`make skills-lint` checks the Python.
