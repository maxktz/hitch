#!/usr/bin/env node

const { chmodSync, copyFileSync, existsSync, mkdirSync, readdirSync, writeFileSync } = require("node:fs");
const { basename, join } = require("node:path");

const root = join(__dirname, "..");
const artifacts = join(root, "artifacts");
const native = join(root, "native");
const bin = join(root, "bin");

if (!existsSync(artifacts)) {
  console.error("artifacts/ directory not found; run this after actions/download-artifact");
  process.exit(1);
}

mkdirSync(native, { recursive: true });
mkdirSync(bin, { recursive: true });

for (const dirent of readdirSync(artifacts, { withFileTypes: true })) {
  if (!dirent.isDirectory() || !dirent.name.startsWith("hitch-")) {
    continue;
  }
  const name = dirent.name;
  const source = join(artifacts, name, "hitch");
  const target = join(native, basename(name));
  copyFileSync(source, target);
  chmodSync(target, 0o755);
  console.log(`added ${target}`);
}

const shim = join(bin, "hitch");
writeFileSync(
  shim,
  [
    "#!/bin/sh",
    'echo "hitch native binary was not installed correctly. Try reinstalling hitch-cli." >&2',
    "exit 1",
    "",
  ].join("\n"),
);
chmodSync(shim, 0o755);
console.log(`added ${shim}`);
