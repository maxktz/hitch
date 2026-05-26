# Stateful Context Idea

Future concept, not current product behavior.

## Problem

Agents often repeat this loop:

1. `hitch context`
2. `hitch send-keys ...`
3. wait or sleep
4. `hitch context` again

The second context call may repeat output the agent already saw, while missing the fact that another terminal produced new output.

## Possible Direction

Keep `hitch context` stateless by default.

Later, optionally add explicit stateful cursors:

```sh
hitch context --agent codex --since-last
hitch context --agent claude --since-last
```

Hitch could store per-agent read cursors for each terminal output log and report only what changed since that agent last asked.

Example output:

```text
terminals: 2 project, 3 total

----- terminal 1 -----
current dir: ~/dev/app
last input was 4s ago
process is running for 12s

--- new output since last context (18 lines) ---
...

----- terminal 2 -----
current dir: ~/dev/app
last input was 8m ago
no actively running commands
no new output since last context
```

## Design Constraints

- Must be opt-in, not default.
- Must require an explicit agent id; do not guess the current agent.
- Must not hide important terminal state just because output is not new.
- Must work with multiple agents/chats safely.
- Must have a way to reset cursors.
- Must not replace `capture`, which remains the faithful inspection command.

## Related Ideas

`send-keys` could later support wait modes and return new output directly:

```sh
hitch send-keys -t 1 "pnpm dev" Enter --wait quiet:2s --tail 40
```

Possible wait modes:

- `--wait none`: current behavior.
- `--wait output`: return after first new output.
- `--wait quiet:2s`: return after output has been quiet for 2 seconds.
- `--wait time:5s`: wait a fixed time.
- `--wait exit`: wait until foreground process exits.
- `--timeout 30s`: maximum wait.

This may solve most repeated context calls without requiring stateful context.
