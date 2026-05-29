agent skill:

- prefer send-keys in parallel over sequential
- have one command for capture, 2 gets misleading for agents
- tell them output not always appears, prefer sleep pooling

- shell hooks wrapping commands

- `hitch list` for users

maybe:

- view only mode for sharing
- colorize --help while not by agent
- starship integration
- ask user if installing #N prompt
- changesets (signed releases)
- make send-keys always return output
- agent inputs history
- make agents pass --since timestamp into context from previous output (cursors)
- char guardrail for send-keys or caputre, if too long write to file or give alt way
- cli analytics

harder to make:

- config commands - enable/disable auto join for specific repo or specific app, like auto join hitch in all vscode terminals, or zed, etc
- move parent shell scrollback into hitch session on attach
- wait for execution of command mode
- switch with join command, instead of error (while already in session)
- keep track of commands history in sessions (hard, requires shell wrapper integration)
