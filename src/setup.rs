use crate::cli::{SetupArgs, SetupCommand};
use crate::models::UpdateCache;
use crate::style::Style;
use crate::{
    HITCH_VERSION, INSTALL_SOURCE, NPM_PACKAGE_NAME, NPM_REGISTRY_URL, SKILL_MD, SKILL_NAME,
    SKILL_VERSION, UPDATE_CACHE_TTL_SECS, now_epoch, state_dir, update_cache_path,
};
use inquire::Confirm;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn cmd_install_skill() -> io::Result<()> {
    let source = write_embedded_skill_dir()?;
    let source_arg = source.to_string_lossy().into_owned();
    let args = ["--yes", "skills", "add", &source_arg, "--skill", SKILL_NAME];
    println!("running: npx {}", args.join(" "));

    let result = match Command::new("npx").args(args).status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(io::Error::other(format!(
            "skill installer exited with status {status}"
        ))),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "npx not found. Install Node.js and rerun `hitch setup skill`",
        )),
        Err(err) => Err(err),
    };

    let _ = fs::remove_dir_all(source);
    result
}

fn write_embedded_skill_dir() -> io::Result<PathBuf> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let dir = env::temp_dir().join(format!("hitch-skill-{}-{now}", process::id()));
    fs::create_dir_all(&dir)?;
    fs::write(dir.join("SKILL.md"), SKILL_MD)?;
    Ok(dir)
}

pub(crate) fn update_warning() -> Option<String> {
    if INSTALL_SOURCE != "npm" {
        return None;
    }

    let cache = read_update_cache()?;
    if cache.install_source != INSTALL_SOURCE {
        return None;
    }
    if !update_cache_fresh(&cache) {
        return None;
    }
    let latest = cache.latest_version?;
    if version_less_than(HITCH_VERSION, &latest) {
        Some(format!(
            "update available {HITCH_VERSION} -> {latest}, run `npm install -g {NPM_PACKAGE_NAME}`"
        ))
    } else {
        None
    }
}

pub(crate) fn maybe_refresh_update_cache() {
    if INSTALL_SOURCE != "npm" || !update_cache_stale() {
        return;
    }

    let path = update_cache_path();
    let _ = thread::Builder::new()
        .name("hitch-update-check".to_string())
        .spawn(move || {
            let _ = refresh_npm_update_cache(&path);
        });
}

fn update_cache_stale() -> bool {
    let Some(cache) = read_update_cache() else {
        return true;
    };
    if cache.install_source != INSTALL_SOURCE {
        return true;
    }
    !update_cache_fresh(&cache)
}

fn update_cache_fresh(cache: &UpdateCache) -> bool {
    let now = now_epoch();
    cache.checked_at <= now && now.saturating_sub(cache.checked_at) < UPDATE_CACHE_TTL_SECS
}

fn read_update_cache() -> Option<UpdateCache> {
    let raw = fs::read_to_string(update_cache_path()).ok()?;
    serde_json::from_str(&raw).ok()
}

fn refresh_npm_update_cache(path: &Path) -> io::Result<()> {
    let latest = fetch_npm_latest_version();
    let cache = UpdateCache {
        checked_at: now_epoch(),
        install_source: INSTALL_SOURCE.to_string(),
        latest_version: latest,
    };

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(&cache).unwrap())?;
    Ok(())
}

fn fetch_npm_latest_version() -> Option<String> {
    let response = minreq::get(NPM_REGISTRY_URL).with_timeout(3).send().ok()?;
    let raw = response.as_str().ok()?;
    let value = serde_json::from_str::<serde_json::Value>(raw).ok()?;
    let tag = if HITCH_VERSION.contains('-') {
        "beta"
    } else {
        "latest"
    };
    value
        .get("dist-tags")
        .and_then(|tags| tags.get(tag).or_else(|| tags.get("latest")))
        .and_then(|version| version.as_str())
        .map(str::to_string)
}

