# hitch

`hitch` gives AI coding agents visibility into terminal sessions you already have running.

It starts lightweight, resumable shell sessions and lets agents inspect or interact with them before spawning duplicate dev servers, tunnels, watchers, or REPLs.

## Install

For now, build from source:

```sh
cargo install --git https://github.com/maxktz/hitch
```

NPM distribution is planned.

## Usage

Start a hitch-backed shell:

```sh
hitch
```

List sessions visible from the current project:

```sh
hitch list
```

Join an existing session:

```sh
hitch join 1
```

Send input to a session:

```sh
hitch send-keys -t 1 "npm run dev" Enter
```

Read recent output:

```sh
hitch capture-pane -t 1 -p
```

Install the optional agent skill:

```sh
hitch install-skill
```

## Notes

- `hitch list` is scoped to the current directory by default.
- `hitch list --all` shows sessions across projects.
- `attach`, `detach`, `list-sessions`, and `list-panes` are supported compatibility aliases.
- macOS is the primary target right now.
