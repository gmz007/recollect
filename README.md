# Recollect

**Find photos and video moments by describing them.** Type "kids on the beach at sunset" and
Recollect finds the matching pictures and the moments in your videos, entirely on your own
computer.

- **Local and private.** Indexing and search run on your machine. No cloud, no accounts, no
  telemetry. The only network access is a one-time model download that you confirm.
- **Fast.** Uses your GPU (via WebGPU) when there is one, otherwise the CPU.
- Linux, Windows and macOS.

> Status: early development (v0.1). See [docs/design.md](docs/design.md) for how it works.

## Features

- **A calm, modern look:** square thumbnails edge to edge with small gaps, date headings, light and dark
  colors that follow your system (or pick one in **Settings**), a **Large** text size option, and
  plain wording throughout. Notices appear as small toasts at the bottom instead of a status bar.
- **Photos** (the timeline): all your photos and videos grouped by day ("Today", "Yesterday",
  "Sunday 15 November"; the heading of the day at the top stays in view while you scroll), from the
  capture date in the file (falling back to the file date), newest first. Hover a photo and click
  the circle in its corner to select it; hover a day's heading to select the whole day. Pinch, or
  hold Ctrl/Cmd and scroll, or press Ctrl/Cmd with `=`, `-` and `0`, to change the photo size
  (also in **Settings** as Small, Medium or Large). Browsing works before smart search is turned
  on.
- **Timeline ruler** at the right edge instead of a scroll bar: a column of small dashes with the
  years written beside them. Press or drag anywhere on it to scroll; in Photos the position on it
  is a point in time, so you can scrub to any month. A small label names the date while you point
  at it or scroll. In albums, Recently added and search results it follows the scroll position.
  Press `Tab` to reach the years and `Enter` to jump to one. It is not shown when everything fits
  in the window.
- **Albums** show a plain grid without day headings (Favorites and folders keep them).
- **Folders** (click one in the sidebar; **+** opens *Add a folder* with your Pictures, Videos,
  Desktop and Downloads folders as suggestions; hover a folder and click **×** to stop showing
  it, your files are never touched) and **Recently added**.
- **Viewer:** click any photo or video (or press `Enter` on the one you are on) to see it filling
  the window below a bar, with `←`/`→` (or the round buttons at the edges) to step through the
  list you came from, `Home`/`End`, `Z`, a double-click or a pinch for 100 % zoom (drag to pan; up to
  4096 px per side, so very large photos are scaled down to that) and `Esc` to go back. The bar
  says where you came from (Photos, an album, Results, …) next to the back arrow, shows the date,
  time and position ("7 of 689") in the middle, and has four icon buttons (each with a tooltip):
  **Favorite** (`F`), **Add to album…**, **Info** (`I`, a panel with date, camera, size, folder,
  albums and Find similar) and **⋯** (Open with default app, Show in folder, Zoom, Find similar).
  A filmstrip of the neighbouring items floats at the bottom and fades when the pointer rests;
  on a video it shows the matching moments as chips and a **Play at 1:23** button instead.
- **Search results** are headed by the number found ("12 results for “beach”", or "Photos similar
  to …"). Below it are the filters: **All**, **Photos** and **Videos** (the same choice as in the
  top bar), **When** (a year or a range of years) and **Sort** (Best match, Newest, Oldest). When
  the filters leave nothing, **Clear filters** brings the results back; while photos are still
  being prepared for search, a note says how many are left, since more results may appear. Every
  result offers **Find similar** (the button in its top right corner while hovered), shows the
  year it was taken in its bottom right corner, and a video's tile shows the matching moment,
  which plays from that point when clicked. **×** in the search box clears the search.
