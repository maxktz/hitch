# Terminal Awareness (hitch)

Your user may run agent-visible terminals through `hitch`. You can see local project sessions by running `hitch list` (or `hitch list --json` for structured output).

Before starting dev servers, tunnels, or any long-running processes:
1. Run `hitch list` to check if one is already running in the current directory
2. If a matching session exists, use it — do NOT start a duplicate process
3. If you need to interact with a session, use hitch commands directly: `hitch send-keys -t <session> "command" Enter` and `hitch capture-pane -t <session> -p`
4. Use `hitch list --all` only when you intentionally need sessions from other projects

The list shows each session's directory, running command, status, and recent output — including URLs like localhost ports.
