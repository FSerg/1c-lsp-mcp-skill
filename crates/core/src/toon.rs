use serde_json::{Map, Value};
use std::collections::BTreeSet;

pub fn format_response(root_name: &str, value: &Value, use_toon: bool) -> String {
    if !use_toon {
        return serde_json::to_string_pretty(value).unwrap_or_default();
    }

    let flattened = flatten_lsp(value.clone());
    let wrapped = if matches!(flattened, Value::Array(_)) {
        let mut obj = Map::new();
        obj.insert(root_name.to_string(), flattened);
        Value::Object(obj)
    } else {
        flattened
    };

    to_toon(&wrapped)
}

fn flatten_lsp(value: Value) -> Value {
    match value {
        Value::Object(map) => flatten_object(map),
        Value::Array(items) => {
            let processed: Vec<Value> = items.into_iter().map(flatten_lsp).collect();
            Value::Array(normalize_array_keys(processed))
        }
        other => other,
    }
}

fn flatten_object(map: Map<String, Value>) -> Value {
    let mut flattened = Map::new();
    for (key, value) in map {
        let snake_key = snake_case(&key);
        if snake_key == "code_description" {
            continue;
        }
        let flat_value = flatten_lsp(value);
        if snake_key == "tags" {
            if let Value::Array(items) = &flat_value {
                if items.is_empty() {
                    continue;
                }
            }
        }
        flattened.insert(snake_key, flat_value);
    }

    if let Some((sl, sc, el, ec)) = extract_raw_range(&flattened) {
        return Value::Object(make_short_range(sl, sc, el, ec));
    }

    if let Some(location) = extract_raw_location(&flattened) {
        return Value::Object(make_location_map(
            location.uri,
            location.sl,
            location.sc,
            location.el,
            location.ec,
        ));
    }

    loop {
        let before: BTreeSet<String> = flattened.keys().cloned().collect();
        flattened = apply_r1(flattened);
        flattened = apply_r2(flattened);
        flattened = apply_r3(flattened);
        let after: BTreeSet<String> = flattened.keys().cloned().collect();
        if before == after {
            break;
        }
    }

    Value::Object(flattened)
}

fn apply_r1(map: Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, value) in map {
        if let Some((sl, sc, el, ec)) = extract_short_range(&value) {
            let prefix = snake_case(&key);
            out.insert(format!("{prefix}_sl"), Value::from(sl));
            out.insert(format!("{prefix}_sc"), Value::from(sc));
            out.insert(format!("{prefix}_el"), Value::from(el));
            out.insert(format!("{prefix}_ec"), Value::from(ec));
        } else {
            out.insert(key, value);
        }
    }
    out
}

fn apply_r2(map: Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, value) in map {
        if let Some(location) = extract_location(&value) {
            let prefix = snake_case(&key);
            out.insert(format!("{prefix}_uri"), Value::String(location.uri));
            out.insert(format!("{prefix}_sl"), Value::from(location.sl));
            out.insert(format!("{prefix}_sc"), Value::from(location.sc));
            out.insert(format!("{prefix}_el"), Value::from(location.el));
            out.insert(format!("{prefix}_ec"), Value::from(location.ec));
        } else {
            out.insert(key, value);
        }
    }
    out
}

fn apply_r3(map: Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, value) in map {
        if (key == "from" || key == "to") && value.is_object() {
            if let Value::Object(sub) = value {
                for (sub_key, sub_value) in sub {
                    out.insert(format!("{key}_{sub_key}"), sub_value);
                }
            }
        } else {
            out.insert(key, value);
        }
    }
    out
}

struct ExtractedLocation {
    uri: String,
    sl: u64,
    sc: u64,
    el: u64,
    ec: u64,
}

fn extract_raw_range(map: &Map<String, Value>) -> Option<(u64, u64, u64, u64)> {
    if map.len() != 2 {
        return None;
    }

    let start = map.get("start")?.as_object()?;
    let end = map.get("end")?.as_object()?;
    if start.len() != 2 || end.len() != 2 {
        return None;
    }

    Some((
        start.get("line")?.as_u64()?,
        start.get("character")?.as_u64()?,
        end.get("line")?.as_u64()?,
        end.get("character")?.as_u64()?,
    ))
}

