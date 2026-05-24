import { execFileSync } from "child_process";
import { existsSync, readFileSync, statSync } from "fs";
import { resolve } from "path";
import chalk from "chalk";
import {
  isSessionAlive,
  isSocketAttached,
  readSessions,
  type SessionRecord,
} from "./state.js";

interface ListOptions {
  dir?: string;
  json?: boolean;
  head: string;
  tail: string;
}

function timeAgo(ms: number): string {
  const diff = Math.max(0, Math.floor((Date.now() - ms) / 1000));
  if (diff < 60) return `${diff}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}

function readLines(path: string, limit: number): string[] {
  if (!existsSync(path)) return [];
  const raw = readFileSync(path, "utf8");
  return stripAnsi(raw)
    .split("\n")
    .map((line) => line.replace(/\r/g, ""))
    .filter((line) => line.trim())
    .slice(-limit);
}

function stripAnsi(value: string): string {
  return value
    .replace(/\x1B\][^\x07]*(?:\x07|\x1B\\)/g, "")
    .replace(/\x1B[78=<>]/g, "")
    .replace(/\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~])/g, "")
    .replace(/[\x00-\x08\x0B\x0C\x0E-\x1F\x7F]/g, "");
}

function lastActivity(session: SessionRecord): number {
  try {
    return statSync(session.log).mtimeMs;
  } catch {
    return Date.parse(session.createdAt);
  }
}

function readPid(session: SessionRecord): number | null {
  try {
    const pid = Number(readFileSync(session.pidFile, "utf8").trim());
    return Number.isFinite(pid) ? pid : null;
  } catch {
    return null;
  }
}

function commandForPid(pid: number | null, fallback: string): string {
  if (!pid) return fallback;
  try {
    const output = execFileSync("ps", ["-p", String(pid), "-o", "command="], {
      encoding: "utf8",
    });
    return output.trim() || fallback;
  } catch {
    return fallback;
  }
}

function toResult(session: SessionRecord, tailN: number) {
  const alive = isSessionAlive(session);
  const attached = isSocketAttached(session.socket);
  const pid = readPid(session);
  return {
    session: session.id,
    dir: session.cwd,
    latestCommand: commandForPid(pid, session.shell),
    running: alive,
    attached,
    pid,
    lastActivity: timeAgo(lastActivity(session)),
    output: {
      tail: readLines(session.log, tailN),
    },
  };
}

export function listSessions(options: ListOptions) {
  let sessions = readSessions();
  if (options.dir) {
    const filterDir = resolve(options.dir);
    sessions = sessions.filter((session) => session.cwd.startsWith(filterDir));
  }

  sessions.sort((a, b) => lastActivity(b) - lastActivity(a));

  const tailN = Number.parseInt(options.tail, 10) || 5;

  if (options.json) {
    console.log(JSON.stringify(sessions.map((session) => toResult(session, tailN)), null, 2));
    return;
  }

  if (!sessions.length) {
    console.log("No matching muxi sessions found.");
    return;
  }

  for (const session of sessions) {
    const result = toResult(session, tailN);
    const attachedTag = result.attached ? " (attached)" : "";
    const status = result.running ? "running" : "exited";

    console.log(chalk.dim(`-- Session ${result.session}${attachedTag} ${"-".repeat(30)}`));
    console.log(`Dir: ${result.dir}`);
    console.log(`Command: ${result.latestCommand}`);
    console.log(`Status: ${status}`);
    console.log(`Last activity: ${result.lastActivity}`);

    if (result.output.tail.length > 0) {
      console.log("");
      console.log(chalk.dim(`--- output tail (${result.output.tail.length} lines) ---`));
      result.output.tail.forEach((line) => console.log(`  ${line}`));
    }

    console.log("");
  }
}
