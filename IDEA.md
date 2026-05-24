# muxi

> Give AI coding agents visibility into the terminals you already have running.

## The Problem

AI coding agents (Claude Code, Codex, OpenCode, Cursor, etc.) are blind to the
terminals a developer already has open. They can run commands, but they have no
idea what's *already running* on the machine.

In practice this causes constant friction:

- You have a dev server running in one terminal. The agent doesn't know, so it
  starts a **second** one — port conflict, or a duplicate process silently
  eating resources.
- You have a tunnel (ngrok/cloudflared), a watcher, a database, a log tail
  running across **four** different terminal tabs for a single project. The
  agent can't see any of them, so it can't reason about the system it's
  supposed to be helping with.
- The agent kills or restarts something, not realizing you were watching its
  output in another pane.
- You end up manually copy-pasting "here's what's running" context into the
  agent over and over.

The fundamental gap: **the agent's view of the world stops at the commands it
personally ran.** Everything you're running in your own terminals is invisible
to it.

## The Idea

`muxi` makes every terminal you open part of a shared, inspectable layer
that agents can read.

Two halves:

1. **Invisible capture.** Every new terminal window is automatically wrapped in
   a tmux session — but it's made to feel exactly like a normal terminal. You
   don't think about tmux. You don't see tmux. You just open terminals and use
   them like always.

2. **Agent-readable context.** Because every terminal now lives inside tmux,
   a single command — `muxi list` — can enumerate all of them and report
   rich context for each: the working directory, the command currently running
   (with full arguments), whether it's still running or finished, how recently
   it was active, and a head/tail snippet of its output.

Agents are told (via their global instruction files) to run `muxi list`
before starting any long-running process. So instead of blindly spawning a
duplicate dev server, the agent first looks, sees "there's already a `bun dev`
running in this directory, last active 3s ago," and uses it.

The result, in the user's words: *"I literally sometimes have four different
terminals to just run a single project in development, and now my agents manage
all that — I don't even touch it."*

## How It Works

### 1. Invisible tmux wrapping

A shell snippet is added to `.zshrc` via:

```sh
eval "$(muxi init)"
```

`muxi init` prints a guard that, for every new interactive shell, replaces
itself with a tmux session:

```sh
if [ -z "$TMUX" ] && [ -z "$MUXI_DISABLED" ] && [ -z "$INSIDE_EMACS" ] \
   && [ -z "$VSCODE_INJECTION" ] && [[ "$TERM_PROGRAM" != "vscode" ]]; then
  exec tmux new-session <per-session settings> || true
fi
```

Key design points:

- **`exec`** replaces the shell process with tmux. This is deliberate: when the
  tmux session exits, the terminal window closes — just like a real shell would.
  The tmux session genuinely *is* the terminal, not a thing layered on top of it.
- **Guards** prevent double-wrapping and stay out of the way of environments
  that manage their own shells: existing tmux sessions (`$TMUX`), the escape
  hatch (`$MUXI_DISABLED`), Emacs, and VS Code's integrated terminal.
- **`|| true`** keeps a non-zero tmux exit from cascading into the terminal and
  causing it to close unexpectedly. (This was the fix for an early
  terminal-crash bug.)
- **Per-session settings**, not global config. The settings are passed inline on
  the `new-session` command (`\; set ...` chain) rather than written to
  `~/.tmux.conf`. This is intentional: people who use tmux for real should not
  have their global tmux behavior hijacked by muxi.

The per-session settings make tmux feel like a plain terminal:

- `set mouse on` — scrolling and selection work like a normal terminal
- `set escape-time 0` — no input lag on the Esc key
- `set history-limit 50000` — deep scrollback
- A minimal status bar (see below)

### 2. The escape hatch

Sometimes you genuinely want a raw terminal — to use your native terminal
features, or to launch your own custom tmux setup. `muxi exit` handles this:

```sh
tmux detach -E "MUXI_DISABLED=1 zsh"
```

This detaches the proxy and drops you into a plain shell with
`MUXI_DISABLED=1` set, so the `.zshrc` guard won't immediately re-wrap you.

### 3. Reading the sessions: `muxi list`

This is the payload — the command agents actually call.

For every pane across every tmux session it reports:

