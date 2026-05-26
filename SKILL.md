---
name: hitch
description: Use when you need to inspect or control user's own terminals, or if asked to do something with user's specific terminal, or before starting a dev server, watcher, tunnel, REPL, build, or log tail that may already be running by the user.
---

If skill activated with no concrete task provided - don't act, just remember it. This is reference knowledge for you on how to use hitch.

`hitch` CLI for agents to see and control shared terminal sessions that the user already has opened.

Useful for tasks when you need to collaborate with the user in the same terminal and you both want to interact with it. Like long running tasks, dev servers, watchers, tunnels, builds. Think of it like tmux but with proper agent tools.

## Command Reference

### `hitch context [TERMINAL]`

Use first. Shows compact terminal state and recent useful output.

Forms:

- `hitch context` - project terminals only
- `hitch context <terminal>` - one terminal, more detailed output, usually paired with specific --tail
- `hitch context --dir <dir>` - terminals whose origin or current dir is inside `<dir>`
- `hitch context --all` - all terminals, includes outside of the project.

Options:

- `--tail <n>` - recent visible output lines. Default: 20, or 80 for `context <terminal>`
- `--head <n>` - active process head lines. Default: 5, or 10 for `context <terminal>`

Output is summarized for agents: no colors, blank lines removed.

### `hitch send-keys -t <terminal> [OPTIONS] <keys...>`

Similar to how tmux send-keys works.
Sends input to a terminal.
If `--tail` or `--wait` is passed, it also prints new output produced after sending keys. (will not include previous history output)

Common keys:

- `Enter`
- `Tab`
- `Escape` / `Esc`
- `Space`
- `Backspace` / `BSpace`
- `C-c`, `C-u`, etc.

Options:

- `-t, --target <terminal>` - terminal id.
- `--wait output` - wait until new output appears.
- `--wait finish` - wait until the foreground command finishes.
- `--wait quiet:<duration>` - wait until output stops changing for duration, e.g. `quiet:2s`.
- `--wait time:<duration>` - wait fixed time before returning output, e.g. `time:5s`.
- `--timeout <duration>` - max wait time. Default: `30s`.
- `--tail <n>` - print this many new visible output lines after sending.
- `--force` - send input even if the terminal has a running process.

> Options must come before keys!

Durations support `ms`, `s`, `m`.

Examples:

- `hitch send-keys -t 1 C-u "npm test" Enter`
- `hitch send-keys -t 1 --wait quiet:2s --tail 40 C-u "npm run dev" Enter`
- `hitch send-keys -t 1 --wait finish --tail 80 C-u "npm test" Enter`

Output behavior:

- Without `--wait` or `--tail`, `send-keys` sends input but prints nothing.
- If `--wait` is set, Hitch waits, then prints new output from that terminal.
- If `--tail <n>` is set, Hitch prints up to `<n>` new visible output lines after sending.
- `--wait` without `--tail` defaults to printing up to 40 new visible output lines, so if you expect an output shorter than 40 lines, passing --tail is not necessary

--wait is made to be used instead of commands like "sleep", to not be polling the terminal yourself, prefer using it if needed.

While waiting until finish of command, and expect it to be longer than 30s, pass custom --timeout. But usually you may not want to not wait such longer for processes. Or you may choose to wait for longer commands on your own, by polling output tail after a while.

### `hitch capture -t <terminal> [OPTIONS]`

Mirrors tmux capture-pane behavior. To capture one terminal, prefer `context <terminal>` over `capture`; use `capture` only if more options needed

Also works with alias `hitch capture-pane`.

Options:

- `-t <terminal>` - terminal id.
- `-p` - print to stdout, tmux-compatible.
- `-S <start>` - start line. Negative values count from the end, e.g. `-S -100`.
- `-E <end>` - end line.
- `-e` - raw/escape output, tmux-compatible.

Accepted compatibility no-op flags:

- `-C`, `-J`, `-N`, `-T`, `-a`, `-q`

## Rules

- Terminal ids are numbers. If user says "hitch 2", "terminal 2", or "session 2", they likely mean terminal id `2`.
- Before interrupting user's processes with `C-c`, make sure the user asked for it or it is clearly necessary.
- Do not send shell commands to terminals with running processes unless you intend to interact with that process. Hitch refuses this by default and prints terminal context; use `--force` only when intentional. A sequence starting with `C-c` is allowed because it interrupts the running process first.
