#!/usr/bin/env bash
# Generate the AUR `vellum` PKGBUILD (built from source) and its .SRCINFO.
#
#   RELEASE_TAG=v2.0.0 .github/scripts/generate_aur_files.sh [OUT_DIR]
#
# Environment:
#   RELEASE_TAG      release tag, e.g. v2.0.0 (falls back to GITHUB_REF_NAME)
#   PKGREL           AUR pkgrel (default 1)
#   AUR_MAINTAINER   maintainer line (default CPT-Dawn <ping@cptdawn.dev>)
#   GITHUB_REPOSITORY, GITHUB_SERVER_URL   where the source tarball lives
#   SOURCE_TARBALL   checksum this local file instead of downloading the tag's
#                    archive (dry runs before a tag exists)
#
# .SRCINFO is written with `makepkg --printsrcinfo`, so run this on Arch as a
# normal user. Without makepkg only the PKGBUILD is written.
set -euo pipefail

tag="${RELEASE_TAG:-${GITHUB_REF_NAME:-}}"
if [[ -z "$tag" ]]; then
  echo "RELEASE_TAG is required (example: v2.0.0)." >&2
  exit 1
fi
if [[ ! "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]]; then
  echo "Release tag must look like v2.0.0 or v2.0.0-rc.1 (received: $tag)." >&2
  exit 1
fi

out="${1:-.}"
version="${tag#v}"
# pkgver may not contain '-': 2.0.0-rc.1 becomes 2.0.0rc.1, which vercmp
# sorts before 2.0.0.
pkgver="${version//-/}"
pkgrel="${PKGREL:-1}"
maintainer="${AUR_MAINTAINER:-CPT-Dawn <ping@cptdawn.dev>}"
repo="${GITHUB_REPOSITORY:-CPT-Dawn/Vellum}"
server_url="${GITHUB_SERVER_URL:-https://github.com}"
url="${server_url}/${repo}"
# GitHub names the archive's top directory <repo>-<tag without the v>.
srcdir_name="${repo#*/}-${version}"

if [[ -n "${SOURCE_TARBALL:-}" ]]; then
  sha256="$(sha256sum "$SOURCE_TARBALL" | awk '{print $1}')"
else
  tmp_tarball="$(mktemp)"
  trap 'rm -f "$tmp_tarball"' EXIT
  curl -fsSL "${url}/archive/refs/tags/${tag}.tar.gz" -o "$tmp_tarball"
  sha256="$(sha256sum "$tmp_tarball" | awk '{print $1}')"
fi

mkdir -p "$out"

# Everything below is written verbatim except the ${...} in the header block.
cat > "$out/PKGBUILD" <<EOF
# Maintainer: ${maintainer}
pkgname=vellum
pkgver=${pkgver}
pkgrel=${pkgrel}
pkgdesc="Featherweight wallpaper manager for Wayland: a tiny daemon, CLI and TUI"
arch=('x86_64' 'aarch64')
url="${url}"
license=('MIT OR Apache-2.0')
depends=('glibc' 'libgcc')
makedepends=('cargo')
conflicts=('vellum-git')
_tag=${tag}
_srcdir=${srcdir_name}
source=("\$pkgname-\$pkgver.tar.gz::\$url/archive/refs/tags/\$_tag.tar.gz")
sha256sums=('${sha256}')
EOF

cat >> "$out/PKGBUILD" <<'EOF'

prepare() {
  cd "$_srcdir"
  export RUSTUP_TOOLCHAIN=stable
  cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"
}

build() {
  cd "$_srcdir"
  export RUSTUP_TOOLCHAIN=stable
  export CARGO_TARGET_DIR=target
  cargo build --frozen --release --workspace
  bash packaging/gen-docs.sh target/release/vellum target/dist
}

check() {
  cd "$_srcdir"
  export RUSTUP_TOOLCHAIN=stable
  export CARGO_TARGET_DIR=target
  # Unit tests only: the integration suites drive a headless Sway.
  cargo test --frozen --release --workspace --lib --bins
}

package() {
  cd "$_srcdir"
  install -Dm755 target/release/vellum target/release/vellumd -t "$pkgdir/usr/bin"
  install -Dm644 packaging/systemd/vellumd.service -t "$pkgdir/usr/lib/systemd/user"

  install -Dm644 target/dist/completions/vellum.bash \
    "$pkgdir/usr/share/bash-completion/completions/vellum"
  install -Dm644 target/dist/completions/_vellum -t "$pkgdir/usr/share/zsh/site-functions"
  install -Dm644 target/dist/completions/vellum.fish -t "$pkgdir/usr/share/fish/vendor_completions.d"
  install -Dm644 target/dist/man/*.1 -t "$pkgdir/usr/share/man/man1"

  install -Dm644 README.md -t "$pkgdir/usr/share/doc/$pkgname"
  install -Dm644 LICENSE-MIT LICENSE-APACHE -t "$pkgdir/usr/share/licenses/$pkgname"
}
EOF

if command -v makepkg > /dev/null; then
  (cd "$out" && makepkg --printsrcinfo > .SRCINFO)
else
  echo "makepkg not found: wrote PKGBUILD only." >&2
fi
