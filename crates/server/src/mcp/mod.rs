mod protocol;

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};

use lsp_skill_core::LspManager;

use protocol::*;

// ── State & types ────────────────────────────────────────────────────────────

#[derive(Clone)]
struct McpState {
    manager: Arc<LspManager>,
    server_name: &'static str,
    server_instructions: &'static str,
    tools: &'static [ToolDef],
}

struct ToolDef {
    name: &'static str,
    description: &'static str,
    input_schema: fn() -> Value,
}

pub enum McpKind {
    Diagnostics,
    Navigation,
}

// ── Router ───────────────────────────────────────────────────────────────────

pub fn router(manager: Arc<LspManager>, kind: McpKind) -> Router {
    let (server_name, server_instructions, tools) = match kind {
        McpKind::Diagnostics => (
            "1c-lsp-diagnostics",
            "MCP-сервер статического анализа кода 1С:Предприятие 8 (BSL). \
             Использует bsl-language-server для проверки .bsl файлов на синтаксические ошибки, \
             предупреждения и замечания анализатора кода. \
             Передавайте file_path относительно корня проекта (project_root_path). \
             Заголовок x-project-id обязателен в каждом запросе. \
             Если проект ещё индексируется (warming_up), результаты могут быть неполными. \
             Если проект не готов, инструмент вернёт ошибку с описанием состояния.",
            DIAGNOSTICS_TOOLS,
        ),
        McpKind::Navigation => (
            "1c-lsp-navigation",
            "MCP-сервер семантической навигации по коду 1С:Предприятие 8 (BSL). \
             Использует bsl-language-server для навигации по символам: получение структуры модуля \
             (symbols), переход к определению (definition), поиск всех ссылок (references), \
             входящие вызовы (incoming_calls), исходящие вызовы (outgoing_calls), \
             поиск символов по проекту (workspace_symbols). \
             Координаты line и character — zero-based (начиная с 0). \
             Передавайте file_path относительно корня проекта (project_root_path). \
             Заголовок x-project-id обязателен в каждом запросе. \
             Предпочитайте эти инструменты текстовому поиску (grep) для надёжной навигации по коду 1С.",
            NAVIGATION_TOOLS,
        ),
    };

    let state = McpState {
        manager,
        server_name,
        server_instructions,
        tools,
    };

    Router::new()
        .route(
            "/mcp",
            post(handle_post).get(handle_get).delete(handle_delete),
        )
        .with_state(state)
}

// ── Tool definitions ─────────────────────────────────────────────────────────

static DIAGNOSTICS_TOOLS: &[ToolDef] = &[ToolDef {
    name: "diagnostics",
    description: "Выполняет статический анализ файла 1С (BSL) через bsl-language-server \
                  и возвращает список диагностик: синтаксические ошибки, предупреждения, \
                  замечания анализатора кода. Результат — массив LSP Diagnostic с полями \
                  range, severity (1=Error, 2=Warning, 3=Information, 4=Hint), message, source, code. \
                  Используйте для проверки кода до и после редактирования, а также для объяснения ошибок. \
                  Первый запрос к файлу может занять больше времени (файл открывается в LSP-сессии). \
                  Пустой результат не гарантирует отсутствие ошибок — возможно, индексация ещё идёт.",
    input_schema: || {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Путь к .bsl файлу относительно корня проекта (project_root_path). \
                                    Не используйте абсолютные пути. Сохраняйте кириллицу в именах каталогов. \
                                    Пример: 1c-src/Configuration/CommonModules/ОбщийМодуль1/Module.bsl"
                }
            },
            "required": ["file_path"]
        })
    },
}];

static FILE_PATH_DESC: &str = "Путь к .bsl файлу относительно корня проекта (project_root_path). \
    Не используйте абсолютные пути. Сохраняйте кириллицу в именах каталогов. \
    Пример: 1c-src/Configuration/CommonModules/ОбщийМодуль1/Module.bsl";
static LINE_DESC: &str = "Номер строки в файле (zero-based, начиная с 0). Первая строка файла = 0";
static CHARACTER_DESC: &str =
    "Позиция символа в строке (zero-based, начиная с 0). Первый символ строки = 0. \
     Проверяйте точность — ошибка на ±1 приводит к пустому или неверному результату";

