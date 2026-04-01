#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${GITHUB_REF_NAME:-}" ]]; then
  echo "GITHUB_REF_NAME is required (example: v1.0.0)." >&2
  exit 1
fi

release_tag="${GITHUB_REF_NAME}"
if [[ "$release_tag" != v* ]]; then
  echo "Release tag must start with 'v' (received: $release_tag)." >&2
  exit 1
fi

pkgname="vellum"
_reponame="Vellum"
pkgver="${release_tag#v}"
pkgrel="${PKGREL:-1}"
maintainer="${AUR_MAINTAINER:-CPT-Dawn <dawnsp0456@gmail.com>}"
repo="${GITHUB_REPOSITORY:-CPT-Dawn/Vellum}"
server_url="${GITHUB_SERVER_URL:-https://github.com}"

tarball_url="${server_url}/${repo}/archive/refs/tags/${release_tag}.tar.gz"

tmp_tarball="$(mktemp)"
trap 'rm -f "$tmp_tarball"' EXIT

curl -fsSL "$tarball_url" -o "$tmp_tarball"
sha256="$(sha256sum "$tmp_tarball" | awk '{print $1}')"

cat > PKGBUILD <<EOF
# Maintainer: ${maintainer}
pkgname=${pkgname}
_reponame=${_reponame}
pkgver=${pkgver}
pkgrel=${pkgrel}
pkgdesc="Wayland wallpaper stack with daemon and TUI"
arch=('x86_64')
url="${server_url}/${repo}"
license=('GPL3')
makedepends=('cargo' 'pkgconf')
provides=("vellum")
conflicts=("vellum-git")
install="\${pkgname}.install"
source=("\${pkgname}-\${pkgver}.tar.gz::${tarball_url}")
sha256sums=('${sha256}')

prepare() {
  cd "\${_reponame}-\${pkgver}"
  export CARGO_HOME="\${srcdir}/cargo-home"
  cargo fetch --locked
}

build() {
  cd "\${_reponame}-\${pkgver}"
  export CARGO_HOME="\${srcdir}/cargo-home"
  export CARGO_TARGET_DIR="\${srcdir}/target"
  cargo build --release --frozen --locked --workspace --bins
}

package() {
  cd "\${_reponame}-\${pkgver}"

  install -Dm755 "\${srcdir}/target/release/vellum" "\${pkgdir}/usr/bin/vellum"
  install -Dm755 "\${srcdir}/target/release/vellum-daemon" "\${pkgdir}/usr/bin/vellum-daemon"

  install -Dm644 packaging/systemd/user/vellum-daemon.service \\
    "\${pkgdir}/usr/lib/systemd/user/vellum-daemon.service"

  # Ship the autostart desktop entry as an example so it stays opt-in.
  install -Dm644 packaging/autostart/vellum.desktop \\
    "\${pkgdir}/usr/share/doc/\${pkgname}/examples/vellum.desktop"

  install -Dm644 README.md "\${pkgdir}/usr/share/doc/\${pkgname}/README.md"
  install -Dm644 LICENSE "\${pkgdir}/usr/share/licenses/\${pkgname}/LICENSE"
}
EOF

cat > .SRCINFO <<EOF
pkgbase = ${pkgname}
    pkgdesc = Wayland wallpaper stack with daemon and TUI
    pkgver = ${pkgver}
    pkgrel = ${pkgrel}
    url = ${server_url}/${repo}
    arch = x86_64
    license = GPL3
    makedepends = cargo
    makedepends = pkgconf
    depends = liblz4
    provides = vellum
    conflicts = vellum-git
    source = ${pkgname}-${pkgver}.tar.gz::${tarball_url}
    sha256sums = ${sha256}

pkgname = ${pkgname}
EOF