fn extract_short_range(value: &Value) -> Option<(u64, u64, u64, u64)> {
    let map = value.as_object()?;
    if map.len() != 4 {
        return None;
    }

    Some((
        map.get("sl")?.as_u64()?,
        map.get("sc")?.as_u64()?,
        map.get("el")?.as_u64()?,
        map.get("ec")?.as_u64()?,
    ))
}

fn extract_raw_location(map: &Map<String, Value>) -> Option<ExtractedLocation> {
    if map.len() != 2 {
        return None;
    }

    let uri = map.get("uri")?.as_str()?.to_string();
    let range = map.get("range")?;
    let (sl, sc, el, ec) = extract_short_range(range)?;

    Some(ExtractedLocation {
        uri,
        sl,
        sc,
        el,
        ec,
    })
}

fn extract_location(value: &Value) -> Option<ExtractedLocation> {
    let map = value.as_object()?;
    if map.len() != 5 {
        return None;
    }

    Some(ExtractedLocation {
        uri: map.get("uri")?.as_str()?.to_string(),
        sl: map.get("range_sl")?.as_u64()?,
        sc: map.get("range_sc")?.as_u64()?,
        el: map.get("range_el")?.as_u64()?,
        ec: map.get("range_ec")?.as_u64()?,
    })
}

fn make_short_range(sl: u64, sc: u64, el: u64, ec: u64) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert("sl".to_string(), Value::from(sl));
    map.insert("sc".to_string(), Value::from(sc));
    map.insert("el".to_string(), Value::from(el));
    map.insert("ec".to_string(), Value::from(ec));
    map
}

fn make_location_map(uri: String, sl: u64, sc: u64, el: u64, ec: u64) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert("uri".to_string(), Value::String(uri));
    map.insert("range_sl".to_string(), Value::from(sl));
    map.insert("range_sc".to_string(), Value::from(sc));
    map.insert("range_el".to_string(), Value::from(el));
    map.insert("range_ec".to_string(), Value::from(ec));
    map
}

fn snake_case(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 4);
    for (index, ch) in value.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if index > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn normalize_array_keys(items: Vec<Value>) -> Vec<Value> {
    if items.is_empty() || !items.iter().all(Value::is_object) {
        return items;
    }

    let lsp_detected = items.iter().any(|item| {
        item.as_object().is_some_and(|map| {
            map.keys().any(|key| {
                matches!(key.as_str(), "sl" | "sc" | "el" | "ec")
                    || key.ends_with("_sl")
                    || key.ends_with("_sc")
                    || key.ends_with("_el")
                    || key.ends_with("_ec")
                    || key.ends_with("_uri")
                    || key.starts_with("from_")
                    || key.starts_with("to_")
            })
        })
    });
    if !lsp_detected {
        return items;
    }

    let mut key_union = BTreeSet::new();
    for item in &items {
        if let Some(map) = item.as_object() {
            for key in map.keys() {
                key_union.insert(key.clone());
            }
        }
    }

    items
        .into_iter()
        .map(|item| match item {
            Value::Object(mut map) => {
                for key in &key_union {
                    map.entry(key.clone()).or_insert(Value::Null);
                }
                Value::Object(map)
            }
            other => other,
        })
        .collect()
}

fn to_toon(value: &Value) -> String {
    let mut out = String::new();
    emit_value(value, 0, &mut out);
    out.trim_end().to_string()
}

const INDENT: &str = "  ";

fn emit_value(value: &Value, depth: usize, out: &mut String) {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            push_indent(depth, out);
            out.push_str(&emit_primitive(value));
            out.push('\n');
        }
        Value::Object(map) => emit_object(map, depth, out),
        Value::Array(items) => emit_array(None, items, depth, out),
    }
}

fn emit_object(map: &Map<String, Value>, depth: usize, out: &mut String) {
    for (key, value) in map {
        match value {
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
                push_indent(depth, out);
                out.push_str(key);
                out.push_str(": ");
                out.push_str(&emit_primitive(value));
                out.push('\n');
            }
            Value::Object(sub) => {
                push_indent(depth, out);
                out.push_str(key);
                out.push_str(":\n");
                emit_object(sub, depth + 1, out);
            }
            Value::Array(items) => emit_array(Some(key.as_str()), items, depth, out),
        }
    }
}