static NAVIGATION_TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "symbols",
        description: "Возвращает структуру модуля 1С (BSL): список процедур, функций, \
                      переменных и областей (regions) с их позициями и иерархией. \
                      Результат — DocumentSymbol[] (с hierarchy и children) или SymbolInformation[]. \
                      Используйте для понимания структуры незнакомого модуля перед редактированием. \
                      Работает с общими модулями, модулями объектов, менеджеров, форм и команд.",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": FILE_PATH_DESC
                    }
                },
                "required": ["file_path"]
            })
        },
    },
    ToolDef {
        name: "definition",
        description: "Находит определение (объявление) символа в указанной позиции файла 1С (BSL). \
                      Возвращает Location, Location[] или LocationLink[] с файлом и позицией объявления. \
                      Используйте для перехода от вызова процедуры/функции к её реализации, \
                      в том числе в другие модули. Если результат null — символ не распознан \
                      или позиция указана неточно.",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": FILE_PATH_DESC
                    },
                    "line": {
                        "type": "integer",
                        "description": LINE_DESC
                    },
                    "character": {
                        "type": "integer",
                        "description": CHARACTER_DESC
                    }
                },
                "required": ["file_path", "line", "character"]
            })
        },
    },
    ToolDef {
        name: "references",
        description: "Находит все места использования (ссылки) символа в указанной позиции \
                      по всему проекту 1С. Возвращает массив Location[] со всеми файлами и позициями, \
                      где символ вызывается или упоминается. Включает само объявление в результат \
                      (includeDeclaration: true). Используйте перед изменением или удалением \
                      процедуры/функции для оценки области влияния. Предпочитайте этот инструмент \
                      текстовому поиску (grep) для надёжного определения зависимостей.",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": FILE_PATH_DESC
                    },
                    "line": {
                        "type": "integer",
                        "description": LINE_DESC
                    },
                    "character": {
                        "type": "integer",
                        "description": CHARACTER_DESC
                    }
                },
                "required": ["file_path", "line", "character"]
            })
        },
    },
    ToolDef {
        name: "workspace_symbols",
        description: "Ищет символы (процедуры, функции, переменные) по всему проекту 1С \
                      по текстовому запросу. Возвращает массив SymbolInformation[] с полями \
                      name, kind, containerName, location. Используйте когда знаете имя символа, \
                      но не знаете в каком файле он определён. Предпочитайте точные имена \
                      или уникальные фрагменты. Пустой запрос вернёт все символы проекта \
                      (избегайте для больших проектов).",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Текстовый запрос для поиска символов. Используйте точное имя \
                                        процедуры/функции или его часть. \
                                        Пример: ПолучитьФункциональнуюОпцию"
                    }
                },
                "required": ["query"]
            })
        },
    },
    ToolDef {
        name: "incoming_calls",
        description: "Находит все места, откуда вызывается процедура или функция в указанной позиции. \
                      Сначала подготавливает элемент call hierarchy через prepareCallHierarchy, \
                      затем возвращает IncomingCall[] с caller и ranges. Если позиция не указывает \
                      на процедуру или функцию, результат будет null.",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": FILE_PATH_DESC
                    },
                    "line": {
                        "type": "integer",
                        "description": LINE_DESC
                    },
                    "character": {
                        "type": "integer",
                        "description": CHARACTER_DESC
                    }
                },
                "required": ["file_path", "line", "character"]
            })
        },
    },
    ToolDef {
        name: "outgoing_calls",
        description: "Находит все процедуры и функции, которые вызываются из процедуры или функции \
                      в указанной позиции. Сначала подготавливает элемент call hierarchy через \
                      prepareCallHierarchy, затем возвращает OutgoingCall[] с callee и ranges. \
                      Если позиция не указывает на процедуру или функцию, результат будет null.",
        input_schema: || {
            json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": FILE_PATH_DESC
                    },
                    "line": {
                        "type": "integer",
                        "description": LINE_DESC
                    },
                    "character": {
                        "type": "integer",
                        "description": CHARACTER_DESC
                    }
                },
                "required": ["file_path", "line", "character"]
            })
        },
    },
];

// ── Handlers ─────────────────────────────────────────────────────────────────

async fn handle_post(
    State(state): State<McpState>,
    headers: HeaderMap,
    Json(request): Json<JsonRpcRequest>,
) -> Response {
    // Notifications (no id) — acknowledge without body
    if request.id.is_none() {
        return StatusCode::ACCEPTED.into_response();
    }

    let id = request.id.unwrap();
    let project_id = headers
        .get("x-project-id")
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    let response = match request.method.as_str() {
        "initialize" => handle_initialize(state.server_name, state.server_instructions, id),
        "ping" => JsonRpcResponse::success(id, json!({})),
        "tools/list" => handle_tools_list(state.tools, id),
        "tools/call" => {
            handle_tools_call(&state.manager, state.tools, id, request.params, project_id).await
        }
        other => JsonRpcResponse::error(id, METHOD_NOT_FOUND, format!("Method not found: {other}")),
    };

    Json(response).into_response()
}

/// GET /mcp — SSE stream for server-initiated notifications (not used yet).
async fn handle_get() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}

/// DELETE /mcp — session termination (stateless, always OK).
async fn handle_delete() -> StatusCode {
    StatusCode::OK
}

