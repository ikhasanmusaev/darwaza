use serde::Serialize;

#[derive(Serialize)]
pub struct PingResponse {
    pub ok: bool,
    pub version: String,
}

#[tauri::command]
pub fn ping() -> PingResponse {
    PingResponse {
        ok: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}
