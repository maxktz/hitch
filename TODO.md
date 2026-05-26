- send-keys wait options
- sent-keys send multiple at once
- fix paste bug 

- context flags
- show last lines and context since line

- agent skill:
  - say use sleep to wait for outputs
  - make `list` smart showing more based on which had output most recently or input, etc 
  - reduce agent pooling by wait for output conditions
  - make send keys return things

- fix bug with TUI

- print-skill agent command
- separate --help for agent and for human
- -v for version

- proper installation commands
  - hitch setup wizard
  - install hitch
  - versioning and updates
  - version for skill
  - agent inputs history


- show session number in terminal title

- fix "actively running: node /Users/maxktz/.nvm/versions/node/v23.7.0/bin/pnpm dev"


priority 3:
- option for agent to spin up sessions for user to join 

maybe:
- make send-keys always return output


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
