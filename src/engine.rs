use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use async_channel::Sender;
use mtorrent::app;
use mtorrent::utils::re_exports::mtorrent_dht as dht;
use mtorrent::utils::re_exports::mtorrent_utils::peer_id::PeerId;

use crate::listener::GtkListener;

/// A connected peer.
#[derive(Clone, Debug)]
pub struct PeerInfo {
    pub address: String,
    /// The client the peer says it runs, when it says.
    pub client: Option<String>,
    pub speed_down: u64,
    pub speed_up: u64,
    pub encrypted: bool,
}

/// A snapshot of one torrent, sent from the engine to the window.
#[derive(Clone, Debug)]
pub struct UiUpdate {
    pub info_hash: String,
    pub name: String,
    pub state: TorrentUiState,
    pub downloaded: u64,
    pub total: u64,
    pub peers: usize,
    pub speed_down: u64,
    pub speed_up: u64,
    pub output_dir: PathBuf,
    pub uri: String,
    pub peers_list: Vec<PeerInfo>,
    pub total_pieces: usize,
    pub downloaded_pieces: usize,
    pub sequential: bool,
    /// Downsampled piece-availability map (0..=255 fill per segment, in piece
    /// order from start to finish). Empty when no real state is available.
    pub piece_map: Vec<u8>,
}

impl UiUpdate {
    /// A snapshot without transfer figures, for a torrent that is not transferring.
    pub fn idle(
        info_hash: String,
        name: String,
        state: TorrentUiState,
        output_dir: PathBuf,
        uri: String,
        sequential: bool,
    ) -> Self {
        Self {
            info_hash,
            name,
            state,
            downloaded: 0,
            total: 0,
            peers: 0,
            speed_down: 0,
            speed_up: 0,
            output_dir,
            uri,
            peers_list: Vec::new(),
            total_pieces: 0,
            downloaded_pieces: 0,
            sequential,
            piece_map: Vec::new(),
        }
    }
}

/// The state of a torrent as the window shows it.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum TorrentUiState {
    #[default]
    Downloading,
    Paused,
    Completed,
    Error,
}

/// What the engine tells the window.
pub enum UiEvent {
    Update(UiUpdate),
    Finished {
        info_hash: String,
        error: Option<String>,
    },
}

/// A torrent the engine knows, running or not.
#[derive(Debug)]
struct TorrentEntry {
    /// Held, not read: the task watches the count of this Arc, and dropping the sender
    /// wakes its cancellation branch. Both are None while the torrent is not running.
    _canceller: Option<Arc<()>>,
    _cancel_tx: Option<tokio::sync::oneshot::Sender<()>>,
    /// Set true when this torrent is paused/stopped, so the listener can detect
    /// cancellation atomically rather than racing on `Arc::strong_count`.
    cancel_flag: Arc<AtomicBool>,
    name: String,
    uri: String,
    output_dir: PathBuf,
    ui_tx: Sender<UiEvent>,
    sequential: Arc<AtomicBool>,
    /// The port the torrent listens on while it runs; 0 when mtorrent derives it.
    port: u16,
}

impl TorrentEntry {
    fn new(
        name: String,
        uri: String,
        output_dir: PathBuf,
        sequential: bool,
        ui_tx: Sender<UiEvent>,
    ) -> Self {
        Self {
            _canceller: None,
            _cancel_tx: None,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            name,
            uri,
            output_dir,
            ui_tx,
            sequential: Arc::new(AtomicBool::new(sequential)),
            port: 0,
        }
    }

    /// Ends the running task, if any.
    fn halt(&mut self) {
        // Signal the listener before tearing down, so an in-flight snapshot cannot
        // emit a stale update afterwards.
        self.cancel_flag.store(true, Ordering::Release);
        self._canceller = None;
        self._cancel_tx = None;
    }

