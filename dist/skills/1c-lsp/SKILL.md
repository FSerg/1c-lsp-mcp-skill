---
name: 1c-lsp
description: "Use when developing, reviewing, or debugging 1C:Enterprise 8.3 / BSL code in a repository connected to the local `lsp-skill` service and you need semantic navigation through `bsl-language-server`. Covers `symbols`, `definition`, `references`, `incoming-calls`, `outgoing-calls`, and `workspace-symbols` for configured 1C projects. Prefer this skill over plain text search when you need symbol-aware answers about procedures, functions, variables, common modules, forms, object modules, and cross-file usage in 1C (`1С`) projects."
---

# 1c-lsp

Use the local `lsp-skill` CLI as the primary semantic navigation tool while working on 1C code.

Treat it as a thin wrapper over `bsl-language-server` and LSP. Use it first for symbol-aware navigation, then open the real source files to inspect surrounding code before editing behavior.

## How It Connects

The CLI discovers `PROJECT_ID` from a `.env` file in the current directory or a parent directory, and discovers the server address from `runtime.json`.

If the project is not ready (starting, stopped, error), commands will return an error — handle it when it happens instead of checking upfront. You can run `lsp-skill status` to inspect project state if needed for troubleshooting.

## Use Safe Coordinates And Paths

Each project has two root paths:

- **`root_path`** — the BSL root where `bsl-language-server` indexes code (e.g. `d:\projects\myproject\1c-src`).
- **`project_root_path`** — the root of the entire project where LLM agents and CLI run from (e.g. `d:\projects\myproject`).

Pass `file_path` **relative to `project_root_path`**. The server resolves the path and verifies it falls inside `root_path` automatically.

- Do not use absolute paths. The server rejects absolute or escaping paths.
- Preserve the exact directory names, including Cyrillic segments.
- Convert editor coordinates carefully: LSP `line` and `character` are zero-based.
- Re-check the exact symbol position before calling `definition` or `references`. Off-by-one mistakes are common when converting from editors or code review comments.

Example:

```bash
lsp-skill symbols "1c-src/Configuration/CommonModules/ОбщийМодуль1/Module.bsl"
```

## Choose The Right Command

### `symbols <file_path>`

Use to understand the structure of the current module before editing it.

Expect either:

- `DocumentSymbol[]` with hierarchy and `children`, or
- `SymbolInformation[]` without hierarchy.

Use it to map procedures, functions, module variables, and logical regions in:

- common modules;
- object modules;
- manager modules;
- form modules;
- command modules.

Prefer this over manual scrolling when entering an unfamiliar module.

### `definition <file_path> --line N --col M`

Use when you have a symbol usage and need the declaration or implementation target.

Expect one of the standard LSP result shapes:

- `Location`;
- `Location[]`;
- `LocationLink[]`;
- `null`.

This client advertises `definition.linkSupport = true`, so `LocationLink[]` is a realistic result shape. When several targets are returned, inspect all of them and choose by module type and surrounding code.

Use this before changing a called procedure or function whose origin is not obvious from the file alone.

### `references <file_path> --line N --col M`

Use before changing behavior, deleting code, or evaluating blast radius.

This wrapper sends `includeDeclaration: true`, so the declaration itself may appear in the result set. Deduplicate same-file duplicates before summarizing impact.

Do not use raw text search as a substitute when you need reliable call sites or variable usages.

### `incoming-calls <file_path> --line N --col M`

Use when you need to know who calls the procedure or function at the given position.

This wrapper first sends `prepareCallHierarchy` and then requests `callHierarchy/incomingCalls`. It uses only the first prepared item. For BSL that is a reasonable assumption because function overloading is not used in the language model exposed by the server.

Expect:

- `IncomingCall[]`, or
- `null`.

Each item contains the calling symbol (`from`) and the source ranges (`fromRanges`) where the call happens.

Use this before changing a public or widely reused procedure to assess upstream impact.

### `outgoing-calls <file_path> --line N --col M`

Use when you need to know what the current procedure or function calls.