fn emit_array(name: Option<&str>, items: &[Value], depth: usize, out: &mut String) {
    let name = name.unwrap_or("items");

    if items.is_empty() {
        push_indent(depth, out);
        out.push_str(name);
        out.push_str("[0]:\n");
        return;
    }

    if let Some(headers) = table_headers(items) {
        push_indent(depth, out);
        out.push_str(name);
        out.push('[');
        out.push_str(&items.len().to_string());
        out.push_str("]{");
        out.push_str(&headers.join(","));
        out.push_str("}:\n");

        for item in items {
            let object = item.as_object().expect("table rows must be objects");
            push_indent(depth + 1, out);
            let row: Vec<String> = headers
                .iter()
                .map(|header| emit_primitive(object.get(header).unwrap_or(&Value::Null)))
                .collect();
            out.push_str(&row.join(","));
            out.push('\n');
        }
        return;
    }

    if items.iter().all(is_primitive) {
        let values: Vec<String> = items.iter().map(emit_primitive).collect();
        let inline = format!("[{}]", values.join(","));
        push_indent(depth, out);
        out.push_str(name);
        if values.len() <= 8 && inline.len() <= 80 {
            out.push_str(": ");
            out.push_str(&inline);
            out.push('\n');
            return;
        }

        out.push_str(":\n");
        for value in values {
            push_indent(depth + 1, out);
            out.push_str("- ");
            out.push_str(&value);
            out.push('\n');
        }
        return;
    }

    push_indent(depth, out);
    out.push_str(name);
    out.push_str(":\n");
    for item in items {
        emit_list_item(item, depth + 1, out);
    }
}

fn emit_list_item(value: &Value, depth: usize, out: &mut String) {
    match value {
        Value::Object(map) => emit_object_item(map, depth, out),
        Value::Array(items) => {
            push_indent(depth, out);
            out.push_str("-\n");
            emit_array(Some("items"), items, depth + 1, out);
        }
        primitive => {
            push_indent(depth, out);
            out.push_str("- ");
            out.push_str(&emit_primitive(primitive));
            out.push('\n');
        }
    }
}

fn emit_object_item(map: &Map<String, Value>, depth: usize, out: &mut String) {
    let mut iter = map.iter();
    let Some((first_key, first_value)) = iter.next() else {
        push_indent(depth, out);
        out.push_str("- {}\n");
        return;
    };

    emit_bulleted_key_value(first_key, first_value, depth, out);

    for (key, value) in iter {
        emit_key_value(key, value, depth, out, false);
    }
}

fn emit_key_value(key: &str, value: &Value, depth: usize, out: &mut String, first: bool) {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            if !first {
                push_indent(depth + 1, out);
            }
            out.push_str(key);
            out.push_str(": ");
            out.push_str(&emit_primitive(value));
            out.push('\n');
        }
        Value::Object(map) => {
            if !first {
                push_indent(depth + 1, out);
            }
            out.push_str(key);
            out.push_str(":\n");
            emit_object(map, depth + 2, out);
        }
        Value::Array(items) => emit_array(Some(key), items, depth + 1, out),
    }
}

fn emit_bulleted_key_value(key: &str, value: &Value, depth: usize, out: &mut String) {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            push_indent(depth, out);
            out.push_str("- ");
            out.push_str(key);
            out.push_str(": ");
            out.push_str(&emit_primitive(value));
            out.push('\n');
        }
        Value::Object(map) => {
            push_indent(depth, out);
            out.push_str("- ");
            out.push_str(key);
            out.push_str(":\n");
            emit_object(map, depth + 2, out);
        }
        Value::Array(items) => emit_bulleted_array(key, items, depth, out),
    }
}