    fn idle_update(&self, info_hash: &str, state: TorrentUiState) -> UiUpdate {
        UiUpdate::idle(
            info_hash.to_string(),
            self.name.clone(),
            state,
            self.output_dir.clone(),
            self.uri.clone(),
            self.sequential.load(Ordering::Relaxed),
        )
    }
}

/// What the engine thread needs to run one torrent.
struct StartCmd {
    info_hash: String,
    name: String,
    uri: String,
    output_dir: PathBuf,
    canceller: Arc<()>,
    cancel_rx: tokio::sync::oneshot::Receiver<()>,
    cancel_flag: Arc<AtomicBool>,
    ui_tx: Sender<UiEvent>,
    sequential: Arc<AtomicBool>,
    pwp_port: u16,
}

/// What every torrent task shares: who we are, and where mtorrent's work runs.
#[derive(Clone)]
struct Shared {
    peer_id: PeerId,
    config_dir: PathBuf,
    pwp_runtime: tokio::runtime::Handle,
    storage_runtime: tokio::runtime::Handle,
    dht: dht::CommandSink,
}

/// Starts, pauses and stops torrents. Running torrents are in `active`, the others in
/// `saved`; the tasks themselves run on a thread of the engine's own.
#[derive(Debug)]
pub struct TorrentEngine {
    active: Arc<Mutex<HashMap<String, TorrentEntry>>>,
    saved: Arc<Mutex<HashMap<String, TorrentEntry>>>,
    cmd_tx: tokio::sync::mpsc::Sender<StartCmd>,
    config_dir: PathBuf,
    storage: crate::storage::Storage,
}

