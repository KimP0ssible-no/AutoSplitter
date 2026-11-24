# Contributing to AutoSplitter

Thank you for helping. This page describes how a change gets into AutoSplitter.

## Before you start

- To report a bug or ask for a feature, open an issue with one of the issue forms.
- If you plan a larger change, open an issue first, so that we can agree on the approach before you write code.
- For a small fix, open a pull request directly.

## Build and test

The README section "Build from source" lists what you need. CI runs these commands on Windows and Linux, and a pull request must pass them:

```
sh packaging/fetch-models.sh
cargo clippy --all-targets -- -D warnings
cargo test --release
```

The OCR tests in `tests/ocr.rs` read the screenshots in `tests/images/`. The file name is the expected text: `_` stands for a space, and a `-2` suffix marks a second image of the same text. To add a test image, crop it to the OCR region and name it that way.

To see the windows without opening them, render them to a PNG:

```
cargo run --example ui_snapshot -- out.png main
```

The scenes are `main`, `overlay` and `editor`.

## How we write code

- Make the smallest change that solves the problem, and reuse what the code already has.
- Add one small test for each change that is not trivial.
- Write a comment only when the reason for the code is not clear from the code itself.
- Do not run `cargo fmt`. The code is formatted by hand, and `cargo fmt` rewrites whole files. Match the style of the code around your change.

## Pull requests

- Keep each pull request to one topic.
- In the description, say what changed, why, and how you tested it. The pull request template has these headings.
- For a pull request from a first-time contributor, GitHub holds the CI run until the maintainer approves it.
- The maintainer merges with squash, so the title becomes the commit message on `main`. Use the form "Area: what changed", for example "OCR: reads white text on yellow".

## Licence

Contributions are accepted under the Apache License 2.0, as section 5 of `LICENSE` says. The README section "Licence" explains how Slint's GPL-3.0 applies to the programs.

## Code of conduct

Everyone who takes part in this project follows the [code of conduct](CODE_OF_CONDUCT.md).
