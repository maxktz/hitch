# Terminal Awareness (muxi)

Your user may run agent-visible terminals through `muxi`. You can see local project sessions by running `muxi list` (or `muxi list --json` for structured output).

Before starting dev servers, tunnels, or any long-running processes:
1. Run `muxi list` to check if one is already running in the current directory
2. If a matching session exists, use it — do NOT start a duplicate process
3. If you need to interact with a session, use muxi commands directly: `muxi send-keys -t <session> "command" Enter` and `muxi capture-pane -t <session> -p`
4. Use `muxi list --all` only when you intentionally need sessions from other projects

The list shows each session's directory, running command, status, and recent output — including URLs like localhost ports.
