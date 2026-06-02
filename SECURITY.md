# Security Policy

## Supported versions

Terrana is pre-1.0. Security fixes are applied to the latest released version on the
`main` branch.

| Version | Supported |
|---------|-----------|
| 0.1.x   | ✅        |
| < 0.1.0 | ❌        |

## Reporting a vulnerability

Please report security issues **privately** — do not open a public issue for
suspected vulnerabilities.

- Preferred: use GitHub's [private vulnerability reporting](https://github.com/jmcmeen/terrana/security/advisories/new)
  ("Report a vulnerability" under the repository's Security tab).
- Alternatively, email **johnmcmeen@gmail.com** with the details.

Please include a description, reproduction steps, the input file/query that triggers
it, and the affected version. We aim to acknowledge reports within a few days.

## Threat model

Terrana is designed to be run by an operator over their **own** trusted data files,
typically on localhost or behind their own network controls. Keep this in mind when
deploying:

- **Read-only.** Terrana never writes to the source file; there are no insert/append
  endpoints.
- **No authentication or authorization.** Terrana ships no auth layer and enables
  permissive CORS. Do not expose it directly to untrusted networks — put it behind a
  reverse proxy / auth gateway, or bind it to localhost (the default `--bind` is
  `127.0.0.1`).
- **Untrusted query input.** User-supplied identifiers (column names, `--table`,
  `select`, `where`, `group_by`, `agg`) are validated against an
  alphanumeric/underscore allowlist, and string values are escaped, before being used
  in SQL. The `--table` identifier injection fixed in 0.1.0 is an example of the class
  of issue we treat as a vulnerability.
- **Resource limits.** Result sets are capped (default 1000 rows, hard cap 100000),
  but Terrana does not otherwise rate-limit requests.

If you find a way to bypass the input validation, read or write files outside the
configured source, or otherwise escape the intended read-only, single-file model,
that is a security issue — please report it as above.
