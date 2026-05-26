# hitch

`hitch` gives AI coding agents visibility into terminals you already have running.

It shares lightweight shell terminals and lets agents inspect or interact with them before spawning duplicate dev servers, tunnels, watchers, or REPLs.

## Install

Install with npm:

```sh
npm install -g hitch-cli
hitch
```

Or build from source:

```sh
cargo install --git https://github.com/maxktz/hitch
```

The first `hitch` run installs shell integration before sharing a terminal. Restart existing terminals after setup so your shell picks it up.

Supported npm platforms:

- macOS arm64 / x64
- Linux arm64 / x64

## Release

Create a release commit and tag:

```sh
npm run release -- 0.1.0
git push origin main --tags
```

GitHub Actions builds native Unix binaries, publishes `hitch-cli` to npm, and creates a GitHub release. Npm release binaries check for npm updates in the background using a 6 hour cache; local source builds do not check npm.

## Usage

Start sharing this terminal:

```sh
hitch
# or
hitch start
```

Run the setup wizard:

```sh
hitch setup
```

Install shell integration directly:

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

Show version:

```sh
hitch -v
```

## Notes

- `hitch context` is scoped to the current directory by default.
- `hitch context --all` shows shared terminals across projects.
- `hitch` automatically updates an existing Hitch shell integration block when needed.
- `capture-pane`, `kill-session`, and `kill-sessions` are supported compatibility aliases.
- Windows is not supported.
