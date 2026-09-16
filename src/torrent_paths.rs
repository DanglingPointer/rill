//! Where a torrent's content is on disk, and removing it.

use std::path::{Component, Path, PathBuf};

use mtorrent::utils::re_exports::mtorrent_core::input::MagnetLink;

/// The file or folder mtorrent writes a torrent's content to: the magnet's name, or the
/// stem of the .torrent file, inside `output_dir`. `None` when that name would not be a
/// single entry of `output_dir`.
pub fn content_path(uri: &str, output_dir: &Path) -> Option<PathBuf> {
    use std::str::FromStr;

    let sanitized = crate::engine::sanitize_magnet_dn(uri);
    if let Ok(magnet) = MagnetLink::from_str(&sanitized) {
        return contained_path(output_dir, magnet.name().unwrap_or("unnamed"));
    }
    Path::new(uri)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .and_then(|stem| contained_path(output_dir, stem))
}

/// The folder to show for a torrent: the one its content is in, or `output_dir` until that
/// exists.
pub fn folder_to_open(uri: &str, output_dir: &Path) -> PathBuf {
    content_path(uri, output_dir)
        .filter(|path| path.is_dir())
        .unwrap_or_else(|| output_dir.to_path_buf())
}

/// The .torrent file of a torrent: the one it was added from, or for a magnet link the one
/// mtorrent saves in `output_dir` once it has fetched the metadata, named like the content.
pub fn metainfo_path(uri: &str, output_dir: &Path) -> Option<PathBuf> {
    use std::str::FromStr;

    let sanitized = crate::engine::sanitize_magnet_dn(uri);
    if let Ok(magnet) = MagnetLink::from_str(&sanitized) {
        let name = magnet.name().unwrap_or("unnamed");
        return contained_path(output_dir, &format!("{name}.torrent"));
    }
    Some(PathBuf::from(uri))
}

/// `output_dir/name`, provided `name` is one plain path component: a name taken from a
/// torrent must not reach outside the download folder.
pub fn contained_path(output_dir: &Path, name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains('\\') || name.contains('\0') {
        return None;
    }
    let mut components = Path::new(name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(component)), None) => Some(output_dir.join(component)),
        _ => None,
    }
}

/// Moves a torrent's content, and the .torrent file kept beside it, from `old_dir` to
/// `new_dir`. Returns whether anything moved. Either everything moves or nothing does:
/// what is in the way is found before the first move, and a move that fails part of the
/// way through is undone.
pub fn move_content(uri: &str, old_dir: &Path, new_dir: &Path) -> std::io::Result<bool> {
    use std::io::{Error, ErrorKind};

    let pairs = [
        (content_path(uri, old_dir), content_path(uri, new_dir)),
        // For a torrent added as a file this is the file itself, which lives wherever
        // the user keeps it and stays there.
        (metainfo_path(uri, old_dir), metainfo_path(uri, new_dir)),
    ];
    let mut to_move = Vec::new();
    for (from, to) in pairs {
        let (Some(from), Some(to)) = (from, to) else {
            continue;
        };
        if !from.starts_with(old_dir) || !from.exists() {
            continue;
        }
        if to.exists() {
            return Err(Error::new(
                ErrorKind::AlreadyExists,
                format!("{} is already there", to.display()),
            ));
        }
        to_move.push((from, to));
    }
    if to_move.is_empty() {
        return Ok(false);
    }

    std::fs::create_dir_all(new_dir)?;
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (from, to) in to_move {
        if let Err(e) = std::fs::rename(&from, &to) {
            for (from, to) in moved {
                // Back where it was, so that Rill and the disk still agree.
                if let Err(e) = std::fs::rename(&to, &from) {
                    log::error!(
                        "Failed to move {} back to {}: {e}",
                        to.display(),
                        from.display()
                    );
                }
            }
            return Err(e);
        }
        moved.push((from, to));
    }
    Ok(true)
}

