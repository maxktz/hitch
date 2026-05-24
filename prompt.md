# Terminal Awareness (agentmux)

Your user may run agent-visible terminals through `agentmux shell`. You can see those terminals, plus any other tmux sessions, by running `agentmux list` (or `agentmux list --json` for structured output).

Before starting dev servers, tunnels, or any long-running processes:
1. Run `agentmux list --dir .` to check if one is already running in the current directory
2. If a matching session exists, use it — do NOT start a duplicate process
3. If you need to interact with a session (restart, send input, read output), use tmux directly (e.g., `tmux send-keys -t <session> "command" Enter`)

The list shows each session's directory, running command, status, and recent output — including URLs like localhost ports.
