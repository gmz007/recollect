# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/).

## [Unreleased]

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
