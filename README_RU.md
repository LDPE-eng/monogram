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

<p align="center"><strong>Monogram</strong> — молниеносный, полностью нативный клиент Telegram для Android 7.0 и новее. <strong>Kotlin</strong>, <strong>Jetpack Compose</strong> и <strong>Material 3</strong> сочетают современный Android-интерфейс с независимой библиотекой <a href="https://github.com/monogram-android/mono-rs">mono-rs для MTProto</a>.</p>

> [!IMPORTANT]
> Monogram находится в **активной разработке**. Возможности и архитектура продолжают меняться; возможны ошибки и неполная реализация отдельных функций.

Поддержать проект можно на [Boosty](https://boosty.to/monogram)

## Скриншоты

<div align="center">

| | | | |
|:---:|:---:|:---:|:---:|
| <img src="./documents/1.png" width="180" alt="Monogram 1" /> | <img src="./documents/2.png" width="180" alt="Monogram 2" /> | <img src="./documents/3.png" width="180" alt="Monogram 3" /> | <img src="./documents/4.png" width="180" alt="Monogram 4" /> |

</div>

## Особенности проекта

- **Полностью нативный** — Создан для Android на Kotlin и Jetpack Compose
- **Под ваш экран** — Интерфейс Material 3 адаптируется к телефонам, планшетам и большим экранам
- **Общение с медиа** — Фотографии, видео и анимированные стикеры в переписке
- **Молниеносный, благодаря Rust** — Использует независимую библиотеку [mono-rs](https://github.com/monogram-android/mono-rs) для протокола Telegram MTProto
- **Без NFT и криптовалют** — Monogram не будет включать продвижение NFT, подарки и другие функции Telegram, которые мы считаем выходящими за рамки приложения для обмена сообщениями

## Сборка из исходников

### 1. Требования

- **JDK 17** и версия Android Studio, совместимая с Android Gradle Plugin проекта (см. [каталог версий](gradle/libs.versions.toml)).
- **Android SDK Platform 37**, platform tools и **Android NDK**. В CI используется **NDK r28c**. Для явного выбора NDK задайте `ANDROID_NDK_HOME`; иначе Gradle ищет его в каталоге `ndk` внутри SDK.
- **Rust 1.98 или новее**, Cargo и инструменты сборки C/C++ для вашей ОС, необходимые для нативных зависимостей и генерации UniFFI
- **Git**, перечисленные ниже целевые платформы Rust для Android и **cargo-ndk**:

```console
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
cargo install cargo-ndk --locked
```

### 2. Клонирование репозиториев

```console
git clone --recurse-submodules https://github.com/monogram-android/monogram.git
cd monogram
git submodule update --init --recursive
```

Подмодуль `vendor/mono-rs` локально предоставляет закреплённое рабочее пространство MTProto. Инициализируйте все подмодули командой `git submodule update --init --recursive`.

### 3. Локальные настройки

Скопируйте [local.properties.example](local.properties.example) в `local.properties` и задайте `sdk.dir`, `API_ID` и `API_HASH`.

Unix-подобные системы:

```sh
cp local.properties.example local.properties
```

Windows (PowerShell):

```powershell
Copy-Item local.properties.example local.properties
```

Получите API-реквизиты на [my.telegram.org/apps](https://my.telegram.org/apps). Собрать приложение можно без них, но для входа в Telegram нужны действительные реквизиты. Не добавляйте `local.properties`, ключи подписи и реквизиты сервисных аккаунтов в систему контроля версий, логи или коммиты.

Чтобы подписывать релиз собственным ключом, также задайте:

```properties
RELEASE_STORE_FILE=keystore/release.jks
RELEASE_STORE_PASSWORD=your_store_password
RELEASE_KEY_ALIAS=your_key_alias
RELEASE_KEY_PASSWORD=your_key_password
```

Если `RELEASE_STORE_FILE` не задан, сборки release и beta по умолчанию подписываются отладочным ключом. Параметр `-Punsigned=true` отключает эту подстановку, если релизный ключ не настроен. Отладочная подпись предназначена для разработки и не заменяет ваш ключ для распространения приложения.

### 4. Push-уведомления (необязательно)

- **FCM:** Зарегистрируйте Android-приложение Firebase с идентификатором `org.monogram` (он одинаков для debug, release и beta). Поместите конфигурацию в `app/google-services.json`; Gradle подключает плагин Google Services только при наличии этого файла. Зависимости Firebase остаются в приложении и без файла конфигурации.
- Зарегистрируйте соответствующие реквизиты FCM для своего Telegram API ID на [my.telegram.org/apps](https://my.telegram.org/apps). Не добавляйте `google-services.json` и JSON-файлы `firebase-adminsdk` в систему контроля версий; никогда не включайте ключ сервисного аккаунта в APK.
- Для проверки FCM нужны сервисы Google Play; на эмуляторе используйте образ Google Play
- **UnifiedPush:** Установите и настройте совместимый дистрибьютор на устройстве. Приложение поддерживает регистрацию UnifiedPush.

### 5. Сборка и запуск

Откройте репозиторий в Android Studio, синхронизируйте Gradle и выберите конфигурацию запуска `app` либо выполните команды ниже из корня репозитория. Для `installDebug` нужно подключённое устройство или запущенный эмулятор.

Unix-подобные системы:

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

- `debug`: сборка для разработки
- `release`: оптимизированная сборка с R8
- `beta`: сборка на основе release с отключённым R8 для удобства отладки


### Нативная сборка

При обычной сборке Gradle собирает библиотеки Rust для `armeabi-v7a`, `arm64-v8a` и `x86_64` и генерирует Kotlin-привязки UniFFI из отладочных библиотек для хост-системы. Готовые библиотеки libvpx для видеостикеров находятся в `native/vpx/prebuilt`. Прямой запуск задач нативной сборки для Android:

```sh
./gradlew :native:mtproto:buildNativeMtproto :native:markup:buildNativeMarkup
```

В Windows замените `./gradlew` на `./gradlew.bat`. Используйте `-PskipNativeBuild=true` только для изменений в Kotlin, когда уже есть совместимые нативные библиотеки и сгенерированные привязки. Параметр пропускает и нативную компиляцию, и генерацию привязок; настройка SDK/NDK всё равно обязательна.

## Нативный desktop для Fedora (в разработке)

`desktop/` — отдельное нативное приложение на Rust, Relm4, GTK4 и libadwaita для Fedora 44. Оно напрямую использует закреплённые исходники mono-rs, общий Rust-крейт разметки и tlottie; JVM, Android SDK/NDK, Gradle и Kotlin bridge не нужны. Это разрабатываемая версия с хранением одного аккаунта, а не полнофункциональная замена Android-клиента и не проверенный релиз Fedora.

Модуль реализует настройку API и вход в существующий аккаунт (телефон/код, 2FA, QR и passkeys на клиентском слое); кэшированные и live-списки чатов и истории, папки, поиск и профили; текстовые, медийные и специальные сообщения, черновики, отложенную отправку, ответы/темы, реакции, опросы, checklists и inline-ботов; загрузки, отправку медиа, политики кэша и сети, воспроизведение GStreamer, стикеры, MPRIS и уведомления; прокси и сеансы устройств; внешний вид и локализацию; диагностику, импорт авторизации TDLib и проверку обновлений PackageKit. Полный перечень функций и поведение платформы описаны в [desktop/README.md](desktop/README.md).

Сохраняются важные ограничения: нет регистрации нового аккаунта и multi-account; passkeys используют CTAP2 USB HID без интерактивного ввода PIN; VP9 alpha WebM-стикеры показывают статический thumbnail; кодеки зависят от установленных GStreamer plugins; invite/share deep links и inline-ответы в уведомлениях не поддерживаются; TDLib import переносит только авторизацию; PackageKit лишь проверяет системные репозитории и открывает GNOME Software, но не обновляет приложение сам. Offline-тесты не проверяют live Telegram, GUI-сценарии, аппаратные authenticator'ы, системные сервисы или установленный RPM.

### Зависимости, проверки и сборка в Fedora

Устанавливайте указанные инструменты и библиотеки только с согласия владельца машины. Rust должен соответствовать `desktop/Cargo.toml` (сейчас требуется 1.99 или новее); настройка rustup и runtime-сервисов описана в [desktop/README.md](desktop/README.md).

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

Локальное приложение запускается командой `./desktop/target/release/monogram-desktop`. Для GUI-проверок используйте одноразовые XDG-каталоги и изолированную D-Bus/Secret Service-сессию, никогда не используйте реальные аккаунты в тестовом наборе.

### API credentials для desktop

Получите собственные credentials на [my.telegram.org/apps](https://my.telegram.org/apps). Обычный UI настройки сохраняет API ID/API hash и ключ шифрования сессии через Secret Service (GNOME Keyring или совместимый KWallet), без fallback в plaintext. Для контролируемой разработки `SECRETS` может указывать на файл с режимом `0600`, содержащий `API_ID=...` и `API_HASH=...`; inline-содержимое тоже поддерживается, но не рекомендуется. Этот override имеет приоритет над сохранёнными API credentials и не импортирует их в Secret Service, а ключ сессии по-прежнему требует его. Никогда не помещайте реальные credentials в исходники, команды, логи, issues, CI или RPM metadata.

### Локальная сборка RPM

Из корня репозитория, с Cargo из rustup в `PATH`:

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

На этапе подготовки locked Cargo-зависимости помещаются в vendor-архив; стадии сборки и проверки RPM используют их offline. Эти команды не устанавливают пакет и не обращаются к Telegram. Бинарный RPM и SRPM остаются в `$rpm_topdir`; это локальная упаковка, а не пакет официального репозитория Fedora. Установка, воспроизводимость, GUI и live-вход требуют отдельной проверки до любого заявления о релизе.

## Технологии

- **Языки и протокол:** Kotlin, Rust, MTProto, UniFFI
- **Android-интерфейс и состояние:** Jetpack Compose, Material 3, Decompose, MVIKotlin
- **Desktop:** Rust, Relm4, GTK4, libadwaita, SQLite, Secret Service, GStreamer, MPRIS
- **Общие сервисы:** Koin, Coroutines, Flow, Room
- **Медиа и уведомления:** Media3, Coil, libvpx, tlottie, Firebase Cloud Messaging, UnifiedPush

## Структура проекта

| Путь | Назначение |
|:---|:---|
| `app` | Точка входа, внедрение зависимостей, навигация и push-уведомления |
| `core/*` | Общие модели, база данных, интерфейс, утилиты и доступ к обработке разметки |
| `feature/*` | Авторизация, список чатов, переписка, папки, профили и настройки |
| `network/bridge` | API клиента для Kotlin, преобразование моделей и обработка ошибок |
| `network/http` | Загрузка медиа по HTTP, кеш и очередь |
| `native/mtproto-rs` | Протокольный клиент на Rust и экспорты UniFFI |
| `native/markup-rs` | Обработка Markdown, подсветки синтаксиса и формул на Rust |
| `native/mtproto`, `native/markup` | Фасады Kotlin, сгенерированные привязки и нативные библиотеки |
| `native/vpx` | Готовый декодер VP9 для видеостикеров |
| `desktop` | Нативное Fedora-приложение, Cargo-тесты и Fluent-локализации |
| `packaging/fedora` | Desktop/AppStream metadata, иконка, RPM spec и подготовка offline-исходников |
| `vendor` | Сторонние исходники и подмодули |

Android-модули feature обращаются к `network/bridge`; нативная протокольная логика находится в Rust. Модули feature не должны импортировать сгенерированные типы UniFFI или зависеть друг от друга. Desktop напрямую вызывает mono-rs и не использует Kotlin bridge. Правила разработки и протокольные инварианты описаны в [AGENTS.md](AGENTS.md).

## Участие в разработке

Создавайте ветку от `develop` и направляйте pull request в `develop`. Соблюдайте существующую архитектуру и стиль кода, ограничивайте изменения задачей и прикладывайте понятное описание с результатами подходящих проверок. Соблюдайте [условия использования Telegram API](https://core.telegram.org/api/terms).

Запускайте проверки, относящиеся к изменённому модулю. Примеры:

```sh
./gradlew :network:bridge:test
cargo test --manifest-path native/mtproto-rs/Cargo.toml
cargo test --manifest-path native/markup-rs/Cargo.toml
git diff --check
```

В Windows используйте `./gradlew.bat`. Запускайте только проверки, относящиеся к изменению; работу с реальным Telegram и поведение на устройстве проверяйте отдельно со своими реквизитами.

Для ошибок используйте префикс `[Bug]`, для предложений — `[Feature]` в заголовке issue. См. [трекер ошибок](https://github.com/orgs/monogram-android/projects/3/views/1) и [доску предложений](https://github.com/orgs/monogram-android/projects/5/views/1).

## Переводы

Строки интерфейса находятся в `core/ui/src/main/res/values/strings.xml` и файлах `feature/*/src/main/res/values/strings.xml`. Добавляйте или обновляйте `values-<locale>/strings.xml` в соответствующих модулях (например, `values-de/strings.xml`). Сохраняйте имена ресурсов, параметры форматирования и формы множественного числа. Отправляйте переводы через pull request. Синхронизируйте переводы README с английской версией.

## Лицензия

Monogram распространяется под [GNU General Public License v3.0](LICENSE). Для сторонних компонентов и нативных крейтов действуют лицензии, указанные в соответствующих каталогах исходников.
