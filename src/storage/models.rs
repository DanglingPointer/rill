use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// A torrent as the database keeps it.
#[derive(Debug, Clone)]
pub struct SavedTorrent {
    pub info_hash: String,
    pub name: String,
    pub uri: String,
    /// "downloading", "paused", "completed" or "error".
    pub state: String,
    pub downloaded: u64,
    pub total: u64,
    pub output_dir: String,
    pub added_at: i64,
    pub completed_at: Option<i64>,
    pub last_active: i64,
    pub total_pieces: u64,
    pub downloaded_pieces: u64,
    pub sequential: bool,
}

impl SavedTorrent {
    pub fn new(
        info_hash: String,
        name: String,
        uri: String,
        state: String,
        downloaded: u64,
        total: u64,
        output_dir: PathBuf,
    ) -> Self {
        let now = unix_time();
        Self {
            info_hash,
            name,
            uri,
            state,
            downloaded,
            total,
            output_dir: output_dir.to_string_lossy().to_string(),
            added_at: now,
            completed_at: None,
            last_active: now,
            total_pieces: 0,
            downloaded_pieces: 0,
            sequential: false,
        }
    }

    pub fn output_dir_path(&self) -> PathBuf {
        PathBuf::from(&self.output_dir)
    }
}

#[derive(Debug, Clone)]
pub struct AppSettings {
    pub download_folder: String,
    pub window_width: i32,
    pub window_height: i32,
    pub window_maximized: bool,
    pub log_level: String,
    pub max_active_downloads: i32,
    pub pwp_port: u16,
    /// What the torrent list is ordered by; see `window::SortOrder`.
    pub sort_order: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            download_folder: default_download_folder(),
            window_width: 375,
            window_height: 480,
            window_maximized: false,
            log_level: "info".to_string(),
            max_active_downloads: 3,
            pwp_port: 0,
            sort_order: "added".to_string(),
        }
    }
}

impl AppSettings {
    pub fn download_folder_path(&self) -> PathBuf {
        PathBuf::from(&self.download_folder)
    }
}

pub fn default_download_folder() -> String {
    dirs_next::download_dir()
        .or_else(dirs_next::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .to_string_lossy()
        .to_string()
}

/// Seconds since the Unix epoch, as the database stores times.
pub fn unix_time() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}
