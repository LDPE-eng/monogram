%bcond_without system_rust
%global debug_package %{nil}

Name:           monogram-desktop
Version:        0.4.3
Release:        1%{?dist}
Summary:        Native GTK desktop client for the Monogram project
License:        GPL-3.0-only AND MIT AND MPL-2.0
URL:            https://github.com/monogram-android/monogram
Source0:        %{name}-%{version}.tar.gz
Source1:        %{name}-%{version}-cargo-vendor.tar.gz

%if %{with system_rust}
BuildRequires:  cargo >= 1.99
BuildRequires:  rust >= 1.99
%endif
BuildRequires:  gcc
BuildRequires:  gcc-c++
BuildRequires:  make
BuildRequires:  cmake
BuildRequires:  findutils
BuildRequires:  pkgconfig
BuildRequires:  pkgconfig(gtk4) >= 4.16
BuildRequires:  pkgconfig(libadwaita-1) >= 1.6
BuildRequires:  pkgconfig(gstreamer-1.0) >= 1.24
BuildRequires:  pkgconfig(gstreamer-base-1.0) >= 1.24
BuildRequires:  pkgconfig(gstreamer-app-1.0) >= 1.24
BuildRequires:  pkgconfig(gstreamer-video-1.0) >= 1.24
BuildRequires:  pkgconfig(gstreamer-audio-1.0) >= 1.24
BuildRequires:  pkgconfig(libsecret-1)
BuildRequires:  pkgconfig(sqlite3)
BuildRequires:  pkgconfig(libudev)
BuildRequires:  desktop-file-utils
BuildRequires:  appstream
BuildRequires:  /usr/bin/kwin_wayland
BuildRequires:  /usr/bin/dbus-run-session
Requires:       gtk4 >= 4.16
Requires:       libadwaita >= 1.6
Requires:       gstreamer1 >= 1.24
Requires:       gstreamer1-plugins-base >= 1.24
Requires:       gstreamer1-plugins-good >= 1.24
Requires:       libsecret
Requires:       sqlite-libs
Recommends:     gstreamer1-plugin-openh264
Recommends:     (gnome-keyring or kf6-kwallet)

%description
Monogram Desktop is the native Rust, GTK4 and libadwaita desktop application
for the Monogram project. This development version is not a feature-complete
replacement for the Android client. It does not include Telegram API credentials.

%prep
%autosetup -n %{name}-%{version} -a 1
mkdir -p desktop/.cargo
cat > desktop/.cargo/config.toml <<'EOF'
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "../cargo-vendor"
EOF

%build
cd desktop
export CARGO_INCREMENTAL=0
cargo build --release --frozen --jobs %{_smp_build_ncpus}

%install
install -Dpm0755 desktop/target/release/%{name} %{buildroot}%{_bindir}/%{name}
install -Dpm0644 packaging/fedora/io.github.monogram.desktop.desktop \
    %{buildroot}%{_datadir}/applications/io.github.monogram.desktop.desktop
install -Dpm0644 packaging/fedora/io.github.monogram.desktop.svg \
    %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/io.github.monogram.desktop.svg
install -Dpm0644 packaging/fedora/io.github.monogram.desktop.metainfo.xml \
    %{buildroot}%{_metainfodir}/io.github.monogram.desktop.metainfo.xml
find cargo-vendor vendor/mono-rs vendor/tlottie native/markup-rs -type f \
    \( -iname 'LICENSE*' -o -iname 'COPYING*' -o -iname 'NOTICE*' \
       -o -iname 'COPYRIGHT*' -o -iname 'UNLICENSE*' \) \
    -exec sh -c 'destination=$1; shift; for license do
        install -Dpm0644 "$license" "$destination/$license" || exit
    done' sh %{buildroot}%{_licensedir}/%{name}/third-party {} +

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/io.github.monogram.desktop.desktop
appstreamcli validate --no-net --strict \
    %{buildroot}%{_metainfodir}/io.github.monogram.desktop.metainfo.xml
export CARGO_INCREMENTAL=0
bash packaging/fedora/run-headless-tests.sh \
    cargo test --manifest-path desktop/Cargo.toml --workspace --all-targets \
    --frozen --jobs %{_smp_build_ncpus}

%files
%license LICENSE
%license vendor/mono-rs/docs/THIRD_PARTY_NOTICES.md
%license %{_licensedir}/%{name}/third-party
%doc desktop/README.md
%{_bindir}/%{name}
%{_datadir}/applications/io.github.monogram.desktop.desktop
%{_datadir}/icons/hicolor/scalable/apps/io.github.monogram.desktop.svg
%{_metainfodir}/io.github.monogram.desktop.metainfo.xml

%changelog
* Wed Nov 11 2026 Monogram contributors <noreply@monogram.invalid> - 0.4.3-1
- Package the native desktop development application with offline Cargo sources.
