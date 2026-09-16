# Using Rill

## Adding a torrent

The + button offers a .torrent file or a magnet link; Ctrl+O and Ctrl+N do the same. A
.torrent file or a magnet link can also be opened with Rill from the file manager or a
browser, or dropped on the window. A second launch while Rill is running hands its link
or file to the running instance.

The add dialog shows the folder the torrent will be saved to; picking another one there
changes it for this torrent only. The default is set in Preferences. "Start Immediately"
off adds the torrent paused. "Sequential Download" fetches pieces from the start of the
torrent to the end instead of rarest first, so a video or an archive can be opened before
the download completes; it can be turned on or off later in the torrent's details.

A link that is not a magnet link, or a file that is not a torrent, is refused in the
dialog.

## The list

Torrents are grouped into Downloading, Paused and Finished; a torrent that failed is
among the finished ones with a red icon. The button at the end of a row pauses, resumes
or deletes, and the context menu (right click, a long press, Shift+F10 or the Menu key)
has the rest: Retry for a failed torrent and Open Folder.

Ctrl+F searches the names. Selection mode, from the main menu or Select in a context
menu, acts on several torrents at once: the bar at the bottom resumes, pauses or deletes
them, Ctrl+A selects all, Delete deletes and Escape leaves.

Deleting always asks first. It removes the torrent from the list; the downloaded files go
only when "Also delete the downloaded files" is checked.

## The queue

At most "Simultaneous Downloads" torrents (Preferences, 3 by default) download at a time.
When one more starts, the newest is paused until a slot frees up; when one finishes or is
paused, the oldest waiting torrent starts. A torrent you paused yourself is never started
by the queue.

## Details

Activating a row opens the torrent's details. Overview has the progress, a map of which
pieces are on disk, speeds, peers, the time left, the sequential switch, the download
folder and the source, which a click copies. Moving the sequential switch on a running
torrent restarts it, so it reconnects to its peers. Files lists what the torrent contains, once
its metadata is known; for a magnet link that is after it has been fetched from peers.
Peers lists the connections with their client, speed and whether they are encrypted, and
Trackers the trackers the torrent names.

## Closing the window

On a desktop with a system tray, closing the window hides it and the transfers go on; the
tray icon brings the window back or quits. Without a tray, closing the window quits.
Quitting pauses every transfer, and they come back paused the next time Rill starts.

## Where things are kept

Torrents and settings are in `~/.local/share/rill/torrents.db`, with the DHT state and
copies of added .torrent files beside it. Preferences sets the default download folder,
the number of simultaneous downloads, the listening port (running torrents take one port
each, counting up from it; 0 gives each torrent a port derived from it) and how much Rill
logs to its standard error.

## Keyboard shortcuts

Ctrl+? lists them all.
