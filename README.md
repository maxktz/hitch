# hitch

`hitch` gives AI coding agents visibility into terminals you already have running.

It shares lightweight shell terminals and lets agents inspect or interact with them before spawning duplicate dev servers, tunnels, watchers, or REPLs.

## Install

For now, build from source:

```sh
cargo install --git https://github.com/maxktz/hitch
```

NPM distribution is planned.

## Usage

Start sharing this terminal:

```sh
hitch
# or
hitch start
```

Install shell integration:

```sh
hitch setup shell
```

Show terminal state and compact context for the current project:

```sh
hitch context
```

Show more context for one terminal:

```sh
hitch context 1
```

Stop sharing this terminal:

```sh
hitch stop
```

Send input to a terminal:

```sh
hitch send-keys -t 1 "npm run dev" Enter
```

Send input, wait for output to settle, and print new output:

```sh
hitch send-keys -t 1 --wait quiet:2s --tail 40 "npm run dev" Enter
```

Read a faithful terminal transcript:

```sh
hitch capture -t 1 -p
```

Install the optional agent skill:

```sh
hitch setup skill
```

## Notes

- `hitch context` is scoped to the current directory by default.
- `hitch context --all` shows shared terminals across projects.
- `capture-pane`, `kill-session`, and `kill-sessions` are supported compatibility aliases.
- macOS is the primary target right now.