- **Settings** (the gear in the top bar, or Ctrl/Cmd+,) opens in its own window, with a list of
  sections on the left; opening it again brings the open window forward, and Esc or the window's
  close button closes only Settings. (Where the system can't open a second window, it shows as a
  sheet over the main one.) Changes apply at once; only changing the search style asks first,
  because it makes Recollect look at your photos again.
  - **Appearance:** color (System, Light, Dark), text size and photo size (Small, Medium,
    Large).
  - **Features:** switches, all on by default, for **On this day**, **Tips**, **Remember recent
    searches**, **Example searches** and **Progress in the sidebar**. Turning recent searches off
    stops recording new ones and keeps the old ones until **Clear now**. **Show again** brings
    back the tips you closed.
  - **Smart search:** whether it is on, off or downloading; a **Search style** of *Best quality* or
    *Faster* (changing it asks first; the download size is shown); whether to use the graphics card; a folded **Details**
    part with the name, license and source of what it downloads.
  - **Library:** what Recollect is working on right now, with **Pause**/**Resume** and **Stop**
    (shown whenever something runs, even with the sidebar progress turned off), your folders
    (**Stop showing…** asks in the row and never touches your files), **Add a folder…**, how many photos and videos are ready for search and how many couldn't be
    opened (**Show** lists them, **Try again** retries), and **Check for new photos**. A folded
    **Advanced** part has the technical options.
  - **Storage:** how much room photo previews take (with **Clear previews**), how much they may
    take, and where Recollect keeps its data, with an
    **Open folder** button for each.
  - **About:** the version, the privacy promise (nothing leaves your computer; the only use of
    the internet is the smart-search download) and the licenses of what Recollect is built on.
- **Favorites:** click the star that appears in the corner of a tile when you point at it (or
  press `F` on the item you are on) to mark it; the **Favorites** view in the sidebar lists them.
- **Albums:** make your own albums (**New album…** in the sidebar, or from any **Add to album…**
  menu). An album opens as a plain grid; its heading says how many photos and videos it holds and
  the years they span, with **Add photos** (Photos in selection mode, a bar above the grid says
  "Choose photos to add to …"; **Add to …** adds them with Undo, **Cancel** or Esc goes back
  unchanged) and a **⋯** menu with **Rename…** and **Delete album…** (asks first; deleting an album
  never deletes photos or videos). An empty album says so and offers **Add photos**; in the sidebar
  it shows a neutral cover. Albums only hold references:
  files stay where they are, and an album forgets files that leave the library.
- **Multi-select** in every grid (timeline views, albums and search results): click the circle in
  a tile's corner, or `Ctrl+click` (`Cmd+click` on macOS) to toggle an item, `Shift+click` to
  select a range, `Ctrl+A` (`Cmd+A`) to select everything in the list, `Shift+←/→/↑/↓` to extend
  the selection; `Esc` or **Cancel** clears it. While items are selected a colored bar replaces
  the heading with "3 selected", **Add to album…**, **Favorite**, **Unfavorite** and, inside an
  album, **Remove from album**.
- **Activity card** at the bottom of the sidebar, only while something is happening ("Looking for
  new photos…", "Reading photo dates… 1,234 of 5,000", "Getting your photos ready for search…
  45 %", "Downloading smart search… 120 of 485 MB"), with pause and cancel.
- **Help** (the **?** button or F1, on every browsing screen): how search works and the keyboard
  shortcuts in sections (browsing, photo size, selecting, viewing).
- **On this day:** photos taken on today's date in earlier years, in a banner at the top of Photos (the ✕ hides it until tomorrow)
  (photos dated only by their file time are left out; 29 February shows on 28 February in years
  without one).
- Text search over images and video moments ("a dog in the snow").
- Video results show the moment's timestamp; **Play at** opens the video at that time in mpv or VLC
  when installed (otherwise in the default player).
- **Search by date and sort:** while searching, **When** limits results to a year or a range of
  years, and **Sort** orders them by relevance, newest or oldest.
- **Find similar** to any result.
- All / Photos / Videos filter next to the search box (applies to the album views too).
- Background preparation of your photos for search with pause, resume and cancel; you can search
  while it runs.
- Incremental scans (**Check for new photos** next to **Folders**): new, changed and deleted files
  are picked up; unchanged files are skipped.
- Keyboard: `Ctrl+F` (`Cmd+F` on macOS) focuses the search box, the arrow keys move the selection
  (`Shift` + arrows extends it, `Ctrl+A` / `Cmd+A` selects all), `Enter` opens the selected item
  in the viewer, `F` toggles favorite (for every selected item when several are), `Esc` clears the
  selection.

## How it works

Recollect uses a CLIP-style model (SigLIP or CLIP, run with ONNX Runtime) that maps both images and
text into the same vector space. Every image, and frames sampled about every 2 seconds from every
video, is turned into a vector once; a search turns your words into a vector and finds the closest
ones. Your media files are only read, never modified.

## Supported formats

- Images: JPEG (`jpg`, `jpeg`, `jpe`, `jfif`), PNG, WebP, GIF, BMP, TIFF (`tif`, `tiff`).
- Videos: `mp4`, `m4v`, `mov`, `mkv`, `webm`, `avi`, `wmv`, `mpg`, `mpeg`, `mts`, `m2ts`, `3gp`,
  `ogv`, `flv` (anything your FFmpeg build can decode).

Files are recognised by extension (case-insensitive). Hidden files and folders are skipped.

## Building

Requirements:

- Rust (stable, 1.90 or newer)
- FFmpeg development libraries (libavcodec, libavformat, libavutil, libavfilter, libavdevice,
  libswscale, libswresample), `pkg-config`, and clang (for `bindgen`)
  - Debian/Ubuntu: `sudo apt install libavcodec-dev libavformat-dev libavutil-dev libavfilter-dev libavdevice-dev libswscale-dev libswresample-dev pkg-config clang`
  - macOS: `brew install ffmpeg pkg-config`
  - Windows: download a *shared* FFmpeg 8 build (e.g. an `lgpl-shared` build from
    [FFmpeg-Builds](https://github.com/FFmpeg-Builds/FFmpeg-Builds/releases)), set `FFMPEG_DIR` to it,
    put its `bin` directory on `PATH`, and install LLVM for `bindgen` (set `LIBCLANG_PATH` to
    LLVM's `bin` directory if it is not found).
- On Linux, the usual GUI libraries (X11/Wayland, xkbcommon, Vulkan or OpenGL).

```bash
cargo build --release
./target/release/recollect          # start the app
```

The `ort` crate downloads ONNX Runtime at build time and copies its shared libraries (ONNX Runtime
and the WebGPU library Dawn) next to the binary. `build.rs` makes the binary look for them in its
own directory, so **keep them together** with the executable when you copy the app elsewhere. See
[docs/packaging.md](docs/packaging.md).

## First run

The first time Recollect starts with no folders it shows a short welcome, one question per
screen, with progress dots and a **Back** button (and **I'll set it up myself** on the first
screen if you would rather not):

1. **Welcome**: what Recollect does ("Find any photo by describing it") and that everything stays
   on your computer. Button: **Get started**.
2. **Where are your photos?** The standard folders that exist (Pictures and Videos are ticked;
   Desktop and Downloads are offered by name), **Add a folder…** (a folder picker; under
   **More options** you can type a path instead). **Continue** adds exactly the folders that are
   ticked or added; it stays off until at least one folder is chosen.
3. **Turn on smart search**: the **Best quality** and **Faster** choices with their download
   sizes (the one that suits your computer is marked), and a privacy note. **Turn on**
   starts the one-time download; **Not now** leaves it off. Nothing is downloaded
   without this choice, and without smart search you can still browse your library by date.
4. **You're all set**: **Start** adds the folders, looks through them, then starts the download
   if you chose it, and shows Photos. Progress shows at the bottom left and you can browse right
   away; search works once the download finishes.

People who already have folders never see the welcome. Afterwards, short one-time tips appear at
useful moments (turn them off or bring them back in Settings, Features), the search field
shows example and recent searches when you click it (stored on your computer only; **Clear recent
searches** removes them), and the **?** button opens a Help sheet (getting started, searching,
organizing, keyboard shortcuts, privacy).

### Models

| Id | Model | Download | License | Notes |
|---|---|---|---|---|
| `siglip-base-16` | SigLIP base/16 (Google) | 486 MB | Apache-2.0 | best quality; default with a discrete GPU |
| `clip-vit-b32` | CLIP ViT-B/32 (OpenAI) | 608 MB | MIT | about 6x faster than SigLIP on a CPU; default without a GPU |

Models are downloaded from the ONNX exports in the `Xenova/*` repositories on Hugging Face at a
pinned revision and verified with SHA-256. Switch models in Settings, or for one run with
`--model <id>` (see `recollect models list`). Each model keeps its own embeddings, so files must be indexed with the model you search with.

## Command line

Without a subcommand (or with `gui`) the app starts. Global options, valid with every subcommand:

| Option | Meaning |
|---|---|
| `--home <dir>` | keep all data under this directory (also `RECOLLECT_HOME`) |
| `--model <id>` | use this embedding model for this run |
| `--cpu` | never use the GPU |

| Command | What it does |
|---|---|
| `recollect gui` | start the graphical app (the default) |
| `recollect models list` | list models; `*` marks the selected one |
| `recollect models download [--yes]` | download the selected model; asks first unless `--yes` |
| `recollect folders add <paths>...` | add library folders |
| `recollect folders list` | list library folders |
| `recollect folders remove <path>` | remove a library folder |
| `recollect index [paths]...` | add the given folders (if any) and index everything new or changed |
| `recollect search <query> [-n, --limit N] [--kind image\|video] [--json]` | search; default limit 20; `--json` prints JSON lines |
| `recollect status` | library statistics and recent errors |
| `recollect eval <queries.json> --root <dir> [--json]` | measure search quality with a query set; paths in the query set are relative to `--root` |
| `recollect --home <scratch> bench search [--images N] [--videos N] [--segments N] [--queries N]` | search benchmark on synthetic data (defaults 200000 images, 20000 videos, 50 segments per video, 50 queries) |
| `recollect --home <scratch> bench rescan [--files N]` | scan and unchanged-rescan time for a tree of empty files (default 200000) |

`bench` requires `--home` pointing at a scratch directory.

## Environment variables

| Variable | Effect |
|---|---|
| `RECOLLECT_HOME` | keep all data under this directory (same as `--home`); useful for portable installs |
| `RECOLLECT_NO_GPU` | if set (any value), never use the GPU (same as `--cpu`) |
| `RECOLLECT_LOG` | log filter, e.g. `info` or `recollect=debug`; default `warn`; logs go to stderr |
| `RECOLLECT_FRAME_STATS` | if set, the GUI logs frame-time percentiles to stderr every 10 seconds and repaints continuously so the numbers reflect a busy UI (for diagnosing slowness) |
| `RECOLLECT_SEGMENT_TAU` | tuning knob: cosine similarity (default `0.90`) above which consecutive video frames are merged into one moment |
| `RECOLLECT_TEST_MODELS` | tests only; see [CONTRIBUTING.md](CONTRIBUTING.md) |

## Where data is stored

| What | Linux | macOS | Windows |
|---|---|---|---|
| Data: `library.db`, `models/`, `index-<model>.bin` | `~/.local/share/recollect` | `~/Library/Application Support/recollect` | `%APPDATA%\recollect\data` |
| Cache: `thumbs/` | `~/.cache/recollect` | `~/Library/Caches/recollect` | `%LOCALAPPDATA%\recollect\cache` |
| Config: `settings.toml` | `~/.config/recollect` | `~/Library/Application Support/recollect` | `%APPDATA%\recollect\config` |

- `library.db`: the library (folders, files, embeddings) in SQLite.
- `models/`: downloaded models.
- `index-<model>.bin`: a startup cache of the search index (vectors and capture dates); it is
  rebuilt if missing, stale or written by an older version.
- `thumbs/`: thumbnail cache; safe to delete.
- `settings.toml`: your settings.

With `--home <dir>` or `RECOLLECT_HOME`, these live in `data/`, `cache/` and `config/` below that
directory. Nothing is ever written inside your media folders.

## Privacy

- The only network access is the model download from huggingface.co, which happens only after you
  agree.
- ONNX Runtime telemetry is disabled.
- FFmpeg is restricted to local files, so a crafted video cannot make it touch the network.
- Your media is only read, never modified, and no data leaves your machine.

## Performance

On our test machine (8-core laptop, RTX 3060 Laptop GPU via WebGPU):

- Search over 1.2 million vectors: about 40 ms.
- Startup with 1.2 million vectors: 0.3 s (thanks to the index cache).
- Indexing 12-megapixel photos: about 28 images/s on the CPU with CLIP, about 45 images/s on the GPU
  with SigLIP.
- Video: about 50-70x realtime.

Your numbers will vary with hardware, photo size and codecs.

## Troubleshooting

- **No GPU, or GPU errors.** Recollect falls back to the CPU automatically. Force it with `--cpu`
  or `RECOLLECT_NO_GPU=1`. On a CPU, CLIP is much faster than SigLIP (`--model clip-vit-b32`).
- **"N files could not be read".** Files that failed to decode are listed in the app (and by
  `recollect status`). Use **Retry failed files** after fixing the problem.
- **A drive is not mounted.** Its files are kept in the library, not deleted; they reappear when the
  drive is back.
- **Something is wrong and you need details.** Run with `RECOLLECT_LOG=debug` and look at stderr.

## More

- [docs/design.md](docs/design.md): architecture and design decisions
- [docs/packaging.md](docs/packaging.md): packaging notes
- [CONTRIBUTING.md](CONTRIBUTING.md): how to contribute
- [CHANGELOG.md](CHANGELOG.md): release notes

## Credits

- The interface text is set in [Inter](https://rsms.me/inter/) (SIL Open Font License 1.1, see
  `assets/fonts/Inter-OFL.txt`).
- Icons are [Material Symbols](https://fonts.google.com/icons) (Apache License 2.0), via the
  `egui_material_icons` crate.
- The photos on the first-run screen are 32 CC0 (public domain) photographs from Unsplash; see
  `assets/welcome/README.md` for the list and sources.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT)
at your option. FFmpeg and ONNX Runtime are separate projects with their own licenses; distributed
binaries should use an LGPL build of FFmpeg. The models have their own licenses (see above).
