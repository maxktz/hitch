<br>

<p align="center">
  <a name="readme-top"></a>
  <a href="https://github.com/maxktz/hitch">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="assets/logo-dark.svg">
      <source media="(prefers-color-scheme: light)" srcset="assets/logo-light.svg">
      <img alt="Hitch" src="assets/logo-light.svg" height="72">
    </picture>
  </a>
</p>

<h3 align="center">Share your terminal with coding agents</h3>

<p align="center">
  Let agents inspect and control the terminals you already have running.
</p>

<p align="center">
  <a href="https://github.com/maxktz/hitch"><strong>GitHub</strong></a> ·
  <a href="https://www.npmjs.com/package/hitch-cli"><strong>NPM</strong></a> ·
  <a href="https://x.com/maxktz"><strong>Author</strong></a>
</p>

<p align="center">
  <a href="https://www.npmjs.com/package/hitch-cli"><img src="https://img.shields.io/npm/v/hitch-cli?style=flat-square&color=333" alt="npm version"></a>
  <a href="https://www.npmjs.com/package/hitch-cli"><img src="https://img.shields.io/npm/l/hitch-cli?style=flat-square&color=333" alt="License"></a>
  <a href="https://www.npmjs.com/package/hitch-cli"><img src="https://img.shields.io/npm/dt/hitch-cli?style=flat-square&color=333" alt="npm downloads"></a>
</p>

---

## What is Hitch?

Hitch is a small CLI for sharing real shell terminals with AI coding agents. Start Hitch in a terminal, then agents can see compact context, send keys, and inspect output without starting duplicate dev servers, watchers, tunnels, or REPLs.

It is not a terminal multiplexer UI. Your terminal still feels like a normal shell; Hitch just proxies input/output, records useful context, and exposes agent-friendly commands.

## Install

```sh
npm install -g hitch-cli
hitch
```

The first run installs shell integration. Restart existing terminals after setup so your shell picks it up.

Supported platforms: macOS and Linux on arm64 or x64.

## Usage

Start sharing the current terminal:

```sh
hitch
```

Stop sharing:

```sh
unhitch
# or
hitch off
```

Show shared terminals for the current project:

```sh
hitch context
```

Send input to a terminal:

```sh
hitch send-keys -t 1 "npm run dev" Enter
```

Wait for output to settle and print what changed:

```sh
hitch send-keys -t 1 --wait quiet:1s --tail 40 "npm test" Enter
```

Read a faithful terminal transcript:

```sh
hitch capture -t 1 -p
```

## Agents

Install the Codex/agent skill:

```sh
hitch setup skill
```

Agents should usually start with:

```sh
hitch context
```

Then use `send-keys` only when they intentionally want to interact with a terminal. Hitch refuses to send input into a running process by default unless `--force` is used.

## Release

Create a release commit and tag:

```sh
npm run release -- 0.1.1
git push origin main --tags
```

GitHub Actions builds native binaries, publishes `hitch-cli` to npm, and creates a GitHub release.
