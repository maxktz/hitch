import { spawnSync } from "child_process";

const TMUX_SETTINGS = [
  ["set", "mouse", "on"],
  ["set", "escape-time", "0"],
  ["set", "history-limit", "50000"],
  ["set", "status", "on"],
  ["set", "status-justify", "right"],
  ["set", "status-style", "bg=default,fg=default"],
  ["set", "status-left", ""],
  ["set", "status-left-length", "0"],
  ["set", "status-right", "#[fg=green]● agentmux"],
  ["set", "status-right-length", "12"],
  ["set", "status-position", "bottom"],
];

function tmuxArgs(): string[] {
  return [
    "new-session",
    ...TMUX_SETTINGS.flatMap((setting) => [";", ...setting]),
  ];
}

export function shell() {
  if (process.env.TMUX) {
    console.error("Already inside a tmux session.");
    process.exit(1);
  }

  const result = spawnSync("tmux", tmuxArgs(), { stdio: "inherit" });

  if (result.error) {
    console.error(`Failed to start tmux: ${result.error.message}`);
    process.exit(1);
  }

  process.exit(result.status ?? 0);
}
