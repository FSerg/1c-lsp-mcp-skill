# AGENTS-mcp.md — MCP tools for LLM agents

Instructions for LLM agents (IDE agents, Copilot, Cursor, etc.) that connect to `lsp-skill` MCP servers for 1C:Enterprise 8 (BSL) code analysis and navigation.

## Overview

`lsp-skill-server` exposes two MCP servers over HTTP (Streamable HTTP transport), each on its own port:

| Server | Default port | Purpose |
|---|---|---|
| **1c-lsp-diagnostics** | `9011` | Static analysis: syntax errors, warnings, code analyzer remarks |
| **1c-lsp-navigation** | `9012` | Semantic navigation: symbols, definition, references, call hierarchy, workspace search |

Both servers implement MCP protocol version `2025-03-26` and use JSON-RPC 2.0 over `POST /mcp`.

## Connection

- **Endpoint**: `http://<host>:<port>/mcp`
- **Required header**: `x-project-id` — the project identifier (every tool call requires it).
- The project and server must be pre-configured and running via `lsp-skill-server`.

## Path rules (all tools)

All `file_path` parameters must be **relative to `project_root_path`** (the project working directory, not the BSL source root).

- Do not pass absolute paths — the server rejects them.
- Preserve exact directory names, including Cyrillic segments.
- The server automatically resolves and validates that the file is inside the BSL `root_path`.

Example: `1c-src/Configuration/CommonModules/ОбщийМодуль1/Module.bsl`

## Coordinate rules (definition, references, incoming_calls, outgoing_calls)

LSP coordinates are **zero-based**:
- `line`: first line = 0
- `character`: first character on a line = 0

Verify the exact position before calling `definition`, `references`, `incoming_calls`, or `outgoing_calls` — an off-by-one error leads to empty or incorrect results.

## Server: 1c-lsp-diagnostics (port 9011)

Static analysis of 1C (BSL) code through `bsl-language-server`.

### Tool: `diagnostics`

Analyzes a BSL file and returns an array of LSP Diagnostic objects.

**When to use**: before edits (baseline), after edits (verify fix), when explaining errors to the user.

**Parameters**:

| Name | Type | Required | Description |
|---|---|---|---|
| `file_path` | string | yes | Path to .bsl file relative to project_root_path |

**Response**: array of diagnostics, each with:
- `range` — start/end positions in the file
- `severity` — 1=Error, 2=Warning, 3=Information, 4=Hint
- `message` — human-readable description
- `source` — analyzer that produced the diagnostic (e.g. `bsl-language-server`)
- `code` — diagnostic rule identifier
- `tags`, `relatedInformation` — additional context

**Notes**:
- First request to a file takes longer (opens it in the LSP session).
- Empty result does not guarantee the file is clean — indexing may be in progress.
- During `warming_up` state, diagnostics may be partial.

## Server: 1c-lsp-navigation (port 9012)

Semantic navigation across 1C (BSL) code through `bsl-language-server`. Prefer these tools over text search (grep) for reliable code navigation. Besides symbols, definitions, and references, the server also exposes direct call hierarchy for procedures and functions.

### Tool: `symbols`

Returns the structure of a BSL module: procedures, functions, variables, and regions with their positions and hierarchy.

**When to use**: to understand an unfamiliar module before editing, to get a list of all procedures/functions in a file.

**Parameters**:

| Name | Type | Required | Description |
|---|---|---|---|
| `file_path` | string | yes | Path to .bsl file relative to project_root_path |

**Response**: `DocumentSymbol[]` (with hierarchy and `children`) or `SymbolInformation[]`. Each symbol has `name`, `kind`, `range`, `selectionRange`.

Works with: common modules, object modules, manager modules, form modules, command modules.

### Tool: `definition`

Finds the declaration/definition of a symbol at the given position, including cross-module navigation.

**When to use**: to jump from a procedure/function call to its implementation, to find where a variable is declared.

