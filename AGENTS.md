# Monogram agent guide

Monogram is an Android Telegram client with a separate native Fedora desktop
module. Android uses Kotlin and Jetpack Compose for the application and UI, and
Rust with UniFFI for the MTProto and markup clients; its minimum SDK is 24.
The desktop application uses Rust, Relm4, GTK4, and libadwaita, with mono-rs and
the shared Rust markup crate through local path dependencies.

## Working rules

- Inspect the relevant module, existing patterns, and current diff before
  editing. Keep changes narrow and do not rewrite unrelated user changes.
- Prefer the smallest clear implementation. Do not add abstractions, comments,
  dependencies, or files unless they solve a demonstrated problem.
- Do not add comments that restate code, describe obvious steps, or narrate the
  diff. Comment only non-obvious constraints, invariants, or workaround reasons.
- Reuse existing components, helpers, theme tokens, state models, and resource
  patterns before creating new ones. Search `core/ui` and the affected feature
  first. Extend a shared component when the behavior and ownership are truly
  shared; keep feature-specific behavior local.
- Do not duplicate Compose UI to make a small variation. Add a parameter only
  when it has a meaningful caller and preserves a simple API.
- Do not add tests merely to increase coverage or test implementation details.
  Add or update tests only for changed behavior, a fixed regression, a data
  contract, or a high-risk boundary. Prefer focused unit tests; use
  instrumentation tests only when the behavior requires Android rendering,
  lifecycle, storage, or interaction.
- Follow the owning crate's Rust test layout. Native crates keep unit tests in
  `tests/unit`; desktop uses `desktop/tests/unit`, with media unit tests under
  `desktop/src/media/tests/unit`. Reference unit tests from the owning source
  module with `#[cfg(test)]` and `#[path = "..."]` so private-module access
  stays intact; do not embed test modules in production source files. Put
  integration tests in the crate's `tests` directory.
- Run checks proportional to the change and report exactly what was run. Do
  not claim device, hardware, GUI, installed-package, or live Telegram
  verification without performing it.

## Repository layout and boundaries

| Path | Responsibility |
| --- | --- |
| `app` | Android entry point, dependency injection, root navigation, push, crash screen |
| `core/*` | Shared Android models, database, UI, markup access, and common utilities |
| `feature/*` | Android authentication, chats, dialogs, folders, profiles, and settings |
| `network/bridge` | Kotlin MTProto boundary, domain APIs, error mapping, DTOs |
| `network/http` | HTTP media downloads, cache, and queue |
| `native/mtproto-rs` | Rust MTProto client and UniFFI exports for Android |
| `native/markup-rs` | Shared Rust Markdown, syntax highlighting, and math parser |
| `desktop` | Native Fedora Rust/Relm4/GTK4/libadwaita application, tests, and Fluent locales |
| `packaging/fedora` | Desktop file, AppStream metadata, icon, RPM spec, and source preparation |
| `vendor` | Third-party code and submodules; do not edit for app or desktop work |

Android dependencies flow from `app` to features, then to `core` and `network`.
Features must not depend on other features or import generated UniFFI types.
Android protocol calls go through `network/bridge`; protocol state and native
logic belong in Rust.

The desktop crate is a separate Cargo workspace rooted at `desktop/Cargo.toml`.
It does not use Gradle, the Android SDK/NDK, Kotlin `network/bridge`, or UniFFI.
It calls `mono-proto-client` directly and shares `native/markup-rs`; keep its
GTK UI, worker/client boundary, storage, and platform integration within
`desktop`. Do not make Android modules depend on desktop code. Initialize all
submodules, including `vendor/mono-rs` and `vendor/tlottie`, but never edit
`vendor` for application work.

## Kotlin and Compose

- Use Decompose for navigation and MVIKotlin stores for stateful features.
  Composables receive immutable state and intent callbacks; stores stay outside
  composables.
- Collect flows with `collectAsStateWithLifecycle`, use stable lazy-list keys,
  and move blocking work off the main thread while preserving cancellation.
- Handle loading, empty, error, offline, and retry states where applicable.
  Map native failures to `TelegramError` in `network/bridge`; UI must not branch
  on raw TL error names.
- Put user-visible text in Android resources. Reuse Material 3 theme values and
  shared components from `core/ui`. Icon-only actions need content
  descriptions.
- Keep navigation state in the navigation stack, not local composable state,
  when the state represents the selected screen or conversation.

## Fedora desktop

- Keep the application `#![forbid(unsafe_code)]`. Run blocking protocol,
  Secret Service, PackageKit, and filesystem work off the GTK main context;
  preserve request cancellation and do not touch GTK objects from workers.
- Store API credentials, proxy secrets, and the session encryption key through
  Secret Service. Never add a plaintext fallback. The `SECRETS` environment
  variable is a development override for an API credentials file (or inline
  contents); do not put real values in commands, tests, logs, or commits.
- Use isolated XDG directories for GUI checks and never use live accounts in
  the default test suite. The current session layout supports one account;
  do not imply multi-account support or live feature parity.
- Preserve the explicit hardware and media fallbacks: CTAP2 passkeys currently
  use USB HID without interactive PIN entry, and VP9 alpha WebM stickers fall
  back to a static thumbnail. PackageKit update checks are read-only; opening
  GNOME Software is not an application-managed update.
- Keep Fedora RPM packaging offline after `cargo vendor`, honor the Rust MSRV,
  and validate the desktop/AppStream metadata. Building an RPM is not evidence
  that installation, GUI behavior, hardware, or Telegram login was verified.

