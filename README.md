# Dingen for the terminal

A terminal client for [Dingen](https://github.com/KleijnICT/dingen-app), a
self-hosted GTD task manager. It works with any Dingen server through the
sync protocol and keeps a local replica, so it works offline.

Status: early development, nothing to install yet. The core is there (step 2);
the terminal app itself is not started.

## Technology

Rust with ratatui, for macOS and Linux.

This repository also holds the `dingen-core` crate (storage, sync and
logic) in `crates/dingen-core`, shared with
[dingen-app-linux](https://github.com/KleijnICT/dingen-app-linux). Keep it
free of terminal code.

## The core

`dingen-core` passes **version 2** of the conformance test set (sync
protocol 4): all 492 cases in 13 suites. A copy of the set is in
`spec/conformance`; refresh it with `scripts/update-spec.sh` and raise
`CONFORMANCE_VERSION` in `crates/dingen-core/src/lib.rs` when it changes.

```sh
cargo test                      # the conformance set and the sync algorithm
cargo clippy --all-targets      # lint
```

One test runs against a real server, when you name one:

```sh
DINGEN_TEST_SERVER=http://localhost:8080 DINGEN_TEST_EMAIL=you@example.com \
  DINGEN_TEST_PASSWORD=... cargo test --test server
```

What is in it: the hybrid logical clock, ULIDs, order keys, calendar dates,
repeat rules, the lists and containers, the writes for every user action, the
strict entity shapes, a replica in SQLite (records, queue, cursor) with the
client algorithm of the protocol, and the HTTP client for signing in and
syncing.

## Rules

- The client talks to the server only through the sync protocol (version 4)
  and signs in with a token. There is no separate REST API.
- Behaviour is defined by the spec and the conformance test set in the main
  repository (`spec/protocol.md`, `spec/entities.md`,
  `spec/conformance/*.json`). Do not translate the TypeScript code by hand:
  build against the spec and make the test set pass. This repository records
  which version of the test set it passes.
- The design follows `DESIGN.md` in the main repository, within what is
  normal on the platform. Native conventions come first.
- Commit messages in English.

## Scope of version 1: daily use

In:

- Server URL and sign-in, including two-factor authentication
- Offline use with a local replica and sync
- Inbox, Today, Upcoming, Someday and Logbook
- Areas, projects and headings
- Creating and editing tasks: date, deadline, daypart, duration, repeat,
  checklist and tags
- Search, light and dark
- Reminders scheduled locally on the device

Not yet: review, calendar events, administration, push through a relay,
quick capture from other apps, location reminders.

## Steps

1. Skeleton: project, CI, an empty app that starts
2. Core: storage, sync and logic; the conformance test set passes
3. Reading: sign-in, all lists, areas and projects
4. Writing: create, edit, complete and reorder tasks
5. Finishing: search, local reminders, light and dark, first test build

Step 2 is done for the core. Steps 1 and 3 to 5 for the terminal app are open.

## License

GPL-3.0, see [LICENSE](LICENSE).
