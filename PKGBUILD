# Maintainer: WMDE <https://wmde.fun>
# Contributor: System76 <info@system76.com> (original cosmic-bg)
#
# Builds our fork Lin-WMDE/wmde-bg (branch wmde). Standalone WMDE component:
# installs alongside cosmic-bg (own binary + own config namespace fun.wmde.Background),
# so NO conflicts/replaces cosmic-bg.
pkgname=wmde-bg
pkgver=1.2.0
pkgrel=2
pkgdesc="WMDE wallpaper daemon (fork of cosmic-bg) - reads the fun.wmde.Background config"
arch=('x86_64')
url="https://wmde.fun"
license=('MPL-2.0')
# depends: wayland client (libwayland-client via smithay-client-toolkit); most image codecs
# are static Rust crates, but avif decoding uses the system dav1d (image/avif-native, COSMIC
# 1.4). Verify with namcap after first build.
depends=('glibc' 'gcc-libs' 'wayland' 'dav1d')
makedepends=('rust' 'cargo' 'just' 'git' 'wayland' 'libxkbcommon' 'clang' 'lld' 'pkgconf' 'dav1d')
# NOTE: Cargo.toml patches libcosmic to the sibling ../libcosmic checkout. The build
# harness arranges it next to $srcdir; a standalone makepkg run without that layout
# fails dependency resolution.
source=("$pkgname::git+https://github.com/Lin-WMDE/wmde-bg.git#branch=wmde")
sha256sums=('SKIP')

pkgver() {
  cd "$srcdir/$pkgname"
  # WMDE unified version: 1.5 (libcosmic base) . <commits since nearest tag> . g<short>.
  local desc
  desc=$(git describe --long --tags --abbrev=7 2>/dev/null || true)
  if [ -n "$desc" ]; then
    printf '1.5.%s.g%s' "$(printf '%s' "$desc" | sed -E 's/.*-([0-9]+)-g[0-9a-f]+$/\1/')" "$(git rev-parse --short=7 HEAD)"
  else
    printf '1.5.%s.g%s' "$(git rev-list --count HEAD)" "$(git rev-parse --short=7 HEAD)"
  fi
}

build() {
  cd "$srcdir/$pkgname"
  # x86-64-v3 (AVX2/BMI2) baseline for the WMDE repo; runs on Haswell+ (and the VM).
  export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C target-cpu=x86-64-v3"
  just build-release
}

package() {
  cd "$srcdir/$pkgname"
  # installs /usr/bin/wmde-bg and the default schema to
  # /usr/share/wmde/fun.wmde.Background/v1/ (APPID from justfile; config root is wmde)
  just rootdir="$pkgdir" prefix=/usr install
  install -Dm644 LICENSE.md "$pkgdir/usr/share/licenses/$pkgname/LICENSE.md"
  # WMDE default wallpaper referenced by the default schema + Entry::fallback() (CC0).
  install -Dm644 data/backgrounds/wmde-default.jpg "$pkgdir/usr/share/backgrounds/wmde/wmde-default.jpg"
}
