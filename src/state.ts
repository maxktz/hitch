import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync } from "fs";
import { homedir } from "os";
import { dirname, join, resolve } from "path";
import { fileURLToPath } from "url";

export interface SessionRecord {
  id: string;
  cwd: string;
  socket: string;
  log: string;
  pidFile: string;
  masterPidFile?: string;
  createdAt: string;
  shell: string;
}

export function stateDir(): string {
  const stateHome = process.env.XDG_STATE_HOME || join(homedir(), ".local", "state");
  return join(stateHome, "muxi");
}

export function sessionsDir(): string {
  return join(stateDir(), "sessions");
}

export function ensureStateDirs() {
  mkdirSync(sessionsDir(), { recursive: true, mode: 0o700 });
}

export function nativeHelperPath(): string {
  const here = dirname(fileURLToPath(import.meta.url));
  return resolve(here, "..", "native", "muxi-pty", "muxi-pty");
}

export function sessionPath(id: string): string {
  return join(sessionsDir(), id);
}

export function readSessions(): SessionRecord[] {
  ensureStateDirs();
  return readdirSync(sessionsDir(), { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => readSession(entry.name))
    .filter((record): record is SessionRecord => record !== null)
    .filter((record) => {
      if (isSessionAlive(record)) return true;
      rmSync(sessionPath(record.id), { recursive: true, force: true });
      return false;
    });
}

export function readSession(id: string): SessionRecord | null {
  try {
    const raw = readFileSync(join(sessionPath(id), "session.json"), "utf8");
    return JSON.parse(raw) as SessionRecord;
  } catch {
    return null;
  }
}

export function isSocketAttached(socketPath: string): boolean {
  try {
    return Boolean(statSync(socketPath).mode & 0o100);
  } catch {
    return false;
  }
}

export function isSessionAlive(session: SessionRecord): boolean {
  return existsSync(session.socket);
}
