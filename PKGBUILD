# Maintainer: WMDE <https://wmde.fun>
# Contributor: System76 <info@system76.com> (original cosmic-bg)
#
# Builds our fork Lin-WMDE/wmde-bg (branch wmde). Standalone WMDE component:
# installs alongside cosmic-bg (own binary + own config namespace fun.wmde.Background),
# so NO conflicts/replaces cosmic-bg.
pkgname=wmde-bg
pkgver=1.2.0
pkgrel=1
pkgdesc="WMDE wallpaper daemon (fork of cosmic-bg) - reads the fun.wmde.Background config"
arch=('x86_64')
url="https://wmde.fun"
license=('MPL-2.0')
# depends: wayland client (libwayland-client via smithay-client-toolkit); image codecs are
# static Rust crates. Verify with namcap after first build.
depends=('glibc' 'gcc-libs' 'wayland')
makedepends=('rust' 'cargo' 'just' 'git' 'wayland' 'clang' 'lld' 'pkgconf')
source=("$pkgname::git+https://github.com/Lin-WMDE/wmde-bg.git#branch=wmde")
sha256sums=('SKIP')

pkgver() {
  cd "$srcdir/$pkgname"
  git describe --long --tags --abbrev=7 2>/dev/null | sed 's/^epoch-//;s/^v//;s/\([^-]*-g\)/r\1/;s/-/./g' ||
    printf '1.2.0.r%s.g%s' "$(git rev-list --count HEAD)" "$(git rev-parse --short=7 HEAD)"
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
  # /usr/share/cosmic/fun.wmde.Background/v1/ (APPID from justfile)
  just rootdir="$pkgdir" prefix=/usr install
  install -Dm644 LICENSE.md "$pkgdir/usr/share/licenses/$pkgname/LICENSE.md"
}
