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
    fn content_path_uses_the_torrent_file_stem() {
        let output_dir = Path::new("/tmp/rill-downloads");

        assert_eq!(
            content_path("/tmp/source/Foo.torrent", output_dir),
            Some(output_dir.join("Foo"))
        );
    }
}
