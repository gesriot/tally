# Tally

Десктопный планировщик дел для Windows, Linux и macOS на Rust и Slint. Модель и поведение – по мотивам Things 3: «Входящие», «Сегодня» с блоком «Вечер», «Планы», «В любое время», «Когда-нибудь», «Журнал», области, проекты с разделами, теги, чек-листы, сроки. Данные хранятся локально в SQLite.

## Возможности

- Списки «Входящие», «Сегодня» (с «Вечером»), «Планы» по дням, «В любое время», «Когда-нибудь», «Журнал» и «Корзина».
- Области, проекты с разделами и кольцом прогресса, теги (в том числе вложенные), чек-листы, заметки, срок и дата начала.
- Быстрый ввод с разбором дат по-русски: «Позвонить завтра», «Отчёт в пятницу вечером», «Поездка 15 дек».
- Перетаскивание задач внутри списка, между разделами и в боковую панель.
- Повторяющиеся задачи и проекты – по календарю и после выполнения; напоминания на время.
- Отмена и повтор (`Ctrl+Z`, `Ctrl+Shift+Z`) с общим для окна и консоли журналом.
- Быстрый поиск по названиям, заметкам и тегам, фильтр по тегам.
- Трей, глобальная горячая клавиша быстрого ввода, один экземпляр окна на базу.
- Светлая и тёмная темы, импорт и экспорт JSON, суточные копии базы.
- Консоль `tally` с той же базой: `add`, `list`, `done`, `move`, `search`, `undo`, `redo`, `quick-add`.

## Сборка и запуск

```sh
cargo run -p tally-ui                    # окно (рендерер Skia)
cargo run -p tally-cli -- add Позвонить маме завтра
cargo run -p tally-cli -- list           # «Сегодня»
```

Первая сборка с рендерером Skia долгая: Skia скачивает готовые бинарники, без сети она собирается из исходников. Запасной рендерер – femtovg: `cargo run -p tally-ui --no-default-features --features femtovg`.

Выпускной `tally-ui.exe` для Windows со значком (в `dist/windows`; нужен Windows SDK с `rc.exe`): `scripts/package-windows.ps1`. В Windows окно открывается без консоли, поэтому сообщения об ошибках запуска (например, неверный `--db`) в терминале не видны – для диагностики есть консольная программа `tally`.

В Linux для сборки нужны `libfontconfig1-dev`, `libfreetype6-dev`, `libxkbcommon-dev`, `libwayland-dev`, `libx11-dev`, `libxcursor-dev` и `libgl1-mesa-dev`.

Демо-база с примерами (из корня репозитория):

```sh
cargo build -p tally-cli -p tally-ui
scripts/seed-demo.sh /tmp/demo.db
TALLY_TODAY=2026-11-20 target/debug/tally-ui --db /tmp/demo.db
```

По умолчанию база – `tally.db` в папке данных пользователя (Windows – `%APPDATA%\Tally\data`, macOS – `~/Library/Application Support/Tally`, Linux – `~/.local/share/tally`); `--db <ПУТЬ>` и переменная `TALLY_DB` задают свою.

## Документация

- [Руководство по окну](docs/user-guide.md) – что умеет Tally и горячие клавиши.
- [Консоль `tally`](crates/tally-cli/README.md) – команды, отмена, коды выхода.
- [Памятка быстрого ввода](crates/tally-when/quick-entry-ru.md) – какие даты понимает Tally.
- [Архитектура](docs/architecture.md) – крейты, модель данных, журнал действий.
- [Заметки о Slint](docs/slint-notes.md) – что проверено при выборе библиотеки интерфейса.

## Устройство репозитория

```
crates/        крейты workspace (tally-core, tally-rank, tally-when, tally-store,
               tally-app, tally-client, tally-cli, tally-ui)
assets/        шрифт Inter (SIL OFL 1.1), значок приложения
docs/          руководство, архитектура
scripts/       демо-база, значки, сборка exe для Windows, проверка быстрого ввода
```

Подробнее о крейтах – в [архитектуре](docs/architecture.md).

## Разработка

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI (`.github/workflows/ci.yml`) делает то же на Linux, Windows и macOS и проверяет сборку с рендерером femtovg. Тесты окна идут на тестовой платформе Slint без экрана.

## Лицензия

Код – [MIT](LICENSE). Шрифт Inter – SIL Open Font License 1.1 ([`assets/fonts/Inter-LICENSE.txt`](assets/fonts/Inter-LICENSE.txt)).
