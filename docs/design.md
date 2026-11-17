# Recollect design notes

How Recollect works and why it is built this way. Read this before larger changes.

## Overview

One Rust package (library + binary). The binary runs the GUI by default and has CLI
subcommands (`index`, `search`, `eval`, `bench`, …). Both talk to the same facade,
`library::Library`, which owns:

- the **SQLite database** (`store::db`) — the source of truth: folders, files, embedded items;
- the **in-memory vector index** (`store::index`) — rebuilt from the database, cached on disk;
- the **background worker** — one job at a time: model download, indexing, model switch.

The UI thread never touches the database or the models directly: searches, thumbnails and
library statistics run on worker threads and report back through channels.

Lock order is always **database, then index**. Code that changes both (indexing, folder removal,
model switch) updates the index while holding the database lock, so the two never disagree.

## Models and inference

Search uses a dual encoder: an image tower and a text tower that map into one vector space.

- **Runtime: ONNX Runtime** (`ort`). Mature CPU kernels and prebuilt binaries for all three
  platforms. GPU acceleration uses ONNX Runtime's **WebGPU** execution provider (Vulkan, D3D12,
  Metal), which needs no vendor SDK. The CUDA provider was rejected because it requires the user
  to install matching CUDA and cuDNN versions. Telemetry is disabled in code.
- **Models** (`model::registry`), downloaded on first use after the user agrees, pinned to a
  Hugging Face revision and verified by SHA-256:
  - *SigLIP base/16* (Apache-2.0), the default when a discrete GPU is present: clearly the best
    retrieval quality of the candidates we measured, but ~6× slower than CLIP on a CPU. Its text
    tower is the int8 export (same retrieval quality as fp32, a third of the memory).
  - *CLIP ViT-B/32* (MIT), the default without a GPU: fast on CPUs, somewhat less accurate.
  - The vision towers stay fp32: int8 vision weights cost several points of recall.
- SigLIP on WebGPU needs graph optimization level 2 (level 3 breaks it) and at most 16 images
  per batch. On the GPU every batch is padded to a fixed size so buffers can be reused; if GPU
  inference still fails, the session is rebuilt once and then indexing continues on the CPU.

## Images

Large JPEGs are decoded with **DCT-domain downscaling** (`jpeg-decoder`'s `scale`), which is
several times faster than a full decode followed by a resize; decoding, not inference, is the
bottleneck for 12 MP photos on a CPU. Other formats use the `image` crate. EXIF orientation is
always applied; resizing uses `fast_image_resize`. Thumbnails are produced lazily when results
are shown and cached on disk (bounded, least recently used first).

## Video

Videos are sampled with FFmpeg (`ffmpeg-next`, in-process). The sampler works on **packets**,
buffering one GOP (keyframe to keyframe) at a time:

- short GOPs (≤ 4 s): only the keyframe is decoded, and only when a sample is due;
- long GOPs: packets are decoded in order until every sample due inside the GOP (one per 2 s)
  was produced; very large GOPs are decoded as they stream.

This does not rely on decoders honouring "keyframes only" (VP9/AV1 decoders ignore it) and
bounds the gap between samples even for files with one keyframe per minute. Rotation metadata,
non-square pixels, timestamp discontinuities and damaged tails are handled. FFmpeg is restricted
to the `file` protocol so a crafted file can never make it use the network.

Consecutive frames whose embeddings are very similar (cosine ≥ 0.90 with the segment's first
frame, at most 60 s) are merged into one **segment** stored as the mean vector. This roughly
halves the number of vectors with no loss in retrieval quality. Several videos are decoded in
parallel and share embedding batches, which matters for libraries of short clips.

## Vector index

At the intended scale (≈200k images plus a few thousand hours of video, i.e. one to two
million vectors) an **exact brute-force scan** is fast enough and simpler than an approximate
index:

- vectors are stored as **i8** (symmetric scalar quantization, fixed scale per model): a quarter
  of the memory of f32 and faster to scan; ranking with i8 scores matches f32 ranking in our
  quality measurements, so results are not re-scored;
- the scan runs on all cores; on x86-64 an AVX2 build of the dot product is selected at runtime;
- results are grouped per file (a video appears once, with its best moments);
- each vector also carries its file's **capture time** (the timeline's local wall-clock time, or the
  file time when no date is known), so a date range is checked inside the scan with one integer
  comparison. The column is filled from the database at startup and when indexing adds vectors;
  when the date pass later learns a date for an already embedded file, the column is patched in
  place and the database epoch moves so the cache on disk is rewritten.

