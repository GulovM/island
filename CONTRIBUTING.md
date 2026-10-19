# Участие в разработке

## Сборка и проверка

```powershell
cargo build --release
cargo test
cargo clippy --release
```

Нужны Rust (stable, MSVC) и Windows SDK (для иконки и сведений о версии в exe). Программа рисует интерфейс сама
(Direct2D), поэтому визуальные изменения проверяйте на живом экране: запускайте копию с временным `APPDATA`,
чтобы не затронуть свой `config.toml`.

Обзор структуры кода и история решений — в [docs/PLAN.md](docs/PLAN.md).

## Правила

- Бюджеты программы: CPU в простое ≈ 0%, RAM ≤ 30 МБ, exe ≤ 2 МБ. Не добавляйте таймеры, которые будят процесс в простое.
- Новые зависимости — только при острой необходимости; сейчас используется только `windows-rs`.
- Комментарии и сообщения коммитов — на русском или английском, в стиле окружающего кода.
- Для логики без UI пишите модульные тесты (`cargo test`).

## Выпуск релиза

1. Поднимите версию в `Cargo.toml` и `island.rc` (`FILEVERSION`, `FileVersion`, `ProductVersion`), добавьте запись в `CHANGELOG.md`.
2. Закоммитьте и поставьте тег: `git tag v0.2.0 && git push --tags`.
3. GitHub Actions (`.github/workflows/release.yml`) соберёт установщик, portable-архив и `SHA256SUMS.txt` и приложит их к релизу.

Не копируйте в проект код, графику и другие ресурсы Hitodama и иных приложений — только собственная реализация
(см. [NOTICE.md](NOTICE.md)).

Локально то же самое делает `powershell -ExecutionPolicy Bypass -File tools\package.ps1`.
