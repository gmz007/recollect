# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/).

## [Unreleased]

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
