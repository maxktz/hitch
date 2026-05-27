<br>

<p align="center">
  <a name="readme-top"></a>
  <a href="https://github.com/maxktz/hitch">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="assets/logo-dark.svg">
      <source media="(prefers-color-scheme: light)" srcset="assets/logo-light.svg">
      <img alt="Hitch" src="assets/logo-light.svg" height="68">
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

Hitch is a lightweight CLI for sharing real terminal with AI coding agents. run `hitch` and agents get your terminals context, can run keys or commands, and inspect output.

Helps with agents running duplicate dev servers, tunnels, and having to copy-paste logs to your agent.

It is not a terminal multiplexer UI like `tmux`. Your terminal still feels like a normal shell; Hitch just proxies input/output, records useful context, and exposes agent-friendly commands.

## Install

```sh
npm install -g hitch-cli
hitch
```

Wizard on first run will help you setup `SKILL.md` with [skills.sh](https://skills.sh) CLI.

> Supported platforms: macOS and Linux on arm64 or x64.

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

> This is all a human will need! Rest of the commands built for agents.

## SKILL.md

It will help you install it on startup, but to reinstall or update run this:

```sh
hitch setup # recommended, uses 'npx skills'
# or
npx skills add https://github.com/maxktz/hitch skill hitch
```

> If you're curious how this works for the agent, run `hitch --skill` to see the skill content :D
