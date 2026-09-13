# Security

Please report a vulnerability privately rather than in a public issue: through
[a private advisory](https://github.com/sachesi/rill/security/advisories/new) on GitHub,
or by mail to sachesi <xsachesi@pm.me>. Say what you found, how to reproduce it and which
version you ran; a fix is worked out with you before anything is published.

Only the latest release gets fixes.

## What counts

Rill parses data that strangers send it: .torrent files, magnet links, and whatever peers
and trackers say on the network. The parts where a mistake matters most:

- Paths. File names inside a torrent, a magnet link's display name and the name mtorrent
  derives a folder from must stay inside the download folder, for writing the content and
  for deleting it. A way to write, open or delete elsewhere is a vulnerability.
- Parsing of metainfo, magnet links, peer messages and tracker replies, in Rill and in the
  patched mtorrent-core under `third_party/`: a crash, a hang or unbounded memory from
  input a peer, a tracker or a torrent file controls.
- Trackers a torrent names must not reach link-local or unspecified addresses, so a
  torrent cannot make Rill send requests into the local network or to a cloud metadata
  service.

Leaking which torrents you download to the peers and trackers of those torrents is how
BitTorrent works, not a vulnerability.