// ── MCP method implementations ───────────────────────────────────────────────

fn handle_initialize(server_name: &str, server_instructions: &str, id: Value) -> JsonRpcResponse {
    JsonRpcResponse::success(
        id,
        json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": server_name,
                "version": env!("CARGO_PKG_VERSION")
            },
            "instructions": server_instructions
        }),
    )
}

fn handle_tools_list(tools: &[ToolDef], id: Value) -> JsonRpcResponse {
    let list: Vec<Value> = tools
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": (t.input_schema)()
            })
        })
        .collect();

    JsonRpcResponse::success(id, json!({ "tools": list }))
}

async fn handle_tools_call(
    manager: &LspManager,
    tools: &[ToolDef],
    id: Value,
    params: Option<Value>,
    project_id: Option<String>,
) -> JsonRpcResponse {
    let Some(project_id) = project_id else {
        return JsonRpcResponse::error(
            id,
            INVALID_REQUEST,
            "Заголовок x-project-id обязателен".to_string(),
        );
    };

    let params = params.unwrap_or(Value::Null);
    let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);

    // Check tool exists in this server
    if !tools.iter().any(|t| t.name == tool_name) {
        return tool_error(id, format!("Неизвестный инструмент: {tool_name}"));
    }

    let result = match tool_name {
        "diagnostics" => call_diagnostics(manager, &project_id, &arguments).await,
        "symbols" => call_symbols(manager, &project_id, &arguments).await,
        "definition" => call_definition(manager, &project_id, &arguments).await,
        "references" => call_references(manager, &project_id, &arguments).await,
        "incoming_calls" => call_incoming_calls(manager, &project_id, &arguments).await,
        "outgoing_calls" => call_outgoing_calls(manager, &project_id, &arguments).await,
        "workspace_symbols" => call_workspace_symbols(manager, &project_id, &arguments).await,
        _ => Err(format!("Неизвестный инструмент: {tool_name}")),
    };

    match result {
        Ok(value) => {
            let text = serde_json::to_string_pretty(&value).unwrap_or_default();
            JsonRpcResponse::success(
                id,
                json!({
                    "content": [{ "type": "text", "text": text }]
                }),
            )
        }
        Err(err) => tool_error(id, err),
    }
}

fn tool_error(id: Value, message: String) -> JsonRpcResponse {
    JsonRpcResponse::success(
        id,
        json!({
            "content": [{ "type": "text", "text": message }],
            "isError": true
        }),
    )
}

// ── Tool call implementations ────────────────────────────────────────────────

fn extract_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("Параметр {key} обязателен"))
}

fn extract_u32(args: &Value, key: &str) -> Result<u32, String> {
    args.get(key)
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .ok_or_else(|| format!("Параметр {key} обязателен"))
}

async fn call_diagnostics(
    manager: &LspManager,
    project_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let file_path = extract_str(args, "file_path")?;
    manager
        .diagnostics(project_id, file_path)
        .await
        .map_err(|e| e.to_string())
}

async fn call_symbols(
    manager: &LspManager,
    project_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let file_path = extract_str(args, "file_path")?;
    manager
        .symbols(project_id, file_path)
        .await
        .map_err(|e| e.to_string())
}

async fn call_definition(
    manager: &LspManager,
    project_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let file_path = extract_str(args, "file_path")?;
    let line = extract_u32(args, "line")?;
    let character = extract_u32(args, "character")?;
    manager
        .definition(project_id, file_path, line, character)
        .await
        .map_err(|e| e.to_string())
}

async fn call_references(
    manager: &LspManager,
    project_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let file_path = extract_str(args, "file_path")?;
    let line = extract_u32(args, "line")?;
    let character = extract_u32(args, "character")?;
    manager
        .references(project_id, file_path, line, character)
        .await
        .map_err(|e| e.to_string())
}

async fn call_workspace_symbols(
    manager: &LspManager,
    project_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let query = extract_str(args, "query")?;
    manager
        .workspace_symbols(project_id, query)
        .await
        .map_err(|e| e.to_string())
}

async fn call_incoming_calls(
    manager: &LspManager,
    project_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let file_path = extract_str(args, "file_path")?;
    let line = extract_u32(args, "line")?;
    let character = extract_u32(args, "character")?;
    manager
        .incoming_calls(project_id, file_path, line, character)
        .await
        .map_err(|e| e.to_string())
}

async fn call_outgoing_calls(
    manager: &LspManager,
    project_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let file_path = extract_str(args, "file_path")?;
    let line = extract_u32(args, "line")?;
    let character = extract_u32(args, "character")?;
    manager
        .outgoing_calls(project_id, file_path, line, character)
        .await
        .map_err(|e| e.to_string())
}
