use gloo_net::http::{Request, RequestBuilder};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::app::{
    ErrorResponse, JavaCheckResult, ProjectPayload, ProjectSnapshot, SettingsPayload,
};

#[derive(Serialize)]
pub struct BrowseRequest {
    pub path: Option<String>,
    pub show_files: bool,
    pub extension: Option<String>,
}

#[derive(Deserialize)]
pub struct BrowseResponse {
    pub current: String,
    pub parent: Option<String>,
    pub entries: Vec<BrowseEntry>,
}

#[derive(Clone, Deserialize)]
pub struct BrowseEntry {
    pub name: String,
    pub is_dir: bool,
}

pub async fn get_projects() -> Result<Vec<ProjectSnapshot>, String> {
    json_request(Request::get("/api/projects")).await
}

pub async fn create_project(payload: &ProjectPayload) -> Result<ProjectSnapshot, String> {
    json_body_request(Request::post("/api/projects"), payload).await
}

pub async fn update_project(id: &str, payload: &ProjectPayload) -> Result<ProjectSnapshot, String> {
    json_body_request(Request::put(&format!("/api/projects/{id}")), payload).await
}

pub async fn delete_project(id: &str) -> Result<(), String> {
    let response = Request::delete(&format!("/api/projects/{id}"))
        .send()
        .await
        .map_err(|err| err.to_string())?;
    if response.ok() {
        return Ok(());
    }
    Err(parse_error(response).await)
}

pub async fn start_project(id: &str) -> Result<ProjectSnapshot, String> {
    json_request(Request::post(&format!("/api/projects/{id}/start"))).await
}

pub async fn stop_project(id: &str) -> Result<ProjectSnapshot, String> {
    json_request(Request::post(&format!("/api/projects/{id}/stop"))).await
}

pub async fn get_project_logs(id: &str, tail: usize) -> Result<Vec<String>, String> {
    json_request(Request::get(&format!(
        "/api/projects/{id}/logs?tail={tail}"
    )))
    .await
}

pub async fn clear_project_logs(id: &str) -> Result<(), String> {
    let response = Request::delete(&format!("/api/projects/{id}/logs"))
        .send()
        .await
        .map_err(|err| err.to_string())?;
    if response.ok() {
        return Ok(());
    }
    Err(parse_error(response).await)
}

pub async fn get_settings() -> Result<SettingsPayload, String> {
    json_request(Request::get("/api/settings")).await
}

pub async fn update_settings(payload: &SettingsPayload) -> Result<SettingsPayload, String> {
    json_body_request(Request::put("/api/settings"), payload).await
}

pub async fn check_java() -> Result<JavaCheckResult, String> {
    json_request(Request::post("/api/settings/check-java")).await
}

pub async fn browse(request: &BrowseRequest) -> Result<BrowseResponse, String> {
    json_body_request(Request::post("/api/browse"), request).await
}

async fn parse_error(response: gloo_net::http::Response) -> String {
    match response.json::<ErrorResponse>().await {
        Ok(body) => body.error,
        Err(_) => format!("Ошибка HTTP {}", response.status()),
    }
}

async fn json_request<T>(request: RequestBuilder) -> Result<T, String>
where
    T: DeserializeOwned,
{
    let response = request.send().await.map_err(|err| err.to_string())?;
    if response.ok() {
        return response.json::<T>().await.map_err(|err| err.to_string());
    }
    Err(parse_error(response).await)
}

async fn json_body_request<T, P>(request: RequestBuilder, payload: &P) -> Result<T, String>
where
    T: DeserializeOwned,
    P: Serialize + ?Sized,
{
    let request = request.json(payload).map_err(|err| err.to_string())?;
    let response = request.send().await.map_err(|err| err.to_string())?;
    if response.ok() {
        return response.json::<T>().await.map_err(|err| err.to_string());
    }
    Err(parse_error(response).await)
}