**Parameters**:

| Name | Type | Required | Description |
|---|---|---|---|
| `file_path` | string | yes | Path to .bsl file relative to project_root_path |
| `line` | integer | yes | Line number (zero-based, first line = 0) |
| `character` | integer | yes | Character position in line (zero-based, first char = 0) |

**Response**: `Location`, `Location[]`, `LocationLink[]`, or `null`. Each location has `uri` and `range`. `null` means the symbol was not recognized or the position is imprecise.

### Tool: `references`

Finds all usages (references) of a symbol at the given position across the entire project.

**When to use**: before changing or deleting a procedure/function to assess blast radius, to find all call sites.

**Parameters**:

| Name | Type | Required | Description |
|---|---|---|---|
| `file_path` | string | yes | Path to .bsl file relative to project_root_path |
| `line` | integer | yes | Line number (zero-based, first line = 0) |
| `character` | integer | yes | Character position in line (zero-based, first char = 0) |

**Response**: `Location[]` — all files and positions where the symbol is called or mentioned. Includes the declaration itself (`includeDeclaration: true`).

**Prefer this over text search (grep)** for reliable dependency analysis in 1C code.

### Tool: `incoming_calls`

Finds all direct callers of the procedure or function at the given position.

**When to use**: before changing a procedure/function to understand who calls it.

**Parameters**:

| Name | Type | Required | Description |
|---|---|---|---|
| `file_path` | string | yes | Path to .bsl file relative to project_root_path |
| `line` | integer | yes | Line number (zero-based, first line = 0) |
| `character` | integer | yes | Character position in line (zero-based, first char = 0) |

**Response**: `CallHierarchyIncomingCall[]` or `null`. The server first prepares the call hierarchy item and then returns direct callers with `from` and `fromRanges`.

### Tool: `outgoing_calls`

Finds all direct callees of the procedure or function at the given position.

**When to use**: to understand what the current procedure/function depends on before editing it.

**Parameters**:

| Name | Type | Required | Description |
|---|---|---|---|
| `file_path` | string | yes | Path to .bsl file relative to project_root_path |
| `line` | integer | yes | Line number (zero-based, first line = 0) |
| `character` | integer | yes | Character position in line (zero-based, first char = 0) |

**Response**: `CallHierarchyOutgoingCall[]` or `null`. The server first prepares the call hierarchy item and then returns direct callees with `to` and `fromRanges`.

### Tool: `workspace_symbols`

Searches for symbols (procedures, functions, variables) across the entire project by text query.

**When to use**: when you know the symbol name (or part of it) but not which file it's in.

**Parameters**:

| Name | Type | Required | Description |
|---|---|---|---|
| `query` | string | yes | Text query — exact name or distinctive fragment. Example: `ПолучитьФункциональнуюОпцию` |

**Response**: `SymbolInformation[]` with `name`, `kind`, `containerName`, `location`.

Prefer exact names or distinctive fragments. Avoid empty queries on large projects.

## Interpretation rules

- Treat results as semantic evidence, not as full business-logic proof.
- Treat missing results as ambiguous, not final. Common causes: indexing in progress, wrong coordinates, wrong relative path, dynamic dispatch.
- Cite exact returned ranges, files, and messages when summarizing findings.
- Re-run diagnostics after edits — do not assume the issue is resolved.
- Prefer semantic navigation (`symbols`, `definition`, `references`, `incoming_calls`, `outgoing_calls`) over text search when available.

## Error handling

If a tool returns an error:
- **Project not ready** (`starting`, `stopped`, `error`, `warming_up`) — the project is still initializing or has a problem. Do not retry immediately; inform the user.
- **File not found** — verify the path is relative to `project_root_path` and the file exists inside `root_path`.
- **Missing `x-project-id`** — the header is required for every `tools/call` request.
- **Connection refused** — `lsp-skill-server` is not running.
