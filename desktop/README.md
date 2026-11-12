# Monogram Desktop — Fedora 44

Нативный desktop-модуль на Rust, Relm4, GTK4 и libadwaita. Он использует
`mono-proto-client` из закреплённого `vendor/mono-rs`, общий
`native/markup-rs` и `vendor/tlottie` как локальные path dependencies. JVM,
Android SDK/NDK, Gradle и Kotlin `network/bridge` для desktop-сборки не нужны.

Это разрабатываемая версия, а не полная замена Android-клиента и не проверенный
релиз Fedora. Реализация функции в коде, успешные offline-тесты, release build
или наличие RPM не доказывают live-совместимость с Telegram. Текущая модель
хранения рассчитана на **один аккаунт** (`account-1.session`).

## Реализованные возможности

Ниже перечислены реализованные кодовые пути и UI, а не результаты live-проверки
каждой функции:

- **Настройка и вход:** ввод собственных API ID/API hash, подключение, вход по
  телефону и коду, повторная отправка кода, пароль 2FA, QR-вход, восстановление
  зашифрованной сессии, выход с очисткой аккаунта. Клиентский слой также
  реализует passkey-вход; UI настроек позволяет создавать, перечислять и
  удалять passkeys.
- **Чаты и навигация:** сетевая и кэшированная загрузка чатов и истории с
  пагинацией, live updates, поиск контактов, глобальный и внутричатовый поиск,
  профили, участники, общие чаты и медиа профиля. Папки можно просматривать,
  создавать, изменять, удалять и переупорядочивать.
- **Сообщения:** текст с entities и общей Markdown/markup-обработкой, ответы и
  темы, silent/scheduled sending, отключение preview, сохранение pending sends и
  повтор после ошибки, редактирование, удаление, пересылка, реакции и списки
  reacted peers, read/unread, закреплённые сообщения и треды ответов. Есть облачные
  черновики с проверкой revision и очередь scheduled messages с редактированием,
  переносом, удалением и отправкой сейчас.
- **Медиа:** отправка фото, видео, документов и альбомов с подписями, замена
  медиа, аватары, thumbnails, полные и range-загрузки, progress/cancellation,
  pending media и retry. Кэш имеет лимиты, статистику и очистку по категории или
  чату; auto-download, autoplay и preload настраиваются для Wi-Fi/mobile/roaming.
  Воспроизведение использует GStreamer, поддерживает очередь, seek, скорость,
  mute, loop и MPRIS. Реализованы TGS-стикеры, WebM VP9 без alpha, saved GIFs,
  sticker packs, custom emoji и недавние реакции.
- **Специальное содержимое:** опросы и викторины, голосование/отзыв голоса,
  закрытие опроса и список голосовавших; checklists с добавлением, отметками и
  редактированием пунктов; контакты и их импорт, dice, обычная и live location,
  inline bot results и callback buttons, mentions, read receipts и forum topics.
- **Web и ссылки:** web previews, Instant View, связанные статьи и внутренние
  якоря, исправление preview для поддержанных сервисов, открытие медиа и внешних
  URL через allowlist. Обрабатываются username/private-channel message deep links.
- **Связь и уведомления:** SOCKS5, HTTP, HTTPS и MTProto proxies, импорт proxy
  links, выбор транспорта, активация/direct, probe и latency, Secret Service для
  proxy secrets. Есть настройки уведомлений по умолчанию, для чатов и тем,
  исключения, freedesktop notifications с действиями open/read/mute/settings,
  список устройств, QR-подтверждение новых сессий и отзыв сессий.
- **Интерфейс:** адаптивный GTK/libadwaita layout, системная/светлая/тёмная тема,
  10 встроенных локалей, настройки акцента и отображения чатов/composer, локальные
  и Telegram wallpapers с dim/blur/intensity/rotation.
- **Обслуживание:** поиск, просмотр, очистка и экспорт sanitized logs, debug
  stats и crash report, импорт авторизационного ключа из TDLib в новую
  зашифрованную mono-rs session, проверка системных обновлений через PackageKit
  и запуск GNOME Software.

## Подтверждённые ограничения

- **Нет полного live parity.** Offline unit/integration tests используют mocks,
  fakes, временные файлы и локальные backend-двойники. Они не выполняют вход в
  Telegram и не подтверждают каждый сетевой сценарий. GUI, установленный RPM,
  Secret Service реального пользователя, PackageKit, GeoClue, notifications,
  MPRIS и аппаратные ключи требуют отдельных проверок.
- **Один аккаунт, без регистрации.** Переключение и одновременная работа
  нескольких аккаунтов не реализованы; `RegistrationRequired` отображается как
  неподдерживаемая регистрация нового аккаунта.
- **FIDO2 только USB HID.** Passkeys используют CTAP2/WebAuthn с ES256 через
  `authenticator`; платформенные internal, NFC, BLE/hybrid authenticators не
  подключены. Интерактивный PIN/UV flow не реализован (`pin: None`), поэтому
  ключи, которым требуется ввод PIN, могут вернуть unsupported. Аппаратный FIDO2
  и Telegram passkey login/register не проверены live.
