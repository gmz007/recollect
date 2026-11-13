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

**Index cache.** Decoding every vector from SQLite at startup takes seconds at a million
vectors, so the i8 index is also written to `data/index-<model>.bin`. Each change to stored items
increments an epoch in the database; the cache carries that epoch, the model id and a random
library id, and is only used if all match. It is written after jobs, never while indexing writes.

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
