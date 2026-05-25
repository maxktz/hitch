---
name: hitch
description: Use when you need to inspect or control a terminal session the user already has open — before starting a dev server, watcher, tunnel, REPL, build, or log tail (to avoid duplicates), or to read pane output and send keys to an existing session. Covers hitch list, capture-pane, send-keys, and session ids.
---

# Hitch

Use `hitch` to inspect and control terminal sessions the user already has open.

You can reuse your experience with `tmux` for the agent-facing commands: `capture-pane` and `send-keys` intentionally mirror tmux-style workflows.

Before starting a dev server, watcher, tunnel, REPL, build, or log tail, check for an existing session:

```sh
hitch list
```

Use `hitch list --all` only when you intentionally need sessions outside the current project. Plain `hitch list` is project-scoped.

If `hitch list` shows the needed server, watcher, tunnel, REPL, or log already running, use that session instead of starting a duplicate. Use the exact numeric session id from `hitch list`.

Session ids are always numbers. If the user says "hitch 2", "session 2", or "terminal 2", they probably mean hitch session id `2`.

Read output:

```sh
hitch capture-pane -t <session> -p -S -100
```

Send a command to an idle shell:

```sh
hitch send-keys -t <session> C-u "command" Enter
```

When sending commands, prefer starting with `C-u`; it clears any partially typed prompt input before sending the command.

Before sending normal commands, check `hitch list`. If the session is actively running something, only interrupt/restart it when the user asked for that or it is clearly required.

Interrupt only when safe or requested:

```sh
hitch send-keys -t <session> C-c
```

If unsure, inspect output first:

```sh
hitch capture-pane -t <session> -p -S -100
```

Join interactively only when necessary:

```sh
hitch join <session>
```

## Patterns

Restart a dev server in the session that was already running it:

```sh
hitch send-keys -t 2 C-c C-u "npm run dev" Enter
```

Inspect recent logs before acting:

```sh
hitch capture-pane -t 2 -p -S -100
```

Run a command in an idle shell:

```sh
hitch send-keys -t 2 C-u "npm test" Enter
```
