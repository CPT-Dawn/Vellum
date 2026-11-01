#!/usr/bin/env bash
# Pack a release tarball from a finished `cargo build --release --workspace`.
#
#   .github/scripts/package_release.sh <version> <target-triple> [BIN_DIR] [OUT_DIR]
#
# BIN_DIR defaults to target/release, OUT_DIR to dist. Writes
# OUT_DIR/vellum-<version>-<target>.tar.gz and a matching .sha256. The tarball
# holds both binaries, the systemd user unit, completions, man pages, README
# and both licenses. Run it from the repository root.
set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "usage: $0 <version> <target-triple> [bin-dir] [out-dir]" >&2
  exit 2
fi

version="$1"
target="$2"
bin_dir="${3:-target/release}"
out_dir="${4:-dist}"
name="vellum-${version}-${target}"

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
root="$stage/$name"
mkdir -p "$root"

install -m755 "$bin_dir/vellum" "$bin_dir/vellumd" "$root/"
install -m644 packaging/systemd/vellumd.service README.md LICENSE-MIT LICENSE-APACHE "$root/"
bash packaging/gen-docs.sh "$root/vellum" "$root"

mkdir -p "$out_dir"
tar --sort=name --owner=0 --group=0 --numeric-owner \
  -C "$stage" -czf "$out_dir/$name.tar.gz" "$name"
(cd "$out_dir" && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
echo "$out_dir/$name.tar.gz"
