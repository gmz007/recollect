# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.6.2] - 2026-11-15

### Fixed

- A video on a drive that drops out while it is being read is retried later instead of being
  kept as fully looked at with only its first minutes; such drive and network errors are no
  longer recorded as unreadable files. A damaged video is still kept as far as it could be read.
  Files left for the next run are counted separately in the run summary.
- A video whose header declares an absurd pixel shape no longer asks for an enormous frame.
- The camera of a photo or video is kept when its clock was never set.
- Saving the search data to disk no longer stalls a running scan, browsing and searches.
- Searches that take longer than a few seconds while photos are being looked at show their
  results instead of restarting forever, and pause between refreshes; emptying the search field
  clears the search for good.
- A photo that makes a decoder crash no longer stops thumbnails and previews from loading, and
  a thumbnail that failed (say, while a drive was offline) is tried again.
- Keyboard: Enter goes to the button that has focus; keys go to an open menu instead of the
  photos or the viewer behind it; the remove button of a folder row is visible when it has
  focus; a refused album name leaves the field ready for typing.
- The note that some folders could not be reached is shown after every run that skipped a
  folder, also a very short one.
- A slider drag cut short by closing Settings is not applied later; a typed folder must be a
  full path; a failed download always removes its partial file.

### Changed

- Internal restructuring: the largest source files are split by concern (see "Code layout" in
  `docs/design.md`) and functions are kept to 50 lines of code, enforced by the lint
  configuration. Folder and album changes in the library database are single transactions.

## [0.6.1] - 2026-11-15

### Fixed

- A photo or video that makes a decoder crash is marked as unreadable instead of stopping the
  whole run; crafted video timestamps and time bases no longer overflow or stop sampling.
- Lists that stopped refreshing after a failed background read recover; stale selections can't
  point past the end of a list; worker threads always shut down.
- Error messages keep the file they are about; settings and thumbnails are written through
  unique temporary files.

### Changed

- Stricter lint configuration (clippy pedantic, documented `unsafe`, reasons on every
  suppression) and internal restructuring of the interface code. Unknown values in the library
  database are reported instead of being silently reinterpreted.
- `parking_lot` now uses the same version as the windowing libraries.

## [0.6.0] - 2026-11-14

### Added

