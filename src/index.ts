#!/usr/bin/env node
import { Command } from "commander";
import { listSessions } from "./list.js";
import { init } from "./init.js";
import { shell } from "./shell.js";
import {
  attachSession,
  capturePane,
  currentSessionInfo,
  killSession,
  sendKeys,
} from "./session-commands.js";

const program = new Command();

program
  .name("muxi")
  .description("Give AI agents visibility into your terminal sessions")
  .version("0.1.0");

program
  .command("list")
  .description("List muxi sessions with rich context")
  .option("--dir <path>", "Filter by working directory (includes subdirectories)")
  .option("--json", "Output as JSON")
  .option("--head <n>", "Lines from start of command output", "5")
  .option("--tail <n>", "Lines from end of command output", "5")
  .action(listSessions);

program
  .command("list-sessions")
  .description("List muxi sessions")
  .option("--dir <path>", "Filter by working directory (includes subdirectories)")
  .option("--json", "Output as JSON")
  .option("--head <n>", "Lines from start of command output", "5")
  .option("--tail <n>", "Lines from end of command output", "5")
  .action(listSessions);

program
  .command("list-panes")
  .description("List muxi sessions using tmux-compatible naming")
  .option("--dir <path>", "Filter by working directory (includes subdirectories)")
  .option("--json", "Output as JSON")
  .option("--head <n>", "Lines from start of command output", "5")
  .option("--tail <n>", "Lines from end of command output", "5")
  .action(listSessions);

program
  .command("init")
  .description("Output shell snippet for .zshrc/.bashrc")
  .action(init);

program
  .command("shell")
  .description("Start an agent-visible shell in this terminal")
  .action(shell);

program
  .command("attach <session>")
  .description("Attach to a muxi session")
  .action(attachSession);

program
  .command("send-keys [keys...]")
  .description("Send tmux-style keys to a session")
  .requiredOption("-t, --target <session>", "Target session")
  .action(sendKeys);

program
  .command("capture-pane")
  .description("Print captured output for a session")
  .requiredOption("-t, --target <session>", "Target session")
  .option("-p, --print", "Print output", true)
  .option("-S, --start <line>", "tmux-compatible start line; accepted for compatibility")
  .option("--tail <lines>", "Only print the last N lines")
  .option("--raw", "Keep ANSI escape sequences")
  .action(capturePane);

program
  .command("kill-session <session>")
  .description("Terminate and remove a muxi session")
  .action(killSession);

program
  .command("info")
  .description("Show current muxi session info")
  .action(currentSessionInfo);

program
  .command("exit")
  .description("Explain how to detach from a muxi shell")
  .action(() => {
    console.log("Detach from a muxi shell with Ctrl-\\.");
  });

program.parse();
