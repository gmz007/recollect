# Contributing to Recollect

Thanks for your interest! Recollect is a small project; issues and pull requests are welcome.

## Ground rules

- **Privacy first.** Recollect must never send user data anywhere. The only network access is
  the model download in `src/model/download.rs`, after explicit consent. Don't add telemetry,
  crash reporting, update checks or remote APIs.
- **Never modify the user's media.** Library folders are read-only for Recollect.
- Don't commit model weights or media files. Tests create their own tiny fixtures.

## Before you open a pull request

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo nextest run          # or: cargo test
cargo test --doc
cargo deny check           # when dependencies change
```

Tests that need the real models are marked `#[ignore]`; run them with
`cargo nextest run --run-ignored all` after `recollect models download`.

For UI changes, include a screenshot.

## Licensing

By contributing you agree that your contributions are licensed under MIT OR Apache-2.0, like the
rest of the project.
