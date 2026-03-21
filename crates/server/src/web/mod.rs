use axum::extract::Path;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;

use crate::runtime::ServerState;

mod embedded;

pub fn router() -> Router<ServerState> {
    Router::new()
        .route("/", get(index))
        .route("/{*path}", get(asset))
}

async fn index() -> Response {
    serve_embedded("index.html")
}

async fn asset(Path(path): Path<String>) -> Response {
    serve_embedded(&path)
}

fn serve_embedded(path: &str) -> Response {
    let requested = if path.is_empty() { "index.html" } else { path };
    if let Some(file) = embedded::FRONTEND_DIR.get_file(requested) {
        return file_response(requested, file.contents());
    }

    if let Some(file) = embedded::FRONTEND_DIR.get_file("index.html") {
        return file_response("index.html", file.contents());
    }

    StatusCode::NOT_FOUND.into_response()
}

fn file_response(path: &str, contents: &'static [u8]) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_str(mime.as_ref()).unwrap(),
        )],
        contents,
    )
        .into_response()
}
