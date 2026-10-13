name := 'wmde-bg'

# Чтобы shell() работал и на Windows с Git Bash; на Linux игнорируется
set windows-shell := ["C:\\Program Files\\Git\\bin\\sh.exe", "-cu"]

# Use mold linker if clang and mold exists.
clang-path := `which clang || true`
mold-path := `which mold || true`

linker-arg := if clang-path != '' {
    if mold-path != '' {
        '-C linker=' + clang-path + ' -C link-arg=--ld-path=' + mold-path + ' '
    } else {
        ''
    }
} else {
    ''
}

export RUSTFLAGS := linker-arg + env_var_or_default('RUSTFLAGS', '')

rootdir := ''
prefix := '/usr'


base-dir := absolute_path(clean(rootdir / prefix))

cargo-target-dir := env('CARGO_TARGET_DIR', 'target')
bin-src := cargo-target-dir / 'release' / name
bin-dst := base-dir / 'bin' / name

# Default recipe which runs `just build-release`
default: build-release

# Runs `cargo clean`
clean:
    cargo clean

# `cargo clean` and removes vendored dependencies
clean-dist: clean
    rm -rf .cargo vendor vendor.tar

# Compiles with debug profile
build-debug *args:
    cargo build {{args}}

# Compiles with release profile
build-release *args: (build-debug '--release' args)

# Compiles release profile with vendored dependencies
build-vendored *args: vendor-extract (build-release '--frozen --offline' args)

# Runs a clippy check
check *args:
    cargo clippy --all-features {{args}} -- -W clippy::pedantic

# Runs a clippy check with JSON message format
check-json: (check '--message-format=json')

# Run with debug logs
run *args:
    env RUST_LOG=debug RUST_BACKTRACE=1 cargo run --release {{args}}

# Installs files
#
# The wallpaper default is NOT installed from here: which image the desktop starts on is
# part of the theme and ships in the wmde-themes package. This daemon owns the image file
# (installed by the PKGBUILD) and the code that draws it, not the choice.
install:
    install -Dm0755 {{bin-src}} {{bin-dst}}

# Uninstalls installed files
uninstall:
    rm {{bin-dst}}

# Vendor dependencies locally
vendor:
    mkdir -p .cargo
    cargo vendor --sync Cargo.toml --sync config/Cargo.toml | head -n -1 > .cargo/config.toml
    echo 'directory = "vendor"' >> .cargo/config.toml
    tar pcf vendor.tar vendor
    rm -rf vendor

# Extracts vendored dependencies
vendor-extract:
    #!/usr/bin/env sh
    rm -rf vendor
    tar pxf vendor.tar

# Версия пакета как pkgver() в PKGBUILD: 1.10.0.<commits>.g<sha>.
# Теги epoch-* дают только счётчик через git describe, fallback — rev-list.
wmde-version := shell('desc=$(git describe --long --tags --abbrev=7 2>/dev/null || true); if [ -n "$desc" ]; then printf "1.10.0.%s.g%s" "$(printf "%s" "$desc" | sed -E "s/.*-([0-9]+)-g[0-9a-f]+$/\\1/")" "$(git rev-parse --short=7 HEAD)"; else printf "1.10.0.%s.g%s" "$(git rev-list --count HEAD)" "$(git rev-parse --short=7 HEAD)"; fi')
# CPU-baseline: пусто = чистый x86-64 (для deb/rpm по умолчанию), для v3: WMDE_CPU=x86-64-v3
wmde-cpu := env('WMDE_CPU', '')

# Раскладывает собранное в target/staging (тот же набор, что PKGBUILD кладёт в $pkgdir:
# бинарь через just install + дефолтная обоина напрямую, как в package())
package-staging staging="target/staging":
    #!/usr/bin/env bash
    set -e
    rm -rf {{staging}}
    if [ -n "{{wmde-cpu}}" ]; then export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C target-cpu={{wmde-cpu}}"; fi
    just build-release
    just rootdir="{{staging}}" prefix=/usr install
    install -Dm0644 data/backgrounds/wmde-default.jpg "{{staging}}/usr/share/backgrounds/wmde/wmde-default.jpg"
    # makepkg на Arch стрипает бинарники сам, в deb/rpm это делаем руками
    strip --strip-unneeded "{{staging}}/usr/bin/"* 2>/dev/null || true
    just package-doc "{{staging}}"

# doc для deb/rpm: копирайт + changelog.yaml (nFPM сам разложит changelog
# в changelog.Debian.gz для deb и в %changelog для rpm)
package-doc staging="target/staging":
    #!/usr/bin/env bash
    set -e
    mkdir -p "{{staging}}/usr/share/doc/{{name}}"
    cp packaging/copyright "{{staging}}/usr/share/doc/{{name}}/copyright"
    chmod 644 "{{staging}}/usr/share/doc/{{name}}/copyright"
    printf -- '- semver: %s\n  date: %s\n  packager: WMDE <wmde@wmde.fun>\n  urgency: low\n  distribution: unstable\n  deb:\n    urgency: low\n    distributions:\n      - unstable\n  changes:\n    - note: Automated build via nFPM (see INSTALL.deb-rpm.md).\n' "{{wmde-version}}-1" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > target/changelog.yaml

# Стейджинг из вендора: сборка без сети к git-зависимостям.
# vendor.tar генерим один раз при наличии сети (`just vendor`), сам тарболл
# в git не коммитится (см. .gitignore), на офлайн-машину его привозим файлом.
# Соседний чек-аут (../libcosmic) нужен и тут: path-зависимости вендором не покрываются.
package-staging-vendored staging="target/staging":
    #!/usr/bin/env bash
    set -e
    rm -rf {{staging}}
    if [ ! -f vendor.tar ]; then just vendor; fi
    if [ -n "{{wmde-cpu}}" ]; then export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C target-cpu={{wmde-cpu}}"; fi
    just build-vendored # сам распакует vendor.tar и соберёт --frozen --offline
    just rootdir="{{staging}}" prefix=/usr install
    install -Dm0644 data/backgrounds/wmde-default.jpg "{{staging}}/usr/share/backgrounds/wmde/wmde-default.jpg"
    strip --strip-unneeded "{{staging}}/usr/bin/"* 2>/dev/null || true
    just package-doc "{{staging}}"
    rm -rf vendor .cargo

# Собирает .deb через nFPM (нужен nfpm в PATH)
deb:
    #!/usr/bin/env bash
    set -e
    just package-staging
    export WMDE_VERSION="{{wmde-version}}"
    nfpm package --config packaging/nfpm.yaml --packager deb --target target/

# Собирает .rpm через nFPM (нужен nfpm в PATH)
rpm:
    #!/usr/bin/env bash
    set -e
    just package-staging
    export WMDE_VERSION="{{wmde-version}}"
    nfpm package --config packaging/nfpm.yaml --packager rpm --target target/

# .deb из вендора (офлайн-сборка)
deb-vendored:
    #!/usr/bin/env bash
    set -e
    just package-staging-vendored
    export WMDE_VERSION="{{wmde-version}}"
    nfpm package --config packaging/nfpm.yaml --packager deb --target target/

# .rpm из вендора (офлайн-сборка)
rpm-vendored:
    #!/usr/bin/env bash
    set -e
    just package-staging-vendored
    export WMDE_VERSION="{{wmde-version}}"
    nfpm package --config packaging/nfpm.yaml --packager rpm --target target/
