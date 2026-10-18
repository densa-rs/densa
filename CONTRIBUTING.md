# Contributing to densa

Thank you for helping. This guide is everything you need to change densa: how the workspace
is laid out, what the code must guarantee, and the checks a pull request has to pass.

## Before you start

- **Bugs:** open an issue with what you did, what you expected and what happened, your OS,
  GPU and Rust version.
- **Larger changes** (a new crate, a public API, a new dependency): open an issue first, so
  the design is agreed before the code is written.
- **Picking up an issue:** say so in the issue, and keep to its scope. Issues labeled
  `good first issue` are self-contained.
- **Security issues:** do not open a public issue; follow the
  [security policy](https://github.com/densa-rs/.github/blob/main/SECURITY.md).

## How the workspace is laid out

Every `densa-*` crate in `crates/` is a library; applications live in `examples/`. The
[architecture diagram](README.md#architecture) shows the three lanes.

| Crate | Role |
|---|---|
| `densa-core` | Shared primitives (`Color`, `Point`, `Rect`, `Size`); no dependencies |
| `densa-text` | Fonts, rasterization, the glyph atlas |
| `densa-paragraph` | Shaped paragraphs, selection, `TextEditor` (on parley) |
| `densa-draw` | The GPU-free display list: `Canvas` (with clipping), `InstanceBuffer` |
| `densa-ui` | The widget tree: layout (flexbox, via taffy), events, focus, themes |
| `densa-widgets` | Labels, buttons, text inputs, scroll views, splitters, panels |
| `densa-grid` | The data grid widget over a `GridSource` |
| `densa-render` | wgpu: draws a frame's instances in one call |
| `densa-window` | Runs a `Ui` in a winit window, drawing with `densa-render` |
| `densa-data` | Live Arrow tables, views and grouped aggregates |
| `densa-arrow` | Arrow tables and live views in the grid |
| `densa-sync` | Lock-free hand-off between threads (triple buffer) |
| `densa-platform` | Thread placement and priority (the only crate with FFI) |
| `densa-fmt` | Formatting into stack buffers |
| `densa-alloc` | A counting allocator for tests |
| `densa-bench` | Criterion benchmarks for every crate |

Rules that keep it that way:

- **Dependencies point down.** Apps use `densa-window` → `densa-widgets` → `densa-ui` →
  `densa-draw` and `densa-paragraph` → `densa-text` → `densa-core`; `densa-arrow` →
  `densa-grid` → `densa-draw`; `densa-render` → `densa-draw`. Only `densa-render` and
  `densa-window` depend on a GPU or window crate, so everything below them runs, and is
  tested, headless.
- **No domain in the libraries.** A library crate that needs a market, a symbol or a price is
  wrong; generic APIs (`GridSource`, `Column`, `CellStyle`) let applications bring their
  domain. Domain code belongs in `examples/`.
- **Third-party types stay inside.** Public APIs use densa's own types (wrap parley, taffy,
  winit, wgpu or skrifa types); Apache Arrow types are the one exception, by design.
- **Pinned dependencies:** wgpu 30, winit 0.30, parley 0.12 with skrifa 0.44 (one font parser
  in the tree), taffy 0.15 (flexbox only). Not used on purpose: tokio, `rand`, glyphon. Propose new dependencies in an
  issue.

## What densa expects of code

- **Frames do not allocate.** Code on a per-frame path (drawing, scrolling, hit-testing,
  moving a caret, reading a view) must not allocate once content is cached. Prove it with a
  test using `densa-alloc`'s counting allocator, as the `tests/zero_alloc.rs` files do; such
  a test starts with `assert!(densa_alloc::is_active())`, or it passes without measuring.
- **Tests must catch bugs.** Prefer tests that compare against a simple model (see
  `crates/densa-data/tests/model.rs`). Then break the code on purpose (flip a comparison,
  drop a branch, return early), run the tests, and restore it: if no test fails, add one, or
  explain in the pull request why the change cannot be observed.
- **No panics on public input.** Return an error for anything a caller can get wrong.
- **Decide an API's shape before others depend on it:** `#[non_exhaustive]` on public enums
  and structs that may grow, wrapper types instead of foreign ones.
- **`unsafe` and atomics:** every `unsafe` block has a `// SAFETY:` comment (Clippy enforces
  it); every atomic operation has a comment justifying its ordering; concurrency primitives
  go through the `crate::sync` shim so loom can check them.
- **Measure, don't claim.** Performance work comes with a benchmark in `densa-bench`
  (benchmark threads pin themselves to performance cores; compare runs on one machine). Keep
  machine-specific numbers out of the README and the documentation.
- **English** in code, comments, documentation, examples, test data and screenshots.

## Checks

Run these before opening a pull request; CI runs them on Linux, macOS and Windows.

```sh
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p densa-paragraph --features system-fonts --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo +1.90 check --workspace --all-targets   # minimum supported Rust version
cargo tree -d -e normal                       # must not list skrifa or read-fonts
```

Set `DENSA_SKIP_GPU_TESTS=1` on machines without a GPU or desktop session.

When you change `densa-sync`, `densa-alloc` or `densa-platform`, also run:

```sh
RUSTFLAGS="--cfg loom" cargo test -p densa-sync --release --test loom_triple_buffer
cargo +nightly miri test --target x86_64-unknown-linux-gnu -p densa-sync -p densa-alloc -p densa-platform
```

Miri runs against the Linux target on purpose: with a custom global allocator, Miri reports a
false positive in the Windows `System` allocator. After changing memory orderings in the
triple buffer, run `scripts/ordering_mutants.ps1`; after changing thread placement or priority
on Windows, `scripts/pinning_experiment.ps1` and `scripts/priority_experiment.ps1`.

## Pull requests

1. Branch from `main` and keep the pull request to one change, with its tests.
2. Write commit messages as `type: description` (`feat`, `fix`, `refactor`, `docs`, `test`,
   `chore`, `perf`, `ci`), with a body that says why. Keep them to the change: no tool or
   attribution trailers.
3. Update what your change affects: rustdoc, the README (crate table, usage, limitations) and
   examples. If it changes what densa can do, say so in the description; the maintainers
   keep the website and the repository description in sync.
4. For a visible change, add a screenshot taken from an example, with English text.
5. Leave personal tooling out of commits (editor settings, local notes, agent
   configuration).

By contributing, you agree that your contributions are dual-licensed under MIT or
Apache-2.0, as densa is.
