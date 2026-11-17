mod auth;
mod config;
mod install;

use std::sync::Mutex;
use tauri::{Emitter, State};

#[derive(Default)]
struct Game(Mutex<Option<std::process::Child>>);

#[tauri::command] async fn me() -> Result<Option<auth::Me>, String> { auth::me().await }
#[tauri::command] async fn login() -> Result<auth::Me, String> { auth::login().await }
#[tauri::command] async fn logout() { auth::logout().await }
#[tauri::command] async fn latest() -> Result<install::Latest, String> { install::latest().await }

#[tauri::command]
async fn install_game(app: tauri::AppHandle) -> Result<(), String> {
    let r = install::manifest().await?;
    let mut last = 0u64;
    install::install(&r, |done, total| {
        // 진행률 이벤트는 0.5% 단위로만 보낸다
        if done == total || done.saturating_sub(last) * 200 >= total { last = done; let _ = app.emit("progress", (done, total)); }
    }).await
}

#[tauri::command]
async fn launch_game(game: State<'_, Game>) -> Result<(), String> {
    let r = install::manifest().await?;
    if !install::is_installed(&r.version) { return Err("먼저 게임을 설치해 주세요.".into()); }
    let mut taken = game.0.lock().unwrap().take();
    let res = install::launch(&r.version, &mut taken).await;
    *game.0.lock().unwrap() = taken;
    res
}

pub fn run() {
    tauri::Builder::default()
        .manage(Game::default())
        .invoke_handler(tauri::generate_handler![me, login, logout, latest, install_game, launch_game])
        .run(tauri::generate_context!())
        .expect("런처를 시작하지 못했습니다");
}
