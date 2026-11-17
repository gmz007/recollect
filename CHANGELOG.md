# Changelog

All notable changes to this project, newest first. Generated from the commit history by
[git-cliff](https://git-cliff.org) with the rules in `cliff.toml`; dates are UTC. Versions follow
[Semantic Versioning](https://semver.org/).

## [0.6.2] - 2026-11-15

### Fixed

- Survive flaky drives and hostile video headers while indexing

  - A read error late in a video ended it as fully indexed. Now only
    damaged data ends a file early; when the storage or network went away
    the file fails and is retried, and such FFmpeg errors count as
    temporary instead of being recorded as decode failures.
  - An implausible sample aspect ratio is ignored instead of asking the
    scaler for an enormous frame.
  - Files left for a retry are no longer counted as processed; the run
    summary reports them separately.
  - A database error while recording one failed video no longer aborts
    the whole run (images and videos share one helper).
  - The camera is kept when a file has no plausible capture date.
  - The index cache is not saved while a job runs: the save holds the
    index for the whole write and stalled the job, database readers and
    searches behind it. The job saves when it ends.
  - Folder removal, folder sync, album rename and recording an error are
    single immediate transactions; the library id is created atomically.
  - A failed download always removes its partial file.
  - Smaller: bounded per-block search heaps, a validated segment
    threshold override, and a precise description of the category metric.

- **ui:** Keep searches, thumbnails and keyboard use from getting stuck

  - A decoder panic on one file no longer kills a thumbnail or preview
    worker and leaves the image loading forever; a failed image is tried
    again after 30 seconds (a drive may be back).
  - The live refresh of results while photos are being looked at waits
    for the answer still awaited instead of replacing it, so slow searches
    show their results.
  - Emptying the search field clears the search completely; a late answer
    no longer refills the results.
  - Enter goes to the widget that has keyboard focus and opens the
    selected photo only when nothing is focused.
  - Escape and the arrows belong to an open viewer menu, not the viewer.
  - A slider drag cut short by closing Settings is not applied later.
  - A refused album name leaves the field ready for typing.
  - A typed folder must be a full path to an existing folder (one helper
    for the sidebar and the welcome screens).
  - The remove button of a folder row is drawn when it has keyboard focus.
  - Hidden zones of small tiles no longer take clicks; the number of
    threads is read once; the help text no longer says where the Turn on
    button is.

- Report unreachable folders after every finished run, however short

  The note that some folders could not be reached was shown only if a
  frame happened to be drawn while the run was still going, so a quick run
  said nothing. Finished runs are now counted in the status and each one
  is reported once.

  Three tests depended on timing and failed now and then under load: the
  one for this note, a database test that assumed two inserts fall in the
  same second, and a helper that did not wait for an album view to load
  its own list. All three are deterministic now.

- Tell damaged videos from unreachable storage, and keep keys and retries in check

  Follow-ups to the previous fixes:

  - FFmpeg reports an I/O error for some damaged files too. Such a file
    was left for a retry on every run and lost the frames already decoded.
    An I/O error now counts as "storage gone" only if the file cannot be
    read right now; otherwise the file is kept as far as it was read, or
    recorded as unreadable.
  - The live refresh of results waits three seconds after an answer, not
    after the question, so a slow search no longer reruns back to back.
  - Grid keys (favorite, arrows) no longer act behind an open menu; this
    was fixed for the viewer only.
  - A thumbnail being tried again keeps its failed look until the answer
    arrives, and the retry respects the queue limit.
  - Hidden controls of small tiles are not offered to assistive
    technology.
  - Creating an album checks the name and inserts in one immediate
    transaction.

### Changed inside

- Split the library and database modules by concern

  library.rs (2600 lines) and store/db.rs (2100 lines) each held one type
  with every method in a single impl block. Both are now directories whose
  files group the methods by what they do:

  - library/: status, jobs (the background worker and what waits for it),
    writes (favorites and albums through the writer thread), folders,
    model (choice, switch, download), index_cache, tests
  - store/db/: schema (creation and upgrades), files, items, timeline,
    albums, tests

  Pure moves: no behaviour changes, and every item keeps its path through
  re-exports.

- **ui:** Split the app, timeline, viewer and settings modules

  Four UI files had grown past a thousand lines each. They are now split
  by concern, as exact moves of the existing code:

  - ui/: notices, snapshot, dialogs, grid (selection and keyboard moves
    shared by both grids), results (search flow and its grid), favorites;
    mod.rs keeps App, its construction and the frame loop
  - ui/timeline/: dates, layout, load, memories, view, tests
  - ui/viewer/: entry, toolbar, info, canvas, tests
  - ui/settings/: widgets, smart_search, library, storage, tests

  Unit tests moved next to the code they test. No behaviour changes;
  items keep the visibility they had.

- **media:** Split the video module into stream, convert, sample and still

  The file had passed a thousand lines. Opening files and reading packets,
  converting frames, sampling a whole video and fetching one frame are now
  separate files; the code is moved unchanged.

- Cut the long functions outside the UI into named steps

  Fourteen functions of 54 to 93 lines (opening the library, loading the
  index cache, the folder sync, the indexing loops, the sampler, search,
  and the command-line, benchmark and evaluation entry points) are split
  along the steps they already had. No behaviour changes: lock scopes,
  transactions and the order of work are the same.

- **ui:** Cut the long screen functions into the regions they draw

  Eighteen functions in the album, sidebar, settings, viewer and welcome
  screens are split by region or step (one function per dialog, toolbar
  cluster, settings card, kind of album change). The viewer geometry is a
  pure function with tests.

  Shared code replaces copies: one builder for icon-and-label buttons
  (five copies) and one confirm dialog (three copies).

  Screens are unchanged pixel for pixel.

- **ui:** Cut the long app, grid, header and tile functions into steps

  The remaining nineteen long UI functions (building the app, the frame,
  the results and timeline grids, headers, toasts, empty states, the
  search help, tile painting, timeline loading) are split along the steps
  or regions they already had. What the two grids share is now shared
  code: recording tile events, the context menu, the scroll area and the
  favorite key.

  Screens are unchanged pixel for pixel.

- **ui:** Give the selection bar, folder dialogs and shared dialogs their own files

  albums.rs and sidebar.rs had grown to hold code that is not about albums
  or the sidebar. Moved, unchanged:

  - selection_bar.rs: the bar above a grid while items are selected
  - folders.rs: adding a folder (suggestions, picker, typed path) and the
    confirmation for taking one out
  - dialogs.rs: the modal window frame and the confirm dialog
  - grid.rs: the tile context menu and which grid is the active one

- Make helpers private where only their own file uses them

  Moving code into smaller files had kept every item as visible as it was
  in the old, larger module. Seventy-two functions, constants and types
  that nothing outside their file names are private again.

### Build and dependencies

- Keep functions to 50 lines of code

  The clippy threshold for function length goes from its default of 100
  to 50, now that every function fits. Scenario tests, which read top to
  bottom, are exempt per test module. The design notes gain a short map
  of the code layout.


## [0.6.1] - 2026-11-14

### Fixed

- Harden untrusted input handling and adopt a strict lint table

  - Panics in image decoding and date reading are contained per file
    instead of aborting the whole run; embedding counts are checked.
  - Hostile video headers: saturating timestamp math, invalid time bases
    rejected, non-finite durations ignored; bounded camera strings;
    malformed model output shapes return errors.
  - Errors keep their context (paths, GPU rebuild failure, DB failures
    while recording a broken file); unique temp files for settings and
    thumbnails; immediate transaction when adding folders; named worker
    threads that report spawn failures; no nested status/pending locks.
  - UI: background reads recover from a dropped sender, stale selection
    indices are clamped, worker pools can't miss the shutdown wake-up,
    no Instant underflow, switching search quality clears pending
    searches.
  - CLI commands are split so impossible commands can't reach the
    library runner; eval input uses typed kinds and rejects bad entries.
  - Cargo `[lints]`: clippy pedantic plus unsafe, unwrap and cast lints;
    every suppression carries a reason; every unsafe block documents its
    safety argument. The ui module is private.

### Changed inside

- Split UI state, share background helpers and shorten functions

  - Shared helpers for background reads and latest-wins workers; one
    texture loader behind the thumbnail cache and viewer previews.
  - Grid state shared by the timeline and search results; dialog state
    grouped; mutually exclusive flags turned into enums (tile marks,
    welcome actions, what to do after adding folders).
  - Every function under 100 lines; tile construction in one builder;
    layout and timing numbers are named constants.
  - Stored kinds, date sources and file states convert with TryFrom and
    reject unknown values; named structs replace repeated tuples; Debug
    for the core types.
  - UI tests split by feature; the too_many_lines and
    struct_excessive_bools lints are enabled.

### Build and dependencies

- **deps:** Use the parking_lot version shared with eframe and wgpu

  Drops the duplicate parking_lot, parking_lot_core and lock_api copies
  from the dependency tree.


## [0.6.0] - 2026-11-14

### Added

- **ui:** Welcome flow, empty states, search help, tips and help sheet

  - First launch walks through four short steps: what Recollect does,
    which folders to show (Pictures and Videos suggested), turning on
    smart search (Best quality or Faster, or skip), and what happens next.
    Nothing changes until Start.
  - Every view has an empty state with one sentence and one button.
  - Focusing the empty search field shows example searches and recent
    searches (kept on this computer, clearable).
  - One-time tips at the right moment, one at a time; "Show tips again"
    in Settings.
  - The help button opens a help sheet: getting started, searching,
    organizing, keyboard shortcuts and privacy.
  - Error messages say what happened and what to do, with Try again where
    possible and technical details folded away.

### Fixed

- **ui:** Clearer first run, safer search history and queued jobs

  - Folders step: no Skip that silently adds folders; "Continue without
    folders" when nothing is ticked; friendly names with paths on hover;
    typed paths under More options; hint when Pictures doesn't exist.
  - One skip wording ("Not now", "I'll set it up myself"); the last step
    explains what happens after Start.
  - Jobs requested while another runs are queued: chosen folders are
    scanned before the smart-search download, and a failed download
    leaves them scanned; folders are added in one pass with nested ones
    dropped; "Check for new photos" while busy says it will run next.
  - Recent searches are recorded only when a search runs, never on Esc
    or clicking away; example searches work from the keyboard.
  - Error wording comes from typed error kinds; no duplicate error text;
    the hardware check runs once; help text corrected.


## [0.5.0] - 2026-11-14

### Added

- **ui:** Modern look and plain language

  - New visual system: Inter font, Material Symbols icons, light and dark
    themes following the system (or chosen in Settings), larger-text
    option, rounded cards and calmer colors.
  - Top bar with a prominent search field and example, icon filters,
    help and settings; sidebar with icons for Photos, Recently added,
    Favorites, Albums and Folders; an Add a folder dialog that suggests
    standard folders; removing a folder asks first and explains that the
    files stay.
  - Square, rounded thumbnails; hover shows a select circle and a
    favorite star; videos show their length; click opens the photo; a
    selection bar offers the bulk actions.
  - View headers with title, counts and compact grouping/sort/size
    controls; pop-up messages replace the status bar; an activity card
    explains what is happening in plain words.
  - Plain language throughout ("smart search", "getting your photos
    ready"), with technical details behind "Details"/"Advanced", and a
    test that keeps technical words off the main screens.

- **ui:** Settings sheet, search results header and viewer toolbar

  - Settings is a sheet with sections: Appearance (theme, text size),
    Smart search (on/off status, Best quality vs Faster explained, switch
    confirmation, download, graphics card), Library (folders, plain counts,
    files that couldn't be opened with Try again, check for new photos),
    Storage (space for previews, where data is kept) and About (version,
    privacy promise, open-source licenses); technical knobs stay folded
    under Advanced.
  - Search results: "Results for …" header with When / Type / Sort chips
    and a clear button; no scores or timings.
  - Viewer: icon toolbar (back, favorite, add to album, info, open,
    show in folder, zoom) with the position "7 of 689"; Play at for videos.
  - Dialogs dim and block the window behind them; smooth hover shading on
    thumbnails; the sort control says what it does.

### Fixed

- **ui:** Clearer controls and fixes from usability testing

  - One type filter (top bar); search text centered in its field; viewer
    toolbar shows words next to icons on wide windows and its position
    never covers buttons; one-line smart-search banner whose "Not now" is
    remembered; Settings content clear of the scrollbar; labelled size
    slider; Pause/Resume/Stop as words; the viewer names the folder and
    says "Date from the file"; dismissible messages; better contrast.
  - The "folders couldn't be reached" notice shows again; each grid has
    its own selection mode; repeated errors are announced again; changing
    appearance during a quality switch is kept; settings are applied on
    change instead of every frame; no disk checks every frame; results
    title follows the current search; shared helpers for sizes and for
    files that couldn't be opened.


## [0.4.0] - 2026-11-14

### Added

- **ui:** Add albums and multi-select

  - Albums: create, rename and delete them in the sidebar (names unique,
    deleting an album never deletes files); album views work like the
    timeline; "Add to album…" from the detail panel, the viewer or a
    selection, including creating a new album on the way.
  - Multi-select in every grid: Ctrl/Cmd+click, Shift+click ranges,
    Ctrl/Cmd+A, Shift+arrows; a selection bar adds to an album, favorites,
    unfavorites or removes from the album; the selection survives reloads.
  - One ordered writer for favorites and album changes, so "create album
    then add" never reorders and failures are reported.

### Fixed

- Make album changes safe and keyboard-proof

  - Shortcuts never fire while typing in a field, inside album dialogs or
    behind them; Enter never confirms deleting an album.
  - Removing items and deleting albums take effect only once saved; a
    failure changes nothing and stays visible. Creating an album with
    items is one transaction; adding to a deleted album says so.
  - The album list is fetched with two queries and only when albums
    change; bulk removal is a single pass; selections in search results
    count files, not hits; bulk favorite failures give one notice.
  - Errors stay on screen ahead of later messages; right-click on tiles
    offers Add to album / Remove from album for the whole selection;
    album names are limited to 100 characters and shortened in lists;
    queued changes are saved before the app exits; tiles report their
    selected state to assistive technology.


## [0.3.0] - 2026-11-14

### Added

- **ui:** Add a viewer, favorites, search by date and On this day

  - Viewer: double-click or Enter opens a full-window view with previews
    decoded at window size (neighbours prefetched), arrows/Home/End to
    move through the list it came from, an info panel (date, camera, size,
    duration), zoom to 100 %, Play at for videos, Esc back to the grid.
  - Favorites: F or a button toggles; stars on tiles; a Favorites view.
  - Search by date: "When" (any time, this/last year, a year, custom
    range) filters inside the index scan; sort by relevance, newest or
    oldest. The vector index keeps each item's capture time (cache v3).
  - On this day: a strip of earlier years' photos from today's date at
    the top of the timeline.
  - Without a model the status bar says files become searchable after
    the download.
  - A UI test no longer depends on how quickly a view loads.

### Fixed

- Order favorite writes, free viewer memory and unify capture-date rules

  - Favorites are written by one ordered writer per library (rapid toggles
    coalesce); failures revert the star and show a notice; pending marks
    survive reloads; the Favorites view refreshes after changes and drops
    unfavorited items at once.
  - Viewer: shares the list it was opened from instead of copying it;
    the item on screen decodes before prefetches; previews are freed on
    close (at most two full-size textures); notices show inside it; Find
    similar closes it like the other paths.
  - One rule for files without a capture date, used by every reader; the
    date filter and date sort use the same value; vectors of files that
    vanish mid-run are skipped; dates learned late patch the index once
    per pass.
  - "When" lists the years of the whole library; On this day skips
    file-dated items, shows Feb 29 photos on Feb 28 in other years and
    refreshes at midnight.
  - Model integration tests keep their temporary library next to the
    model files (hard links, nothing large in /tmp).


## [0.2.0] - 2026-11-13

### Added

- **library:** Read capture dates and add timeline queries

  - Database schema v2 (migrated in place from v1): capture time, date
    source and camera per file, favorites flag, time added, albums and
    album items.
  - Capture dates from EXIF (DateTimeOriginal, DateTimeDigitized,
    DateTime), video metadata (QuickTime creation date, creation_time) or
    the file time, stored as local wall-clock time so items group by the
    day they were taken; implausible dates are ignored.
  - A fast date pass runs before embedding, so the timeline is complete
    early; indexing without a downloaded model now scans and reads dates.
  - Timeline query with folder, type, favorites and album filters and
    three orders; `recollect status` shows where dates came from;
    `bench timeline` measures loading a large timeline.

- **ui:** Browse the library as a timeline when not searching

  - Timeline is the default view: all photos and videos by capture date,
    grouped by day, month or year with counts; newest or oldest first;
    adjustable tile size; video durations on tiles. Only visible rows are
    drawn, so large libraries scroll smoothly.
  - Date navigator (years and months with counts) jumps to a section.
  - Recently added and per-folder views; the type filter applies to all.
  - Detail panel shows when an item was taken, the camera and size.
  - A missing model no longer hides the library: browsing works right
    away and a banner offers the download; only search waits for it.
  - "Clear search" returns from results to the album.

### Fixed

- Make first runs, date reading and timeline reloads robust

  - A successful model download (or model switch) continues straight into
    indexing; starting a download cancels a running scan instead of being
    refused; startup scans and Rescan work without a model.
  - Files that cannot be read right now keep an unknown date and are
    retried, instead of getting their file time for good; huge TIFFs are
    not parsed for EXIF; video durations are read with the dates.
  - The timeline reloads periodically only while files are being added,
    sorts outside the database lock, keeps the photo at the top of the
    view in place, and never lays out old items under a new view.
  - Arrow keys follow tile rows across sections; navigator month jumps
    land on the month; same-named folders are told apart in the sidebar.
  - The database is backed up before its one-time upgrade, upgrades run in
    an immediate transaction, and interrupted first starts still upgrade.
  - Descriptive benchmark labels.


## [0.1.0] - 2026-11-13

### Added

- Add models, image indexing, SQLite store, i8 vector index, search, eval and CLI

  - Model registry (SigLIP base/16, CLIP B/32) with pinned revisions and SHA-256;
    consented download is the only network code.
  - ONNX Runtime embedder with WebGPU -> CPU fallback, telemetry disabled.
  - DCT-scaled JPEG decoding with EXIF orientation; fast_image_resize.
  - SQLite library (folders/files/items) with incremental sync; in-memory
    i8 brute-force index with f32 re-scoring; per-file grouping of hits.
  - Indexer pipeline (parallel decode -> batched embedding -> DB/index),
    pause/cancel; eval command for measuring search quality; CLI.

- **ui:** Add egui app, video frame sampler and thumbnails

  - eframe app: search with debounce and latest-wins worker, media-type
    filter, thumbnail grid (lazy LIFO loader + disk cache), detail panel
    (open / show in folder / find similar / video moments), library panel,
    status bar with pause/cancel, first-run model consent, settings window.
  - GOP-aware FFmpeg sampler (keyframe-only for short GOPs, in-order decode
    for long ones), display-matrix rotation and SAR handling, frame_at.
  - kittest UI tests; only glyphs from egui's bundled fonts are used.

- **video:** Index videos with segment merging

  Sampled frames are embedded in batches on the indexer thread
  while the sampler decodes on another; consecutive similar frames are
  merged into segments (mean vector, max 60 s).

  Also hardens the core:
  - No panics on vector-dimension mismatch; find-similar refuses items of
    another model; search worker survives panics.
  - Model switch runs as a job and persists settings only on success.
  - Sampler: oversized GOPs stream without flushing; unknown-duration last
    GOP decoded in order; no per-frame deep copies; frame_at honours the
    stream start offset; timestamp fallback; FFmpeg restricted to file://.
  - Indexer: decoders stop when embedding/writing fails; unreadable
    subfolders are not emptied; index updated under the DB lock.
  - DB: put_items skips removed files instead of failing the batch;
    busy_timeout; batched IN() lookups; per-model aggregate stats.
  - Download: connect/response timeouts, byte cap. Still images > 512 MB
    are refused.
  - UI: library stats fetched off the UI thread; folder removal in the
    background; App cancels work on close; pause hidden while downloading.

- **ui:** Navigate results with the keyboard; optional frame-time stats

  Arrow keys move the selection through the grid (scrolling it into view)
  and Enter opens the selected result; the search box keeps its keys.
  With RECOLLECT_FRAME_STATS set, the app repaints continuously and logs
  frame-time percentiles every 10 seconds.

### Fixed

- **model:** Recover from WebGPU failures, store video display size, add model test

  - WebGPU: run fixed-size (zero-padded) batches so buffers are reused;
    on an inference failure rebuild the GPU session once, then fall back
    to the CPU instead of failing the run (SigLIP hit sporadic
    "Invalid Buffer" errors with varying batch shapes).
  - Videos store their original display size (SAR + rotation applied),
    not the sampled frame size.
  - tests/models.rs (ignored unless RECOLLECT_TEST_MODELS is set): batch
    padding and GPU vs CPU give the same embeddings.

- Harden GPU recovery, job handling, video sampling and search

  - Embedder: warm-up bypasses GPU recovery (no unbounded rebuild loop);
    inputs validated before inference so bad input is not a GPU fault.
  - Library: lock-free busy flag and job counter; wait() joins without
    holding the worker lock; folder removal retries if another job takes
    the worker; text encoder reloaded when the model changed.
  - Sampler: backwards timestamp jumps (spliced streams) get an offset so
    sampling continues; read errors after progress end the file instead of
    discarding it; duration never below what was decoded; short-GOP samples
    not emitted right after a previous sample; keyframe-only decode falls
    back to in-order decoding; EAGAIN on send drains and retries; bounded
    EAGAIN on read; non-video streams discarded at demux; frame_at past the
    end returns the last frame and errors instead of returning a wrong one.
  - Video segments merged online (flat memory for long videos); segment
    ends never before their start.
  - Search: index scan can exclude a file (find similar); the scan widens
    when long videos crowd out distinct files; only the best items of
    candidate files are re-scored.
  - Scan: per-entry read errors protect their directory during sync.

- Keep the library intact with missing media; clearer job handling and status

  - Transient failures (file or drive gone, I/O errors) leave files pending
    instead of marking them failed; "Retry failed files" in the UI.
  - A folder that scans empty while the database has files (unmounted
    drive, empty mount point) is skipped, not emptied; reported in the
    run summary.
  - Folders added while a job runs are picked up (rescan flag; the indexer
    loops); folder add and removal run off the UI thread and never cancel
    the running job; sync of a removed folder is a no-op (no FK failure).
  - Status text distinguishes up to date / done with files waiting /
    stopped; indexed vs failed counts; unavailable folders. last_run is
    cleared when a run starts.
  - Download can be cancelled; "Not now" leads back to the download;
    exit waits at most 3 s for the worker.
  - Pause also pauses scanning. Corrupt settings / unknown model fall back
    to defaults; startup errors show a window instead of exiting silently;
    busy_timeout set before other pragmas.
  - Thumbnail cache pruned every 10 min and after settings changes, by
    last use (mtime touched on cache hits). Settings window movable.
  - Tests: deterministic cancel/resume (asserts pending work), UI test
    without a racing rescan, new regression tests.

- **index:** Make the index cache safe and cheap to keep up to date

  - Cache saves are serialized and write unique temp files; abandoned temp
    files (untouched for 10 minutes) are removed at startup.
  - The cache header carries a random library id (format v2), so a reset or
    restored database never loads a stale cache; the header is validated
    against the file length before allocating.
  - The items epoch only changes when stored items change; unchanged
    indexes are not rewritten. Saves happen after jobs (with the busy flag
    already cleared) and after CLI commands, never at open.
  - Model and index are swapped together under the database lock; saves
    check model, dimension and item count consistently.
  - A panicking video decoder fails only that file; the video consumer
    stops on cancel; two parallel decoders on CPU, three on GPU.
  - SigLIP's int8 text tower uses its own file name, so existing installs
    fetch only that file; files no longer part of a model are removed
    after a download.
  - bench reports whether startup used the cache and saves it before
    measuring.
  - Code comments explain design choices inline or point to docs/design.md.

- **bench:** Write the index cache after a rebuild, before measuring
- **ui:** Tighten keyboard navigation and frame-stats logging

  - Keep the selected tile in view using the exact row stride of the grid
    (row height plus item spacing).
  - Arrow keys and Enter only act while the results grid is visible and
    the settings window is closed; unmodified keys only; repeated presses
    in one frame each count; clicking a tile no longer blocks arrows; only
    a focused text field keeps its keys.
  - RECOLLECT_FRAME_STATS enables its own log output by default.
  - UI tests disable frame stats and never load placeholder model files.
  - Docs: Esc clears the selection; Cmd+F on macOS.

### Faster

- Add index cache, AVX2 scan, int8 SigLIP text tower, parallel video sampling and benchmarks

  - recollect bench search|rescan (synthetic library; never without --home).
  - Index cache file (index-<model>.bin) stamped with a DB items epoch;
    startup at 1.2 M vectors 15.6 s -> 0.34 s. Only written when the
    in-memory index mirrors the database.
  - i8 scan dispatches to an AVX2 build of the dot product at runtime.
  - Ranking uses i8 scores directly; f32 re-scoring removed (no measurable
    quality difference, ~30 ms of random disk reads saved).
  - SigLIP text tower switched to the int8 ONNX export (retrieval quality within noise,
    330 MB less RAM and download).
  - Videos: 3 samplers decode in parallel and share embedding batches;
    60 short clips 22.5 s -> 5.3 s.
  - Search phases logged at debug level.

### Documentation

- Add design and packaging notes
- Rewrite README and CONTRIBUTING, add CHANGELOG

  README covers features, formats, building per OS, first run and models,
  the full CLI, environment variables, data locations, privacy, measured
  performance and troubleshooting.

- **cli:** Correct the model download size in the help text
