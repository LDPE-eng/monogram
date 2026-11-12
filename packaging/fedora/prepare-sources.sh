#!/usr/bin/env bash
set -euo pipefail

if [[ $# != 1 ]]; then
    printf 'Usage: %s RPM_TOPDIR\n' "$0" >&2
    exit 2
fi

repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
topdir=$(realpath -m -- "$1")
version=$(sed -n 's/^Version:[[:space:]]*//p' "$repo/packaging/fedora/monogram-desktop.spec")
name="monogram-desktop-$version"

for file in desktop/Cargo.lock vendor/mono-rs/Cargo.toml native/markup-rs/Cargo.toml; do
    if [[ ! -f "$repo/$file" ]]; then
        printf 'Required source is missing: %s\n' "$file" >&2
        exit 1
    fi
done

mkdir -p "$topdir"/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS}
staging=$(mktemp -d "$topdir/.monogram-sources.XXXXXXXX")
trap 'rm -rf -- "$staging"' EXIT

cargo vendor --manifest-path "$repo/desktop/Cargo.toml" --locked --versioned-dirs \
    "$staging/cargo-vendor" > "$staging/cargo-config.toml"
tar -C "$staging" -czf "$topdir/SOURCES/$name-cargo-vendor.tar.gz" cargo-vendor

tar -C "$repo" \
    --exclude='.git' --exclude='target' --exclude='.cargo' \
    --exclude='desktop/scratch' --exclude='*.rpm' --exclude='*.log' \
    --exclude='.env' --exclude='.env.*' --exclude='local.properties' \
    --exclude='*.pem' --exclude='*.key' --exclude='*.p12' \
    --exclude='*.jks' --exclude='*.keystore' \
    --exclude='google-services.json' --exclude='GoogleService-Info.plist' \
    --transform="s,^,$name/," \
    -czf "$topdir/SOURCES/$name.tar.gz" \
    LICENSE desktop/Cargo.toml desktop/Cargo.lock desktop/src desktop/tests \
    desktop/locales desktop/README.md native/markup-rs vendor/mono-rs vendor/tlottie \
    packaging/fedora
install -m0644 "$repo/packaging/fedora/monogram-desktop.spec" "$topdir/SPECS/monogram-desktop.spec"
printf 'Sources prepared in %s/SOURCES\n' "$topdir"