## Localization

- Every new or changed user-visible string must be added to the base resource
  and translated in every locale already present in that same module. Do not
  leave new strings only in English.
- Preserve resource names, format placeholders, escaping, plurals, and quantity
  behavior exactly. Translate natural language, not identifiers or markup.
- Check both `core/ui/src/main/res/values*/strings.xml` and the relevant
  `feature/*/src/main/res/values*/strings.xml` files. Do not add translations
  to unrelated modules.
- Desktop Fluent catalogs are generated from the pinned Android resources by
  `desktop/tools/convert_locales.py`; desktop-only strings live in each
  `desktop/locales/<locale>/desktop-extra.ftl`. Translate every desktop locale,
  do not hand-edit generated `monogram.ftl` or `manifest.json`, and run the
  converter plus localization contract tests after resource changes.
- When a locale is not reasonably translatable from the available context,
  flag it explicitly instead of silently copying English. Keep locale files
  syntactically valid.
- Keep translated README files aligned when documentation text changes.

## Data, security, and protocol invariants

- Never log API credentials, authorization keys, passwords, phone codes, session
  snapshots, message bodies, or sensitive media data. Keep crash logs equally
  sanitized.
- Room is a cache: schema changes require a forward migration and version bump.
  Preserve login identity across migrations.
- Outgoing messages use a fresh random id and remain pending across process
  death until `updateMessageID` is received.
- Persist `pts`, `qts`, `seq`, and `date` per account, with channel `pts`
  separately. Apply updates only when cursors match; otherwise recover with the
  appropriate difference request.
- Respect Telegram history limits (`limit <= 100`), bounded/cancellable media
  transfers, and file-reference refresh rules. `FILE_MIGRATE_X` changes only
  the media DC.
- Preserve `CancellationException` as cancellation. Handle Telegram errors in
  `network/bridge`, including migration, auth, flood-wait, file-reference,
  and unknown/internal errors according to existing mappings.
- Do not hand-edit generated UniFFI bindings or generated error catalogs.
  Protocol-layer changes require the corresponding Rust, generated binding,
  ABI, and contract-test updates.

## Verification

Use the wrapper for the host OS: `./gradlew.bat` on Windows and `./gradlew`
elsewhere. Select the smallest relevant checks, for example:

```console
./gradlew.bat :feature:dialog:test
./gradlew.bat :core:ui:test
cargo test --manifest-path native/mtproto-rs/Cargo.toml
cargo test --manifest-path native/markup-rs/Cargo.toml
git diff --check
```

Use `-PskipNativeBuild=true` only for Kotlin-only work when generated bindings
and compatible native libraries are already available. Native changes require
the appropriate native build and relevant Kotlin contract checks.

For Fedora desktop work, use its own workspace from the repository root:

```console
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
appstreamcli validate --no-net --strict packaging/fedora/io.github.monogram.desktop.metainfo.xml
git diff --check
```

Use `python3 desktop/tools/convert_locales.py --check` for a quick generated
localization check. See `desktop/README.md` for Fedora dependencies and the
`packaging/fedora/prepare-sources.sh` plus `rpmbuild -ba --without system_rust`
flow. Live Telegram credentials, real accounts, hardware authenticators, manual
GUI workflows, PackageKit services, and device behavior are never part of the
default test suite; the wrapper only automates tests in an isolated headless
Wayland/D-Bus session.

## Setup

Copy `local.properties.example` to `local.properties` and set `sdk.dir`,
`API_ID`, and `API_HASH`. Keep credentials and generated Google/Firebase files
untracked. Initialize submodules when needed:

```console
git submodule update --init --recursive
```

The MTProto crate uses the pinned `vendor/mono-rs` submodule through local path dependencies. Initialize submodules with `git submodule update --init --recursive`.

## References

| Topic | Reference |
| --- | --- |
| MTProto 2.0 | https://core.telegram.org/mtproto |
| Encryption and `msg_key` | https://core.telegram.org/mtproto/description |
| Transports | https://core.telegram.org/mtproto/mtproto-transports |
| Service messages | https://core.telegram.org/mtproto/service_messages |
| Auth-key vectors | https://core.telegram.org/mtproto/samples-auth_key |
| Security | https://core.telegram.org/mtproto/security_guidelines |
| Authentication and SRP | https://core.telegram.org/api/auth and https://core.telegram.org/api/srp |
| Datacenters | https://core.telegram.org/api/datacenter |
| Invoking and layers | https://core.telegram.org/api/invoking and https://core.telegram.org/api/layers |
| Updates and errors | https://core.telegram.org/api/updates and https://core.telegram.org/api/errors |
| Files and references | https://core.telegram.org/api/files and https://core.telegram.org/api/file-references |
| Folders and offsets | https://core.telegram.org/api/folders and https://core.telegram.org/api/offsets |
| API id and terms | https://core.telegram.org/api/obtaining_api_id and https://core.telegram.org/api/terms |
| Telegram TL schemas | https://github.com/monogram-android/mono-rs/blob/main/docs/THIRD_PARTY_NOTICES.md |
| MTProto runtime | https://github.com/monogram-android/mono-rs |
| Telegram Android | https://github.com/DrKLO/Telegram |
| Grammers | https://github.com/LonamiWebs/grammers |
| Telegram Desktop | https://github.com/telegramdesktop/tdesktop |