impl TorrentEngine {
    pub fn new(
        peer_id: PeerId,
        config_dir: PathBuf,
        pwp_handle: tokio::runtime::Handle,
        storage_handle: tokio::runtime::Handle,
        dht_sink: dht::CommandSink,
        storage: crate::storage::Storage,
    ) -> Self {
        log::info!("Creating torrent engine, config_dir: {:?}", config_dir);
        // Bounded so a wedged recv loop applies backpressure instead of growing
        // the queue without limit. The loop drains commands promptly in normal
        // operation, so the capacity is never approached.
        const CMD_QUEUE_CAP: usize = 256;
        let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<StartCmd>(CMD_QUEUE_CAP);

        let shared = Shared {
            peer_id,
            config_dir: config_dir.clone(),
            pwp_runtime: pwp_handle,
            storage_runtime: storage_handle,
            dht: dht_sink,
        };
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();

            rt.block_on(async {
                let local = tokio::task::LocalSet::new();
                local
                    .run_until(async {
                        while let Some(cmd) = cmd_rx.recv().await {
                            tokio::task::spawn_local(run_torrent(cmd, shared.clone()));
                        }
                    })
                    .await;
            });
        });

        Self {
            active: Arc::new(Mutex::new(HashMap::new())),
            saved: Arc::new(Mutex::new(HashMap::new())),
            cmd_tx,
            config_dir,
            storage,
        }
    }

    pub fn start(
        &self,
        name: String,
        uri: String,
        output_dir: PathBuf,
        sequential: bool,
        ui_tx: Sender<UiEvent>,
    ) -> String {
        let info_hash = torrent_id(&uri);
        let mut map = lock_recover(&self.active, "active map");

        if let Some(existing) = map.get(&info_hash) {
            log::info!("Torrent already active: {} ({})", name, info_hash);
            existing.sequential.store(sequential, Ordering::Relaxed);
            // Re-add with a possibly-changed sequential flag: notify the UI so the
            // displayed setting does not go stale. Zeroed counters are backfilled
            // from the previous update by the UI's coalescing logic.
            let _ = ui_tx.try_send(UiEvent::Update(
                existing.idle_update(&info_hash, TorrentUiState::Downloading),
            ));
            return info_hash;
        }

        log::info!(
            "Starting torrent: {} ({}) with sequential={}",
            name,
            info_hash,
            sequential
        );

        let torrent = TorrentEntry::new(name, uri, output_dir, sequential, ui_tx.clone());
        // Immediately notify UI of the new downloading torrent
        let update = torrent.idle_update(&info_hash, TorrentUiState::Downloading);
        if let Err(err) = self.launch(&info_hash, torrent, &mut map) {
            let (_, e) = *err;
            drop(map);
            log::error!("Failed to queue torrent start {}: {}", info_hash, e);
            let _ = ui_tx.try_send(UiEvent::Finished {
                info_hash: info_hash.clone(),
                error: Some("Engine unavailable".into()),
            });
            return info_hash;
        }
        drop(map);

        let _ = ui_tx.try_send(UiEvent::Update(update));
        info_hash
    }

    /// Adds a torrent in a paused state without starting the download.
    pub fn add_paused(
        &self,
        name: String,
        uri: String,
        output_dir: PathBuf,
        sequential: bool,
        ui_tx: Sender<UiEvent>,
    ) -> String {
        let info_hash = self.add_paused_silent(name, uri, output_dir, sequential, ui_tx.clone());
        // Notify the UI of the new paused torrent, or of the (possibly changed)
        // sequential flag on re-add.
        if let Some(torrent) = lock_recover(&self.saved, "saved map").get(&info_hash) {
            let _ = ui_tx.try_send(UiEvent::Update(
                torrent.idle_update(&info_hash, TorrentUiState::Paused),
            ));
        }
        info_hash
    }

    /// Adds a torrent in a paused state, without telling the window.
    pub fn add_paused_silent(
        &self,
        name: String,
        uri: String,
        output_dir: PathBuf,
        sequential: bool,
        ui_tx: Sender<UiEvent>,
    ) -> String {
        let info_hash = torrent_id(&uri);
        let mut map = lock_recover(&self.saved, "saved map");

        if let Some(existing) = map.get(&info_hash) {
            log::info!("Torrent already saved/paused: {} ({})", name, info_hash);
            existing.sequential.store(sequential, Ordering::Relaxed);
            return info_hash;
        }

        log::info!(
            "Adding paused torrent: {} ({}) with sequential={}",
            name,
            info_hash,
            sequential
        );
        map.insert(
            info_hash.clone(),
            TorrentEntry::new(name, uri, output_dir, sequential, ui_tx),
        );
        info_hash
    }

    /// Hands `torrent` to the engine thread and records it as running. The torrent comes
    /// back when the thread cannot take it.
    fn launch(
        &self,
        info_hash: &str,
        mut torrent: TorrentEntry,
        active: &mut HashMap<String, TorrentEntry>,
    ) -> Result<(), Box<(TorrentEntry, String)>> {
        let canceller = Arc::new(());
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let port = listening_port(
            self.storage.pwp_port(),
            active.values().map(|torrent| torrent.port),
        );
        let cmd = StartCmd {
            info_hash: info_hash.to_string(),
            name: torrent.name.clone(),
            uri: torrent.uri.clone(),
            output_dir: torrent.output_dir.clone(),
            canceller: Arc::clone(&canceller),
            cancel_rx,
            cancel_flag: Arc::clone(&cancel_flag),
            ui_tx: torrent.ui_tx.clone(),
            sequential: Arc::clone(&torrent.sequential),
            pwp_port: port,
        };
        if let Err(e) = self.cmd_tx.try_send(cmd) {
            return Err(Box::new((torrent, e.to_string())));
        }
        torrent._canceller = Some(canceller);
        torrent._cancel_tx = Some(cancel_tx);
        torrent.cancel_flag = cancel_flag;
        torrent.port = port;
        active.insert(info_hash.to_string(), torrent);
        Ok(())
    }

    /// Moves a torrent that terminated with an error from the active map to the
    /// saved map, so the UI can offer resume (which restarts the task) instead of
    /// leaving a dead entry that can only be paused.
    pub fn mark_failed(&self, info_hash: &str) {
        let mut active_map = lock_recover(&self.active, "active map");
        if let Some(mut torrent) = active_map.remove(info_hash) {
            torrent.halt();
            drop(active_map);
            lock_recover(&self.saved, "saved map").insert(info_hash.to_string(), torrent);
        }
    }

    /// Stops and removes the torrent from the engine entirely.
    pub fn stop(&self, info_hash: &str) {
        log::info!("Stopping torrent: {}", info_hash);
        let mut active = lock_recover(&self.active, "active map");
        if let Some(mut torrent) = active.remove(info_hash) {
            torrent.halt();
        }
        drop(active);
        lock_recover(&self.saved, "saved map").remove(info_hash);
    }

    /// Gives a torrent the name it has learnt from its metadata, for the snapshots of its
    /// next run.
    pub fn rename(&self, info_hash: &str, name: &str) {
        for map in [&self.active, &self.saved] {
            if let Some(torrent) = lock_recover(map, "torrent map").get_mut(info_hash) {
                torrent.name = name.to_string();
            }
        }
    }

    /// Sets the sequential download flag for a torrent.
    pub fn set_sequential(&self, info_hash: &str, sequential: bool) {
        log::info!("Toggling sequential for {}: {}", info_hash, sequential);
        let active_map = lock_recover(&self.active, "active map");
        if let Some(torrent) = active_map.get(info_hash) {
            torrent.sequential.store(sequential, Ordering::Relaxed);
            return;
        }
        drop(active_map);
        let saved_map = lock_recover(&self.saved, "saved map");
        if let Some(torrent) = saved_map.get(info_hash) {
            torrent.sequential.store(sequential, Ordering::Relaxed);
        }
    }

    /// Toggles the torrent between paused and downloading states.
    pub fn toggle(&self, info_hash: &str) {
        let mut active_map = lock_recover(&self.active, "active map");
        let mut saved_map = lock_recover(&self.saved, "saved map");
        if let Some(mut torrent) = active_map.remove(info_hash) {
            log::info!("Pausing torrent: {}", info_hash);
            torrent.halt();
            saved_map.insert(info_hash.to_string(), torrent);
        } else if let Some(torrent) = saved_map.remove(info_hash) {
            log::info!("Resuming torrent: {}", info_hash);
            // Resume: move from saved to active, restart download. `active` stays locked
            // from the check above to the insert, so a concurrent start/toggle for the
            // same hash cannot double-dispatch.
            drop(saved_map);
            if let Err(err) = self.launch(info_hash, torrent, &mut active_map) {
                let (torrent, e) = *err;
                log::error!("Failed to queue torrent resume {}: {}", info_hash, e);
                let _ = torrent.ui_tx.try_send(UiEvent::Finished {
                    info_hash: info_hash.to_string(),
                    error: Some("Engine unavailable".into()),
                });
                drop(active_map);
                lock_recover(&self.saved, "saved map").insert(info_hash.to_string(), torrent);
            }
        }
    }

    /// Pauses all currently active torrents.
    pub fn pause_all(&self) {
        log::info!("Pausing all active torrents in engine");
        let mut active_map = lock_recover(&self.active, "active map");
        let mut saved_map = lock_recover(&self.saved, "saved map");
        for (info_hash, mut torrent) in active_map.drain() {
            torrent.halt();
            saved_map.insert(info_hash, torrent);
        }
    }

    /// Returns true if the torrent is currently active and downloading/seeding.
    pub fn is_active(&self, info_hash: &str) -> bool {
        lock_recover(&self.active, "active map").contains_key(info_hash)
    }

    /// The data directory, where copied .torrent files are kept.
    pub fn config_dir(&self) -> &PathBuf {
        &self.config_dir
    }
}