- **Session name** and whether a human is currently attached ("human watching")
- **Directory** (tilde-shortened)
- **Latest command** — the full command line *with arguments*, not just the
  process name. tmux's `pane_current_command` only gives you `bun`; muxi
  walks the process tree via `ps` to recover the real `bun dev`.
- **Status** — whether that command is still running or has finished
- **Activity** — how long ago the session was last active ("3s ago", "2m ago",
  "1h ago", "4d ago")
- **Output** — a configurable head/tail snippet of the pane's scrollback,
  extracted intelligently by locating the last shell prompt

Flags:

- `--dir <path>` — filter to a directory **and its subdirectories**
  (`startsWith` matching, so it works inside monorepos)
- `--json` — structured output for programmatic consumption
- `--head <n>` / `--tail <n>` — how much output to show (default 5/5)

### 4. Telling the agents to use it

The whole thing only works if agents actually *look* before they leap. So
muxi's prompt instructions are installed into the global instruction files
of the major agents:

- Claude Code → `~/.claude/CLAUDE.md`
- Codex → `~/.codex/AGENTS.md`
- OpenCode → `~/.config/opencode/AGENTS.md`

The canonical prompt lives in `prompt.md`. It instructs agents, before starting
any dev server / tunnel / long-running process, to run `muxi list`, check for
an existing local matching process, reuse it if present, and interact with
existing sessions via muxi commands (`send-keys`, `capture-pane`, etc.) rather
than spawning duplicates. `muxi list --all` is available when the agent
intentionally needs sessions from other projects.

## Architecture

A single Rust CLI that owns terminal proxying, session metadata, agent
inspection, and input forwarding.

| File | Responsibility |
|------|----------------|
| `src-rs/main.rs` | Native CLI, PTY broker, attach loop, session registry, output capture, and tmux-like agent commands. |
| `Cargo.toml` | Rust package definition and binary target. |
| `prompt.md`    | Canonical agent prompt, copied into agent config files. |

### Rust internals

- `muxi` creates a session registry entry, forks a PTY-backed shell, and
  attaches the current terminal as a client.
- Sessions use short global numeric IDs (`1`, `2`, `3`, ...), while `muxi list`
  defaults to the current directory so local sessions stay prioritized.
- A Unix-domain socket carries a small packet protocol for attach, detach,
  resize, and pushed input.
- PTY output is streamed to attached clients and appended to a session log for
  `capture-pane` and `list`.
- `send-keys` writes tmux-style key tokens into the session socket.
- When the last attached terminal detaches, the session owner terminates the
  child shell and removes the socket, so `list` only shows actively used
  terminals.

## Design Principles (learned the hard way)

1. **It must feel like a normal terminal.** If the user ever has to think "oh
   right, I'm in muxi," the abstraction has failed. No visible chrome, no
   keybinding surprises beyond detach, no lag.

2. **Don't hijack real terminal tools.** muxi should not depend on the user's
   tmux sessions, tmux config, shell prompt, or IDE terminal settings.

3. **Always provide an escape hatch.** `Ctrl-\` detaches and the MVP destroys
   sessions when the last terminal detaches.

4. **Agents prefer familiar commands.** muxi keeps tmux-like command names such
   as `send-keys`, `capture-pane`, `list-sessions`, and `list-panes`, but they
   operate on muxi sessions.

5. **Change one thing at a time.** The status-bar work repeatedly broke terminal
   opening. The working config must always be preserved, and changes made
   minimally and tested before moving on.

## Status

**Working well:**

- ✅ Rust native CLI builds and runs as the package binary.
- ✅ `muxi` starts a transparent PTY-backed shell.
- ✅ `muxi list` shows actively attached local muxi terminals.
- ✅ `muxi list --all` shows active sessions across projects.
- ✅ `muxi send-keys` can send commands into a live terminal.
- ✅ `muxi capture-pane` reads recent captured output.
- ✅ Monorepo-aware `--dir` filtering.
- ✅ Startup message shows the session id and detach key.
- ✅ Sessions disappear when the last terminal detaches.

**Open problems:**

- ⚠️ `capture-pane` is log-based, not a full terminal screen reconstruction.
- ⚠️ Captured prompt output can still contain some terminal-control leftovers.

## Future

- Add proper binary release/install flow.
- Improve output cleanup or add a real terminal-screen model.
- Add stronger process/current-command detection.
- Consider optional prompt integration later; startup message and `muxi info`
  are enough for the MVP.
