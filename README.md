<h1 align="center">
  <a href="https://github.com/monogram-android/monogram"><img width="130" height="130" alt="Monogram" src="./documents/monogram.png" /></a>
  <br />Monogram
</h1>

<p align="center">
  <a href="./LICENSE"><img alt="GPLv3" src="https://img.shields.io/badge/License-GPLv3-blue.svg" /></a>
  <a href="https://github.com/monogram-android/monogram/stargazers"><img alt="GitHub stars" src="https://img.shields.io/github/stars/monogram-android/monogram" /></a>
  <a href="https://deepwiki.com/monogram-android/monogram"><img alt="Ask DeepWiki" src="https://deepwiki.com/badge.svg" /></a>
  <img alt="Kotlin + Rust" src="https://img.shields.io/badge/Kotlin_+_Rust-MTProto-blue" />
  <a href="https://boosty.to/monogram"><img alt="Boosty" src="https://img.shields.io/badge/Boosty-Support-ff6f61" /></a>
</p>

<p align="center"><a href="README.md">English</a> · <a href="README_RU.md">Русский</a> · <a href="README_TR.md">Türkçe</a> · <a href="README_KOR.md">한국어</a> · <a href="README_UR.md">اُردو</a> · <a href="README_ES.md">Español</a></p>

<p align="center"><strong>Monogram</strong> is a blazing-fast, fully native Telegram client for Android 7.0 and newer. Built with <strong>Kotlin</strong>, <strong>Jetpack Compose</strong>, and <strong>Material 3</strong>, it pairs a modern Android interface with the independent <a href="https://github.com/monogram-android/mono-rs">mono-rs MTProto library</a>.</p>

> [!IMPORTANT]
> Monogram is in **active development**. Features and architecture are still evolving; bugs and incomplete behavior are possible.