- **VP9 alpha — статический fallback.** WebM с `codec-alpha` намеренно не
  проигрывается как анимация: показывается статический thumbnail/последний кадр.
  Обычный VP9 зависит от доступного `avdec_vp9` или `vp9dec`; TGS требует tlottie.
  Набор остальных аудио/видеоформатов зависит от установленных GStreamer plugins.
- **PackageKit не обновляет приложение сам.** Проверка только вызывает
  `GetUpdates` для системного пакета `monogram-desktop`; кнопка installer лишь
  запускает `gnome-software`. Локальный RPM не создаёт репозиторий или update
  metadata, поэтому наличие обновлений зависит от настроенных системных repos.
- **Deep links и notifications неполные.** Invite/share deep links сейчас
  возвращают unsupported; inline reply в freedesktop notifications не реализован.
- **TDLib import переносит только авторизацию.** История, медиа и настройки TDLib
  не импортируются. Импорт разрешён только в отсутствующий или пустой session
  destination и не меняет исходную TDLib database.

## Зависимости и Rust

Команды рассчитаны на Fedora 44, выполняются из корня репозитория и не меняют
Android-модули или содержимое `vendor/`. Системные пакеты устанавливайте только
с согласия владельца машины:

```sh
sudo dnf install git curl ca-certificates tar gzip findutils util-linux \
  gcc gcc-c++ make cmake pkgconf-pkg-config python3 \
  gtk4-devel libadwaita-devel libsecret-devel sqlite-devel \
  gstreamer1-devel gstreamer1-plugins-base-devel gstreamer1-plugins-good \
  systemd-devel rpm-build redhat-rpm-config desktop-file-utils appstream \
  /usr/bin/kwin_wayland /usr/bin/dbus-run-session

git submodule update --init --recursive

curl --proto '=https' --tlsv1.2 --fail --silent --show-error \
  https://sh.rustup.rs -o /tmp/monogram-rustup-init.sh
sh /tmp/monogram-rustup-init.sh -y --no-modify-path --profile minimal \
  --default-toolchain stable --component rustfmt --component clippy
. "$HOME/.cargo/env"
rustup toolchain install stable --profile minimal --component rustfmt --component clippy
rustup default stable
rustc --version
```

Требуется Rust не ниже `rust-version` в `desktop/Cargo.toml` (сейчас 1.99;
у локальной зависимости `native/markup-rs` — 1.98). Если установленный stable
старше этого требования, сборка должна завершиться ошибкой: MSRV не понижайте.
Системная версия Rust из Fedora не заменяет выбранную rustup toolchain.

Runtime-пакеты: `gtk4`, `libadwaita`, `gstreamer1`,
`gstreamer1-plugins-base`, `gstreamer1-plugins-good`, `libsecret`, `sqlite-libs`.
Для Secret Service нужен разблокированный GNOME Keyring или совместимый
провайдер KWallet в пользовательской D-Bus-сессии. Конкретные кодеки зависят
от установленных GStreamer plugins; RPM не обещает воспроизведение всех форматов.
PackageKit/GNOME Software, GeoClue, notification server и MPRIS используют
соответствующие системные сервисы, если они доступны.

## Проверки и локальная сборка

```sh
cd desktop
cargo fmt --package monogram-desktop -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
bash ../packaging/fedora/run-headless-tests.sh \
  cargo test --workspace --all-targets --locked
python3 tools/test_localization.py
cargo build --release --locked
cd ..

python3 desktop/tools/convert_locales.py --check
desktop-file-validate packaging/fedora/io.github.monogram.desktop.desktop
appstreamcli validate --no-net --strict \
  packaging/fedora/io.github.monogram.desktop.metainfo.xml
git diff --check
```

Package-scoped rustfmt намеренно проверяет только desktop crate: `--all` также
обходит локальные path dependencies из `vendor/` и не должен использоваться для
их автоматического переформатирования. Cargo проверяет только модули, объявленные
в дереве crate. Наличие нового `.rs` рядом с `main.rs` или `ui/mod.rs` само по
себе не означает, что этот код был скомпилирован или протестирован.
`run-headless-tests.sh` создаёт одноразовые HOME/XDG-каталоги, отдельную
D-Bus-сессию и виртуальный KWin Wayland output, не используя основной desktop.
`test_localization.py` дополнительно проверяет точное совпадение generated Fluent
resources и 19 300 offline rendering vectors; это не live Telegram test.

Для ручного запуска локального бинарника в графической сессии:

```sh
./desktop/target/release/monogram-desktop
```

Запускайте GUI-проверки с отдельными XDG-каталогами и изолированной
D-Bus/Secret Service-сессией, не с реальными учётными записями. GTK использует
доступный Wayland/X11 backend. Сам RPM не настраивает display server или
пользовательскую D-Bus-сессию.

## Telegram API credentials: Secret Service и SECRETS

Получайте собственные API ID и API hash только через
<https://my.telegram.org/apps> по правилам
<https://core.telegram.org/api/obtaining_api_id>. Не используйте идентификаторы
Android-сборки как встроенные desktop-секреты.

