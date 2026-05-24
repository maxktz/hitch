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
any dev server / tunnel / long-running process, to run `muxi list --dir .`,
check for an existing matching process, reuse it if present, and interact with
existing sessions via tmux directly (`send-keys`, etc.) rather than spawning
duplicates.

## Architecture

A small Node.js + TypeScript CLI, built with `commander` and `chalk`.

| File | Responsibility |
|------|----------------|
| `src/index.ts` | CLI entry point. Wires up `list`, `init`, `exit` commands. |
| `src/init.ts`  | Emits the `.zshrc` shell snippet (the tmux-wrapping guard + per-session settings). |
| `src/tmux.ts`  | tmux interaction layer: enumerate panes, capture output, resolve full command via process tree. |
| `src/list.ts`  | The `list` command: enrich each pane, format pretty/JSON output, time-ago formatting. |
| `prompt.md`    | Canonical agent prompt, copied into agent config files. |

### `src/tmux.ts` internals

- `getPanes()` — `tmux list-panes -a -F <fields>` pulling session_activity,
  session_name, session_attached, pane_id, pane_current_path,
  pane_current_command, pane_pid.
- `getFullCommand(shellPid)` — `ps -e -o pid=,ppid=,args=` to find the child
  process of the shell and recover the full command line with arguments.
- `capturePaneOutput(paneId)` — `tmux capture-pane -t <id> -p -S -` for full
  scrollback.

### `src/list.ts` internals

- Sorts panes by activity, most recent first.
- `extractCommandOutput()` — finds the last prompt line (`❯`, `$`, `▶`, `%`)
  and returns the output after it.
- `extractLastCommand()` — pulls the command text from the last prompt line
  (used for idle shells).
- `enrichPane()` — combines process-tree inspection with scrollback analysis.
- `timeAgo()` — `<60s` → "Xs ago", `<1h` → "Xm ago", `<1d` → "Xh ago",
  else "Xd ago".

## Design Principles (learned the hard way)

1. **It must feel like a normal terminal.** If the user ever has to think "oh
   right, I'm in tmux," the abstraction has failed. No visible tmux chrome, no
   keybinding surprises, no lag.

2. **Don't hijack real tmux.** Use per-session inline settings, never global
   `~/.tmux.conf`. People who actually use tmux must be unaffected.

3. **Always provide an escape hatch.** `muxi exit` + `MUXI_DISABLED`.

4. **Agents prefer fewer flags.** A solution that requires the agent to remember
   `-L muxi` on every tmux command was rejected — the friction defeats the
   purpose. The tool should "just work" on the default socket.

5. **Change one thing at a time.** The status-bar work repeatedly broke terminal
   opening. The working config must always be preserved, and changes made
   minimally and tested before moving on.

## Status

**Working well:**

- ✅ Core `muxi list` — confirmed excellent with real-world multi-terminal
  dev workflows.
- ✅ Invisible tmux auto-wrapping via `eval "$(muxi init)"`.
- ✅ Full-command resolution (shows `bun dev`, not `bun`).
- ✅ Monorepo-aware `--dir` filtering.
- ✅ Agent prompt integration for Claude Code, Codex, OpenCode.
- ✅ `muxi exit` escape hatch.
- ✅ Status bar badge: `● muxi` shown in green on the right.

**Open problems:**

- ⚠️ The default tmux window list (`0:zsh*`) still shows on the left of the
  status bar. Every attempt to hide it (setting `window-status-format` /
  `window-status-current-format` to empty, via the `\;` chain, `source-file`,
  or a second `.zshrc` pass) has broken terminal opening. A safe approach is
  still needed.
- ⚠️ Desired final badge: `● muxi [0]` (fold the session number in) with
  nothing else in the bar.

## Future

- Make the project public on npm.
- bash support (currently zsh-focused).
- Possibly extend agent integrations and refine the badge once the window-list
  hiding is solved safely.
