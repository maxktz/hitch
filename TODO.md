- proper installation commands
  - readme
  - js -> ts
  - analytics
  - changesets (signed releases)

- show session number in terminal title

- show last lines and context since line

agent skill:
  - prefer send-keys in parallel over sequential
  - fix agent using --dir 
  - fix agents inspecting each terminal separately

- shell hooks wrapping commands

priority 3:
- option for agent to spin up sessions for user to join 

maybe:
- make send-keys always return output
- agent inputs history


harder to make:
- config commands - enable/disable auto join for specific repo or specific app, like auto join hitch in all vscode terminals, or zed, etc
- move parent shell scrollback into hitch session on attach
- wait for execution of command mode
- switch with join command, instead of error (while already in session)
- keep track of commands history in sessions (hard, requires shell wrapper integration)


tests:
- zsh with p10k and oh my zsh
- zsh without p10k 
- zsh without oh my zsh
- default shells
