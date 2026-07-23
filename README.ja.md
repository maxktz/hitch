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

<p align="center">
  <a href="README.md">English</a> · <a href="README.zh-CN.md">简体中文</a> · <strong>日本語</strong>
</p>

<h3 align="center">コーディングエージェントとターミナルを共有</h3>

<p align="center">
  すでに実行中のターミナルをエージェントが確認し、操作できるようにします。
</p>

<p align="center">
  <a href="https://github.com/maxktz/hitch"><strong>GitHub</strong></a> ·
  <a href="https://www.npmjs.com/package/hitch-cli"><strong>NPM</strong></a> ·
  <a href="https://x.com/maxktz"><strong>作者</strong></a>
</p>

<p align="center">
  <a href="https://www.npmjs.com/package/hitch-cli"><img src="https://img.shields.io/npm/v/hitch-cli?style=flat-square&color=333" alt="npm バージョン"></a>
  <a href="https://www.npmjs.com/package/hitch-cli"><img src="https://img.shields.io/npm/l/hitch-cli?style=flat-square&color=333" alt="ライセンス"></a>
  <a href="https://www.npmjs.com/package/hitch-cli"><img src="https://img.shields.io/npm/dt/hitch-cli?style=flat-square&color=333" alt="npm ダウンロード数"></a>
</p>

<p align="center">
  <img alt="Hitch のプレビュー" src="assets/preview.jpg" width="900">
</p>

---

## Hitch とは？

Hitch は、実際のターミナルを AI コーディングエージェントと共有するための軽量 CLI です。`hitch` を実行すると、エージェントはターミナルのコンテキストを取得し、キー入力やコマンドを実行して、出力を確認できるようになります。

エージェントが開発サーバーやトンネルを重複して起動することや、ログをエージェントへコピー＆ペーストする手間を減らします。

`tmux` のようなターミナルマルチプレクサ UI ではありません。ターミナルは通常のシェルとしてそのまま使えます。Hitch は入出力を中継し、有用なコンテキストを記録して、エージェント向けのコマンドを公開するだけです。

## インストール

```sh
npm install -g hitch-cli
hitch
```

初回実行時のウィザードが、[skills.sh](https://skills.sh) CLI を使用して `SKILL.md` の設定を支援します。

> 対応プラットフォーム：arm64 または x64 の macOS と Linux。

## 使い方

現在のターミナルの共有を開始：

```sh
hitch
```

共有を停止：

```sh
unhitch
# or
hitch off
```

> ユーザーが必要とする操作はこれだけです！残りのコマンドはエージェント向けに用意されています。

## SKILL.md

このファイルは起動時の Hitch インストールをエージェントに案内します。再インストールまたは更新するには、次を実行してください：

```sh
hitch setup # recommended, uses 'npx skills'
# or
npx skills add https://github.com/maxktz/hitch skill hitch
```

> エージェント側での仕組みに興味がある場合は、`hitch --skill` を実行して内容を確認してください :D