/// The port a torrent about to run listens on: the first from `base` up that no running
/// torrent has, or 0, for mtorrent to derive one from the torrent, when `base` is 0. Each
/// torrent needs a port of its own: mtorrent's listeners share a port, and the system would
/// hand a connection for one torrent to any of them.
fn listening_port(base: u16, taken: impl IntoIterator<Item = u16>) -> u16 {
    if base == 0 {
        return 0;
    }
    let taken: std::collections::HashSet<u16> = taken.into_iter().collect();
    (base..=u16::MAX)
        .find(|port| !taken.contains(port))
        .unwrap_or(0)
}

/// Runs one torrent on the engine thread until it ends or is cancelled, and tells the
/// window how it went.
async fn run_torrent(cmd: StartCmd, shared: Shared) {
    let StartCmd {
        info_hash,
        name,
        uri,
        output_dir,
        canceller,
        cancel_rx,
        cancel_flag,
        ui_tx,
        sequential,
        pwp_port,
    } = cmd;
    // mtorrent derives the metainfo filename and the download subfolder from the
    // magnet's `dn` value, then writes the fetched metainfo with a bare fs::write (no
    // parent mkdir). A `dn` containing a path separator points at a non-existent
    // subdir, so the write fails with ENOENT ("No such file or directory") right
    // after metadata is fetched. Sanitise `dn` so the derived path stays inside the
    // output dir.
    let uri = sanitize_magnet_dn(&uri);

    // Ensure the download dir exists; mtorrent's magnet preliminary stage writes the
    // fetched metainfo into output_dir before content storage is created, which fails
    // with ENOENT if the dir is missing.
    if let Err(e) = std::fs::create_dir_all(&output_dir) {
        log::warn!("Failed to create output dir {:?}: {}", output_dir, e);
    }

    let downloaded_bytes = Arc::new(Mutex::new(0u64));
    let total_bytes = Arc::new(Mutex::new(0u64));

    let listener = GtkListener::new(
        Arc::downgrade(&canceller),
        Arc::clone(&cancel_flag),
        ui_tx.clone(),
        info_hash.clone(),
        name.clone(),
        uri.clone(),
        output_dir.clone(),
        Arc::clone(&downloaded_bytes),
        Arc::clone(&total_bytes),
        Arc::clone(&sequential),
    );
    let config = app::main::Config {
        local_peer_id: shared.peer_id,
        output_dir: output_dir.clone(),
        config_dir: shared.config_dir,
        use_upnp: false,
        // Port 0 means "unset": let mtorrent pick a stable port (port_from_hash)
        // instead of binding an ephemeral one and announcing port 0 to trackers.
        pwp_port: (pwp_port != 0).then_some(pwp_port),
        bind_interface: None,
    };
    let ctx = app::main::Context {
        dht_handle: Some(shared.dht),
        pwp_runtime: shared.pwp_runtime,
        storage_runtime: shared.storage_runtime,
    };

    let mut rx = cancel_rx;
    let is_seq = sequential.load(Ordering::Relaxed);
    let result = mtorrent::utils::re_exports::mtorrent_core::SEQUENTIAL
        .scope(sequential, async {
            tokio::select! {
                res = app::main::single_torrent(&uri, listener, config, ctx) => Some(res),
                _ = &mut rx => {
                    log::info!("Torrent task paused/cancelled: {}", info_hash);
                    None
                }
            }
        })
        .await;

    if let Some(res) = result {
        if Arc::strong_count(&canceller) > 1 {
            match &res {
                Ok(_) => log::info!("Torrent completed: {}", info_hash),
                Err(e) => log::error!("Torrent failed: {}: {}", info_hash, e),
            }
            let _ = ui_tx
                .send(UiEvent::Finished {
                    info_hash,
                    error: res.err().map(|e| e.to_string()),
                })
                .await;
        }
    } else {
        let update = UiUpdate {
            downloaded: *lock_recover(&downloaded_bytes, "downloaded bytes"),
            total: *lock_recover(&total_bytes, "total bytes"),
            // No name: the one the window has may be newer than the one this run began
            // with.
            ..UiUpdate::idle(
                info_hash,
                String::new(),
                TorrentUiState::Paused,
                output_dir,
                uri,
                is_seq,
            )
        };
        let _ = ui_tx.send(UiEvent::Update(update)).await;
    }
}

