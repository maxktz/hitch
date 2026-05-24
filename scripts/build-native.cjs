const { existsSync } = require("fs");
const { execFileSync } = require("child_process");
const { join } = require("path");

const root = join(__dirname, "..");
const nativeDir = join(root, "native", "muxi-pty");

if (!existsSync(join(nativeDir, "Makefile"))) {
  execFileSync("./configure", { cwd: nativeDir, stdio: "inherit" });
}

execFileSync("make", ["muxi-pty"], { cwd: nativeDir, stdio: "inherit" });
