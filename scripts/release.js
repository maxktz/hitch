#!/usr/bin/env node

const { execFileSync } = require("node:child_process");
const { readFileSync, writeFileSync } = require("node:fs");
const { join } = require("node:path");

const version = process.argv[2];
if (!version || !/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) {
  console.error("usage: npm run release -- <version>");
  console.error("example: npm run release -- 0.1.1");
  process.exit(1);
}

const root = join(__dirname, "..");
const cargoPath = join(root, "Cargo.toml");
const cargo = readFileSync(cargoPath, "utf8");
writeFileSync(
  cargoPath,
  cargo.replace(/^version\s*=\s*"[^"]+"/m, `version = "${version}"`),
);

execFileSync("node", [join(root, "scripts", "sync-version.js")], {
  cwd: root,
  stdio: "inherit",
});

execFileSync("cargo", ["build"], { cwd: root, stdio: "inherit" });

execFileSync("git", ["add", "Cargo.toml", "Cargo.lock", "package.json", "SKILL.md"], {
  cwd: root,
  stdio: "inherit",
});
execFileSync("git", ["commit", "-m", `Release v${version}`], {
  cwd: root,
  stdio: "inherit",
});
execFileSync("git", ["tag", `v${version}`], { cwd: root, stdio: "inherit" });

console.log("");
console.log(`created release commit and tag v${version}`);
console.log("push with:");
console.log("  git push origin main --tags");
