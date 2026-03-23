---
name: 1c-check
description: "Use when developing, reviewing, or debugging 1C:Enterprise 8.3 / BSL code in a repository connected to the local `lsp-skill` service and you need syntax and static diagnostics through `bsl-language-server`. Covers `diagnostics` for configured 1C projects. Use this skill before edits, after edits, and when explaining parser or analyzer errors in 1C (`1С`) modules."
---

# 1c-check

Use the local `lsp-skill` CLI to inspect syntax and static diagnostics for 1C code.

Treat it as a thin wrapper over `bsl-language-server` and LSP. Use it before edits, after edits, and when explaining an existing syntax or analyzer problem.

## How It Connects

The CLI discovers `PROJECT_ID` from a `.env` file in the current directory or a parent directory, and discovers the server address from `runtime.json`.

If the project is not ready (starting, stopped, error), commands will return an error — handle it when it happens instead of checking upfront. You can run `lsp-skill status` to inspect project state if needed for troubleshooting.

## Use Safe Paths

Each project has two root paths:

- **`root_path`** — the BSL root where `bsl-language-server` indexes code (e.g. `d:\projects\myproject\1c-src`).
- **`project_root_path`** — the root of the entire project where LLM agents and CLI run from (e.g. `d:\projects\myproject`).

Pass `file_path` **relative to `project_root_path`**. The server resolves the path and verifies it falls inside `root_path` automatically.

- Do not use absolute paths. The server rejects absolute or escaping paths.
- Preserve the exact directory names, including Cyrillic segments.

Example:

```bash
lsp-skill diagnostics "1c-src/Configuration/CommonModules/ОбщийМодуль1/Module.bsl"
```

## Choose The Right Command

### `diagnostics <file_path>`

Use before edits, after edits, and when explaining an existing problem.

Expect a diagnostics payload with:

- `uri`;
- `diagnostics[]`;
- per-diagnostic fields such as `range`, `severity`, `message`, `source`, `code`, `tags`, `relatedInformation`.

The first request to a file opens it in the LSP session and then requests diagnostics through LSP pull diagnostics (`textDocument/diagnostic`). The response is returned immediately after the server answers.

## Use Recommended Workflows

### Check A File Before Or After Changes

1. Run `lsp-skill diagnostics <file>`.
2. Open the exact ranges mentioned by the diagnostics.
3. After edits, run `diagnostics` again and compare the result.

### Explain A Syntax Or Analyzer Error

1. Run `lsp-skill diagnostics <file>`.
2. Group diagnostics by severity and range.
3. Quote the exact `message`, `source`, `code`, and location when summarizing the issue.
## Apply Interpretation Rules

- Treat diagnostics as tool output, not as full proof that the file is correct.
- Treat missing diagnostics as ambiguous, not final. Common causes: indexing still in progress, wrong relative path, or server-side analysis limitations.
- Cite exact returned ranges and messages when summarizing findings.
- Re-run diagnostics after edits instead of assuming the issue is resolved.

## Respect Scope Limits

`bsl-language-server` supports many LSP capabilities, but this skill covers only:

- `status`;
- `diagnostics`.

Do not claim this skill can perform symbol navigation, references search, rename, hover, completion, formatting, code actions, call hierarchy, or automatic refactoring.

## Troubleshoot Carefully

- If a command returns an error about project state, run `lsp-skill status` to see `status.status`, `status.error`, and `progress`.
  - `warming_up`: diagnostics may be partial — warn the user.
  - `error`: stop and report the message from `status.error`.
  - `starting` or `stopped`: the project is not yet available.
- If the CLI cannot connect, `lsp-skill-server` is unreachable.
- If a file is reported missing, re-check that the path is relative to `project_root_path` and that the resolved file falls inside the BSL root (`root_path`).
- If all requests fail, suspect server availability, invalid `PROJECT_ID`, wrong configured project root, missing `bsl-language-server` JAR, or Java runtime problems.
- Remember that `bsl-language-server` requires Java 17+.

## Use Concrete Examples

```bash
lsp-skill status
lsp-skill diagnostics "1c-src/Configuration/Documents/Заказ/Forms/Форма/Module.bsl"
```
