#!/usr/bin/env node
import { Command } from "commander";
import { listSessions } from "./list.js";

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

program.parse();
