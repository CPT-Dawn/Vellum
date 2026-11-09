# Vellum

## Agent skills

### Issue tracker

Issues live in GitHub Issues on `CPT-Dawn/Vellum`, managed via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default vocabulary: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `GLOSSARY.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.

## Commands

```sh
cargo build --workspace --release          # what ships; release vellumd must stay < 640 KiB
cargo build --workspace                    # before tests: e2e suites run the built vellumd
cargo test --workspace --all-targets       # unit + headless-Sway e2e + lightness gates
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo deny check
bash -c 'source docs/demo/sandbox.sh; vellum set dusk.jpg; grim -o HEADLESS-1 /tmp/shot.png; demo_stop'  # try it without touching the desktop
```

Release (no separate doc): bump `version` in the root `Cargo.toml`, `cargo update -w`, add a CHANGELOG section and compare link, commit `chore(release): vX.Y.Z, ...`, then `git tag -s vX.Y.Z -m "Vellum X.Y.Z: ..."` and `git push origin main vX.Y.Z`. `release.yml` builds both tarballs, creates the GitHub Release and pushes `vellum` and `vellum-bin` to the AUR. Watch it with `gh run watch <id> --exit-status`. The AUR RPC can lag behind; check `https://aur.archlinux.org/cgit/aur.git/plain/.SRCINFO?h=vellum`.

Live smoke on the real desktop: back up `~/.local/state/vellum`, `vellum kill`, run `target/release/vellumd`, exercise it, then kill it, restore the State files and run `vellum status`, which auto-starts `/usr/bin/vellumd` again. Hyprland here uses a Lua config, so `hyprctl dispatch exec` fails.
