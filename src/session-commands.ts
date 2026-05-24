import { execFileSync, spawnSync } from "child_process";
import { existsSync, readFileSync, rmSync } from "fs";
import { join } from "path";
import {
  nativeHelperPath,
  readSession,
  readSessions,
  sessionPath,
  type SessionRecord,
} from "./state.js";

interface TargetOptions {
  target?: string;
}

interface CaptureOptions extends TargetOptions {
  print?: boolean;
  tail?: string;
  raw?: boolean;
}

function findSession(id: string | undefined): SessionRecord {
  if (!id) {
    throw new Error("Missing target session. Use -t <session>.");
  }

  const exact = readSession(id);
  if (exact) return exact;

  const matches = readSessions().filter((session) => session.id.startsWith(id));
  if (matches.length === 1) return matches[0];
  if (matches.length > 1) {
    throw new Error(`Ambiguous session "${id}". Matches: ${matches.map((s) => s.id).join(", ")}`);
  }
  throw new Error(`No such muxi session: ${id}`);
}

function runHelper(args: string[], session: SessionRecord, input?: string): number {
  const result = spawnSync(nativeHelperPath(), args, {
    stdio: input === undefined ? "inherit" : ["pipe", "inherit", "inherit"],
    input,
    env: {
      ...process.env,
      MUXI_SESSION: session.id,
      MUXI_SOCKET: session.socket,
      MUXI_LOG: session.log,
      MUXI_PID_FILE: session.pidFile,
    },
  });

  if (result.error) {
    throw result.error;
  }

  return result.status ?? 0;
}

function keyToBytes(key: string): string {
  if (key === "Enter" || key === "C-m") return "\n";
  if (key === "Tab") return "\t";
  if (key === "Escape" || key === "Esc") return "\x1b";
  if (key === "Space") return " ";
  if (key === "Backspace" || key === "BSpace") return "\x7f";

  const ctrl = key.match(/^C-(.)$/);
  if (ctrl) {
    return String.fromCharCode(ctrl[1].toUpperCase().charCodeAt(0) & 0x1f);
  }

  return key;
}

function stripAnsi(value: string): string {
  return value
    .replace(/\x1B\][^\x07]*(?:\x07|\x1B\\)/g, "")
    .replace(/\x1B[78=<>]/g, "")
    .replace(/\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~])/g, "")
    .replace(/[\x00-\x08\x0B\x0C\x0E-\x1F\x7F]/g, "");
}

function readPid(path: string | undefined): number | null {
  if (!path) return null;
  try {
    const pid = Number(readFileSync(path, "utf8").trim());
    return Number.isFinite(pid) ? pid : null;
  } catch {
    return null;
  }
}

export function attachSession(id: string) {
  const session = findSession(id);
  process.exit(runHelper(["-a", session.socket, "-z", "-r", "none"], session));
}

export function sendKeys(keys: string[], options: TargetOptions) {
  const session = findSession(options.target);
  const payload = keys.map(keyToBytes).join("");
  process.exit(runHelper(["-p", session.socket], session, payload));
}

export function capturePane(options: CaptureOptions) {
  const session = findSession(options.target);
  if (!existsSync(session.log)) return;

  const raw = readFileSync(session.log, "utf8");
  const text = options.raw ? raw : stripAnsi(raw).replace(/\r/g, "");
  const tail = Number.parseInt(options.tail || "", 10);
  const output = Number.isFinite(tail) && tail > 0
    ? text.split("\n").slice(-tail).join("\n")
    : text;

  process.stdout.write(output.endsWith("\n") ? output : `${output}\n`);
}

export function killSession(id: string) {
  const session = findSession(id);
  const childPid = readPid(session.pidFile);
  const masterPid = readPid(session.masterPidFile);

  for (const pid of [masterPid, childPid]) {
    if (!pid) continue;
    try {
      process.kill(pid, "SIGTERM");
    } catch {
      // The process may already be gone.
    }
  }

  rmSync(sessionPath(session.id), { recursive: true, force: true });
}

export function currentSessionInfo() {
  if (!process.env.MUXI_SESSION) {
    console.log("Not inside a muxi session.");
    process.exit(1);
  }

  console.log(`Session: ${process.env.MUXI_SESSION}`);
  if (process.env.MUXI_SOCKET) console.log(`Socket: ${process.env.MUXI_SOCKET}`);
}
