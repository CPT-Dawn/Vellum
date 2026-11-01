# Contributing

Thanks for wanting to make Vellum better. This page gets you from a fresh
clone to a pull request. [ARCHITECTURE.md](ARCHITECTURE.md) explains how the
pieces fit, and [GLOSSARY.md](GLOSSARY.md) lists the words we use for them:
please say Monitor, not output or screen, in code, docs and issues.

## What you need

- Rust stable, 1.99 or newer (`rustup update stable`)
- `sway` 1.10 or newer and `grim` for the end-to-end tests. They run a
  headless Sway, so you don't need to be running Sway yourself. Older Sways
  lack alpha-modifier, so the GPU Fade test can't pass there.
- Optional: `dav1d` to build the `avif` feature, `cargo-deny` for the
  license and advisory check.

On Arch: `pacman -S rustup sway grim`.

## Build

```sh
cargo build --workspace            # target/debug/vellum and vellumd
cargo build --workspace --release  # what ships: LTO, stripped
```

`vellum` finds `vellumd` next to itself, so `target/debug/vellum set ...`
uses the matching Daemon.

## Test

```sh
cargo build --workspace     # first: the e2e tests run the built vellumd
cargo test --workspace
```

That runs three kinds of tests:

- **Unit tests** next to the code, including the TUI's `update` and `view`
  through a fake (`crates/vellum/src/tui/harness.rs`).
- **End-to-end tests** in `crates/vellum/tests/`. Each one starts its own
  headless Sway (`WLR_BACKENDS=headless`, pixman renderer) in a temp dir
  that serves as its `XDG_RUNTIME_DIR`, `HOME` and every `XDG_*` folder,
  runs the real `vellumd` and `vellum` against it, and checks the pixels
  with grim. They never touch your session, Config or State, and they run
  in parallel. The harness is `crates/vellum/tests/support/mod.rs`.
- **Lightness gates** in `crates/vellum/tests/lightness.rs`: idle RSS, heap,
  wakeups and leftover frame memory of the real Daemon.

Running one suite with `cargo test --test monitors` does not rebuild
`vellumd`, so run `cargo build --workspace` first or you test a stale
Daemon. Other useful runs:

```sh
cargo test -p vellum --all-features                     # with AVIF and JXL
VELLUM_BLESS=1 cargo test -p vellum --bin vellum golden  # re-bless golden render images
INSTA_UPDATE=always cargo test -p vellum --bin vellum tui # accept TUI snapshot changes
```

Check re-blessed images and snapshots by eye before you commit them.

Before you push, run what CI runs:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI also fails if the release `vellumd` grows past 640 KiB.

## Try it by hand without touching your desktop

Source the demo sandbox from a fresh bash. It starts a headless Sway with
two Monitors (1920x1080 and a portrait 1080x1920), fills a Library with
sample wallpapers and points that shell's `HOME`, `XDG_*` folders and
`WAYLAND_DISPLAY` at it:

```sh
cargo build --release
bash
source docs/demo/sandbox.sh
vellum set dusk.jpg
grim -o HEADLESS-1 /tmp/shot.png   # look at the result
demo_stop && exit
```

To watch it live, run a nested Sway in a window instead
(`WLR_BACKENDS=wayland sway`), then use `WAYLAND_DISPLAY` of that Sway with
your `vellum` commands.

## Demo and screenshots

The README images are made from the sandbox, never from a real desktop:

- `docs/demo/screenshots.sh` regenerates `assets/screenshots/` (needs grim,
  kitty and ImageMagick).
- `vhs docs/demo/demo.tape` records `assets/demo.gif` with
  [VHS](https://github.com/charmbracelet/vhs).

Regenerate them when the TUI changes visibly.

## Benchmarks

`bench/bench.sh` measures `vellumd` next to swaybg, hyprpaper and awww
(whichever are installed). See [docs/benchmarks.md](docs/benchmarks.md).

## Style

- Keep the Daemon small. It must not link image decoders, toml or serde
  ([ADR 0003](docs/adr/0003-decoder-free-pixel-free-daemon.md)), and a test
  enforces that. Anything heavy belongs in `vellum`.
- New Config keys get a default, a precise error message and an entry in
  [docs/configuration.md](docs/configuration.md). A test checks that every
  example in that file parses, and that every TUI Action is documented.
- New TUI colors go through the theme roles in
  [docs/brand.md](docs/brand.md), never a raw color.
- Every file `vellum` writes goes through `atomic::write_atomically`,
  never a fixed temp name (two `vellum` processes often save at once).

## Commits and pull requests

We use [Conventional Commits](https://www.conventionalcommits.org): a type,
an optional scope, and a subject of 72 characters or fewer in the
imperative. A little humor is welcome; clarity comes first.

```text
feat(tui): add a Favorites-only filter
fix(vellumd): keep the old frame on screen during a scale change
docs(readme): explain why GNOME can't run Vellum
```

Common types are `feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `ci`
and `chore`. Breaking changes get a `!` after the type and a
`BREAKING CHANGE:` footer.

Open pull requests against `main`. One logical change per pull request,
with tests for behavior you add or fix. When users would notice the
change, add a line to the topmost, unreleased section of
[CHANGELOG.md](CHANGELOG.md).

## License

By contributing, you agree that your contributions are dual-licensed under
the [MIT](LICENSE-MIT) and [Apache 2.0](LICENSE-APACHE) licenses, like the
rest of Vellum.