This wrapper first sends `prepareCallHierarchy` and then requests `callHierarchy/outgoingCalls`. It uses only the first prepared item.

Expect:

- `OutgoingCall[]`, or
- `null`.

Each item contains the called symbol (`to`) and the ranges (`fromRanges`) where the outgoing call occurs.

Use this to understand dependencies inside an unfamiliar procedure before changing its behavior.

### `workspace-symbols <query>`

Use when you know the symbol name or part of it, but not the file.

Expect project-wide symbol results with fields such as:

- `name`;
- `kind`;
- `containerName`;
- `location`.

LSP 3.17 allows an empty query to request all symbols, but avoid that unless the project is small or the user explicitly asks for exhaustive enumeration.

Do not assume strict regex semantics just because the local CLI help mentions regex. The LSP spec defines `query` as a search string interpreted in a relaxed way, and server behavior may differ. Prefer exact symbol names or distinctive fragments first.

## Use Recommended Workflows

### Understand An Unfamiliar Module

1. Run `lsp-skill symbols <file>`.
2. Run `definition` on the key calls you do not recognize.
3. Run `incoming-calls` and `outgoing-calls` on the main procedure or function you plan to change.
4. Run `references` when you need the full usage list rather than only the call hierarchy.
5. Open the returned files and inspect real code before editing.

### Estimate Change Impact

1. Find the declaration with `definition` or `workspace-symbols`.
2. Run `incoming-calls` for direct callers and `references` for the broader usage list.
3. Run `outgoing-calls` if internal dependencies also matter.
4. Group usages by module and scenario.
5. Only then describe the likely impact of the change.

## Apply Interpretation Rules

- Treat LSP output as semantic evidence, not as a full business-logic proof.
- Verify non-trivial conclusions in source code before changing behavior.
- Treat missing results as ambiguous, not final. Common causes: indexing still in progress, wrong zero-based coordinates, wrong relative path, unresolved dynamic usage, or unsupported server behavior.
- Prefer semantic navigation over `grep` when both are available.
- Use plain text search only as a secondary check or to inspect context around already-found symbols.
- Cite exact returned files and positions when summarizing findings.

## Respect Scope Limits

`bsl-language-server` supports many LSP capabilities, but this local wrapper currently exposes in this skill only:

- `status`;
- `symbols`;
- `references`;
- `definition`;
- `incoming-calls`;
- `outgoing-calls`;
- `workspace-symbols`.

Do not claim this skill can perform diagnostics, rename, hover, completion, formatting, code actions, or automatic refactoring unless those operations are added to the local wrapper or covered by another skill.

## Troubleshoot Carefully

- If a command returns an error about project state, run `lsp-skill status` to see `status.status`, `status.error`, and `progress`.
  - `warming_up`: results may be partial — warn the user.
  - `error`: stop and report the message from `status.error`.
  - `starting` or `stopped`: the project is not yet available.
- If the CLI cannot connect, `lsp-skill-server` is unreachable.
- If a file is reported missing, re-check that the path is relative to `project_root_path` and that the resolved file falls inside the BSL root (`root_path`).
- If all requests fail, suspect server availability, invalid `PROJECT_ID`, wrong configured project root, missing `bsl-language-server` JAR, or Java runtime problems.
- Remember that `bsl-language-server` requires Java 17+.

## Use Concrete Examples

```bash
lsp-skill status
lsp-skill symbols "1c-src/Configuration/Documents/Заказ/Forms/Форма/Module.bsl"
lsp-skill definition "1c-src/Configuration/Documents/Заказ/Forms/Форма/Module.bsl" --line 119 --col 7
lsp-skill references "1c-src/Configuration/Documents/Заказ/Forms/Форма/Module.bsl" --line 119 --col 7
lsp-skill incoming-calls "1c-src/Configuration/Documents/Заказ/Forms/Форма/Module.bsl" --line 119 --col 7
lsp-skill outgoing-calls "1c-src/Configuration/Documents/Заказ/Forms/Форма/Module.bsl" --line 119 --col 7
lsp-skill workspace-symbols "ПолучитьФункциональнуюОпцию"
```
