# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`lsp-skill` — менеджер BSL Language Server для 1С проектов. Предоставляет HTTP API + SSE для управления несколькими проектами, каждый из которых имеет свой экземпляр bsl-language-server (Java процесс). Работает как консольный HTTP сервер на всех платформах (Linux, macOS, Windows).

## Commands

### Build

```bash
# Сборка фронтенда (нужна перед cargo build)
./scripts/build-frontend.sh

# Сборка сервера и CLI
cargo build -p lsp-skill-server -p lsp-skill-cli

# Релизная сборка (текущая платформа)
./scripts/release.sh

# Релизная сборка (конкретный target)
./scripts/release.sh x86_64-unknown-linux-gnu
./scripts/release.sh x86_64-pc-windows-gnu
./scripts/release.sh aarch64-apple-darwin x86_64-apple-darwin
```

### Development

```bash
# Запуск сервера (UI доступен на http://127.0.0.1:4000)
cargo run -p lsp-skill-server

# Проверка компиляции без сборки
cargo check

# Проверка фронтенда (WASM таргет)
cargo check -p lsp-skill-frontend --target wasm32-unknown-unknown
```

### Tests

```bash
# Все тесты
cargo test

# Тесты конкретного крейта
cargo test -p lsp-skill-core

# Конкретный тест
cargo test --lib db::tests::crud_roundtrip_works
```

### Lint & Format

```bash
cargo clippy
cargo fmt --check
cargo fmt
```

## Architecture

Четыре крейта в workspace:

```
crates/
├── core/      — ядро: конфиг, БД (SQLite), LspManager, LSP клиент (JSON-RPC)
├── server/    — HTTP сервер (Axum), SSE, управление службой
├── frontend/  — Dioxus/WASM UI, собирается отдельно через scripts/build-frontend.sh
└── cli/       — CLI клиент, обращается к серверу через HTTP
```

### Поток данных

1. **Сервер** при старте читает `config.toml`, открывает `data.db` (SQLite), восстанавливает проекты.
2. **LspManager** (`crates/core/src/manager.rs`) — центральный компонент. Управляет жизненным циклом проектов: создание/старт/стоп, запуск Java процесса, обмен JSON-RPC 2.0 через stdio.
3. **LSP транспорт** (`crates/core/src/lsp/`) — `transport.rs` читает/пишет LSP Content-Length frames, `client.rs` отправляет запросы и ждёт ответов.
4. **SSE** (`GET /api/events`) — фронтенд и CLI подписываются на события: `ProjectStatusChanged`, `IndexingProgress`, `DiagnosticsUpdated`, `LogLine`, и др. (см. `crates/core/src/events.rs`).
5. **Frontend** компилируется в WASM, встраивается в бинарник сервера через `build.rs`. При разработке `build.rs` ищет артефакты в `target/frontend-dist/` или `static/`.

### API Endpoints

- `GET/PUT /api/settings` — конфигурация
- `POST /api/settings/check-java` — проверка Java 17+
- `GET/POST/PUT/DELETE /api/projects` — CRUD проектов
- `POST /api/projects/{id}/start|stop` — управление
- `POST /api/projects/{id}/diagnostics|symbols|references|definition|incoming-calls|outgoing-calls|workspace-symbols` — LSP-запросы
- `GET /api/projects/{id}/logs` — логи проекта
- `GET /api/events` — SSE стрим
- `POST /api/browse` — файловый браузер

### MCP серверы

Два отдельных MCP-сервера (Streamable HTTP, JSON-RPC 2.0 через `POST /mcp`), каждый на своём порту:

- **1c-lsp-diagnostics** (порт `9011`) — инструмент `diagnostics`
- **1c-lsp-navigation** (порт `9012`) — инструменты `symbols`, `definition`, `references`, `incoming_calls`, `outgoing_calls`, `workspace_symbols`

Включаются через `config.toml` (`mcp_diagnostics_enabled`, `mcp_navigation_enabled`) или Web UI. Реализация: `crates/server/src/mcp/`.

### Конфигурация и пути

Конфиг хранится через `directories::ProjectDirs` с qualifier `("ru", "Infostart", "LspSkill")`:
- `config.toml` — `jar_path`, `listen_host` (default: `0.0.0.0`), `http_port` (default: `4000`), `log_level`
- `data.db` — SQLite база с проектами
- `runtime.json` — текущий хост/порт запущенного сервера (CLI его читает для подключения)
- `logs/` — ротируемые логи (10MB файл, 100MB всего)

### Сервер (lsp-skill-server)

Консольное приложение с подкомандами:
- Без аргументов — запуск HTTP сервера в foreground
- `service install` — установить и запустить как службу (systemd на Linux, launchd на macOS, sc.exe на Windows)
- `service uninstall` — остановить и удалить службу
- `service status` — показать статус службы

### CLI (lsp-skill)

CLI находит `PROJECT_ID` из `.env` файла (поиск вверх по дереву каталогов), подключается к серверу через `runtime.json`.

Команды: `status`, `diagnostics <file>`, `symbols <file>`, `references <file> --line N --col N`, `definition <file> --line N --col N`, `incoming-calls <file> --line N --col N`, `outgoing-calls <file> --line N --col N`, `workspace-symbols <query>`, `install-path`

## Project Structure

```
scripts/
├── build-frontend.sh  — сборка WASM-фронтенда (Dioxus + Tailwind)
└── release.sh         — релизная сборка бандла (принимает target как аргумент)

dist/                  — файлы, включаемые в релизный бандл (помимо бинарников)
├── .env.example       — пример .env для CLI
├── AGENTS-skills.md   — инструкции для LLM-агентов по CLI-инструментам
├── AGENTS-mcp.md      — инструкции для LLM-агентов по MCP-инструментам
├── README.html        — инструкция по быстрому запуску
├── pics/              — картинки для README.html
└── skills/             — скилы (навыки) для LLM-агентов
    ├── 1c-check/SKILL.md  — диагностика
    └── 1c-lsp/SKILL.md    — навигация
.github/workflows/
├── ci.yml             — CI: fmt, clippy, test, WASM check (на push/PR в main)
└── release.yml        — Release: сборка бандлов на 4 target'а (на тег v*)
```

## Frontend Build Details

`scripts/build-frontend.sh`:
1. Устанавливает `dioxus-cli v0.6.3` в `.tools/dx-0.6.3` (если не установлен)
2. `npm ci` + `npm run build:css` (Tailwind + DaisyUI)
3. `dx build --platform web --release`
4. Копирует в `target/frontend-dist/`
5. Встраивает CSS в `index.html`

При изменении только Rust-кода сервера/ядра пересборка фронтенда не нужна.

## Platform Notes

- Все платформы: консольный HTTP сервер, управление через CLI или Web UI в браузере.
- Linux: служба через systemd (user unit, без root).
- macOS: служба через launchd (user agent, без root).
- Windows: служба через sc.exe (требует права администратора).
- WASM: фронтенд компилируется для `wasm32-unknown-unknown`.
