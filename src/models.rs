use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SessionRecord {
    pub(crate) id: String,
    pub(crate) cwd: String,
    pub(crate) socket: String,
    pub(crate) log: String,
    #[serde(rename = "pidFile")]
    pub(crate) pid_file: String,
    #[serde(rename = "masterPidFile")]
    pub(crate) master_pid_file: String,
    #[serde(rename = "createdAt")]
    pub(crate) created_at: String,
    pub(crate) shell: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct UpdateCache {
    #[serde(rename = "checkedAt")]
    pub(crate) checked_at: u64,
    #[serde(rename = "installSource")]
    pub(crate) install_source: String,
    #[serde(rename = "latestVersion")]
    pub(crate) latest_version: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct SessionState {
    #[serde(rename = "activeCommand")]
    pub(crate) active_command: Option<String>,
    #[serde(rename = "commandRunning")]
    pub(crate) command_running: bool,
    #[serde(rename = "commandStartedAt")]
    pub(crate) command_started_at: Option<u64>,
    #[serde(rename = "commandFinishedAt")]
    pub(crate) command_finished_at: Option<u64>,
    #[serde(rename = "lastActivityAt")]
    pub(crate) last_activity_at: Option<u64>,
    #[serde(rename = "foregroundPgrp")]
    pub(crate) foreground_pgrp: Option<i32>,
    #[serde(rename = "currentDir")]
    pub(crate) current_dir: Option<String>,
}
