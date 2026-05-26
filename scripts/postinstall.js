#!/usr/bin/env node

const { chmodSync, copyFileSync, existsSync } = require("node:fs");
const { join } = require("node:path");

const root = join(__dirname, "..");
const platform = process.platform;
const arch = process.arch;
const source = join(root, "native", `hitch-${platform}-${arch}`);
const target = join(root, "bin", "hitch");

if (!existsSync(source)) {
  console.error("");
  console.error(`hitch does not include a binary for ${platform}-${arch}`);
  console.error("Supported package binaries are in the native/ directory.");
  console.error("");
  process.exit(1);
}

copyFileSync(source, target);
chmodSync(target, 0o755);

console.log("");
console.log("hitch installed");
console.log("");
console.log("Run setup to finish installation:");
console.log("  hitch setup");
console.log("");