- A first-run **welcome**: four screens (what Recollect does, where your photos are, turn on
  smart search, you're all set) with progress dots and Back (and "I'll set it up myself" on the first screen). It is shown once, only when the
  library has no folders; the `welcome_done` setting remembers it.
- **Empty states** with an icon, one sentence and one button for a library without folders,
  Photos, Recently added, Favorites, a folder, an album, searches with no results (with a button
  to clear the date filter) and smart search being off.
- **Search help**: clicking the empty search field shows example searches and up to eight recent
  searches (`recent_searches`, kept on this computer, with **Clear recent searches**).
- One-time **tips** (search, selecting several photos, filling an album) that appear one at a
  time and stay dismissed (`tips_dismissed`); **Show tips again** in Settings, Appearance.
- A **Help sheet** like Settings, with Getting started, Searching, Organizing, Keyboard shortcuts
  and Privacy.

### Changed

- Error messages say what happened and what to do, with a **Try again** button for downloads and
  scans and the technical text folded under **Details**.

## [0.5.0] - 2026-11-14

### Added

- A new visual system: Inter text, Material Symbols icons, light and dark colors that follow the
  system by default, and a **Large** text size. `theme` (`System`, `Light`, `Dark`) and
  `text_size` (`Normal`, `Large`) are new settings; older settings files load unchanged.
- Top bar with a centered search field ("Search your photos — try “dog on the beach”"), a compact
  All / Photos / Videos filter, a **Help** window (how search works, keyboard shortcuts) and a
  settings button.
- Sidebar with Photos, Recently added, Favorites, Albums (+) and Folders (+). **Add a folder**
  dialog with a folder picker, suggestions from Pictures, Videos, Desktop and Downloads, and a
  path field. A confirmation before a folder stops being shown ("Your files stay where they
  are."). **Jump to date** is collapsible.
- An activity card in the sidebar that appears only while something runs, in plain words, with
  pause and cancel; a small "Smart search is off" card with **Turn on**.
- Hover controls on tiles: a selection circle and a favorite star; a duration pill on videos.
- A selection bar ("3 selected" with Add to album…, Favorite, Unfavorite, Remove from album,
  Cancel) that replaces the heading while items are selected.
- Toast notifications (errors have a close button and stay longer).
- A **Settings** sheet with Appearance, Smart search, Library, Storage and About sections and a
  folded Advanced part. Changes apply at once; switching between *Best quality* and *Faster*
  smart search asks first. Library shows the folders (with **Stop showing…**), plain counts, the
  files that couldn't be opened with **Try again**, and **Check for new photos**; Storage sets the
  space for previews and shows where Recollect keeps its data.
- Search results header: "Results for “beach”" (or "Photos similar to …") with a count, **When**,
  **Type** and **Sort** menus, and a **Clear search** button.
- Viewer top bar with icon buttons and tooltips (Back, Favorite, Add to album…, Info, Open with
  default app, Show in folder, Zoom), the position ("7 of 689") in the middle and a
  **Play at 1:23** button on videos.

### Changed

- Thumbnails are square, center-cropped and 4 px apart, without grey boxes. A single click opens
  the viewer; the selection circle and Ctrl/Cmd/Shift-click select. The right-hand detail panel
  is gone (the viewer's info panel shows the details).
- Each view has a large title with a count ("1,204 photos and 37 videos") and compact controls:
  a Days / Months / Years menu, a sort button and a thumbnail size slider.
- The status bar is gone; messages are toasts. Raw counters and the model name left the sidebar.
- Plain wording everywhere: "smart search" instead of "model", "getting your photos ready for
  search" instead of "indexing", "Check for new photos" instead of "Rescan", no scores or timings.
  Settings keeps technical options under **Advanced**.
- Error and info toasts last 10 and 5 seconds (was 8 and 6).
- The sort control in view headers says what it is set to ("Newest first" menu) instead of an
  arrow icon.
- Dialogs (add a folder, albums, confirmations) now dim the window behind them and block clicks
  there until they are answered.
- The shade on a hovered tile fades smoothly at the top and bottom instead of ending in a hard
  edge.
- Files that couldn't be read are described as "couldn't be opened".

## [0.4.0] - 2026-11-14

### Added

- Albums: create, rename and delete albums from the sidebar; an album view that groups and sorts
  like the timeline; "Add to album…" for the selected item (detail panel and viewer) with
  "New album…" to create one on the spot. Deleting an album keeps the files; removing a library
  folder drops its files from albums but keeps the albums.
- Multi-select in every grid: `Ctrl/Cmd+click` toggles, `Shift+click` selects a range, `Ctrl/Cmd+A`
  selects all, `Shift`+arrows extend, plain arrows collapse to one item and `Esc` clears. A bar
  above the grid offers Add to album…, Favorite, Unfavorite, Remove from album (in albums) and
  Clear selection. The selection survives reloads.
- Favorite marks and album changes share one ordered background writer, so "create an album,
  then add items" can never be reordered. Queued writes are flushed when the app closes.
- Right-click menu on tiles: "Add to album…" and (in albums) "Remove from album" act on the whole
  selection when the tile is part of it.
- Album names are limited to 100 characters; long names are shortened in the sidebar and menus.
- Error messages stay visible for 8 seconds and are not replaced by later messages.

## [0.3.0] - 2026-11-14

### Added

- Viewer: double-click or press `Enter` on a photo or video frame to open it as large as the
  window allows, with an info panel, previous/next through the list you came from, 100 % zoom
  with panning (up to 4096 px per side: very large photos are scaled down to that), and a large
  frame for videos (at the matching moment for search hits). Messages such as "No video player
  found" appear at the bottom of the viewer.
- Favorites: mark items with `F` or the Favorite button; stars on tiles and a Favorites view in
  the sidebar. Marks are saved in order by one background writer, shown at once, kept when a
  reload carries older values, and taken back with a message if saving fails.
- Search by date: a "When" selector (any time, this year, last year, a given year or a range of
  years) and a "Sort" selector (relevance, newest, oldest) while searching.
- "On this day" strip above the timeline with photos taken on today's date in earlier years
  (not for photos dated only by their file time; 29 February shows on 28 February in years
  without one; the strip updates at midnight).

### Changed

- Files without a known capture date use the same date (their file time) in the timeline, the
  info panel, the search index and the date filter; search results sort by the date the filter
  used.
- Reading dates patches the search index once per run instead of once per batch.
- The "When" selector lists the years of the whole library, read in the background.
- The viewer shares the list it was opened on instead of copying it, so opening it on a very
  large library is instant; closing it frees its previews.
- The search index cache now records capture dates, so it is rebuilt once after updating.
- Without a downloaded model the status bar says how many files will become searchable after the
  download instead of "still waiting".

## [0.2.0] - 2026-11-14

### Added

- Album browsing: a timeline of all photos and videos grouped by day, month or year, sortable
  newest or oldest first, with an adjustable tile size.
- Date navigator in the sidebar to jump to a year or month.
- Folder view and a "Recently added" list.
- Browsing works before the search model is downloaded; the one-time download prompt is now a
  banner above the album instead of a full-screen card.

### Changed

- The library database is upgraded in place the first time this version starts. A backup of the
  old database is kept as `library.db.v1.bak` next to it; older versions of Recollect cannot
  open the upgraded database.
- A model is no longer needed to browse: folders are scanned and dated on start, with or
  without one. Once the model download finishes, indexing starts automatically.

## [0.1.0] - 2026-11-14

### Added

- Text search over photos and video moments, running entirely on your computer.
- Images (JPEG, PNG, WebP, GIF, BMP, TIFF) and videos (MP4, MOV, MKV, WebM, AVI and more, via FFmpeg).
- Video results with timestamps and "Play at", which opens mpv or VLC at that moment when installed.
- Find similar, and an All / Images / Videos filter.
- Background indexing with pause, resume and cancel; search works while indexing runs.
- Incremental rescans that pick up new, changed and deleted files; unmounted drives are kept.
- Failed files are listed with a "Retry failed files" action.
- Keyboard shortcuts: Ctrl+F (Cmd+F on macOS) focuses search, arrow keys move the selection, Enter opens, Esc clears the selection.
- Two models, downloaded after your consent: SigLIP base/16 (default with a discrete GPU) and
  CLIP ViT-B/32 (default without a GPU); switchable in Settings or with `--model`.
- GPU acceleration through WebGPU with automatic CPU fallback.
- Command-line interface: `models`, `folders`, `index`, `search`, `status`, `eval` and `bench`.
- Linux, Windows and macOS support.