/// Removes a torrent's content without following a symbolic link at `path`.
pub fn remove_content(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contained_path_rejects_path_like_names() {
        let output_dir = Path::new("/tmp/rill-downloads");

        assert_eq!(contained_path(output_dir, ""), None);
        assert_eq!(contained_path(output_dir, "."), None);
        assert_eq!(contained_path(output_dir, ".."), None);
        assert_eq!(contained_path(output_dir, "show/season"), None);
        assert_eq!(contained_path(output_dir, "show\\season"), None);
        assert_eq!(
            contained_path(output_dir, "show"),
            Some(output_dir.join("show"))
        );
    }

    #[test]
    fn content_path_uses_the_sanitized_magnet_name() {
        let output_dir = Path::new("/tmp/rill-downloads");
        let uri = "magnet:?xt=urn:btih:0123456789012345678901234567890123456789&dn=Show%20%2F%20Season%201";

        assert_eq!(
            content_path(uri, output_dir),
            Some(output_dir.join("Show _ Season 1"))
        );
    }

    #[test]
    fn folder_to_open_is_the_content_folder_once_it_exists() {
        let output_dir = std::env::temp_dir().join(format!("rill-open-{}", std::process::id()));
        // A single-file torrent: the file is film.mkv, its folder is named after the
        // .torrent file.
        let uri = "/tmp/source/film.torrent";
        std::fs::create_dir_all(&output_dir).unwrap();
        assert_eq!(folder_to_open(uri, &output_dir), output_dir);

        std::fs::create_dir(output_dir.join("film")).unwrap();
        let folder = folder_to_open(uri, &output_dir);
        std::fs::remove_dir_all(&output_dir).unwrap();
        assert_eq!(folder, output_dir.join("film"));
    }

    #[test]
    fn metainfo_path_of_a_magnet_is_beside_its_content() {
        let output_dir = Path::new("/tmp/rill-downloads");
        let uri = "magnet:?xt=urn:btih:0123456789012345678901234567890123456789&dn=Show%20%2F%20Season%201";

        assert_eq!(
            metainfo_path(uri, output_dir),
            Some(output_dir.join("Show _ Season 1.torrent"))
        );
        assert_eq!(
            metainfo_path("/tmp/source/Foo.torrent", output_dir),
            Some(PathBuf::from("/tmp/source/Foo.torrent"))
        );
    }

    #[test]
    fn moving_a_torrent_takes_its_content_and_its_saved_metainfo_along() {
        let root = std::env::temp_dir().join(format!("rill-move-{}", std::process::id()));
        let (old_dir, new_dir) = (root.join("old"), root.join("new"));
        let uri = "magnet:?xt=urn:btih:0123456789012345678901234567890123456789&dn=Show";
        std::fs::create_dir_all(old_dir.join("Show")).unwrap();
        std::fs::write(old_dir.join("Show/episode.mkv"), b"data").unwrap();
        std::fs::write(old_dir.join("Show.torrent"), b"metainfo").unwrap();

        assert!(move_content(uri, &old_dir, &new_dir).unwrap());
        assert!(new_dir.join("Show/episode.mkv").exists());
        assert!(new_dir.join("Show.torrent").exists());
        assert!(!old_dir.join("Show").exists());

        // Nothing to move the second time, and nothing is overwritten.
        assert!(!move_content(uri, &old_dir, &new_dir).unwrap());
        std::fs::create_dir_all(old_dir.join("Show")).unwrap();
        let err = move_content(uri, &old_dir, &new_dir).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(old_dir.join("Show").exists());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_move_that_cannot_finish_leaves_everything_where_it_was() {
        let root = std::env::temp_dir().join(format!("rill-move-part-{}", std::process::id()));
        let (old_dir, new_dir) = (root.join("old"), root.join("new"));
        let uri = "magnet:?xt=urn:btih:0123456789012345678901234567890123456789&dn=Show";
        std::fs::create_dir_all(old_dir.join("Show")).unwrap();
        std::fs::write(old_dir.join("Show.torrent"), b"metainfo").unwrap();
        // Something of the metainfo's name is already in the new folder, so the content
        // must not be moved either.
        std::fs::create_dir_all(&new_dir).unwrap();
        std::fs::write(new_dir.join("Show.torrent"), b"older").unwrap();

        let err = move_content(uri, &old_dir, &new_dir).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(
            old_dir.join("Show").is_dir(),
            "the content was moved anyway"
        );
        assert!(old_dir.join("Show.torrent").exists());
        assert!(!new_dir.join("Show").exists());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn moving_a_torrent_leaves_the_file_it_was_added_from_alone() {
        let root = std::env::temp_dir().join(format!("rill-move-file-{}", std::process::id()));
        let (source, old_dir, new_dir) = (root.join("source"), root.join("old"), root.join("new"));
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(old_dir.join("film")).unwrap();
        let uri = source.join("film.torrent");
        std::fs::write(&uri, b"metainfo").unwrap();

        let uri = uri.to_string_lossy().into_owned();
        assert!(move_content(&uri, &old_dir, &new_dir).unwrap());
        assert!(new_dir.join("film").is_dir());
        // The .torrent file the user chose stays where the user keeps it.
        assert!(source.join("film.torrent").exists());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn content_path_uses_the_torrent_file_stem() {
        let output_dir = Path::new("/tmp/rill-downloads");

        assert_eq!(
            content_path("/tmp/source/Foo.torrent", output_dir),
            Some(output_dir.join("Foo"))
        );
    }
}
