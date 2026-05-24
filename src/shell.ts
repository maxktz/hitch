import { spawnSync } from "child_process";
import { existsSync, mkdirSync, rmSync, writeFileSync } from "fs";
import { basename, join } from "path";
import { ensureStateDirs, nativeHelperPath, sessionPath, type SessionRecord } from "./state.js";

function slug(value: string): string {
  return value
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 32);
}

function createSessionId(cwd: string): string {
  const prefix = slug(basename(cwd)) || "session";
  const stamp = new Date()
    .toISOString()
    .replace(/[-:]/g, "")
    .replace(/\..+/, "")
    .replace("T", "-");
  return `${prefix}-${stamp}-${process.pid}`;
}

export function shell() {
  if (process.env.MUXI_SESSION) {
    console.error(`Already inside muxi session ${process.env.MUXI_SESSION}.`);
    process.exit(1);
  }

  ensureStateDirs();

  const cwd = process.cwd();
  const id = createSessionId(cwd);
  const dir = sessionPath(id);
  mkdirSync(dir, { recursive: true, mode: 0o700 });

  const shellPath = process.env.SHELL || "/bin/zsh";
  const record: SessionRecord = {
    id,
    cwd,
    socket: join(dir, "session.sock"),
    log: join(dir, "output.log"),
    pidFile: join(dir, "child.pid"),
    masterPidFile: join(dir, "master.pid"),
    createdAt: new Date().toISOString(),
    shell: shellPath,
  };
  writeFileSync(join(dir, "session.json"), JSON.stringify(record, null, 2));

  const helper = nativeHelperPath();
  if (!existsSync(helper)) {
    console.error(`Native helper not found at ${helper}. Run \`pnpm run build\`.`);
    process.exit(1);
  }

  console.log(`muxi: joined session ${id}`);
  console.log("muxi: detach with Ctrl-\\");

  const result = spawnSync(helper, ["-A", record.socket, "-z", "-r", "none", shellPath], {
    stdio: "inherit",
    env: {
      ...process.env,
      MUXI: "1",
      MUXI_SESSION: id,
      MUXI_SOCKET: record.socket,
      MUXI_LOG: record.log,
      MUXI_PID_FILE: record.pidFile,
      MUXI_MASTER_PID_FILE: record.masterPidFile,
    },
  });

  if (result.error) {
    console.error(`Failed to start muxi shell: ${result.error.message}`);
    process.exit(1);
  }

  rmSync(dir, { recursive: true, force: true });
  process.exit(result.status ?? 0);
}
