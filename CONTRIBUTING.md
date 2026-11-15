# Contributing to Recollect

Thanks for your interest! Recollect is a small project; issues and pull requests are welcome.

## Ground rules

- **Privacy first.** Recollect must never send user data anywhere. The only network access is
  the model download in `src/model/download.rs`, after explicit consent. Don't add telemetry,
  crash reporting, update checks or remote APIs.
- **Never modify the user's media.** Library folders are read-only for Recollect.
- Don't commit model weights or media files. Tests create their own tiny fixtures.

## Before you open a pull request

Run all of these; CI runs the first four on Linux, Windows and macOS:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo nextest run
cargo test --doc
cargo deny check           # when dependencies change
```

You need the FFmpeg development libraries (see the README) and the `ffmpeg` command-line tool,
which video tests use to generate tiny clips. Set `RECOLLECT_NO_GPU=1` to keep tests on the CPU.

Tests that need the real models are marked `#[ignore]`. Download the models
(`recollect models download`, once per model, with `--model` to pick it), then point
`RECOLLECT_TEST_MODELS` at a directory with one subdirectory per model id, each holding the
downloaded files (`vision_model.onnx`, the text model, `tokenizer.json`):

```
$RECOLLECT_TEST_MODELS/clip-vit-b32/
$RECOLLECT_TEST_MODELS/siglip-base-16/
```

(These are the `models/<id>` directories inside the data directory.) Then run
`cargo nextest run --run-ignored all`. Models that are missing are skipped.

- Changes that may affect **search quality** (preprocessing, models, segmenting): run
  `recollect eval <queries.json> --root <dir>` before and after and include the numbers.
- Changes that may affect **performance** (index, scanning, decoding): run
  `recollect --home <scratch dir> bench search` or `bench rescan` and include the numbers. Use a
  release build for measurements.
- For UI changes, include a screenshot.

## Commit messages

Use [Conventional Commits](https://www.conventionalcommits.org/), e.g. `fix(video): handle files
without a duration` or `feat(ui): keyboard navigation`.

Release notes are generated from these messages, so write them for the people who will read the
notes: the subject says what changed for a user of Recollect, the body adds the details worth
knowing. `feat`, `fix`, `perf`, `refactor`, `build` and `docs` commits appear in the notes;
`test`, `ci` and `chore` do not.

## Releases

Versions are tagged without a prefix (`0.6.2`). [git-cliff](https://git-cliff.org) builds the
notes from the commit history with the rules in `cliff.toml`:

```bash
git cliff --unreleased                                      # what the next release holds so far
git cliff --unreleased --tag X.Y.Z --prepend CHANGELOG.md   # add its section to the changelog
```

To release: bump the version in `Cargo.toml`, add the section to `CHANGELOG.md` with the second
command, commit as `chore(release): X.Y.Z`, tag the commit `X.Y.Z` and push the branch and the
tag. The Release workflow then creates the GitHub release with the same notes.

## Design

Read [docs/design.md](docs/design.md) before larger changes; it explains the architecture and why
things are built this way.

## Licensing

By contributing you agree that your contributions are licensed under MIT OR Apache-2.0, like the
rest of the project.