fn emit_bulleted_array(name: &str, items: &[Value], depth: usize, out: &mut String) {
    if items.is_empty() {
        push_indent(depth, out);
        out.push_str("- ");
        out.push_str(name);
        out.push_str("[0]:\n");
        return;
    }

    if let Some(headers) = table_headers(items) {
        push_indent(depth, out);
        out.push_str("- ");
        out.push_str(name);
        out.push('[');
        out.push_str(&items.len().to_string());
        out.push_str("]{");
        out.push_str(&headers.join(","));
        out.push_str("}:\n");

        for item in items {
            let object = item.as_object().expect("table rows must be objects");
            push_indent(depth + 1, out);
            let row: Vec<String> = headers
                .iter()
                .map(|header| emit_primitive(object.get(header).unwrap_or(&Value::Null)))
                .collect();
            out.push_str(&row.join(","));
            out.push('\n');
        }
        return;
    }

    if items.iter().all(is_primitive) {
        let values: Vec<String> = items.iter().map(emit_primitive).collect();
        let inline = format!("[{}]", values.join(","));
        push_indent(depth, out);
        out.push_str("- ");
        out.push_str(name);
        if values.len() <= 8 && inline.len() <= 80 {
            out.push_str(": ");
            out.push_str(&inline);
            out.push('\n');
            return;
        }

        out.push_str(":\n");
        for value in values {
            push_indent(depth + 1, out);
            out.push_str("- ");
            out.push_str(&value);
            out.push('\n');
        }
        return;
    }

    push_indent(depth, out);
    out.push_str("- ");
    out.push_str(name);
    out.push_str(":\n");
    for item in items {
        emit_list_item(item, depth + 1, out);
    }
}

fn push_indent(depth: usize, out: &mut String) {
    for _ in 0..depth {
        out.push_str(INDENT);
    }
}

fn is_primitive(value: &Value) -> bool {
    matches!(
        value,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
    )
}

fn table_headers(items: &[Value]) -> Option<Vec<String>> {
    if items.is_empty() || !items.iter().all(Value::is_object) {
        return None;
    }

    let first_keys: BTreeSet<String> = items[0]
        .as_object()
        .expect("checked above")
        .keys()
        .cloned()
        .collect();

    for item in &items[1..] {
        let keys: BTreeSet<String> = item
            .as_object()
            .expect("checked above")
            .keys()
            .cloned()
            .collect();
        if keys != first_keys {
            return None;
        }
    }

    for item in items {
        for value in item.as_object().expect("checked above").values() {
            if !is_primitive(value) {
                return None;
            }
        }
    }

    Some(first_keys.into_iter().collect())
}

fn emit_primitive(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(boolean) => boolean.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => quote_string_if_needed(text),
        _ => unreachable!("emit_primitive called on non-primitive"),
    }
}

