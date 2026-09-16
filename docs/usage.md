# Using Rill

## Adding a torrent

The + button offers a .torrent file or a magnet link; Ctrl+O and Ctrl+N do the same. A
.torrent file or a magnet link can also be opened with Rill from the file manager or a
browser, or dropped on the window. A second launch while Rill is running hands its link
or file to the running instance.

The add dialog shows the folder the torrent will be saved to; picking another one there
changes it for this torrent only. The default is set in Preferences. A .torrent file says
how big its content is, so the dialog warns when the folder has less room than that; it
is a warning, not a refusal, and a magnet link says nothing about its size until its
metadata arrives. "Start Immediately"
off adds the torrent paused. "Sequential Download" fetches pieces from the start of the
torrent to the end instead of rarest first, so a video or an archive can be opened before
the download completes; it can be turned on or off later in the torrent's details.

A link that is not a magnet link, or a file that is not a torrent, is refused in the
dialog.

## The list

Torrents are grouped into Downloading, Paused and Finished; a torrent that failed is
among the finished ones with a red icon. The button at the end of a row pauses, resumes
or deletes, and the context menu (right click, a long press, Shift+F10 or the Menu key)
has the rest: Retry for a failed torrent, Open Folder, Copy Magnet Link, and Change
Folder, which moves what has been downloaded so far and puts the rest there too. The new
folder has to be on the same disk; Rill says so and changes nothing when the move fails.

Within each group the main menu orders torrents by the date they were added, their name,
their size or how far along they are; torrents that compare equal keep the order they
were added in. Pause All and Resume All, also in the main menu, act on every torrent in
the list; Resume All starts no more than the download limit allows, and the rest follow
as downloads finish.

Rill looks for the files of every torrent that has downloaded something when it starts,
when its window comes to the front, and every ten seconds while the window is in front;
the files of running torrents it looks for every ten seconds all the time. A torrent
whose files were removed or cut short is paused, moves to Paused and says which: none of
its files were found, or some were and how much is left. Resuming it downloads what is
missing again, and so does resuming a torrent whose files went missing too recently to
have been noticed. A download folder that is not there at all, as on a disk that is not
mounted, is left alone: its torrents keep their state until the folder is back.

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
Quitting stops every transfer, and the next time Rill starts each torrent picks up where
it left off: what was downloading, or waiting its turn, goes on, and what you paused
stays paused.

## Where things are kept

Torrents and settings are in `~/.local/share/rill/torrents.db`, with the DHT state and
copies of added .torrent files beside it. Preferences sets the default download folder,
the number of simultaneous downloads, the listening port and how much Rill logs to its
standard error. The port is automatic unless you say otherwise: every torrent listens on
one derived from itself. Turn that off to give a port of your own, which running torrents
take one each of, counting up from it.

## Keyboard shortcuts

Ctrl+? lists them all.