Обычный путь — ввести credentials локально в начальном UI. Положительный API ID
и 32-символьный hex API hash сохраняются отдельными записями Secret Service под
service `io.github.monogram.desktop`. Случайный 32-байтовый ключ шифрования
mono-rs session также создаётся, перечитывается для проверки и хранится в Secret
Service. Proxy credentials хранятся там же. При недоступном или заблокированном
провайдере приложение возвращает ошибку и **не переходит на plaintext**.

Для контролируемой разработки и offline automation поддерживается переменная
`SECRETS`. Если её значение содержит `API_HASH=`, оно разбирается как содержимое
файла; иначе значение считается путём к файлу. Предпочитайте путь к отдельному
файлу с режимом `0600`, чтобы не помещать секрет в environment командной строки:

```sh
# Только синтаксический пример, не действующие credentials.
install -m 600 /dev/null /tmp/monogram-api.env
# Запишите в /tmp/monogram-api.env локально:
# API_ID=123456
# API_HASH=00000000000000000000000000000000
SECRETS=/tmp/monogram-api.env ./desktop/target/release/monogram-desktop
```

Поддерживаются необязательный `export ` и одинарные/двойные кавычки. Дубликаты
`API_ID`/`API_HASH` отклоняются. `SECRETS` имеет приоритет над сохранёнными API
credentials и не импортирует их в Secret Service, поэтому переменная нужна при
каждом таком запуске. Ключ шифрования сессии всё равно требует Secret Service.

Не передавайте credentials, телефон, код, пароль 2FA, ключи сессии или содержимое
сообщений в отчёт об ошибке, CLI-аргументы, GitHub Actions secrets для этих
offline-проверок или shell history. Не включайте их в исходники, `Cargo.toml`,
`.desktop`, spec или RPM. API hash не является ни паролем 2FA, ни ключом сессии.

## Сборка RPM без установки

Следующие команды используют пользовательский rustup из предыдущего раздела.
`--without system_rust` отключает только RPM BuildRequires для системных пакетов
`rust`/`cargo`; проверку MSRV выполняет сам Cargo. Все остальные BuildRequires
проверяются обычным `rpmbuild`. Для сборки системным Rust не ниже MSRV установите
`rust` и `cargo` через DNF и уберите `--without system_rust`.

```sh
export PATH="$HOME/.cargo/bin:$PATH"
rpm_topdir=$(mktemp -d "${TMPDIR:-/tmp}/monogram-rpmbuild.XXXXXXXX")
bash packaging/fedora/prepare-sources.sh "$rpm_topdir"
rpmbuild -ba --without system_rust \
  --define "_topdir $rpm_topdir" --define '_smp_build_ncpus 2' \
  "$rpm_topdir/SPECS/monogram-desktop.spec"

rpm -qpl "$rpm_topdir"/RPMS/*/monogram-desktop-*.rpm
rpm -qp --requires "$rpm_topdir"/RPMS/*/monogram-desktop-*.rpm
rpm -qp --qf '%{NAME} %{VERSION}-%{RELEASE} %{LICENSE}\n' \
  "$rpm_topdir"/RPMS/*/monogram-desktop-*.rpm
```

`prepare-sources.sh` читает текущий checkout, включая незакоммиченные изменения
в desktop, а не `git archive HEAD`. `desktop/Cargo.lock` обязателен. Скрипт
создаёт исходный архив и отдельный архив crates.io-зависимостей, полученных
`cargo vendor --locked`; загрузка зависимостей возможна только на этом шаге.
В `%build` и `%check` используются `--frozen` и архивированные зависимости, поэтому
сам rpmbuild не должен требовать сетевого доступа. `%check` запускает Cargo tests
через тот же isolated Wayland/D-Bus wrapper и повторно валидирует установленные
в buildroot метаданные. Скрипт ничего не записывает в `vendor/`.

Результаты остаются в `$rpm_topdir/RPMS` и `$rpm_topdir/SRPMS`. Эти команды
**не устанавливают RPM**, не запускают клиент и не обращаются к Telegram. Для
распространения сохраняйте вместе бинарный RPM и соответствующий SRPM. Это
локальная упаковка с vendored Cargo-зависимостями, не утверждённый пакет
официального репозитория Fedora; установка и воспроизводимая сборка пакета
должны проверяться отдельно до любого заявления о релизе.

RPM устанавливает только бинарник, `.desktop`, SVG-иконку с буквой M (без
официального логотипа Telegram), AppStream metadata, GPL-3.0, тексты лицензий
и notices vendored-зависимостей и этот README. Идентификатор приложения,
desktop-файла и иконки: `io.github.monogram.desktop`. Кэши меню и иконок
обслуживаются штатными Fedora file triggers; специальных скриптов установки или
доступа к пользовательским данным в пакете нет.

CI `.github/workflows/desktop.yml` повторяет format, Clippy, Cargo tests в
изолированной Wayland/D-Bus-сессии и release build в контейнере `fedora:44`,
валидирует metadata и собирает бинарный RPM/SRPM. CI не запускает отдельный
Python localization suite, не устанавливает RPM, не выполняет ручную GUI-проверку,
не проверяет аппаратные ключи и не выполняет live-вход.