/// Rewrites the `dn` (display name) parameter of a magnet URI so it cannot
/// contain path separators or other filesystem-hostile characters. mtorrent
/// joins the decoded `dn` straight onto the output directory to form the
/// metainfo filename and the content subfolder; an unsanitised `dn` such as
/// "Show / Season 1" yields a path with a missing intermediate directory and
/// the metainfo write fails with ENOENT. Non-magnet URIs are returned
/// unchanged.
pub(crate) fn sanitize_magnet_dn(uri: &str) -> String {
    let Some(dn_pos) = uri.find("dn=") else {
        return uri.to_string();
    };
    let value_start = dn_pos + 3;
    let value_end = uri[value_start..]
        .find('&')
        .map(|i| value_start + i)
        .unwrap_or(uri.len());

    let raw = &uri[value_start..value_end];
    let decoded = urlencoding::decode(raw)
        .map(|s| s.into_owned())
        .unwrap_or_else(|_| raw.to_string());

    let cleaned = clean_name(&decoded);
    let cleaned = if cleaned.is_empty() {
        "torrent".to_string()
    } else {
        cleaned
    };

    if cleaned == raw {
        return uri.to_string();
    }
    let encoded = urlencoding::encode(&cleaned);
    format!("{}{}{}", &uri[..value_start], encoded, &uri[value_end..])
}

