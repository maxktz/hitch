#!/usr/bin/env node

const { copyFileSync, chmodSync, mkdirSync } = require("node:fs");
const { join } = require("node:path");

const platform = process.platform;
const arch = process.arch;

if (platform !== "darwin") {
  console.error(`hitch-cli currently supports macOS only, got ${platform}-${arch}`);
  process.exit(1);
}

const root = join(__dirname, "..");
const source = join(root, "target", "release", "hitch");
const nativeDir = join(root, "native");
const target = join(nativeDir, `hitch-${platform}-${arch}`);

mkdirSync(nativeDir, { recursive: true });
copyFileSync(source, target);
chmodSync(target, 0o755);
console.log(`prepared ${target}`);
