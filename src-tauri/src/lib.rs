mod auth;
mod config;
mod install;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{Emitter, Manager, State};

#[derive(Default)]
struct Game(Mutex<Option<std::process::Child>>);
/// 딥링크(sublevel://start)가 들어왔지만 프런트가 아직 처리하지 않은 상태.
#[derive(Default)]
struct PendingStart(AtomicBool);

#[derive(serde::Serialize, serde::Deserialize)]
struct NewsItem { title: String, #[serde(default)] desc: String, url: String }

#[tauri::command] async fn me() -> Result<Option<auth::Me>, String> { auth::me().await }
#[tauri::command] async fn login() -> Result<auth::Me, String> { auth::login().await }
#[tauri::command] async fn logout() { auth::logout().await }
#[tauri::command] async fn latest() -> Result<install::Latest, String> { install::latest().await }
#[tauri::command] fn take_start(p: State<'_, PendingStart>) -> bool { p.0.swap(false, Ordering::SeqCst) }

/// 공지 타일. 서버의 /news.json 을 시도하고, 없으면 기본 안내를 보여 준다.
#[tauri::command]
async fn news() -> Vec<NewsItem> {
    let base = config::api_base();
    let fetched = async {
        let r = auth::client().get(format!("{base}/news.json")).timeout(std::time::Duration::from_secs(5)).send().await.ok()?;
        if !r.status().is_success() { return None; }
        r.json::<Vec<NewsItem>>().await.ok()
    }.await;
    fetched.filter(|v| !v.is_empty()).unwrap_or_else(|| [
        ("게임 설치 및 실행 안내", "구글 로그인 후 ‘설치’를 누르면 최신 버전을 내려받아 바로 실행합니다."),
        ("문의 및 버그 제보", "플레이 중 문제를 발견하면 사이트의 고객센터로 알려 주세요."),
        ("커뮤니티 이용 안내", "다른 플레이어를 존중하고 규칙을 지켜 주세요. 자세한 내용은 사이트에서 볼 수 있습니다."),
    ].iter().map(|(t, d)| NewsItem { title: t.to_string(), desc: d.to_string(), url: base.clone() }).collect())
}

/// 외부 링크는 우리 사이트(API 주소)로만 연다. 다른 값은 거절.
#[tauri::command]
fn open_link(url: Option<String>) -> Result<(), String> {
    let base = config::api_base();
    let target = match url.as_deref() { None | Some("") => base.clone(), Some(u) => u.to_string() };
    if target != base && !target.starts_with(&format!("{base}/")) { return Err("허용되지 않은 링크입니다.".into()); }
    auth::open_browser(&target)
}

// 진행률 이벤트는 0.5% 단위로만 보낸다
fn progress_emitter(app: tauri::AppHandle) -> impl FnMut(u64, u64) {
    let mut last = 0u64;
    move |done, total| {
        if done == total || done.saturating_sub(last) * 200 >= total { last = done; let _ = app.emit("progress", (done, total)); }
    }
}

#[tauri::command]
async fn install_game(app: tauri::AppHandle) -> Result<(), String> {
    let r = install::manifest().await?;
    install::install(&r, progress_emitter(app)).await
}

#[tauri::command]
async fn verify_files(app: tauri::AppHandle) -> Result<install::VerifyResult, String> {
    let r = install::manifest().await?;
    let a2 = app.clone();
    install::verify(&r, move |d, t| { let _ = a2.emit("verify", (d, t)); }, progress_emitter(app)).await
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

/// 허용하는 딥링크는 정확히 `sublevel://start` 하나(끝의 `/`만 허용). 경로·쿼리·포트·계정 정보가 붙으면 모두 무시한다.
/// URL 값은 어디에도 전달·실행되지 않는다 — 허용 여부(bool)만 쓴다.
pub fn is_start_link(s: &str) -> bool {
    let Ok(u) = reqwest::Url::parse(s) else { return false };
    u.scheme() == "sublevel" && u.host_str() == Some("start") && matches!(u.path(), "" | "/")
        && u.query().is_none() && u.fragment().is_none() && u.port().is_none() && u.username().is_empty() && u.password().is_none()
}

fn on_links<'a>(app: &tauri::AppHandle, urls: impl IntoIterator<Item = &'a str>) {
    if !urls.into_iter().any(is_start_link) { return; }
    app.state::<PendingStart>().0.store(true, Ordering::SeqCst);
    if let Some(w) = app.get_webview_window("main") { let _ = w.show(); let _ = w.unminimize(); let _ = w.set_focus(); }
    let _ = app.emit("deeplink", ());
}

pub fn run() {
    use tauri_plugin_deep_link::DeepLinkExt;
    tauri::Builder::default()
        // 이미 실행 중이면 새 창을 만들지 않고 기존 창을 앞으로 가져온다.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _| {
            on_links(app, argv.iter().map(String::as_str));
        }))
        .plugin(tauri_plugin_deep_link::init())
        .manage(Game::default())
        .manage(PendingStart::default())
        .setup(|app| {
            #[cfg(any(windows, target_os = "linux"))]
            { let _ = app.deep_link().register_all(); } // 개발 실행용. 설치본은 설치기가 등록한다.
            let h = app.handle().clone();
            app.deep_link().on_open_url(move |ev| {
                let u: Vec<String> = ev.urls().iter().map(|u| u.to_string()).collect();
                on_links(&h, u.iter().map(String::as_str));
            });
            if let Ok(Some(urls)) = app.deep_link().get_current() { // URL로 처음 실행된 경우
                let u: Vec<String> = urls.iter().map(|u| u.to_string()).collect();
                on_links(app.handle(), u.iter().map(String::as_str));
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![me, login, logout, latest, take_start, news, open_link, install_game, verify_files, launch_game])
        .run(tauri::generate_context!())
        .expect("런처를 시작하지 못했습니다");
}

#[cfg(test)]
mod tests {
    use super::is_start_link;
    #[test]
    fn only_start_is_allowed() {
        for ok in ["sublevel://start", "sublevel://start/", "SUBLEVEL://start"] { assert!(is_start_link(ok), "{ok}"); }
        for no in ["sublevel://start?cmd=rm", "sublevel://start/../../x", "sublevel://start/x", "sublevel://run", "sublevel://", "sublevel://start#a",
                   "sublevel://start:99", "sublevel://u:p@start", "https://start", "sublevel:start", "sublevel://file:///etc/passwd", "", "sublevel://start%2F"] {
            assert!(!is_start_link(no), "{no}");
        }
    }
}
