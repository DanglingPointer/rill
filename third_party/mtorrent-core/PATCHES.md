# Changes to mtorrent-core

This is mtorrent-core 0.5.2 from crates.io (upstream commit
`28d33a83cb3b46b429b25825d8154668836f62f8` of
[mtorrent](https://github.com/DanglingPointer/mtorrent)), by Mikhail Vasilyev, under the
Apache License 2.0 in [LICENSE](LICENSE). Rill builds against this copy through
`[patch.crates-io]` in its `Cargo.toml`. The files listed below were changed; each
begins with a line saying so.

- `src/lib.rs`, `src/data/piece_tracker.rs`: a task-local `SEQUENTIAL` flag. When it is
  set, missing pieces are requested in index order instead of rarest first, which is what
  Rill's "Sequential Download" switch does. The tracker keeps its per-piece owners in a
  vector indexed by piece to make that ordering cheap.
- `src/data/storage.rs`: file paths from a torrent that are absolute or contain `..` are
  refused before anything is created.
- `src/input/metainfo.rs`: path components `.`, `..`, and any containing `/` or `\` are
  dropped from a torrent's file list, and a file left with an empty path is ignored.
- `src/trackers/url.rs`: trackers on link-local, unspecified or broadcast IP addresses
  (the cloud metadata address among them) are refused, so a torrent cannot make Rill
  send requests into the local network. Loopback stays allowed for local trackers.
- `src/utp/protocol.rs`: the reply delay is computed with wrapping subtraction, since the
  peer's timestamp can be ahead of ours and the subtraction overflowed.

Each change carries tests next to it. The patch can go once upstream has equivalents.
