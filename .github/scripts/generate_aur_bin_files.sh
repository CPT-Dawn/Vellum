#!/usr/bin/env bash
# Generate the AUR `vellum-bin` PKGBUILD (prebuilt release tarballs) and its
# .SRCINFO.
#
#   RELEASE_TAG=v2.0.1 .github/scripts/generate_aur_bin_files.sh [OUT_DIR]
#
# Environment:
#   RELEASE_TAG      release tag, e.g. v2.0.1 (falls back to GITHUB_REF_NAME)
#   PKGREL           AUR pkgrel (default 1)
#   AUR_MAINTAINER   maintainer line (default CPT-Dawn <ping@cptdawn.dev>)
#   GITHUB_REPOSITORY, GITHUB_SERVER_URL   where the GitHub Release lives
#   DIST_DIR         read the .sha256 files from this local directory instead
#                    of downloading them from the release (dry runs)
#
# The checksums come from the .sha256 files that package_release.sh writes
# next to each tarball, so the release must exist first. .SRCINFO is written
# with `makepkg --printsrcinfo`, so run this on Arch as a normal user.
set -euo pipefail

tag="${RELEASE_TAG:-${GITHUB_REF_NAME:-}}"
if [[ -z "$tag" ]]; then
  echo "RELEASE_TAG is required (example: v2.0.1)." >&2
  exit 1
fi
if [[ ! "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]]; then
  echo "Release tag must look like v2.0.1 or v2.0.1-rc.1 (received: $tag)." >&2
  exit 1
fi

out="${1:-.}"
version="${tag#v}"
pkgver="${version//-/}"
pkgrel="${PKGREL:-1}"
maintainer="${AUR_MAINTAINER:-CPT-Dawn <ping@cptdawn.dev>}"
repo="${GITHUB_REPOSITORY:-CPT-Dawn/Vellum}"
server_url="${GITHUB_SERVER_URL:-https://github.com}"
url="${server_url}/${repo}"

# The sha256 of vellum-<version>-<target>.tar.gz.
checksum() {
  local name="vellum-${version}-$1.tar.gz.sha256" line
  if [[ -n "${DIST_DIR:-}" ]]; then
    line="$(cat "$DIST_DIR/$name")"
  else
    line="$(curl -fsSL "${url}/releases/download/${tag}/${name}")"
  fi
  line="${line%% *}"
  if [[ ! "$line" =~ ^[0-9a-f]{64}$ ]]; then
    echo "No valid checksum in $name." >&2
    exit 1
  fi
  printf '%s' "$line"
}

sha_x86_64="$(checksum x86_64-unknown-linux-gnu)"
sha_aarch64="$(checksum aarch64-unknown-linux-gnu)"

mkdir -p "$out"

# Everything below is written verbatim except the ${...} in the header block.
cat > "$out/PKGBUILD" <<EOF
# Maintainer: ${maintainer}
pkgname=vellum-bin
pkgver=${pkgver}
pkgrel=${pkgrel}
pkgdesc="Featherweight wallpaper manager for Wayland: a tiny daemon, CLI and TUI (prebuilt)"
arch=('x86_64' 'aarch64')
url="${url}"
license=('MIT OR Apache-2.0')
depends=('glibc' 'libgcc')
provides=('vellum')
conflicts=('vellum' 'vellum-git')
options=('!debug' '!strip')
_version=${version}
_tag=${tag}
source_x86_64=("\$pkgname-\$pkgver-x86_64.tar.gz::\$url/releases/download/\$_tag/vellum-\$_version-x86_64-unknown-linux-gnu.tar.gz")
source_aarch64=("\$pkgname-\$pkgver-aarch64.tar.gz::\$url/releases/download/\$_tag/vellum-\$_version-aarch64-unknown-linux-gnu.tar.gz")
sha256sums_x86_64=('${sha_x86_64}')
sha256sums_aarch64=('${sha_aarch64}')
EOF

cat >> "$out/PKGBUILD" <<'EOF'

package() {
  cd "vellum-$_version-$CARCH-unknown-linux-gnu"
  install -Dm755 vellum vellumd -t "$pkgdir/usr/bin"
  install -Dm644 vellumd.service -t "$pkgdir/usr/lib/systemd/user"

  install -Dm644 completions/vellum.bash \
    "$pkgdir/usr/share/bash-completion/completions/vellum"
  install -Dm644 completions/_vellum -t "$pkgdir/usr/share/zsh/site-functions"
  install -Dm644 completions/vellum.fish -t "$pkgdir/usr/share/fish/vendor_completions.d"
  install -Dm644 man/*.1 -t "$pkgdir/usr/share/man/man1"

  install -Dm644 README.md -t "$pkgdir/usr/share/doc/$pkgname"
  install -Dm644 LICENSE-MIT LICENSE-APACHE -t "$pkgdir/usr/share/licenses/$pkgname"
}
EOF

if command -v makepkg > /dev/null; then
  (cd "$out" && makepkg --printsrcinfo > .SRCINFO)
else
  echo "makepkg not found: wrote PKGBUILD only." >&2
fi
