# Diagnostic export

Settings → **Export diagnostics…** writes a JSON file you can attach to a bug report. You review the contents first, and Ports writes only the file you choose. It keeps no copy and uploads nothing.

## Schema version 1

| Field        | Type           | Meaning                                                                                                          |
| ------------ | -------------- | ---------------------------------------------------------------------------------------------------------------- |
| `schema`     | number         | Version of this format. It changes when a field is removed or changes meaning; adding a field keeps the version. |
| `app`        | object         | `name` and `version` of Ports.                                                                                   |
| `platform`   | object         | `os` and `arch`.                                                                                                 |
| `exportedAt` | number         | Milliseconds since the Unix epoch.                                                                               |
| `excluded`   | string[]       | Groups left out on purpose: `commands`, `paths`, `settings`.                                                     |
| `discovery`  | object         | `complete`, `servers`, `withoutProcess`, `unmeasured`.                                                           |
| `servers`    | object[]       | One row per listener.                                                                                            |
| `settings`   | object or null | App settings, or `null` when excluded.                                                                           |
| `rules`      | object or null | Port range, shown kinds and counts of each rule, or `null` when excluded. Patterns and keys are never exported.  |

`discovery.complete` is `false` when Ports found a listener but could not identify its process. Each of those rows has `source` set to `unowned`.

### Server rows

`source` is `process`, `container` or `unowned`. A row also carries the port, addresses, kind, process name, status, health, framework and supervisor.

A value Ports could not read is `null`, and its name is listed in the row's `unavailable` array: `pid`, `memoryBytes`, `cpuPercent` or `command`. A value you chose to leave out is also `null`, but its group is in `excluded` and not in `unavailable`. That is how to tell "unknown" from "left out".

`command` is present only when you include commands. `folder` and `cwd` are present only when you include folders.

## What is never exported

- Environment variables and logs.
- Session details and saved project commands.
- Rule patterns, and the keys of hidden, protected, snoozed or watched servers.

## Redaction of commands

Before a command is exported or previewed, Ports:

- Replaces the value of `NAME=value` assignments.
- Replaces the value of flags whose name mentions a token, secret, password, passwd, pwd, key, auth or credential, in both the `--flag=value` and `--flag value` forms.
- Removes credentials from URLs, such as `postgres://user:password@host`.
- Replaces your home folder with `~`.

Redaction goes by these patterns, so it can miss a secret passed in an unusual way. Read the preview before you share a file that includes commands.
