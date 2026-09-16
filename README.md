# Rill

Rill is a small BitTorrent client, written in Rust with GTK 4 and libadwaita on top of
[mtorrent](https://github.com/DanglingPointer/mtorrent). It needs GTK 4.20 and libadwaita
1.8.

Magnet links and .torrent files, added from the window, from the file manager or a
browser, or dropped on the window. Transfers are grouped into downloading, paused and
finished, ordered by when they were added, by name, by size or by how far along they are,
with a limit on how many download at once and the rest queued. A torrent's folder can be
changed after the fact, content and all. The details of a torrent show a map of the pieces
already on disk, its files, the peers it is connected to and its trackers, and switch it
to sequential downloading, so a video can be watched while it arrives. On desktops with a
system tray, closing the window leaves the transfers running in the background, and a
torrent that was downloading when Rill closed carries on the next time it starts.

## Limitations

Rill does not seed. A torrent shares pieces with other peers while it downloads, but stops
once it is complete. There are also no speed limits, no way to pick which files of a torrent
to download, and no way to recheck data already on disk. These are not supported yet.

## Building and installing

    just build
    sudo just install        # or: just prefix=$HOME/.local build install

Build needs Rust 1.95, `blueprint-compiler`, `just`, gettext and the development packages
for GTK and libadwaita. Details, other prefixes and removal are in
[docs/installing.md](docs/installing.md).

## Documentation

- [Installing](docs/installing.md)
- [Using Rill](docs/usage.md)
- [Contributing](CONTRIBUTING.md), including where things are in the code, and
  [reporting a vulnerability](SECURITY.md)

The interface is available in English and Ukrainian.

GPL-3.0-or-later. The engine, [mtorrent](https://github.com/DanglingPointer/mtorrent), is
under the Apache License 2.0.