/// A torrent name as a single file name: separators and control characters become `_`,
/// and surrounding spaces and dots go. Empty when nothing is left.
fn clean_name(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | '\0' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect::<String>()
        .trim()
        .trim_matches('.')
        .to_string()
}

/// Names a magnet link after its info hash when it has no usable name (`dn`) of its own.
/// mtorrent names both the fetched metainfo and the download folder after `dn`, with
/// "unnamed" for a link without one, so two such links saved to one folder would share
/// them.
pub(crate) fn name_nameless_magnet(uri: &str) -> String {
    use mtorrent::utils::re_exports::mtorrent_core::input::MagnetLink;
    use std::str::FromStr;

    let Ok(magnet) = MagnetLink::from_str(uri) else {
        return uri.to_string();
    };
    if magnet
        .name()
        .is_some_and(|name| !clean_name(name).is_empty())
    {
        return uri.to_string();
    }
    let Some((head, query)) = uri.split_once('?') else {
        return uri.to_string();
    };
    let params: Vec<&str> = query
        .split('&')
        .filter(|param| !param.is_empty() && !param.starts_with("dn="))
        .collect();
    format!("{head}?{}&dn={}", params.join("&"), hex(magnet.info_hash()))
}

/// Locks a mutex, recovering from poisoning instead of panicking. A panic in one
/// critical section must not cascade into every later operation; the poison is
/// logged so silent state corruption is at least traceable.
fn lock_recover<'a, T>(m: &'a Mutex<T>, what: &str) -> MutexGuard<'a, T> {
    m.lock().unwrap_or_else(|e| {
        log::error!("Mutex poison detected on {}; recovering inner state", what);
        e.into_inner()
    })
}

