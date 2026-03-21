use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use web_sys::{EventSource, MessageEvent};

use crate::app::ServerEvent;

pub fn subscribe(mut callback: impl FnMut(ServerEvent) + 'static) -> Result<(), String> {
    let source = EventSource::new("/api/events").map_err(js_error)?;
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        if let Some(data) = event.data().as_string() {
            if let Ok(event) = serde_json::from_str::<ServerEvent>(&data) {
                callback(event);
            }
        }
    });

    source
        .add_event_listener_with_callback("message", on_message.as_ref().unchecked_ref())
        .map_err(js_error)?;

    on_message.forget();
    std::mem::forget(source);
    Ok(())
}

fn js_error(value: wasm_bindgen::JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| "Не удалось подписаться на /api/events".to_string())
}
