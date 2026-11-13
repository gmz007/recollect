# Recollect

**Find photos and video moments by describing them.** Type "kids on the beach at sunset" and
Recollect finds the matching pictures and the moments in your videos — entirely on your own
computer.

- **Local and private.** Indexing and search run on your machine. No cloud, no accounts, no
  telemetry. The only network access is a one-time model download that you confirm.
- **Images and videos.** JPEG, PNG, WebP, GIF, BMP, TIFF; any video FFmpeg can decode.
- **Fast.** Search over hundreds of thousands of items in a fraction of a second. Uses your GPU
  (via WebGPU) when there is one, otherwise the CPU.
- Linux, Windows and macOS.

> Status: early development (v0.1). See [docs/design.md](docs/design.md) for how it works.

## How it works

Recollect uses a CLIP-style model (SigLIP or CLIP, run with ONNX Runtime) that maps both images and
text into the same vector space. Every image, and frames sampled every ~2 seconds from every video,
is turned into a vector once; a search turns your words into a vector and finds the closest ones.
Nothing ever leaves your machine; your media files are only read, never modified.

## Building

Requirements:

- Rust (stable, 1.90+)
- FFmpeg development libraries (libavcodec, libavformat, libavutil, libswscale…) and `pkg-config`
  - Debian/Ubuntu: `sudo apt install libavcodec-dev libavformat-dev libavutil-dev libavfilter-dev libavdevice-dev libswscale-dev libswresample-dev pkg-config clang`
  - macOS: `brew install ffmpeg pkg-config`
  - Windows: download a *shared* FFmpeg build (e.g. BtbN's LGPL build), set `FFMPEG_DIR`
    to it, and have LLVM installed for `bindgen`.
- On Linux, the usual GUI libraries (X11/Wayland, xkbcommon, Vulkan or OpenGL).

```bash
cargo build --release
./target/release/recollect          # start the app
```

ONNX Runtime is downloaded by the `ort` crate at build time and its shared libraries are placed
next to the binary; keep them together when you copy the app somewhere else.

## Using it

1. Start Recollect and add a folder with photos or videos.
2. The first time, Recollect asks to download a model (about 0.6–0.8 GB from Hugging Face).
3. Indexing runs in the background; you can search right away while it works.

There is also a command-line interface:

```bash
recollect models list                 # available models; * marks the selected one
recollect models download             # download the selected model (asks first)
recollect index ~/Pictures ~/Videos   # add folders and index them
recollect search "a dog in the snow"  # search from the terminal
recollect status                      # library statistics and errors
```

Use `--model clip-vit-b32` for the faster, smaller model (recommended without a GPU), `--cpu` to
never use the GPU, and `--home <dir>` (or `RECOLLECT_HOME`) to keep all data in one directory.

## Where data is stored

| What | Linux | macOS | Windows |
|---|---|---|---|
| Library database, models | `~/.local/share/recollect` | `~/Library/Application Support/recollect` | `%APPDATA%\recollect\data` |
| Thumbnails cache | `~/.cache/recollect` | `~/Library/Caches/recollect` | `%LOCALAPPDATA%\recollect\cache` |
| Settings | `~/.config/recollect` | `~/Library/Application Support/recollect` | `%APPDATA%\recollect\config` |

## Models

| Model | License | Notes |
|---|---|---|
| SigLIP base patch16-224 (Google) | Apache-2.0 | best quality; default with a discrete GPU |
| CLIP ViT-B/32 (OpenAI) | MIT | ~6× faster on CPU; default without a GPU |

Models are downloaded from the ONNX exports in the `Xenova/*` repositories on Hugging Face at a
pinned revision and verified with SHA-256.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT)
at your option. FFmpeg and ONNX Runtime are separate projects with their own licenses; distributed
binaries should use an LGPL build of FFmpeg.

See [CONTRIBUTING.md](CONTRIBUTING.md) to contribute.