fn hash_uri(uri: &str) -> String {
    use sha1::{Digest, Sha1};
    let mut hasher = Sha1::new();
    hasher.update(uri.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Canonical identity for a torrent: the real BitTorrent info hash (hex) when
/// the URI is a parseable magnet link or `.torrent` file, so the same content
/// added via magnet and via file maps to one entry instead of two concurrent
/// downloads. Falls back to hashing the URI text when nothing parses.
pub(crate) fn torrent_id(uri: &str) -> String {
    use mtorrent::utils::re_exports::mtorrent_core::input::{MagnetLink, Metainfo};
    use std::str::FromStr;

    let path = std::path::Path::new(uri);
    if path.is_file() {
        if let Ok(meta) = Metainfo::from_file(path) {
            return hex(meta.info_hash());
        }
    } else if let Ok(magnet) = MagnetLink::from_str(uri) {
        return hex(magnet.info_hash());
    }
    hash_uri(uri)
}

/// An info hash in lowercase hex.
fn hex(hash: &[u8; 20]) -> String {
    use std::fmt::Write;

    hash.iter().fold(String::with_capacity(40), |mut s, b| {
        let _ = write!(s, "{:02x}", b);
        s
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{TorrentUiState, listening_port, lock_recover, name_nameless_magnet, torrent_id};
    use crate::test_support::{Harness, TestTorrent, closed_addr};

    const WAIT: Duration = Duration::from_secs(20);

    /// A magnet link whose only peer does not exist: the torrent runs and gets nowhere.
    fn magnet_to_nowhere(n: u8) -> String {
        let hash: String = std::iter::repeat_n(format!("{n:02x}"), 20).collect();
        format!("magnet:?xt=urn:btih:{hash}&x.pe={}", closed_addr())
    }

    #[test]
    fn a_started_torrent_runs_until_paused_and_again_once_resumed() {
        let h = Harness::new("engine-toggle", 0);
        let hash = h.engine.start(
            "Name".into(),
            magnet_to_nowhere(1),
            h.output_dir(),
            false,
            h.tx.clone(),
        );
        let first = h.wait_for_update(&hash, WAIT, |_| true);
        assert_eq!(first.state, TorrentUiState::Downloading);
        assert_eq!(first.name, "Name");
        assert!(h.engine.is_active(&hash));

        h.engine.toggle(&hash);
        assert!(!h.engine.is_active(&hash));
        h.wait_for_update(&hash, WAIT, |u| u.state == TorrentUiState::Paused);

        h.engine.toggle(&hash);
        assert!(h.engine.is_active(&hash));
        h.wait_for_update(&hash, WAIT, |u| u.state == TorrentUiState::Downloading);

        h.engine.stop(&hash);
        assert!(!h.engine.is_active(&hash));
        // A stopped torrent is gone: there is nothing to resume.
        h.engine.toggle(&hash);
        assert!(!h.engine.is_active(&hash));
    }

    #[test]
    fn starting_a_running_torrent_again_starts_nothing_new() {
        let h = Harness::new("engine-restart", 0);
        let uri = magnet_to_nowhere(2);
        let hash = h.engine.start(
            String::new(),
            uri.clone(),
            h.output_dir(),
            false,
            h.tx.clone(),
        );
        let again = h
            .engine
            .start(String::new(), uri, h.output_dir(), true, h.tx.clone());
        assert_eq!(hash, again);
        // The second start only reports the new sequential setting.
        h.wait_for_update(&hash, WAIT, |u| u.sequential);

        h.engine.pause_all();
        assert!(!h.engine.is_active(&hash));
        h.engine.toggle(&hash);
        assert!(h.engine.is_active(&hash));
    }

    #[test]
    fn a_torrent_added_paused_starts_when_resumed() {
        let h = Harness::new("engine-paused", 0);
        let hash = h.engine.add_paused(
            "Paused".into(),
            magnet_to_nowhere(3),
            h.output_dir(),
            false,
            h.tx.clone(),
        );
        let update = h.wait_for_update(&hash, WAIT, |_| true);
        assert_eq!(update.state, TorrentUiState::Paused);
        assert!(!h.engine.is_active(&hash));

        h.engine.toggle(&hash);
        assert!(h.engine.is_active(&hash));
    }

    #[test]
    fn a_failed_torrent_can_be_retried() {
        let h = Harness::new("engine-failed", 0);
        let hash = h.engine.start(
            String::new(),
            magnet_to_nowhere(4),
            h.output_dir(),
            false,
            h.tx.clone(),
        );
        h.engine.mark_failed(&hash);
        assert!(!h.engine.is_active(&hash));
        h.engine.toggle(&hash);
        assert!(h.engine.is_active(&hash));
    }

    #[test]
    fn running_torrents_get_consecutive_ports_and_a_new_one_the_first_free() {
        let h = Harness::new("engine-ports", 47_000);
        let start = |n| {
            h.engine.start(
                String::new(),
                magnet_to_nowhere(n),
                h.output_dir(),
                false,
                h.tx.clone(),
            )
        };
        let port = |hash: &str| lock_recover(&h.engine.active, "active map")[hash].port;

        let hashes: Vec<String> = [5, 6, 7].into_iter().map(start).collect();
        let ports: Vec<u16> = hashes.iter().map(|hash| port(hash)).collect();
        assert_eq!(ports, [47_000, 47_001, 47_002]);

        // Paused, the first torrent gives its port up to the next one to start.
        h.engine.toggle(&hashes[0]);
        let fourth = start(8);
        assert_eq!(port(&fourth), 47_000);
        h.engine.toggle(&hashes[0]);
        assert_eq!(port(&hashes[0]), 47_003);
    }

    #[test]
    fn a_torrent_file_shows_its_name_and_size_at_once() {
        let h = Harness::new("engine-file", 0);
        let torrent = TestTorrent::create(h.dir.path(), "From A File", 100_000, 16 * 1024);
        let uri = torrent.metainfo_path.to_string_lossy().into_owned();
        let hash = h
            .engine
            .start("stem".into(), uri, h.output_dir(), false, h.tx.clone());
        assert_eq!(hash, torrent.hex_hash());
        let update = h.wait_for_update(&hash, WAIT, |u| u.total > 0);
        assert_eq!(update.name, "From A File");
        assert_eq!(update.total, 100_000);
        assert_eq!(update.total_pieces, 7);
    }

    #[test]
    fn running_torrents_listen_on_ports_of_their_own() {
        assert_eq!(listening_port(0, [0, 0]), 0);
        assert_eq!(listening_port(6881, []), 6881);
        assert_eq!(listening_port(6881, [6881, 6883]), 6882);
        assert_eq!(listening_port(u16::MAX, [u16::MAX]), 0);
    }

    #[test]
    fn torrent_id_uses_magnet_info_hash() {
        let hex = "0123456789abcdef0123456789abcdef01234567";
        let with_dn = format!("magnet:?xt=urn:btih:{hex}&dn=Some%20Name");
        let without_dn = format!("magnet:?xt=urn:btih:{hex}");
        // Same content, different URI text: identical identity.
        assert_eq!(torrent_id(&with_dn), hex);
        assert_eq!(torrent_id(&with_dn), torrent_id(&without_dn));
    }

    #[test]
    fn torrent_id_falls_back_for_unparseable_uris() {
        let a = torrent_id("not a magnet at all");
        let b = torrent_id("not a magnet at all");
        let c = torrent_id("something else");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 40);
    }

    #[test]
    fn nameless_magnet_links_are_named_after_their_info_hash() {
        let hex = "0123456789abcdef0123456789abcdef01234567";
        let named = format!("magnet:?xt=urn:btih:{hex}&dn=Some%20Name");
        assert_eq!(name_nameless_magnet(&named), named);
        assert_eq!(
            name_nameless_magnet(&format!("magnet:?xt=urn:btih:{hex}&tr=udp%3A%2F%2Fx%3A1")),
            format!("magnet:?xt=urn:btih:{hex}&tr=udp%3A%2F%2Fx%3A1&dn={hex}")
        );
        // A name that cleans up to nothing is no name.
        assert_eq!(
            name_nameless_magnet(&format!("magnet:?dn=..&xt=urn:btih:{hex}")),
            format!("magnet:?xt=urn:btih:{hex}&dn={hex}")
        );
        assert_eq!(name_nameless_magnet("not a magnet"), "not a magnet");
    }
}
