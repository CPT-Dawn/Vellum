#!/usr/bin/env bash
# Generate shell completions and man pages from a built `vellum` binary.
#
#   packaging/gen-docs.sh target/release/vellum OUT
#
# Writes OUT/completions/{vellum.bash,_vellum,vellum.fish} and OUT/man/*.1.
# Used by the release workflow and the AUR PKGBUILD, so file names live here.
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <path/to/vellum> <out-dir>" >&2
  exit 2
fi

vellum="$1"
out="$2"

mkdir -p "$out/completions" "$out/man"
"$vellum" completions bash > "$out/completions/vellum.bash"
"$vellum" completions zsh > "$out/completions/_vellum"
"$vellum" completions fish > "$out/completions/vellum.fish"
"$vellum" man "$out/man"
