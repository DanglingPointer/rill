//! The main window: the torrents grouped by state, search, selection mode, and the
//! `win.*` actions through which the rows, the dialogs and the tray reach the engine.

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use async_channel::{Receiver, Sender};
use gettextrs::{gettext, ngettext};
use gtk::{gdk, gio, glib};

use crate::dialogs::{AddTorrentDialog, TorrentInfoDialog};
use crate::engine::{TorrentEngine, TorrentUiState, UiEvent, UiUpdate};
use crate::storage::{SavedTorrent, Storage};
use crate::torrent_paths;
use crate::torrent_row::TorrentRow;
use crate::tray;

/// What the database last received for a torrent: state, downloaded, total, total
/// pieces, downloaded pieces.
type PersistedSnapshot = (&'static str, u64, u64, u64, u64);

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/io/github/sachesi/rill/ui/window.ui")]
    #[properties(wrapper_type = super::RillWindow)]
    pub struct RillWindow {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub window_title: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub search_button: TemplateChild<gtk::ToggleButton>,
        #[template_child]
        pub search_bar: TemplateChild<gtk::SearchBar>,
        #[template_child]
        pub search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub content_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub downloading_title: TemplateChild<gtk::Label>,
        #[template_child]
        pub downloading_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub paused_title: TemplateChild<gtk::Label>,
        #[template_child]
        pub paused_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub finished_title: TemplateChild<gtk::Label>,
        #[template_child]
        pub finished_list: TemplateChild<gtk::ListBox>,

        #[property(get, set = Self::set_selection_mode)]
        pub selection_mode: Cell<bool>,

        pub engine: OnceCell<Rc<TorrentEngine>>,
        pub storage: OnceCell<Storage>,
        pub tx: OnceCell<Sender<UiEvent>>,
        pub rows: RefCell<HashMap<String, TorrentRow>>,
        pub info_dialogs: RefCell<HashMap<String, TorrentInfoDialog>>,
        /// Skips database writes for snapshots that change nothing it stores.
        pub last_persisted: RefCell<HashMap<String, PersistedSnapshot>>,
        /// Deleted torrents, whose late updates are dropped.
        pub deleted: RefCell<HashSet<String>>,
        /// Torrents the user paused. The download queue never resumes these.
        pub user_paused: RefCell<HashSet<String>>,
        /// Torrents the download queue paused. They are stored as downloading, so they
        /// resume when a slot frees up, after a restart too.
        pub auto_paused: RefCell<HashSet<String>>,
        /// Whether the notice that Rill keeps running in the tray has been sent.
        pub background_notice_sent: Cell<bool>,
        pub queue_check_pending: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RillWindow {
        const NAME: &'static str = "RillWindow";
        type Type = super::RillWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();

            klass.install_action("win.add-file", None, |win, _, _| win.choose_torrent_file());
            klass.install_action("win.add-magnet", None, |win, _, _| win.add_magnet_link(""));
            klass.install_action("win.search", None, |win, _, _| {
                let button = &win.imp().search_button;
                button.set_active(!button.is_active());
            });

            klass.install_action("win.select", None, |win, _, _| win.set_selection_mode(true));
            klass.install_action("win.leave-selection", None, |win, _, _| {
                win.set_selection_mode(false)
            });
            klass.install_action("win.select-all", None, |win, _, _| win.select_all(true));
            klass.install_action("win.select-none", None, |win, _, _| win.select_all(false));
            klass.install_action("win.resume-selected", None, |win, _, _| {
                for hash in win.selected_hashes() {
                    win.resume_torrent(&hash);
                }
                win.set_selection_mode(false);
            });
            klass.install_action("win.pause-selected", None, |win, _, _| {
                for hash in win.selected_hashes() {
                    win.pause_torrent(&hash);
                }
                win.set_selection_mode(false);
            });
            klass.install_action("win.delete-selected", None, |win, _, _| {
                win.confirm_delete(win.selected_hashes());
            });

            let hash = Some(glib::VariantTy::STRING);
            klass.install_action("win.pause-torrent", hash, |win, _, v| {
                if let Some(hash) = v.and_then(|v| v.str()) {
                    win.pause_torrent(hash);
                }
            });
            klass.install_action("win.resume-torrent", hash, |win, _, v| {
                if let Some(hash) = v.and_then(|v| v.str()) {
                    win.resume_torrent(hash);
                }
            });
            klass.install_action("win.delete-torrent", hash, |win, _, v| {
                if let Some(hash) = v.and_then(|v| v.str()) {
                    win.confirm_delete(vec![hash.to_string()]);
                }
            });

            klass.add_binding_action(
                gdk::Key::Escape,
                gdk::ModifierType::empty(),
                "win.leave-selection",
            );
            klass.add_binding_action(
                gdk::Key::Delete,
                gdk::ModifierType::empty(),
                "win.delete-selected",
            );
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for RillWindow {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.update_selection_actions();
            obj.setup_drop_target();
            self.search_bar
                .connect_search_mode_enabled_notify(glib::clone!(
                    #[weak]
                    obj,
                    move |bar| {
                        if !bar.is_search_mode() {
                            obj.imp().search_entry.set_text("");
                        }
                    }
                ));
        }
    }

    impl WidgetImpl for RillWindow {}

    impl WindowImpl for RillWindow {
        fn close_request(&self) -> glib::Propagation {
            let obj = self.obj();
            obj.save_window_state();

            let Some(app) = obj.application() else {
                return self.parent_close_request();
            };
            // Without a tray there is no way back to a hidden window, so closing quits,
            // which pauses every torrent.
            if !tray::is_available() {
                app.quit();
                return glib::Propagation::Stop;
            }
            // Otherwise the window hides and the transfers go on. Say so the first time,
            // since a window vanishing mid-download looks just like quitting.
            if !self.background_notice_sent.replace(true) {
                let notification = gio::Notification::new(&gettext("Rill is still running"));
                notification.set_body(Some(&gettext(
                    "Downloads continue in the background. Use the tray icon to reopen or quit Rill.",
                )));
                app.send_notification(Some("background"), &notification);
            }
            obj.set_visible(false);
            glib::Propagation::Stop
        }
    }

    impl ApplicationWindowImpl for RillWindow {}
    impl AdwApplicationWindowImpl for RillWindow {}

    #[gtk::template_callbacks]
    impl RillWindow {
        #[template_callback]
        fn on_search_changed(&self) {
            self.obj().apply_filter();
        }

        #[template_callback]
        fn on_row_activated(&self, row: &gtk::ListBoxRow) {
            let Some(row) = row.downcast_ref::<TorrentRow>() else {
                return;
            };
            if self.selection_mode.get() {
                row.set_selected(!row.selected());
            } else {
                self.obj().show_info(row);
            }
        }
    }

    impl RillWindow {
        fn set_selection_mode(&self, active: bool) {
            if self.selection_mode.replace(active) == active {
                return;
            }
            let obj = self.obj();
            if active {
                self.search_bar.set_search_mode(false);
            } else {
                for row in self.rows.borrow().values() {
                    row.set_selected(false);
                }
            }
            obj.update_selection_actions();
            obj.notify_selection_mode();
        }
    }
}

glib::wrapper! {
    pub struct RillWindow(ObjectSubclass<imp::RillWindow>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
                    gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl RillWindow {
    pub fn new(
        app: &impl IsA<gtk::Application>,
        engine: Rc<TorrentEngine>,
        storage: Storage,
        saved: Vec<SavedTorrent>,
    ) -> Self {
        let window: Self = glib::Object::builder().property("application", app).build();
        let imp = window.imp();
        let (tx, rx) = async_channel::unbounded();

        let settings = storage.load_settings();
        window.set_default_size(settings.window_width, settings.window_height);
        if settings.window_maximized {
            window.maximize();
        }

        imp.engine.set(engine).ok();
        imp.storage.set(storage).ok();
        imp.tx.set(tx).ok();

        window.restore_torrents(saved);
        window.listen(rx);
        window
    }

    fn engine(&self) -> &TorrentEngine {
        self.imp().engine.get().expect("engine is set in new()")
    }

    pub fn storage(&self) -> &Storage {
        self.imp().storage.get().expect("storage is set in new()")
    }

    fn sender(&self) -> Sender<UiEvent> {
        self.imp().tx.get().expect("sender is set in new()").clone()
    }

    pub fn show_toast(&self, message: &str) {
        self.imp().toast_overlay.add_toast(adw::Toast::new(message));
    }

    pub fn add_magnet_link(&self, uri: &str) {
        let dialog = AddTorrentDialog::new(self);
        dialog.set_magnet(uri);
        dialog.present(Some(self));
    }

    pub fn add_torrent_file(&self, path: &Path) {
        let dialog = AddTorrentDialog::new(self);
        dialog.set_file(path);
        dialog.present(Some(self));
    }

    /// Hands a torrent to the engine, running or paused. The row appears with the
    /// engine's first update.
    pub fn start_torrent(
        &self,
        name: String,
        uri: String,
        dir: PathBuf,
        sequential: bool,
        start_now: bool,
    ) {
        let hash = if start_now {
            self.engine()
                .start(name, uri, dir, sequential, self.sender())
        } else {
            self.engine()
                .add_paused(name, uri, dir, sequential, self.sender())
        };
        // A torrent deleted earlier in this session may come back.
        self.imp().deleted.borrow_mut().remove(&hash);
    }

    /// Where Rill keeps its database and copies of .torrent files.
    pub fn data_dir(&self) -> PathBuf {
        self.engine().config_dir().clone()
    }

    fn engine_rc(&self) -> Rc<TorrentEngine> {
        self.imp()
            .engine
            .get()
            .expect("engine is set in new()")
            .clone()
    }

    fn choose_torrent_file(&self) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(&gettext("Torrent Files")));
        filter.add_mime_type("application/x-bittorrent");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Add Torrent File"))
            .filters(&filters)
            .modal(true)
            .build();
        dialog.open(
            Some(self),
            gio::Cancellable::NONE,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |result| {
                    if let Ok(path) = result.map(|f| f.path()) {
                        match path {
                            Some(path) => window.add_torrent_file(&path),
                            None => window.show_toast(&gettext("Only local files can be added")),
                        }
                    }
                }
            ),
        );
    }

    /// Accepts .torrent files and magnet links dropped on the window.
    fn setup_drop_target(&self) {
        let target = gtk::DropTarget::new(glib::Type::INVALID, gdk::DragAction::COPY);
        target.set_types(&[gdk::FileList::static_type(), glib::Type::STRING]);
        target.connect_drop(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            false,
            move |_, value, _, _| {
                if let Ok(files) = value.get::<gdk::FileList>() {
                    let paths: Vec<PathBuf> = files
                        .files()
                        .iter()
                        .filter_map(|f| f.path())
                        .filter(|p| p.extension().is_some_and(|e| e == "torrent"))
                        .collect();
                    for path in &paths {
                        window.add_torrent_file(path);
                    }
                    return !paths.is_empty();
                }
                let text = value.get::<String>().unwrap_or_default();
                let Some(line) = text.lines().map(str::trim).find(|l| !l.is_empty()) else {
                    return false;
                };
                if line.starts_with("magnet:") {
                    window.add_magnet_link(line);
                    return true;
                }
                let path = gio::File::for_uri(line)
                    .path()
                    .unwrap_or_else(|| PathBuf::from(line));
                if path.extension().is_some_and(|e| e == "torrent") {
                    window.add_torrent_file(&path);
                    return true;
                }
                false
            }
        ));
        self.add_controller(target);
    }

    fn save_window_state(&self) {
        let (width, height) = self.default_size();
        let maximized = self.is_maximized();
        let storage = self.storage().clone();
        storage.execute(move |s| {
            let mut settings = s.load_settings();
            settings.window_width = width;
            settings.window_height = height;
            settings.window_maximized = maximized;
            if let Err(e) = s.save_settings(&settings) {
                log::warn!("Failed to save the window size: {e}");
            }
        });
    }

    // Search and selection

    fn apply_filter(&self) {
        let imp = self.imp();
        let query = imp.search_entry.text().to_lowercase();
        for row in imp.rows.borrow().values() {
            row.set_visible(row.matches(&query));
        }
        self.update_sections();
    }

    fn select_all(&self, selected: bool) {
        for row in self.imp().rows.borrow().values() {
            if row.get_visible() {
                row.set_selected(selected);
            }
        }
    }

    fn selected_hashes(&self) -> Vec<String> {
        self.imp()
            .rows
            .borrow()
            .values()
            .filter(|row| row.selected())
            .map(TorrentRow::info_hash)
            .collect()
    }

    fn update_selection_actions(&self) {
        let active = self.selection_mode();
        let any = !self.selected_hashes().is_empty();
        self.action_set_enabled("win.select", !active);
        self.action_set_enabled("win.leave-selection", active);
        self.action_set_enabled("win.select-all", active);
        self.action_set_enabled("win.select-none", active && any);
        self.action_set_enabled("win.resume-selected", active && any);
        self.action_set_enabled("win.pause-selected", active && any);
        self.action_set_enabled("win.delete-selected", active && any);

        let title = &self.imp().window_title;
        if active {
            let count = self.selected_hashes().len();
            title.set_title(&gettext("Select Torrents"));
            title.set_subtitle(
                &ngettext("%d selected", "%d selected", count as u32)
                    .replace("%d", &count.to_string()),
            );
        } else {
            title.set_title("Rill");
            title.set_subtitle("");
        }
    }

    // Transfers

    fn pause_torrent(&self, hash: &str) {
        let imp = self.imp();
        let Some(row) = imp.rows.borrow().get(hash).cloned() else {
            return;
        };
        if row.state() != TorrentUiState::Downloading {
            return;
        }
        imp.user_paused.borrow_mut().insert(hash.to_string());
        imp.auto_paused.borrow_mut().remove(hash);
        if let Some(mut update) = row.latest() {
            update.state = TorrentUiState::Paused;
            update.speed_down = 0;
            update.speed_up = 0;
            self.process_update(&update);
        }
        self.engine().toggle(hash);
        self.check_queue();
    }

    fn resume_torrent(&self, hash: &str) {
        let imp = self.imp();
        let Some(row) = imp.rows.borrow().get(hash).cloned() else {
            return;
        };
        if !matches!(row.state(), TorrentUiState::Paused | TorrentUiState::Error) {
            return;
        }
        imp.user_paused.borrow_mut().remove(hash);
        imp.auto_paused.borrow_mut().remove(hash);
        if let Some(mut update) = row.latest() {
            update.state = TorrentUiState::Downloading;
            self.process_update(&update);
        }
        self.engine().toggle(hash);
        self.check_queue();
    }

    /// Asks before deleting `hashes`, and whether to delete their downloaded data too.
    fn confirm_delete(&self, hashes: Vec<String>) {
        let Some(first) = hashes.first() else {
            return;
        };
        let (heading, body) = if hashes.len() == 1 {
            let name = self
                .imp()
                .rows
                .borrow()
                .get(first)
                .map(TorrentRow::name)
                .unwrap_or_default();
            (
                gettext("Delete Torrent?"),
                // Translators: %s is the name of a torrent.
                gettext("“%s” will be removed from the list.").replace("%s", &name),
            )
        } else {
            (
                gettext("Delete Torrents?"),
                ngettext(
                    "%d torrent will be removed from the list.",
                    "%d torrents will be removed from the list.",
                    hashes.len() as u32,
                )
                .replace("%d", &hashes.len().to_string()),
            )
        };
        let delete_data =
            gtk::CheckButton::with_label(&gettext("Also delete the downloaded files"));
        let dialog = adw::AlertDialog::builder()
            .heading(heading)
            .body(body)
            .extra_child(&delete_data)
            .close_response("cancel")
            .default_response("cancel")
            .build();
        dialog.add_response("cancel", &gettext("_Cancel"));
        dialog.add_response("delete", &gettext("_Delete"));
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.connect_response(
            Some("delete"),
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |_, _| {
                    for hash in &hashes {
                        window.delete_torrent(hash, delete_data.is_active());
                    }
                    window.set_selection_mode(false);
                }
            ),
        );
        dialog.present(Some(self));
    }

    fn delete_torrent(&self, hash: &str, delete_data: bool) {
        let imp = self.imp();
        imp.deleted.borrow_mut().insert(hash.to_string());
        imp.user_paused.borrow_mut().remove(hash);
        imp.auto_paused.borrow_mut().remove(hash);
        imp.last_persisted.borrow_mut().remove(hash);
        let dialog = imp.info_dialogs.borrow_mut().remove(hash);
        if let Some(dialog) = dialog {
            dialog.close();
        }
        self.engine().stop(hash);

        let row = imp.rows.borrow_mut().remove(hash);
        if let Some(row) = row
            && let Some(list) = row.parent().and_downcast::<gtk::ListBox>()
        {
            list.remove(&row);
        }
        self.update_sections();
        self.update_selection_actions();
        self.check_queue();

        let storage = self.storage().clone();
        let hash = hash.to_string();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                // Read where the data is before the record goes; the worker runs jobs in
                // order, so the delete below comes after this read.
                if delete_data {
                    let key = hash.clone();
                    let record = storage
                        .query(move |s| s.load_torrent(&key).ok().flatten())
                        .await
                        .ok()
                        .flatten();
                    let path = record
                        .and_then(|t| torrent_paths::content_path(&t.uri, &t.output_dir_path()));
                    match path {
                        Some(path) => {
                            let target = path.clone();
                            let result =
                                gio::spawn_blocking(move || torrent_paths::remove_content(&target))
                                    .await;
                            match result {
                                Ok(Ok(())) => log::info!("Deleted {}", path.display()),
                                Ok(Err(e)) => {
                                    log::warn!("Failed to delete {}: {e}", path.display());
                                    window.show_toast(
                                        &gettext("Could not delete the downloaded files: %s")
                                            .replace("%s", &e.to_string()),
                                    );
                                }
                                Err(_) => log::warn!("The delete task panicked"),
                            }
                        }
                        None => {
                            log::warn!("No content path for torrent {hash}");
                            window.show_toast(&gettext("Could not find the downloaded files"));
                        }
                    }
                }
                storage.execute(move |s| {
                    if let Err(e) = s.delete_torrent(&hash) {
                        log::warn!("Failed to delete torrent {hash} from the database: {e}");
                    }
                });
            }
        ));
    }

    fn show_info(&self, row: &TorrentRow) {
        let hash = row.info_hash();
        if let Some(dialog) = self.imp().info_dialogs.borrow().get(&hash) {
            dialog.present(Some(self));
            return;
        }
        let Some(update) = row.latest() else {
            return;
        };
        let dialog = TorrentInfoDialog::new(&update, self.engine_rc(), self.storage().clone());
        dialog.connect_closed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                window.imp().info_dialogs.borrow_mut().remove(&hash);
            }
        ));
        self.imp()
            .info_dialogs
            .borrow_mut()
            .insert(row.info_hash(), dialog.clone());
        dialog.present(Some(self));
    }

    // Updates from the engine

    fn listen(&self, rx: Receiver<UiEvent>) {
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                while let Ok(event) = rx.recv().await {
                    match event {
                        UiEvent::Update(update) => window.process_update(&update),
                        UiEvent::Finished { info_hash, error } => {
                            window.torrent_finished(&info_hash, error)
                        }
                    }
                }
            }
        ));
    }

    /// A torrent task ended, done or failed.
    fn torrent_finished(&self, hash: &str, error: Option<String>) {
        // The final snapshot may never have come (a zero-byte torrent never reports its
        // bytes done), so the row is moved here rather than left downloading.
        let state = if error.is_some() {
            TorrentUiState::Error
        } else {
            TorrentUiState::Completed
        };
        let row = self.imp().rows.borrow().get(hash).cloned();
        if let Some(mut update) = row.as_ref().and_then(TorrentRow::latest) {
            update.state = state;
            update.peers = 0;
            update.speed_down = 0;
            update.speed_up = 0;
            update.peers_list.clear();
            self.process_update(&update);
        }
        let name = row.map_or_else(|| hash.to_string(), |r| r.name());

        match error {
            Some(error) => {
                log::error!("Torrent {hash} failed: {error}");
                // The failed task leaves the engine's running set, so Retry restarts it.
                self.engine().mark_failed(hash);
                self.show_toast(
                    // Translators: %1 is a torrent name, %2 the reason it failed.
                    &gettext("“%1” failed: %2")
                        .replace("%1", &name)
                        .replace("%2", &error),
                );
            }
            None => {
                let key = hash.to_string();
                self.storage().execute(move |s| {
                    if let Err(e) = s.mark_completed(&key) {
                        log::warn!("Failed to mark {key} completed: {e}");
                    }
                });
                let notification = gio::Notification::new(&gettext("Download Complete"));
                notification.set_body(Some(&name));
                if let Some(app) = self.application() {
                    app.send_notification(None, &notification);
                }
            }
        }
    }

    fn process_update(&self, update: &UiUpdate) {
        let imp = self.imp();
        if imp.deleted.borrow().contains(&update.info_hash) {
            return;
        }

        let existing = imp.rows.borrow().get(&update.info_hash).cloned();
        let previous = existing.as_ref().and_then(TorrentRow::latest);
        let mut update = update.clone();
        // Snapshots taken before the metadata arrived, and those the engine makes up
        // itself, carry no sizes, and the engine's no name: keep the last known ones.
        if let Some(previous) = &previous {
            if update.name.is_empty() {
                update.name = previous.name.clone();
            }
            if update.total == 0 {
                update.total = previous.total;
                update.downloaded = previous.downloaded;
            }
            if update.total_pieces == 0 {
                update.total_pieces = previous.total_pieces;
                update.downloaded_pieces = previous.downloaded_pieces;
            }
        }

        let row = match existing {
            Some(row) => row,
            None => match self.insert_torrent(&update) {
                Some(row) => row,
                None => return,
            },
        };
        let old_state = row.state();
        row.update(&update);
        if previous.is_some_and(|previous| previous.name != update.name) {
            self.rename(&update.info_hash, &update.name);
        }
        if let Some(dialog) = imp.info_dialogs.borrow().get(&update.info_hash) {
            dialog.apply_update(&update);
        }
        self.persist(&update);

        if row.parent().is_none() || old_state != update.state {
            if let Some(list) = row.parent().and_downcast::<gtk::ListBox>() {
                list.remove(&row);
            }
            self.list_for(update.state).append(&row);
            self.update_sections();
            self.check_queue();
        }
    }

    /// Keeps the name a torrent's metadata gave it, for its next run and the next session.
    fn rename(&self, hash: &str, name: &str) {
        self.engine().rename(hash, name);
        let (key, name) = (hash.to_string(), name.to_string());
        self.storage().execute(move |s| {
            if let Err(e) = s.update_torrent_name(&key, &name) {
                log::warn!("Failed to save the name of {key}: {e}");
            }
        });
    }

    /// Saves a torrent seen for the first time and makes its row.
    fn insert_torrent(&self, update: &UiUpdate) -> Option<TorrentRow> {
        let mut record = SavedTorrent::new(
            update.info_hash.clone(),
            update.name.clone(),
            update.uri.clone(),
            state_key(update.state).to_string(),
            update.downloaded,
            update.total,
            update.output_dir.clone(),
        );
        record.total_pieces = update.total_pieces as u64;
        record.downloaded_pieces = update.downloaded_pieces as u64;
        record.sequential = update.sequential;
        // Written at once rather than queued: the queue check and a restart must both
        // find the new torrent in the database.
        if let Err(e) = self.storage().save_torrent(&record) {
            log::warn!("Failed to save new torrent: {e}");
            self.engine().stop(&update.info_hash);
            self.show_toast(&gettext("Could not save the torrent: %s").replace("%s", &e));
            return None;
        }
        Some(self.make_row(&update.info_hash))
    }

    fn make_row(&self, hash: &str) -> TorrentRow {
        let row = TorrentRow::new(hash);
        self.bind_property("selection-mode", &row, "selection-mode")
            .sync_create()
            .build();
        row.connect_selected_notify(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.update_selection_actions()
        ));
        let query = self.imp().search_entry.text().to_lowercase();
        row.set_visible(row.matches(&query));
        self.imp()
            .rows
            .borrow_mut()
            .insert(hash.to_string(), row.clone());
        row
    }

    fn list_for(&self, state: TorrentUiState) -> gtk::ListBox {
        let imp = self.imp();
        match state {
            TorrentUiState::Downloading => imp.downloading_list.get(),
            TorrentUiState::Paused => imp.paused_list.get(),
            TorrentUiState::Completed | TorrentUiState::Error => imp.finished_list.get(),
        }
    }

    /// Shows the sections that have rows to show, or the page saying there are none.
    fn update_sections(&self) {
        let imp = self.imp();
        let visible_rows = |list: &gtk::ListBox| {
            let mut child = list.first_child();
            while let Some(widget) = child {
                if widget.get_visible() {
                    return true;
                }
                child = widget.next_sibling();
            }
            false
        };
        let mut any = false;
        for (title, list) in [
            (&imp.downloading_title, &imp.downloading_list),
            (&imp.paused_title, &imp.paused_list),
            (&imp.finished_title, &imp.finished_list),
        ] {
            let shown = visible_rows(list);
            title.set_visible(shown);
            list.set_visible(shown);
            any |= shown;
        }
        let page = if any {
            "list"
        } else if imp.rows.borrow().is_empty() {
            "empty"
        } else {
            "no-results"
        };
        imp.content_stack.set_visible_child_name(page);
    }

    fn restore_torrents(&self, saved: Vec<SavedTorrent>) {
        let tx = self.sender();
        for torrent in saved {
            let state = match torrent.state.as_str() {
                "completed" => TorrentUiState::Completed,
                "error" => TorrentUiState::Error,
                _ => TorrentUiState::Paused,
            };
            let update = UiUpdate {
                downloaded: torrent.downloaded,
                total: torrent.total,
                total_pieces: torrent.total_pieces as usize,
                downloaded_pieces: torrent.downloaded_pieces as usize,
                ..UiUpdate::idle(
                    torrent.info_hash.clone(),
                    torrent.name.clone(),
                    state,
                    torrent.output_dir_path(),
                    torrent.uri.clone(),
                    torrent.sequential,
                )
            };
            // Registered as paused, so that resuming starts the task.
            self.engine().add_paused_silent(
                torrent.name.clone(),
                torrent.uri.clone(),
                torrent.output_dir_path(),
                torrent.sequential,
                tx.clone(),
            );
            let row = self.make_row(&torrent.info_hash);
            row.update(&update);
            self.list_for(state).append(&row);
        }
        self.update_sections();
        // Torrents still stored as downloading, because Rill did not shut down cleanly,
        // are started as far as the queue allows.
        self.check_queue();
    }

    fn persist(&self, update: &UiUpdate) {
        let state = if update.state == TorrentUiState::Paused
            && self.imp().auto_paused.borrow().contains(&update.info_hash)
        {
            // Paused by the queue, not by the user: resume when a slot frees up.
            "downloading"
        } else {
            state_key(update.state)
        };
        let snapshot = (
            state,
            update.downloaded,
            update.total,
            update.total_pieces as u64,
            update.downloaded_pieces as u64,
        );
        let imp = self.imp();
        if imp.last_persisted.borrow().get(&update.info_hash) == Some(&snapshot) {
            return;
        }
        imp.last_persisted
            .borrow_mut()
            .insert(update.info_hash.clone(), snapshot);

        let storage = self.storage().clone();
        let hash = update.info_hash.clone();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let key = hash.clone();
                let (state, downloaded, total, total_pieces, downloaded_pieces) = snapshot;
                let result = storage
                    .query(move |s| {
                        s.update_torrent_state(
                            &key,
                            state,
                            downloaded,
                            total,
                            total_pieces,
                            downloaded_pieces,
                        )
                    })
                    .await
                    .and_then(|r| r);
                if let Err(e) = result {
                    log::warn!("Failed to save the state of {hash}: {e}");
                    // Forget it, so that the next identical snapshot tries again.
                    let mut persisted = window.imp().last_persisted.borrow_mut();
                    if persisted.get(&hash) == Some(&snapshot) {
                        persisted.remove(&hash);
                    }
                }
            }
        ));
    }

    /// Runs the download queue once the current burst of changes is over.
    pub fn check_queue(&self) {
        let imp = self.imp();
        if imp.queue_check_pending.replace(true) {
            return;
        }
        glib::timeout_add_local_once(
            std::time::Duration::from_millis(50),
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move || {
                    window.imp().queue_check_pending.set(false);
                    window.run_queue();
                }
            ),
        );
    }

    /// Pauses the newest downloads above the limit, or starts the oldest waiting ones
    /// while there is room. A torrent the user paused is never started.
    fn run_queue(&self) {
        let imp = self.imp();
        // Read synchronously: a torrent added a moment ago must be in the snapshot.
        let (settings, saved) = self.storage().load_settings_and_torrents();
        let limit = settings.max_active_downloads.max(1) as usize;
        let engine = self.engine();
        let rows = imp.rows.borrow();

        let mut active: Vec<(&str, i64)> = Vec::new();
        let mut waiting: Vec<(&str, i64)> = Vec::new();
        for torrent in &saved {
            let hash = torrent.info_hash.as_str();
            if torrent.state == "completed" || !rows.contains_key(hash) {
                continue;
            }
            if engine.is_active(hash) {
                active.push((hash, torrent.added_at));
            } else if !imp.user_paused.borrow().contains(hash)
                && (torrent.state == "downloading" || imp.auto_paused.borrow().contains(hash))
            {
                waiting.push((hash, torrent.added_at));
            }
        }

        if active.len() > limit {
            active.sort_by_key(|&(_, added)| std::cmp::Reverse(added));
            let excess = active.len() - limit;
            log::info!(
                "{} downloads over the limit of {limit}; pausing {excess}",
                active.len()
            );
            for &(hash, _) in active.iter().take(excess) {
                imp.auto_paused.borrow_mut().insert(hash.to_string());
                engine.toggle(hash);
            }
        } else if active.len() < limit && !waiting.is_empty() {
            waiting.sort_by_key(|&(_, added)| added);
            let room = limit - active.len();
            for &(hash, _) in waiting.iter().take(room) {
                log::info!("Starting queued torrent {hash}");
                imp.auto_paused.borrow_mut().remove(hash);
                engine.toggle(hash);
            }
        }
    }
}

/// How the database spells a state.
fn state_key(state: TorrentUiState) -> &'static str {
    match state {
        TorrentUiState::Downloading => "downloading",
        TorrentUiState::Paused => "paused",
        TorrentUiState::Completed => "completed",
        TorrentUiState::Error => "error",
    }
}
