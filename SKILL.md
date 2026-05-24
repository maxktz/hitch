# Hitch Terminal Sessions

Use this skill when a task may involve terminal state the user already has open: dev servers, test watchers, tunnels, REPLs, build processes, logs, or any long-running command.

`hitch` makes user terminals visible and controllable through lightweight sessions. Prefer reusing an existing matching session over starting duplicate processes.

Install this skill with:

```sh
hitch install-skill
```

## Discovery

Before starting any dev server, tunnel, watcher, or long-running process, run:

```sh
hitch list
```

Use structured output when you need to parse sessions programmatically:

```sh
hitch list --json
```

Use global discovery only when you intentionally need sessions outside the current project:

```sh
hitch list --all
```

`hitch list` is scoped to the current directory by default. A session matches if it was created in that directory tree or if its current shell/process directory is in that directory tree.

## Choosing A Session

Prefer sessions with:

- Matching `current dir`
- Relevant `active command`
- Recent output showing the server, watcher, REPL, or logs you need
- A recent `session last active` time

If a matching active process already exists, do not start a duplicate.

Use the exact numeric session id from `hitch list`. Do not guess ids.

## Interacting

Send keys or commands into a session:

```sh
hitch send-keys -t <session> "command" Enter
```

Common keys:

```sh
hitch send-keys -t <session> C-c
hitch send-keys -t <session> Enter
hitch send-keys -t <session> C-u
```

Read session output:

```sh
hitch capture-pane -t <session> -p
```

Join a session only when you need an interactive terminal view:

```sh
hitch join <session>
```

Leave a joined session:

```sh
hitch leave
```

`attach` and `detach` are supported aliases for agents familiar with tmux, but prefer `join` and `leave` in user-facing explanations.

## Rules

- Always run `hitch list` before starting long-running local processes.
- Reuse matching sessions instead of spawning duplicates.
- Use `hitch list --all` only for deliberate cross-project lookup.
- Prefer `send-keys` and `capture-pane` over opening a new terminal.
- Do not kill or interrupt a session unless the user asked or the task clearly requires it.
- If session output is ambiguous, inspect with `capture-pane` before acting.