fn quote_string_if_needed(value: &str) -> String {
    let needs_quotes = value.is_empty()
        || value.starts_with(' ')
        || value.ends_with(' ')
        || value.chars().any(|ch| {
            matches!(
                ch,
                ',' | ':' | '[' | ']' | '{' | '}' | '#' | '"' | '\n' | '\t' | '\r'
            )
        })
        || looks_like_number(value)
        || looks_like_keyword(value);

    if !needs_quotes {
        return value.to_string();
    }

    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn looks_like_number(value: &str) -> bool {
    value.parse::<f64>().is_ok()
}

fn looks_like_keyword(value: &str) -> bool {
    matches!(value, "true" | "false" | "null")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn flattens_range_in_diagnostic() {
        let value = json!({
            "range": {
                "start": { "line": 10, "character": 4 },
                "end": { "line": 10, "character": 20 }
            },
            "message": "x"
        });

        assert_eq!(
            flatten_lsp(value),
            json!({
                "range_sl": 10,
                "range_sc": 4,
                "range_el": 10,
                "range_ec": 20,
                "message": "x"
            })
        );
    }

    #[test]
    fn flattens_selection_range_snake_case() {
        let value = json!({
            "selectionRange": {
                "start": { "line": 1, "character": 2 },
                "end": { "line": 3, "character": 4 }
            }
        });

        assert_eq!(
            flatten_lsp(value),
            json!({
                "selection_range_sl": 1,
                "selection_range_sc": 2,
                "selection_range_el": 3,
                "selection_range_ec": 4
            })
        );
    }

    #[test]
    fn flattens_location_to_five_fields() {
        let value = json!({
            "location": {
                "uri": "file:///a.bsl",
                "range": {
                    "start": { "line": 1, "character": 0 },
                    "end": { "line": 1, "character": 5 }
                }
            }
        });

        assert_eq!(
            flatten_lsp(value),
            json!({
                "location_uri": "file:///a.bsl",
                "location_sl": 1,
                "location_sc": 0,
                "location_el": 1,
                "location_ec": 5
            })
        );
    }

    #[test]
    fn flattens_location_link_origin_optional() {
        let value = json!([
            {
                "originSelectionRange": {
                    "start": { "line": 0, "character": 0 },
                    "end": { "line": 0, "character": 4 }
                },
                "targetUri": "file:///x.bsl",
                "targetRange": {
                    "start": { "line": 10, "character": 0 },
                    "end": { "line": 20, "character": 0 }
                },
                "targetSelectionRange": {
                    "start": { "line": 10, "character": 0 },
                    "end": { "line": 10, "character": 8 }
                }
            },
            {
                "targetUri": "file:///y.bsl",
                "targetRange": {
                    "start": { "line": 5, "character": 0 },
                    "end": { "line": 15, "character": 0 }
                },
                "targetSelectionRange": {
                    "start": { "line": 5, "character": 0 },
                    "end": { "line": 5, "character": 6 }
                }
            }
        ]);

        assert_eq!(
            flatten_lsp(value),
            json!([
                {
                    "origin_selection_range_sl": 0,
                    "origin_selection_range_sc": 0,
                    "origin_selection_range_el": 0,
                    "origin_selection_range_ec": 4,
                    "target_uri": "file:///x.bsl",
                    "target_range_sl": 10,
                    "target_range_sc": 0,
                    "target_range_el": 20,
                    "target_range_ec": 0,
                    "target_selection_range_sl": 10,
                    "target_selection_range_sc": 0,
                    "target_selection_range_el": 10,
                    "target_selection_range_ec": 8
                },
                {
                    "origin_selection_range_sl": null,
                    "origin_selection_range_sc": null,
                    "origin_selection_range_el": null,
                    "origin_selection_range_ec": null,
                    "target_uri": "file:///y.bsl",
                    "target_range_sl": 5,
                    "target_range_sc": 0,
                    "target_range_el": 15,
                    "target_range_ec": 0,
                    "target_selection_range_sl": 5,
                    "target_selection_range_sc": 0,
                    "target_selection_range_el": 5,
                    "target_selection_range_ec": 6
                }
            ])
        );
    }

    #[test]
    fn inlines_from_with_prefix() {
        let value = json!({
            "from": {
                "name": "Main",
                "uri": "file:///Module.bsl",
                "range": {
                    "start": { "line": 0, "character": 0 },
                    "end": { "line": 30, "character": 0 }
                }
            }
        });

        assert_eq!(
            flatten_lsp(value),
            json!({
                "from_name": "Main",
                "from_uri": "file:///Module.bsl",
                "from_range_sl": 0,
                "from_range_sc": 0,
                "from_range_el": 30,
                "from_range_ec": 0
            })
        );
    }

    #[test]
    fn from_ranges_stays_as_array_with_flattened_elements() {
        let value = json!({
            "fromRanges": [
                {
                    "start": { "line": 5, "character": 8 },
                    "end": { "line": 5, "character": 20 }
                }
            ]
        });

        assert_eq!(
            flatten_lsp(value),
            json!({
                "from_ranges": [
                    { "sl": 5, "sc": 8, "el": 5, "ec": 20 }
                ]
            })
        );
    }

    #[test]
    fn normalizes_missing_keys_with_null() {
        let value = json!([
            {
                "containerName": "One",
                "location": {
                    "uri": "file:///a.bsl",
                    "range": {
                        "start": { "line": 1, "character": 0 },
                        "end": { "line": 2, "character": 0 }
                    }
                }
            },
            {
                "location": {
                    "uri": "file:///b.bsl",
                    "range": {
                        "start": { "line": 3, "character": 0 },
                        "end": { "line": 4, "character": 0 }
                    }
                }
            }
        ]);

        assert_eq!(
            flatten_lsp(value),
            json!([
                {
                    "container_name": "One",
                    "location_uri": "file:///a.bsl",
                    "location_sl": 1,
                    "location_sc": 0,
                    "location_el": 2,
                    "location_ec": 0
                },
                {
                    "container_name": null,
                    "location_uri": "file:///b.bsl",
                    "location_sl": 3,
                    "location_sc": 0,
                    "location_el": 4,
                    "location_ec": 0
                }
            ])
        );
    }

    #[test]
    fn emits_primitives() {
        assert_eq!(to_toon(&json!("hello")), "hello");
        assert_eq!(to_toon(&json!(42)), "42");
        assert_eq!(to_toon(&Value::Null), "null");
    }

    #[test]
    fn emits_nested_object() {
        assert_eq!(to_toon(&json!({"a": {"b": 1}})), "a:\n  b: 1");
    }

    #[test]
    fn emits_tabular_array() {
        assert_eq!(
            to_toon(&json!({"items": [{"b": "x", "a": 1}, {"a": 2, "b": "y"}]})),
            "items[2]{a,b}:\n  1,x\n  2,y"
        );
    }

    #[test]
    fn falls_back_to_nested_when_heterogeneous() {
        assert_eq!(
            to_toon(&json!({"items": [{"a": 1}, {"a": [1, 2]}]})),
            "items:\n  - a: 1\n  - a: [1,2]"
        );
    }

    #[test]
    fn emits_empty_array_as_name_zero() {
        assert_eq!(
            format_response("references", &json!([]), true),
            "references[0]:"
        );
    }

    #[test]
    fn emits_inline_primitive_array() {
        assert_eq!(to_toon(&json!({"items": [1, 2, 3]})), "items: [1,2,3]");
    }

    #[test]
    fn emits_multiline_primitive_array_when_long() {
        assert_eq!(
            to_toon(&json!({"items": [1, 2, 3, 4, 5, 6, 7, 8, 9]})),
            "items:\n  - 1\n  - 2\n  - 3\n  - 4\n  - 5\n  - 6\n  - 7\n  - 8\n  - 9"
        );
    }

    #[test]
    fn quotes_strings_with_delimiters() {
        assert_eq!(quote_string_if_needed("file:///a.bsl"), "\"file:///a.bsl\"");
    }

    #[test]
    fn quotes_strings_with_newlines() {
        assert_eq!(quote_string_if_needed("a\nb"), "\"a\\nb\"");
    }

    #[test]
    fn preserves_cyrillic_unquoted() {
        assert_eq!(quote_string_if_needed("Привет"), "Привет");
    }

    #[test]
    fn quotes_numeric_looking_strings() {
        assert_eq!(quote_string_if_needed("0123"), "\"0123\"");
    }

    #[test]
    fn handles_null_top_level() {
        assert_eq!(format_response("definition", &Value::Null, true), "null");
    }

    #[test]
    fn handles_empty_object() {
        assert_eq!(to_toon(&json!({})), "");
    }

    #[test]
    fn drops_code_description_and_empty_tags() {
        let value = json!({
            "code": "ParseError",
            "codeDescription": { "href": "https://example/ParseError" },
            "message": "boom",
            "range": {
                "start": { "line": 1, "character": 0 },
                "end": { "line": 1, "character": 5 }
            },
            "severity": 1,
            "source": "bsl-language-server",
            "tags": []
        });

        assert_eq!(
            flatten_lsp(value),
            json!({
                "code": "ParseError",
                "message": "boom",
                "range_sl": 1,
                "range_sc": 0,
                "range_el": 1,
                "range_ec": 5,
                "severity": 1,
                "source": "bsl-language-server"
            })
        );
    }

    #[test]
    fn keeps_empty_diagnostics_array() {
        let value = json!({
            "uri": "file:///a.bsl",
            "diagnostics": []
        });

        let rendered = format_response("diagnostics", &value, true);
        assert!(
            rendered.contains("diagnostics[0]:"),
            "expected empty diagnostics marker, got:\n{rendered}"
        );
    }

    #[test]
    fn keeps_empty_children_array() {
        let value = json!({
            "name": "Module",
            "children": []
        });

        assert_eq!(
            flatten_lsp(value),
            json!({
                "name": "Module",
                "children": []
            })
        );
    }

    #[test]
    fn diagnostics_collapse_into_table() {
        let value = json!({
            "diagnostics": [
                {
                    "code": "A",
                    "codeDescription": { "href": "x" },
                    "message": "m1",
                    "range": {
                        "start": { "line": 1, "character": 2 },
                        "end": { "line": 1, "character": 3 }
                    },
                    "severity": 1,
                    "source": "bsl",
                    "tags": []
                },
                {
                    "code": "B",
                    "codeDescription": { "href": "y" },
                    "message": "m2",
                    "range": {
                        "start": { "line": 4, "character": 5 },
                        "end": { "line": 4, "character": 6 }
                    },
                    "severity": 2,
                    "source": "bsl",
                    "tags": []
                }
            ]
        });

        let rendered = format_response("diagnostics", &value, true);
        assert!(
            rendered.starts_with("diagnostics[2]{"),
            "expected tabular output, got:\n{rendered}"
        );
    }
}
