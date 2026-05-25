---
name: hitch
description: Use when the `hitch` CLI (github.com/maxktz/hitch) is available and you need to inspect or control a shared terminal the user already has open — before starting a dev server, watcher, tunnel, REPL, build, or log tail (to avoid duplicates), or to read terminal output and send keys via `hitch list`, `capture`, and `send-keys`. Requires the hitch CLI; does not work standalone.
---

# Hitch

`hitch` lets you — the agent — inspect and control shared terminals the user already has open (dev servers, watchers, tunnels, REPLs, builds, log tails). Its `capture` and `send-keys` commands intentionally mirror the equivalent tmux workflows.

This is reference knowledge for you, not a script to run on sight. **With no concrete task, do nothing** — don't probe terminals or explain hitch to the user. Continue with whatever the user actually asked for.

## When to reach for it

The moment you're about to start a dev server, watcher, tunnel, REPL, build, or log tail, first check for an existing shared terminal instead of spawning a duplicate:

```sh
hitch list
```

Use `hitch list --all` only when you intentionally need terminals outside the current project. Plain `hitch list` is project-scoped.

If `hitch list` shows the needed server, watcher, tunnel, REPL, or log already running, use that terminal instead of starting a duplicate. Use the exact numeric id from `hitch list`.

Hitch terminal ids are always numbers. If the user says "hitch 2", "session 2", or "terminal 2", they probably mean hitch terminal id `2`.

Read output:

```sh
hitch capture -t <terminal> -p -S -100
```

Send a command to an idle shell:

```sh
hitch send-keys -t <terminal> C-u "command" Enter
```

When sending commands, prefer starting with `C-u`; it clears any partially typed prompt input before sending the command.

Before sending normal commands, check `hitch list`. If the terminal is actively running something, only interrupt/restart it when the user asked for that or it is clearly required.

Interrupt only when safe or requested:

```sh
hitch send-keys -t <terminal> C-c
```

If unsure, inspect output first:

```sh
hitch capture -t <terminal> -p -S -100
```

## Patterns

Restart a dev server in the terminal that was already running it:

```sh
hitch send-keys -t 2 C-c C-u "npm run dev" Enter
```

Inspect recent logs before acting:

```sh
hitch capture -t 2 -p -S -100
```

Run a command in an idle shell:

```sh
hitch send-keys -t 2 C-u "npm test" Enter
```
