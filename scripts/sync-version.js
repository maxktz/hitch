#!/usr/bin/env node

const { readFileSync, writeFileSync } = require("node:fs");
const { join } = require("node:path");

const root = join(__dirname, "..");
const cargoPath = join(root, "Cargo.toml");
const packagePath = join(root, "package.json");
const skillPath = join(root, "SKILL.md");

const cargo = readFileSync(cargoPath, "utf8");
const version = cargo.match(/^version\s*=\s*"([^"]+)"/m)?.[1];

if (!version) {
  console.error("Could not read Cargo.toml package version");
  process.exit(1);
}

const pkg = JSON.parse(readFileSync(packagePath, "utf8"));
pkg.version = version;
writeFileSync(packagePath, `${JSON.stringify(pkg, null, 2)}\n`);

const skill = readFileSync(skillPath, "utf8");
const updatedSkill = skill.replace(/^version:\s*.*$/m, `version: ${version}`);
if (updatedSkill === skill && !/^version:/m.test(skill)) {
  console.error("Could not find SKILL.md version field");
  process.exit(1);
}
writeFileSync(skillPath, updatedSkill);

console.log(`synced version ${version}`);
