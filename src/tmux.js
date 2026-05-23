import { execSync } from "child_process";

export function isTmuxRunning() {
  try {
    execSync("tmux list-sessions", { stdio: "pipe" });
    return true;
  } catch {
    return false;
  }
}

export function getPanes() {
  const fields = [
    "#{session_activity}",
    "#{session_name}",
    "#{session_attached}",
    "#{pane_id}",
    "#{pane_current_path}",
    "#{pane_current_command}",
    "#{pane_pid}",
  ];

  const output = execSync(`tmux list-panes -a -F '${fields.join("|")}'`, {
    encoding: "utf-8",
  }).trim();

  if (!output) return [];

  return output.split("\n").map((line) => {
    const [activity, session, attached, paneId, path, command, pid] =
      line.split("|");
    return {
      activity: parseInt(activity),
      session,
      attached: attached === "1",
      paneId,
      path,
      command,
      pid: parseInt(pid),
    };
  });
}

export function getFullCommand(shellPid) {
  try {
    const output = execSync("ps -e -o pid=,ppid=,args=", {
      encoding: "utf-8",
    });

    for (const line of output.trim().split("\n")) {
      const match = line.trim().match(/^(\d+)\s+(\d+)\s+(.+)$/);
      if (!match) continue;

      const ppid = parseInt(match[2]);
      const cmd = match[3];

      if (ppid !== shellPid) continue;
      if (cmd.includes("ps -e") || cmd.includes("agentmux")) continue;

      const parts = cmd.split(" ");
      const basename = parts[0].split("/").pop();
      return [basename, ...parts.slice(1)].join(" ");
    }
    return null;
  } catch {
    return null;
  }
}

export function capturePaneOutput(paneId) {
  try {
    return execSync(`tmux capture-pane -t ${paneId} -p -S -`, {
      encoding: "utf-8",
    });
  } catch {
    return "";
  }
}
