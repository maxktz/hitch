#!/usr/bin/env node
import { execSync } from "child_process";
import { Command } from "commander";
import { listSessions } from "./list.js";
import { init } from "./init.js";
import { shell } from "./shell.js";

const program = new Command();

program
  .name("agentmux")
  .description("Give AI agents visibility into your tmux sessions")
  .version("0.1.0");

program
  .command("list")
  .description("List tmux sessions with rich context")
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
  .command("exit")
  .description("Exit agentmux proxy and drop to a plain terminal")
  .action(() => {
    if (!process.env.TMUX) {
      console.log("Not inside a tmux session.");
      process.exit(1);
    }
    execSync('tmux detach -E "AGENTMUX_DISABLED=1 zsh"', { stdio: "inherit" });
  });

program.parse();