The full f32 vectors stay in SQLite (for "find similar" and future re-quantization).

**Viewer previews.** The viewer decodes previews on its own worker threads at the window size
(longest side at most 2048 px, DCT-scaled for JPEGs; a sharper variant up to 4096 px at 100 % zoom),
keeps about eight textures in memory and prefetches the neighbouring items. Nothing is cached on
disk; the thumbnail is shown scaled up until the preview is ready. Videos show a frame at the
matching moment (or at 10 % of the length, at most 5 s).

**Viewer layout.** A 52 point bar (back with the name of where the viewer was opened from, the day and
time with the position, four icon buttons), the picture filling the rest on the stage colour, an
optional 300 point details panel docked on the right, previous/next buttons at the edges while the
pointer is over the picture, and a filmstrip (`viewer/filmstrip.rs`) at the bottom that fades two
seconds after the pointer last moved. The panel's albums come with the file details, which are
read again when albums change.

**Index cache.** Decoding every vector from SQLite at startup takes seconds at a million
vectors, so the i8 index is also written to `data/index-<model>.bin`. Each change to stored items
increments an epoch in the database; the cache carries that epoch, the model id and a random
library id, and is only used if all match. It is written after jobs, never while indexing writes.

## Albums, favorites and selection

Favorite marks and album changes (create, rename, delete, add, remove) go through **one ordered
write queue** served by a single writer thread, so a burst such as "create an album, add these
items" is applied in the order it was made. Consecutive favorite marks collapse to the latest
value per file; an album change ends such a run. Each write reports back (the new album's id,
how many items changed, or an error such as a duplicate name) and bumps a version counter that
lists and views watch to reload (only when something actually changed). "Create with items" is
one transaction, so a failure leaves nothing behind, and album changes are shown only once the
writer has confirmed them. The sidebar's album list (counts and covers, two grouped queries) is
read in the background when that version moves or a job finishes. Album names are trimmed,
non-empty, at most 100 characters and unique ignoring case, checked by one shared function;
`album_items` rows cascade away with their file or album, so deleting an album or removing a
library folder never touches files and never deletes an album.

Grids keep a **selection by file id** plus a *primary* item (by list index) that drives the
viewer; ids that vanish in a reload are dropped. Range selection runs in
list order from an anchor, so it follows the order on screen. After an action from the
selection bar finishes (an album change, a favorite), the selection is cleared; Undo still works.

## Code layout

One crate; each directory is one type or one screen split by concern, and no source file holds
more than a few hundred lines of code.