Support the project on [Boosty](https://boosty.to/monogram)

## Screenshots

<div align="center">

| | | | |
|:---:|:---:|:---:|:---:|
| <img src="./documents/1.png" width="180" alt="Monogram 1" /> | <img src="./documents/2.png" width="180" alt="Monogram 2" /> | <img src="./documents/3.png" width="180" alt="Monogram 3" /> | <img src="./documents/4.png" width="180" alt="Monogram 4" /> |

</div>

## Project highlights

- **Fully native** — Built for Android with Kotlin and Jetpack Compose
- **Made for your screen** — Material 3 layouts that adapt to phones, tablets, and larger displays
- **Your chats, with media** — Photos, videos, and animated stickers in your conversations
- **Blazingly fast, powered by Rust** — Uses the independent [mono-rs](https://github.com/monogram-android/mono-rs) implementation of Telegram's MTProto protocol
- **No NFT or crypto** — Monogram will not include NFT promotions, gifts, or other Telegram features that we consider outside the scope of a messaging app

## Build from source

### 1. Requirements

- **JDK 17** and an Android Studio version compatible with the project's Android Gradle Plugin (see [the version catalog](gradle/libs.versions.toml))
- **Android SDK Platform 37**, platform tools, and an **Android NDK**. CI uses **NDK r28c**. Set `ANDROID_NDK_HOME` to select an NDK explicitly; otherwise Gradle looks under the SDK's `ndk` directory.
- **Rust 1.98 or newer**, Cargo, and the host platform's C/C++ build tools for native dependencies and UniFFI generation
- **Git**, the Android Rust targets below, and **cargo-ndk**:

```console
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
cargo install cargo-ndk --locked
```

### 2. Clone the repositories

```console
git clone --recurse-submodules https://github.com/monogram-android/monogram.git
cd monogram
git submodule update --init --recursive
```

The `vendor/mono-rs` submodule provides the pinned MTProto workspace locally. Initialize all submodules with `git submodule update --init --recursive`.

### 3. Configure local settings

Copy [local.properties.example](local.properties.example) to `local.properties`, then set `sdk.dir`, `API_ID`, and `API_HASH`.

Unix-like systems:

```sh
cp local.properties.example local.properties
```

Windows (PowerShell):

```powershell
Copy-Item local.properties.example local.properties
```

Obtain your API credentials from [my.telegram.org/apps](https://my.telegram.org/apps). Builds can be produced without them, but Telegram login requires valid credentials. Keep `local.properties`, signing keys, and service-account credentials untracked; never include them in logs or commits.

For your own release signing key, also set:

```properties
RELEASE_STORE_FILE=keystore/release.jks
RELEASE_STORE_PASSWORD=your_store_password
RELEASE_KEY_ALIAS=your_key_alias
RELEASE_KEY_PASSWORD=your_key_password
```

Without `RELEASE_STORE_FILE`, release and beta builds use the debug signing key by default. `-Punsigned=true` disables that fallback when no release key is configured. Debug-signed builds are for development, not distribution under your production signing identity.

### 4. Configure push notifications (optional)

- **FCM:** Register a Firebase Android app for `org.monogram` (the same application ID is used for debug, release, and beta). Place its configuration at `app/google-services.json`; Gradle applies the Google Services plugin only when this file exists. Firebase dependencies remain part of the app even without the configuration file.
- Register the corresponding FCM credentials for your Telegram API ID at [my.telegram.org/apps](https://my.telegram.org/apps). Keep `google-services.json` and any `firebase-adminsdk` JSON untracked; never package a service-account key in the APK.
- FCM testing requires Google Play services; use a Google Play emulator image for emulator checks
- **UnifiedPush:** Install and configure a compatible distributor on the device. The app includes UnifiedPush registration support.

### 5. Build and run

Open the repository in Android Studio, sync Gradle, and select the `app` run configuration, or use the commands below from the repository root. `installDebug` requires a connected device or a running emulator.

Unix-like systems:

```sh
./gradlew :app:assembleDebug
./gradlew :app:installDebug
./gradlew :app:assembleRelease
./gradlew :app:assembleBeta
```

Windows (PowerShell):

```powershell
./gradlew.bat :app:assembleDebug
./gradlew.bat :app:installDebug
./gradlew.bat :app:assembleRelease
./gradlew.bat :app:assembleBeta
```

- `debug`: development build
- `release`: optimized build with R8
- `beta`: release-derived build with R8 disabled for easier debugging


### Native compilation

Gradle builds the Rust libraries for `armeabi-v7a`, `arm64-v8a`, and `x86_64` and regenerates UniFFI Kotlin bindings from host debug libraries as part of the normal build. Prebuilt libvpx libraries are included under `native/vpx/prebuilt` for video stickers. To invoke the Android native compilation tasks directly:

```sh
./gradlew :native:mtproto:buildNativeMtproto :native:markup:buildNativeMarkup
```

On Windows, replace `./gradlew` with `./gradlew.bat`. Use `-PskipNativeBuild=true` only for Kotlin-only work when compatible native libraries and generated bindings already exist. It skips both native compilation and binding regeneration; SDK/NDK configuration is still required.

## Fedora desktop (native, development)

`desktop/` is a separate native Rust, Relm4, GTK4, and libadwaita application for Fedora 44. It uses the pinned mono-rs sources, the shared Rust markup crate, and tlottie directly; no JVM, Android SDK/NDK, Gradle, or Kotlin bridge is required. It is a development version with one-account storage, not a feature-complete Android replacement or a verified Fedora release.

The module implements API configuration and existing-account authentication (phone/code, 2FA, QR, and client-layer passkeys); cached and live chat/history views, folders, search, and profiles; text/media/special messages, drafts, scheduling, replies/topics, reactions, polls, checklists, and inline bots; downloads, uploads, cache/network policies, GStreamer playback, stickers, MPRIS, and notifications; proxies and device sessions; appearance/localization preferences; diagnostics, TDLib authorization import, and PackageKit update checks. See [desktop/README.md](desktop/README.md) for the detailed feature inventory and platform behavior.

Important limits remain: no new-account registration or multi-account support; passkeys use CTAP2 USB HID without interactive PIN support; VP9 alpha WebM stickers use a static thumbnail fallback; codecs depend on installed GStreamer plugins; invite/share deep links and inline notification replies are unsupported; TDLib import transfers authorization only; PackageKit only checks system repositories and opens GNOME Software rather than updating the app itself. Offline tests do not verify live Telegram behavior, GUI workflows, hardware authenticators, system services, or an installed RPM.

### Fedora dependencies, checks, and build

Install the build tools and libraries below only with the machine owner's consent. Rust must satisfy `desktop/Cargo.toml` (currently 1.99 or newer); see [desktop/README.md](desktop/README.md) for rustup setup and runtime services.

```sh
sudo dnf install git curl ca-certificates tar gzip findutils util-linux \
  gcc gcc-c++ make cmake pkgconf-pkg-config python3 \
  gtk4-devel libadwaita-devel libsecret-devel sqlite-devel \
  gstreamer1-devel gstreamer1-plugins-base-devel gstreamer1-plugins-good \
  systemd-devel rpm-build redhat-rpm-config desktop-file-utils appstream \
  /usr/bin/kwin_wayland /usr/bin/dbus-run-session

git submodule update --init --recursive
cd desktop
cargo fmt --package monogram-desktop -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
bash ../packaging/fedora/run-headless-tests.sh \
  cargo test --workspace --all-targets --locked
python3 tools/test_localization.py
cargo build --release --locked
cd ..
desktop-file-validate packaging/fedora/io.github.monogram.desktop.desktop
appstreamcli validate --no-net --strict \
  packaging/fedora/io.github.monogram.desktop.metainfo.xml
```

Run the local application as `./desktop/target/release/monogram-desktop`. Use disposable XDG directories and an isolated D-Bus/Secret Service session for GUI checks, never real test-suite accounts.

### Desktop API credentials

Obtain your own credentials at [my.telegram.org/apps](https://my.telegram.org/apps). The normal setup UI stores API ID/API hash and the session encryption key through Secret Service (GNOME Keyring or compatible KWallet), with no plaintext fallback. For controlled development, `SECRETS` can point to a mode-`0600` file containing `API_ID=...` and `API_HASH=...`; inline contents are also accepted but discouraged. This override takes precedence over stored API credentials and does not import them into Secret Service, while the session key still requires it. Never place real credentials in source, commands, logs, issues, CI, or RPM metadata.

### Local RPM build

From the repository root, with rustup's Cargo on `PATH`:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
rpm_topdir=$(mktemp -d "${TMPDIR:-/tmp}/monogram-rpmbuild.XXXXXXXX")
bash packaging/fedora/prepare-sources.sh "$rpm_topdir"
rpmbuild -ba --without system_rust \
  --define "_topdir $rpm_topdir" --define '_smp_build_ncpus 2' \
  "$rpm_topdir/SPECS/monogram-desktop.spec"
rpm -qpl "$rpm_topdir"/RPMS/*/monogram-desktop-*.rpm
rpm -qp --requires "$rpm_topdir"/RPMS/*/monogram-desktop-*.rpm
```

The preparation step vendors locked Cargo dependencies; the RPM build/check stages use them offline. These commands do not install the package or contact Telegram. Binary RPM and SRPM remain under `$rpm_topdir`; this is local packaging, not an official Fedora repository package. Installation, reproducibility, GUI behavior, and live login require separate verification before any release claim.

## Tech stack

- **Languages and protocol:** Kotlin, Rust, MTProto, UniFFI
- **Android UI and state:** Jetpack Compose, Material 3, Decompose, MVIKotlin
- **Desktop:** Rust, Relm4, GTK4, libadwaita, SQLite, Secret Service, GStreamer, MPRIS
- **Shared services:** Koin, Coroutines, Flow, Room
- **Media and push:** Media3, Coil, libvpx, tlottie, Firebase Cloud Messaging, UnifiedPush

## Project structure

| Path | Responsibility |
|:---|:---|
| `app` | Application entry point, dependency injection, navigation, and push integration |
| `core/*` | Shared models, database, UI, utilities, and markup access |
| `feature/*` | Authentication, chat list, conversations, folders, profiles, and settings |
| `network/bridge` | Kotlin client API, domain mapping, and error handling |
| `network/http` | HTTP media downloads, cache, and queue |
| `native/mtproto-rs` | Rust protocol client and UniFFI exports |
| `native/markup-rs` | Rust Markdown, syntax highlighting, and math parsing |
| `native/mtproto`, `native/markup` | Kotlin facades, generated bindings, and native libraries |
| `native/vpx` | Prebuilt VP9 decoder for video stickers |
| `desktop` | Native Fedora application, Cargo tests, and Fluent localizations |
| `packaging/fedora` | Desktop/AppStream metadata, icon, RPM spec, and offline source preparation |
| `vendor` | Third-party source and submodules |

Android features call `network/bridge`; native protocol logic lives in Rust. Features must not import generated UniFFI types or depend on other feature modules. Desktop calls mono-rs directly and does not use the Kotlin bridge. See [AGENTS.md](AGENTS.md) for development conventions and protocol invariants.

## Contributing

Create your branch from `develop` and target `develop` with your pull request. Follow existing architecture and coding style, keep changes focused, and include a clear description and relevant verification. Respect the [Telegram API Terms of Service](https://core.telegram.org/api/terms).

Run checks appropriate to the changed module. Examples:

```sh
./gradlew :network:bridge:test
cargo test --manifest-path native/mtproto-rs/Cargo.toml
cargo test --manifest-path native/markup-rs/Cargo.toml
git diff --check
```

Use `./gradlew.bat` on Windows. Run only the checks relevant to your change; live Telegram and device behavior require separate validation with your own credentials.

Report bugs with `[Bug]` and feature requests with `[Feature]` in the issue title. See the [Bug Tracker](https://github.com/orgs/monogram-android/projects/3/views/1) and [Feature Board](https://github.com/orgs/monogram-android/projects/5/views/1).

## Translations

UI strings live in `core/ui/src/main/res/values/strings.xml` and the `feature/*/src/main/res/values/strings.xml` files. Add or update `values-<locale>/strings.xml` in each relevant module (for example, `values-de/strings.xml`). Preserve resource names, formatting placeholders, and plural forms. Submit translations in a pull request. Keep translated READMEs aligned with the English version.

## License

Monogram is licensed under the [GNU General Public License v3.0](LICENSE). Third-party components and native crates retain the licenses declared in their respective source directories.