pub(crate) fn outdated_skill_warning() -> Option<String> {
    for path in installed_skill_paths() {
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        let Some(installed) = skill_version(&raw) else {
            continue;
        };
        if version_less_than(&installed, SKILL_VERSION) {
            let root = format_skill_root(&path);
            return Some(format!(
                "agent skill in \"{root}\" is outdated, run `hitch setup` to update"
            ));
        }
    }
    None
}

fn installed_skill_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Ok(cwd) = env::current_dir() {
        paths.push(cwd.join(".agents/skills/hitch/SKILL.md"));
        paths.push(cwd.join(".claude/skills/hitch/SKILL.md"));
    }

    let Some(home) = env::var_os("HOME").map(PathBuf::from) else {
        return paths;
    };

    paths.push(home.join(".agents/skills/hitch/SKILL.md"));
    paths.push(home.join(".claude/skills/hitch/SKILL.md"));
    paths
}

fn format_skill_root(path: &Path) -> String {
    let root = path
        .parent()
        .and_then(Path::parent)
        .unwrap_or(path)
        .to_path_buf();

    if let Ok(cwd) = env::current_dir() {
        if let Ok(relative) = root.strip_prefix(&cwd) {
            if relative.as_os_str().is_empty() {
                return ".".to_string();
            }
            return format!("./{}", relative.display());
        }
    }

    if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
        if let Ok(relative) = root.strip_prefix(home) {
            if relative.as_os_str().is_empty() {
                return "~".to_string();
            }
            return format!("~/{}", relative.display());
        }
    }

    root.display().to_string()
}

fn skill_version(raw: &str) -> Option<String> {
    raw.lines().find_map(|line| {
        let line = line.trim();
        let version = line.strip_prefix("version:")?.trim();
        if version.is_empty() {
            return None;
        }
        Some(version.trim_matches(['"', '\'']).to_string())
    })
}

fn version_less_than(left: &str, right: &str) -> bool {
    let left = parse_version(left);
    let right = parse_version(right);
    let len = left.len().max(right.len());
    for index in 0..len {
        let left_part = left.get(index).copied().unwrap_or(0);
        let right_part = right.get(index).copied().unwrap_or(0);
        if left_part != right_part {
            return left_part < right_part;
        }
    }
    false
}