| Path | What it holds |
|---|---|
| `library/` | `Library`, the thread-safe facade for the GUI and CLI: `jobs` (the single background worker, pause/cancel, requests that wait for it), `writes` (favorites and album changes through one writer thread), `folders`, `model` (choice, switch, download), `index_cache`, `status` |
| `store/db/` | `Db` (SQLite): `schema` (creation and upgrades), `files` (folders, scan sync, dates, counts), `items` (vectors), `timeline`, `albums` |
| `store/index.rs` | the in-memory vector index and its on-disk cache |
| `indexer.rs`, `video_index.rs` | the indexing run: scan, dates, image and video embedding |
| `media/` | `scan`, `image`, `metadata` (capture dates), `video/` (`stream`, `convert`, `sample`, `still`) |
| `model/` | `registry`, `download`, `embedder` |
| `search.rs` | turning index candidates into hits |
| `ui/` | `mod.rs` (the app and its frame loop), `notices`, `snapshot`, `dialogs`, `grid` (selection and keyboard moves shared by both grids), `results` (search), `favorites`, `selection_bar`, `folders`, `timeline/` (loading, day sections and layout, the day headings, the "On this day" banner), `ruler/` (the timeline ruler that replaces the scroll bar), `photo_size` (pinch, Ctrl/Cmd keys and the size chip), `viewer/`, `settings/`, and one file per remaining screen or widget |
| `cli.rs`, `bench.rs`, `eval.rs` | command-line entry points |

Functions are kept to 50 lines of code (a clippy threshold); longer scenario tests are the
exception.

## Interface structure

`ui/style.rs` holds the palettes, fonts (Inter and Material Symbols, with egui's fonts as
fallbacks), text styles and spacing; it is applied once per egui context and again when the theme
or text size setting changes. `ui/tiles.rs` draws a grid tile (center-cropped thumbnail edge to edge; the selection circle
on hover; a selected photo shrinks onto a tinted cell; a white heart for favorites; duration pill) and decides what a click means: a plain click opens the viewer unless the
user is already picking items, in which case it toggles; the selection circle, Ctrl/Cmd and Shift
select. `ui/sidebar.rs` and `ui/chrome.rs` hold the sidebar, the top bar, the view headers, the
toasts and the help window. Notices queue as before (an error outlives later plain messages) and
are drawn as toasts rather than in a status bar.

The timeline groups by capture day when the list loads (not per frame); layout finds the rows in
view by binary search, and the heading of the section at the top of the view is drawn pinned over
the photos, pushed up by the next one. The tile size (96 to 320 points, default 168) is one
setting for every grid; a pinch or Ctrl/Cmd and scroll changes it smoothly and it is saved once
it stops changing. Whether library folders can still be read is checked on a background thread
when the folder list changes and then every 30 seconds, so a slow network drive never holds up a
frame or the library facts the screens read.

The scroll bar of every grid is hidden; the ruler (`ui/ruler/`) is a column beside it that reads
and sets its scroll offset. Views grouped by day (Photos, Favorites, folders) give it a time scale:
one slot per calendar month between the newest and the oldest photo, empty months included, built
from the layout's section offsets when the layout changes. A point of the ruler maps to the
offset by binary search over those offsets (the position within a month maps to the position
within that month's rows; an empty month lands where the next older one starts), and the top of
the ruler is the top of the page, the bottom the end. Albums, Recently added and search results
have no day sections, so there the ruler is the plain scroll fraction and its date label is the
date of the photo at that fraction of the list. Dash positions are cached per layout and ruler
height; per frame only the dashes are drawn and one binary search finds the marker. With nothing
to scroll the ruler is left out. The dashes at years (or months, with three years or fewer) are
keyboard stops. The "On this day" banner asks for a large preview through the viewer's preview
cache and shows the grid thumbnail until it arrives.

## Incremental scanning and robustness

Rescans compare size and modification time. Files are never deleted from the library because of
problems that may be temporary: unreadable subdirectories are protected, a folder that suddenly
scans empty (an unmounted drive) is skipped, and I/O errors leave files pending instead of
failed. Decode errors are recorded and can be retried from the UI.

## Privacy

Only `model::download` uses the network, and only after the user agreed. No telemetry, no
crash reporting, no remote APIs. Library folders are only ever read.

## Measuring

- `recollect eval <queries.json> --root <dir>` measures search quality on a query set
  (caption queries → Recall@K, category queries → precision@10, video queries → Hit@K).
- `recollect --home <scratch> bench search|rescan` measures startup, memory, search latency and
  rescan time on a synthetic library of configurable size.
