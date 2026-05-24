import { resolve } from "path";
import { homedir } from "os";
import chalk from "chalk";
import {
  isTmuxRunning,
  getPanes,
  getFullCommand,
  capturePaneOutput,
  type Pane,
} from "./tmux.js";

interface ListOptions {
  dir?: string;
  json?: boolean;
  head: string;
  tail: string;
}

const SHELLS = new Set(["zsh", "bash", "fish", "sh", "dash"]);

function timeAgo(epochSeconds: number): string {
  const diff = Math.floor(Date.now() / 1000) - epochSeconds;
  if (diff < 60) return `${diff}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}

function shortenHome(p: string): string {
  const home = homedir();
  return p.startsWith(home) ? "~" + p.slice(home.length) : p;
}

function extractCommandOutput(rawOutput: string): string[] {
  const lines = rawOutput.split("\n");
  const promptPatterns = [/❯\s/, /\$\s/, /▶\s/, /%\s/];

  let lastPromptIndex = -1;
  for (let i = lines.length - 1; i >= 0; i--) {
    if (!lines[i].trim()) continue;
    if (promptPatterns.some((p) => p.test(lines[i]))) {
      lastPromptIndex = i;
      break;
    }
  }

  if (lastPromptIndex === -1) {
    return lines.filter((l) => l.trim());
  }

  return lines.slice(lastPromptIndex).filter((l) => l.trim());
}

function extractLastCommand(rawOutput: string): string | null {
  const lines = rawOutput.split("\n");
  const promptPatterns = [
    /❯\s+(.+)/,
    /\$\s+(.+)/,
    /▶\s+(.+)/,
    /%\s+(.+)/,
  ];

  for (let i = lines.length - 1; i >= 0; i--) {
    if (!lines[i].trim()) continue;
    for (const re of promptPatterns) {
      const m = lines[i].match(re);
      if (m) return m[1].trim();
    }
  }
  return null;
}

interface EnrichedPane extends Pane {
  isIdle: boolean;
  outputLines: string[];
}

function enrichPane(pane: Pane): EnrichedPane {
  const isIdle = SHELLS.has(pane.command);
  const rawOutput = capturePaneOutput(pane.paneId);
  const outputLines = extractCommandOutput(rawOutput);

  let command: string;
  if (isIdle) {
    command = extractLastCommand(rawOutput) || "none";
  } else {
    command = getFullCommand(pane.pid) || pane.command;
  }

  return { ...pane, isIdle, command, outputLines };
}

function printJSON(panes: Pane[], headN: number, tailN: number) {
  const results = panes.map((pane) => {
    const p = enrichPane(pane);
    return {
      session: p.session,
      dir: p.path,
      latestCommand: p.command,
      running: !p.isIdle,
      attached: p.attached,
      lastActivity: timeAgo(p.activity),
      output: {
        head: p.outputLines.slice(0, headN),
        tail: p.outputLines.slice(-tailN),
      },
    };
  });
  console.log(JSON.stringify(results, null, 2));
}

function printPretty(panes: Pane[], headN: number, tailN: number) {
  for (const pane of panes) {
    const p = enrichPane(pane);
    const attachedTag = p.attached ? " (human watching)" : "";

    console.log(
      chalk.dim(`── Session ${p.session}${attachedTag} ${"─".repeat(30)}`)
    );
    console.log(`Dir: ${shortenHome(p.path)}`);
    console.log(`Latest command: ${p.command}`);

    if (p.isIdle) {
      console.log("Command finished");
    } else {
      console.log("Command currently running");
    }
    console.log(`Session active ${timeAgo(p.activity)}`);

    if (p.outputLines.length > 0) {
      console.log("");
      const total = p.outputLines.length;
      if (total <= headN + tailN) {
        console.log(chalk.dim("--- last command output ---"));
        p.outputLines.forEach((l) => console.log(`  ${l}`));
      } else {
        console.log(
          chalk.dim(`--- last command output (first ${headN} lines) ---`)
        );
        p.outputLines.slice(0, headN).forEach((l) => console.log(`  ${l}`));
        console.log(
          chalk.dim(`--- last command output (last ${tailN} lines) ---`)
        );
        p.outputLines.slice(-tailN).forEach((l) => console.log(`  ${l}`));
      }
    }

    console.log("");
  }
}

export function listSessions(options: ListOptions) {
  if (!isTmuxRunning()) {
    console.log(options.json ? "[]" : "No tmux sessions running.");
    return;
  }

  let panes = getPanes();
  if (!panes.length) {
    console.log(options.json ? "[]" : "No tmux panes found.");
    return;
  }

  panes.sort((a, b) => b.activity - a.activity);

  if (options.dir) {
    const filterDir = resolve(options.dir);
    panes = panes.filter((p) => p.path.startsWith(filterDir));
  }

  if (!panes.length) {
    console.log(options.json ? "[]" : "No matching sessions found.");
    return;
  }

  const headN = parseInt(options.head) || 5;
  const tailN = parseInt(options.tail) || 5;

  if (options.json) {
    printJSON(panes, headN, tailN);
  } else {
    printPretty(panes, headN, tailN);
  }
}