fn parse_version(version: &str) -> Vec<u64> {
    version
        .split(['.', '-'])
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

pub(crate) fn cmd_print_skill() -> io::Result<()> {
    print!("{}", SKILL_MD);
    if !SKILL_MD.ends_with('\n') {
        println!();
    }
    Ok(())
}

pub(crate) fn cmd_setup(args: &SetupArgs) -> io::Result<()> {
    match args.command {
        Some(SetupCommand::Shell) => cmd_setup_prompt(),
        Some(SetupCommand::Skill) => cmd_install_skill(),
        None => cmd_setup_wizard(),
    }
}

pub(crate) fn cmd_setup_wizard() -> io::Result<()> {
    let style = Style::stdout();
    println!("hitch setup");
    println!();

    cmd_setup_prompt()?;
    println!();

    let install_skill = confirm("Install agent skill?", true)?;
    if install_skill {
        cmd_install_skill()?;
    } else {
        println!(
            "{}",
            style.muted("to install skill later, run `hitch setup`")
        );
    }

    Ok(())
}

pub(crate) fn cmd_welcome_setup() -> io::Result<()> {
    let style = Style::stdout();
    println!();
    println!("{}", style.logo("⣇⡀ ⠄⢀⣆⡀ ⣀⡀⢸⣀"));
    println!("{}", style.logo("⠇⠸ ⠇ ⠣⠄⠘⠤⠄⠸ ⠇"));
    println!("Welcome to setup!");
    println!();

    cmd_setup_prompt_with_messages(false)?;
    println!("{} Shell integration installed", style.success("✓"));
    println!();

    let install_skill = confirm("Install agent skill?", true)?;
    if install_skill {
        cmd_install_skill()?;
    } else {
        println!(
            "{}",
            style.muted("to install skill later, run `hitch setup`")
        );
    }

    println!();
    println!("{} Setup completed", style.success("✓"));
    println!();
    println!("{}", style.muted("How to use:"));
    println!();
    println!(
        "{} Restart your existing terminal",
        style.light_yellow("1.")
    );
    println!("{} Run `{}`", style.light_yellow("2."), style.logo("hitch"));
    println!(
        "{} Ask your agent to see it! {}via /hitch skill{}",
        style.light_yellow("3."),
        style.muted("("),
        style.muted(")")
    );
    Ok(())
}

pub(crate) fn ensure_shell_integration() -> io::Result<()> {
    match shell_integration_state()? {
        ShellIntegrationState::Current => Ok(()),
        ShellIntegrationState::Outdated => {
            if env::var_os("HITCH_NO_AUTO_UPDATE_SHELL").is_none() {
                update_shell_integration_silent()?;
            }
            Ok(())
        }
        ShellIntegrationState::Missing => {
            cmd_welcome_setup()?;
            process::exit(0);
        }
    }
}

fn confirm(prompt: &str, default: bool) -> io::Result<bool> {
    Confirm::new(prompt)
        .with_default(default)
        .prompt()
        .map_err(io::Error::other)
}

pub(crate) fn cmd_setup_prompt() -> io::Result<()> {
    cmd_setup_prompt_with_messages(true)
}

pub(crate) fn cmd_setup_prompt_with_messages(messages: bool) -> io::Result<()> {
    let shell = detect_shell();
    if shell == "zsh" {
        return setup_zsh_family_prompt(messages);
    }

    if shell == "bash" {
        return setup_bash_prompt(messages);
    }

    if shell == "fish" {
        return setup_fish_prompt(messages);
    }

    println!("unsupported shell: {shell}");
    println!("manual prompt segment: show `#$HITCH_SESSION` when HITCH_SESSION is set");
    Ok(())
}

fn detect_shell() -> String {
    env::var("SHELL")
        .ok()
        .and_then(|shell| {
            Path::new(&shell)
                .file_name()
                .map(|name| name.to_string_lossy().into())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn home_file(name: &str) -> Option<PathBuf> {
    env::var_os("HOME").map(|home| PathBuf::from(home).join(name))
}

enum ShellIntegrationState {
    Current,
    Outdated,
    Missing,
}

fn shell_integration_state() -> io::Result<ShellIntegrationState> {
    let Some((path, block)) = essential_shell_integration() else {
        return Ok(ShellIntegrationState::Current);
    };
    let raw = fs::read_to_string(path).unwrap_or_default();
    if !has_hitch_marked_block(&raw) {
        return Ok(ShellIntegrationState::Missing);
    }
    if upsert_marked_block(&raw, block) == raw {
        Ok(ShellIntegrationState::Current)
    } else {
        Ok(ShellIntegrationState::Outdated)
    }
}

fn update_shell_integration_silent() -> io::Result<()> {
    let Some((path, block)) = essential_shell_integration() else {
        return Ok(());
    };
    setup_rc_prompt(&path, block)
}

fn essential_shell_integration() -> Option<(PathBuf, &'static str)> {
    match detect_shell().as_str() {
        "zsh" => home_file(".zshrc").map(|path| (path, zsh_prompt_block())),
        "bash" => home_file(".bashrc").map(|path| (path, bash_prompt_block())),
        "fish" => {
            home_file(".config/fish/conf.d/hitch.fish").map(|path| (path, fish_prompt_block()))
        }
        _ => None,
    }
}

fn setup_zsh_family_prompt(messages: bool) -> io::Result<()> {
    setup_zsh_prompt()?;

    if let Some(p10k_path) = home_file(".p10k.zsh").filter(|path| path.exists()) {
        setup_p10k_prompt(&p10k_path)?;
    }

    if messages {
        println!("shell integration updated");
        println!("restart existing terminals to pick up shell integration");
    }

    Ok(())
}

fn setup_p10k_prompt(path: &Path) -> io::Result<()> {
    let raw = fs::read_to_string(path)?;
    let mut updated = ensure_p10k_left_segment(&raw)?;
    updated = upsert_p10k_prompt_block(&updated)?;
    if updated != raw {
        let _backup = backup_file(path)?;
        fs::write(path, updated)?;
    }
    Ok(())
}

fn setup_zsh_prompt() -> io::Result<()> {
    let Some(path) = home_file(".zshrc") else {
        return Err(io::Error::new(io::ErrorKind::NotFound, "HOME is not set"));
    };
    setup_rc_prompt(&path, zsh_prompt_block())
}

fn setup_bash_prompt(messages: bool) -> io::Result<()> {
    let Some(path) = home_file(".bashrc") else {
        return Err(io::Error::new(io::ErrorKind::NotFound, "HOME is not set"));
    };
    setup_rc_prompt(&path, bash_prompt_block())?;
    if messages {
        println!("shell integration updated");
        println!("restart existing terminals to pick up shell integration");
    }
    Ok(())
}

fn setup_fish_prompt(messages: bool) -> io::Result<()> {
    let Some(path) = home_file(".config/fish/conf.d/hitch.fish") else {
        return Err(io::Error::new(io::ErrorKind::NotFound, "HOME is not set"));
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let raw = fs::read_to_string(&path).unwrap_or_default();
    let updated = upsert_marked_block(&raw, fish_prompt_block());
    if updated != raw {
        if path.exists() {
            let _backup = backup_file(&path)?;
        }
        fs::write(&path, updated)?;
    }
    if messages {
        println!("shell integration updated");
        println!("restart existing fish terminals to pick up shell integration");
    }
    Ok(())
}

fn setup_rc_prompt(path: &Path, block: &str) -> io::Result<()> {
    let raw = fs::read_to_string(path).unwrap_or_default();
    let updated = upsert_marked_block(&raw, block);
    if updated != raw {
        if path.exists() {
            let _backup = backup_file(path)?;
        }
        fs::write(path, updated)?;
    }
    Ok(())
}

fn backup_file(path: &Path) -> io::Result<PathBuf> {
    let dir = state_dir().join("backups");
    fs::create_dir_all(&dir)?;
    let backup = dir.join(format!(
        "{}.{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("config"),
        now_epoch()
    ));
    fs::copy(path, &backup)?;
    Ok(backup)
}

fn upsert_marked_block(raw: &str, block: &str) -> String {
    upsert_marked_block_before(raw, block, "")
}

fn has_hitch_marked_block(raw: &str) -> bool {
    raw.contains("# >>> hitch shell integration >>>")
        || raw.contains("# >>> hitch prompt integration >>>")
}

fn upsert_marked_block_before(raw: &str, block: &str, anchor: &str) -> String {
    const MARKERS: [(&str, &str); 2] = [
        (
            "# >>> hitch shell integration >>>",
            "# <<< hitch shell integration <<<",
        ),
        (
            "# >>> hitch prompt integration >>>",
            "# <<< hitch prompt integration <<<",
        ),
    ];

    for (start_marker, end_marker) in MARKERS {
        if let Some(start) = raw.find(start_marker) {
            if let Some(end_rel) = raw[start..].find(end_marker) {
                let line_start = raw[..start].rfind('\n').map(|index| index + 1).unwrap_or(0);
                let replace_start = if raw[line_start..start].trim().is_empty() {
                    line_start
                } else {
                    start
                };
                let end = start + end_rel + end_marker.len();
                let mut out = String::new();
                out.push_str(&raw[..replace_start]);
                out.push_str(block.trim_end());
                out.push_str(&raw[end..]);
                return out;
            }
        }
    }

    if !anchor.is_empty() {
        if let Some(index) = raw.find(anchor) {
            let mut out = String::new();
            out.push_str(raw[..index].trim_end());
            out.push_str("\n\n");
            out.push_str(block.trim_end());
            out.push_str("\n\n");
            out.push_str(&raw[index..]);
            return out;
        }
    }

    let mut out = raw.trim_end().to_string();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(block.trim_end());
    out.push('\n');
    out
}

fn upsert_p10k_prompt_block(raw: &str) -> io::Result<String> {
    if raw.contains("# >>> hitch shell integration >>>")
        || raw.contains("# >>> hitch prompt integration >>>")
    {
        return Ok(upsert_marked_block(raw, p10k_prompt_block()));
    }

    for anchor in [
        "  # Example of a user-defined prompt segment.",
        "  # Transient prompt works similarly",
        "  # If p10k is already loaded, reload configuration.",
    ] {
        if raw.contains(anchor) {
            return Ok(upsert_marked_block_before(raw, p10k_prompt_block(), anchor));
        }
    }

    Err(io::Error::other(
        "could not find a safe insertion point in ~/.p10k.zsh",
    ))
}

fn ensure_p10k_left_segment(raw: &str) -> io::Result<String> {
    let Some(start) = raw.find("POWERLEVEL9K_LEFT_PROMPT_ELEMENTS=(") else {
        return Err(io::Error::other(
            "could not find POWERLEVEL9K_LEFT_PROMPT_ELEMENTS in ~/.p10k.zsh",
        ));
    };
    let Some(end_rel) = raw[start..].find("\n  )") else {
        return Err(io::Error::other(
            "could not parse POWERLEVEL9K_LEFT_PROMPT_ELEMENTS in ~/.p10k.zsh",
        ));
    };
    let end = start + end_rel;
    let block = &raw[start..end];
    let mut lines: Vec<String> = block.lines().map(str::to_string).collect();
    lines.retain(|line| line.split('#').next().unwrap_or("").trim() != "hitch");

    let insert_at = lines
        .iter()
        .position(|line| line.split('#').next().unwrap_or("").trim() == "newline")
        .unwrap_or(lines.len());
    lines.insert(
        insert_at,
        "    hitch                   # hitch terminal id".to_string(),
    );

    let mut out = String::new();
    out.push_str(&raw[..start]);
    out.push_str(&lines.join("\n"));
    out.push_str(&raw[end..]);
    Ok(out)
}

fn p10k_prompt_block() -> &'static str {
    r##"  # >>> hitch shell integration >>>
  function prompt_hitch() {
    [[ -n "${HITCH_SESSION:-}" ]] && p10k segment -f 2 -t "#${HITCH_SESSION}"
  }
  # <<< hitch shell integration <<<"##
}

fn zsh_prompt_block() -> &'static str {
    r#"# >>> hitch shell integration >>>
function _hitch_prompt_segment() {
  [[ -n "${HITCH_SESSION:-}" ]] && print -n "%F{2}#${HITCH_SESSION}%f "
}

function _hitch_precmd() {
  [[ -n "${POWERLEVEL9K_LEFT_PROMPT_ELEMENTS:-}" ]] && return
  [[ -z "${HITCH_SESSION:-}" ]] && return
  local _hitch_prefix="%F{2}#${HITCH_SESSION}%f "
  [[ "$PROMPT" == "${_hitch_prefix}"* ]] && return
  PROMPT="${_hitch_prefix}${PROMPT}"
}

if [[ -z "${HITCH_PROMPT_INSTALLED:-}" && -z "${POWERLEVEL9K_LEFT_PROMPT_ELEMENTS:-}" ]]; then
  HITCH_PROMPT_INSTALLED=1
  autoload -Uz add-zsh-hook
  add-zsh-hook precmd _hitch_precmd
fi

function _hitch_run() {
    local _hitch_command="$1"
    shift
    local _hitch_bin="${commands[$_hitch_command]:-}"
    if [[ -z "${_hitch_bin:-}" ]]; then
      print -u2 "$_hitch_command: command not found"
      return 127
    fi
    if [[ -z "${HITCH_SESSION:-}" && ( "$#" -eq 0 || "$1" == "on" || "$1" == "start" ) ]]; then
      fc -W 2>/dev/null
    fi

    local _hitch_cwd_file=""
    if [[ -z "${HITCH_SESSION:-}" ]]; then
      _hitch_cwd_file="${TMPDIR:-/tmp}/hitch-cwd-$$-$RANDOM"
      HITCH_CWD_SYNC_FILE="$_hitch_cwd_file" "$_hitch_bin" "$@"
    else
      "$_hitch_bin" "$@"
    fi
    local code=$?
    if [[ -n "$_hitch_cwd_file" && -s "$_hitch_cwd_file" ]]; then
      local _hitch_cwd
      _hitch_cwd="$(cat "$_hitch_cwd_file" 2>/dev/null)"
      if [[ -d "$_hitch_cwd" ]]; then
        cd "$_hitch_cwd"
      fi
    fi
    [[ -n "$_hitch_cwd_file" ]] && rm -f "$_hitch_cwd_file"
    if [[ "$code" -eq 42 ]]; then
      exit
    fi
    return "$code"
}

function hitch() {
  _hitch_run hitch "$@"
}

alias unhitch='hitch off'

function hitch-dev() {
  _hitch_run hitch-dev "$@"
}
# <<< hitch shell integration <<<"#
}

fn bash_prompt_block() -> &'static str {
    r#"# >>> hitch shell integration >>>
_hitch_prompt_segment() {
  [[ -n "${HITCH_SESSION:-}" ]] && printf '#%s ' "$HITCH_SESSION"
}

_hitch_prompt_command() {
  [[ -z "${HITCH_SESSION:-}" ]] && return
  local _hitch_prefix="\\[\\033[32m\\]#${HITCH_SESSION} \\[\\033[0m\\]"
  [[ "$PS1" == "${_hitch_prefix}"* ]] && return
  PS1="${_hitch_prefix}${PS1}"
}

if [[ -z "${HITCH_PROMPT_INSTALLED:-}" ]]; then
  HITCH_PROMPT_INSTALLED=1
  if [[ -n "${PROMPT_COMMAND:-}" ]]; then
    PROMPT_COMMAND="${PROMPT_COMMAND%;}; _hitch_prompt_command"
  else
    PROMPT_COMMAND="_hitch_prompt_command"
  fi
fi

_hitch_run() {
    local _hitch_command="$1"
    shift
    local _hitch_bin
    _hitch_bin="$(type -P "$_hitch_command" 2>/dev/null || true)"
    if [[ -z "${_hitch_bin:-}" ]]; then
      printf '%s: command not found\n' "$_hitch_command" >&2
      return 127
    fi
    if [[ -z "${HITCH_SESSION:-}" && ( "$#" -eq 0 || "$1" == "on" || "$1" == "start" ) ]]; then
      history -a 2>/dev/null
    fi

    local _hitch_cwd_file=""
    if [[ -z "${HITCH_SESSION:-}" ]]; then
      _hitch_cwd_file="${TMPDIR:-/tmp}/hitch-cwd-$$-$RANDOM"
      HITCH_CWD_SYNC_FILE="$_hitch_cwd_file" "$_hitch_bin" "$@"
    else
      "$_hitch_bin" "$@"
    fi
    local code=$?
    if [[ -n "$_hitch_cwd_file" && -s "$_hitch_cwd_file" ]]; then
      local _hitch_cwd
      _hitch_cwd="$(cat "$_hitch_cwd_file" 2>/dev/null)"
      if [[ -d "$_hitch_cwd" ]]; then
        cd "$_hitch_cwd"
      fi
    fi
    [[ -n "$_hitch_cwd_file" ]] && rm -f "$_hitch_cwd_file"
    if [[ "$code" -eq 42 ]]; then
      exit
    fi
    return "$code"
}

hitch() {
  _hitch_run hitch "$@"
}

alias unhitch='hitch off'

hitch-dev() {
  _hitch_run hitch-dev "$@"
}
# <<< hitch shell integration <<<"#
}

fn fish_prompt_block() -> &'static str {
    r#"# >>> hitch shell integration >>>
if not functions -q __hitch_original_fish_prompt
    functions -c fish_prompt __hitch_original_fish_prompt
end

function fish_prompt
    if set -q HITCH_SESSION
        set_color green
        printf '#%s ' $HITCH_SESSION
        set_color normal
    end
    __hitch_original_fish_prompt
end

function __hitch_run
        set -l __hitch_command $argv[1]
        set -e argv[1]
        set -l __hitch_bin (command -s "$__hitch_command")
        if test -z "$__hitch_bin"
            printf '%s: command not found\n' "$__hitch_command" >&2
            return 127
        end
        if not set -q HITCH_SESSION
            if test (count $argv) -eq 0; or test "$argv[1]" = on; or test "$argv[1]" = start
                history save 2>/dev/null
            end
        end

        set -l __hitch_cwd_file
        if not set -q HITCH_SESSION
            set __hitch_cwd_file (mktemp -t hitch-cwd.XXXXXX 2>/dev/null)
            if test -n "$__hitch_cwd_file"
                env HITCH_CWD_SYNC_FILE="$__hitch_cwd_file" "$__hitch_bin" $argv
            else
                "$__hitch_bin" $argv
            end
        else
            "$__hitch_bin" $argv
        end
        set code $status
        if test -n "$__hitch_cwd_file"; and test -s "$__hitch_cwd_file"
            set -l __hitch_cwd (cat "$__hitch_cwd_file" 2>/dev/null)
            if test -d "$__hitch_cwd"
                cd "$__hitch_cwd"
            end
        end
        if test -n "$__hitch_cwd_file"
            rm -f "$__hitch_cwd_file"
        end
        if test $code -eq 42
            exit
        end
        return $code
end

function hitch
    __hitch_run hitch $argv
end

alias unhitch 'hitch off'

function hitch-dev
    __hitch_run hitch-dev $argv
end
# <<< hitch shell integration <<<"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_skill_version_from_frontmatter() {
        assert_eq!(
            skill_version("---\nname: hitch\nversion: 3\n---\n"),
            Some("3".to_string())
        );
        assert_eq!(
            skill_version("---\nversion: \"12\"\n---\n"),
            Some("12".to_string())
        );
        assert_eq!(skill_version("---\nname: hitch\n---\n"), None);
    }

    #[test]
    fn compares_dotted_versions_numerically() {
        assert!(version_less_than("0.2.5", "0.2.6"));
        assert!(version_less_than("0.2.9", "0.2.10"));
        assert!(!version_less_than("0.10.0", "0.2.0"));
        assert!(!version_less_than("1.0.0", "1.0.0"));
    }

    #[test]
    fn prerelease_versions_compare_with_numeric_parts() {
        assert!(version_less_than("1.0.0-beta.1", "1.0.0-beta.2"));
        assert!(!version_less_than("1.0.0-beta.2", "1.0.0-beta.1"));
    }

    #[test]
    fn inserts_marked_block_into_empty_or_existing_rc_file() {
        let block = "# >>> hitch shell integration >>>\nnew\n# <<< hitch shell integration <<<";

        assert_eq!(upsert_marked_block("", block), format!("{block}\n"));
        assert_eq!(
            upsert_marked_block("export PATH=/bin\n", block),
            format!("export PATH=/bin\n\n{block}\n")
        );
    }

    #[test]
    fn replaces_existing_hitch_block_without_touching_surrounding_content() {
        let old = "before\n\n# >>> hitch shell integration >>>\nold\n# <<< hitch shell integration <<<\n\nafter\n";
        let block = "# >>> hitch shell integration >>>\nnew\n# <<< hitch shell integration <<<";

        assert_eq!(
            upsert_marked_block(old, block),
            "before\n\n# >>> hitch shell integration >>>\nnew\n# <<< hitch shell integration <<<\n\nafter\n"
        );
    }

    #[test]
    fn inserts_marked_block_before_anchor() {
        let block = "# >>> hitch shell integration >>>\nnew\n# <<< hitch shell integration <<<";

        assert_eq!(
            upsert_marked_block_before("before\nANCHOR\nafter\n", block, "ANCHOR"),
            "before\n\n# >>> hitch shell integration >>>\nnew\n# <<< hitch shell integration <<<\n\nANCHOR\nafter\n"
        );
    }
}
