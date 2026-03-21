# AGENTS-skills.md — CLI tools for LLM agents

Instructions for LLM agents (Claude Code, Copilot, Cursor, etc.) that use the `lsp-skill` CLI to work with 1C:Enterprise 8 (BSL) code.

## What is lsp-skill

`lsp-skill` is a CLI client to `lsp-skill-server` — a local service that manages `bsl-language-server` (Java) instances for 1C projects. The CLI provides semantic code analysis and navigation powered by LSP, far more reliable than text search (grep) for 1C/BSL code.

## How it connects

The CLI auto-discovers the project:

- **`PROJECT_ID`** — read from a `.env` file found in the current directory or any parent directory.
- **Server address** — read from `runtime.json` written by the running `lsp-skill-server`.

No manual configuration is needed if both are in place.

## Path rules

Each project has two root paths:

- **`root_path`** — the BSL root where `bsl-language-server` indexes code (e.g. `1c-src`).
- **`project_root_path`** — the root of the entire project (working directory for agents).

**Always pass `file_path` relative to `project_root_path`**. The server resolves the path and verifies it falls inside `root_path` automatically.

- Do not use absolute paths — the server rejects them.
- Preserve exact directory names, including Cyrillic segments.

Example: `1c-src/Configuration/CommonModules/ОбщийМодуль1/Module.bsl`

## Coordinate rules

LSP coordinates (`line`, `character` / `--line`, `--col`) are **zero-based**:

- First line of the file = 0
- First character on a line = 0

Double-check position before calling `definition` or `references` — off-by-one errors lead to empty or wrong results.

## Commands

### `lsp-skill status`

Shows project state. **Not required before other commands** — use only for troubleshooting when a command returns an unexpected error.

Returns: `root_path`, `project_root_path`, `status.status`, `status.error`, `progress`.

Project states:
- `ready` — all commands work normally.
- `warming_up` — commands work but results may be incomplete (indexing in progress).
- `starting` / `stopped` — project not yet available, commands will return errors.
- `error` — check `status.error` for details.

### `lsp-skill diagnostics <file_path>`

Runs static analysis on a BSL file. Returns LSP diagnostics: syntax errors, warnings, code analyzer remarks.

**When to use**: before edits (baseline), after edits (verify fix), when explaining errors.

Output fields per diagnostic: `range`, `severity` (1=Error, 2=Warning, 3=Information, 4=Hint), `message`, `source`, `code`, `tags`, `relatedInformation`.

Notes:
- First request to a file takes longer (opens it in the LSP session).
- Empty result with `_note` means "no diagnostics received in time" — not proof the file is clean.
- During `warming_up`, diagnostics may be partial.

```bash
lsp-skill diagnostics "1c-src/Configuration/Documents/Заказ/Forms/Форма/Module.bsl"
```

### `lsp-skill symbols <file_path>`

Returns the structure of a BSL module: procedures, functions, variables, and regions with positions and hierarchy.

**When to use**: to understand an unfamiliar module before editing.

Result: `DocumentSymbol[]` (with hierarchy and `children`) or `SymbolInformation[]`.

Works with: common modules, object modules, manager modules, form modules, command modules.

```bash
lsp-skill symbols "1c-src/Configuration/CommonModules/ОбщийМодуль1/Module.bsl"
```

### `lsp-skill definition <file_path> --line N --col M`

Finds the declaration/definition of a symbol at the given position.

**When to use**: to jump from a procedure/function call to its implementation, including cross-module navigation.

Result: `Location`, `Location[]`, `LocationLink[]`, or `null` (symbol not recognized or position imprecise).

```bash
lsp-skill definition "1c-src/Configuration/Documents/Заказ/Forms/Форма/Module.bsl" --line 119 --col 7
```

### `lsp-skill references <file_path> --line N --col M`

Finds all usages (references) of a symbol at the given position across the entire project.

**When to use**: before changing or deleting a procedure/function, to assess blast radius.

Result: `Location[]` with all files and positions where the symbol is called or mentioned. Includes the declaration itself (`includeDeclaration: true`).

**Prefer this over grep** for reliable dependency analysis in 1C code.

```bash
lsp-skill references "1c-src/Configuration/Documents/Заказ/Forms/Форма/Module.bsl" --line 119 --col 7
```

### `lsp-skill workspace-symbols <query>`

Searches for symbols (procedures, functions, variables) across the entire project by text query.

**When to use**: when you know the symbol name but not which file it's in.

Result: `SymbolInformation[]` with `name`, `kind`, `containerName`, `location`.

Prefer exact names or distinctive fragments. Avoid empty queries on large projects.

```bash
lsp-skill workspace-symbols "ПолучитьФункциональнуюОпцию"
```

## Interpretation rules

- Treat results as semantic evidence, not as full business-logic proof.
- Treat missing results as ambiguous, not final. Common causes: indexing in progress, wrong coordinates, wrong relative path, dynamic dispatch.
- Cite exact returned ranges and messages when summarizing findings.
- Re-run diagnostics after edits — do not assume the issue is resolved.
- Prefer semantic navigation (`symbols`, `definition`, `references`) over `grep` when available.
- Use text search only as a secondary check or to inspect context around found symbols.

## Troubleshooting

- If a command returns an error about project state → run `lsp-skill status` to diagnose.
- If a file is reported missing → verify the path is relative to `project_root_path` and falls inside `root_path`.
- If all requests fail → check server availability, `PROJECT_ID` in `.env`, `runtime.json`, JAR path, Java 17+.
- If the CLI cannot connect → `lsp-skill-server` is not running or unreachable.
