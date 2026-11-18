use crate::config::{api_base, GOOGLE_CLIENT_ID};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const KEY_SERVICE: &str = "kr.steady2vivid.sublevel.launcher";
const KEY_USER: &str = "session";

#[derive(Serialize, Deserialize, Clone)]
pub struct Me { pub id: i64, pub display_name: Option<String>, pub nickname: Option<String>, pub needs_nickname: bool }

pub fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(20)).user_agent("SublevelLauncher/0.1").build().unwrap()
}

fn entry() -> Result<keyring::Entry, String> {
    // 시험이 실제 로그인 항목을 건드리지 않도록 키체인 항목 이름을 바꿀 수 있다.
    let svc = std::env::var("SUBLEVEL_KEY_SERVICE").unwrap_or_else(|_| KEY_SERVICE.into());
    keyring::Entry::new(&svc, KEY_USER).map_err(|_| "키체인을 열 수 없습니다.".to_string())
}
pub fn load_token() -> Option<String> { entry().ok()?.get_password().ok() }
fn save_token(t: &str) -> Result<(), String> { entry()?.set_password(t).map_err(|_| "키체인에 저장하지 못했습니다.".into()) }
pub fn clear_token() { if let Ok(e) = entry() { let _ = e.delete_credential(); } }

pub fn net_err(_: reqwest::Error) -> String { "서버에 연결할 수 없습니다. 인터넷 연결을 확인해 주세요.".into() }

/// 저장된 토큰으로 /api/me. 토큰 없음/만료 → Ok(None)(키체인 정리), 네트워크 오류 → Err.
pub async fn me() -> Result<Option<Me>, String> {
    let Some(t) = load_token() else { return Ok(None) };
    let r = client().get(format!("{}/api/me", api_base())).bearer_auth(t).send().await.map_err(net_err)?;
    match r.status().as_u16() {
        200 => Ok(Some(r.json().await.map_err(|_| "서버 응답을 읽지 못했습니다.".to_string())?)),
        401 | 403 => { clear_token(); Ok(None) }
        s => Err(format!("서버 오류({s}). 잠시 후 다시 시도해 주세요.")),
    }
}

pub async fn logout() {
    if let Some(t) = load_token() {
        let _ = client().post(format!("{}/api/auth/logout", api_base())).bearer_auth(t).send().await;
    }
    clear_token();
}

pub fn pkce() -> (String, String) {
    let mut b = [0u8; 64];
    rand::thread_rng().fill_bytes(&mut b);
    let verifier = B64.encode(b); // 86자
    let challenge = B64.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

fn rand_state() -> String { let mut b = [0u8; 16]; rand::thread_rng().fill_bytes(&mut b); B64.encode(b) }

pub fn open_browser(url: &str) -> Result<(), String> {
    let r = if cfg!(target_os = "macos") { std::process::Command::new("open").arg(url).spawn() }
    else if cfg!(target_os = "windows") { std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn() }
    else { std::process::Command::new("xdg-open").arg(url).spawn() };
    r.map(|_| ()).map_err(|_| "브라우저를 열 수 없습니다.".to_string())
}

/// 요청 첫 줄 "GET /?code=..&state=.. HTTP/1.1" 에서 (code, state, error) 추출.
pub fn parse_callback(req: &str) -> Option<(Option<String>, Option<String>, Option<String>)> {
    let path = req.lines().next()?.split_whitespace().nth(1)?;
    let u = reqwest::Url::parse(&format!("http://127.0.0.1{path}")).ok()?;
    let get = |k: &str| u.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.into_owned());
    Some((get("code"), get("state"), get("error")))
}

fn page(title: &str, sub: &str) -> String {
    format!("<!doctype html><meta charset=utf-8><title>Sublevel</title><body style=\"background:#0B0F14;color:#E6ECF2;font-family:sans-serif;text-align:center;padding-top:20vh\"><h2>{title}</h2><p style=\"color:#98A5B3\">{sub}</p>")
}

async fn respond(s: &mut tokio::net::TcpStream, status: &str, body: &str) {
    let _ = s.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
    let _ = s.shutdown().await;
}

/// 루프백 콜백을 기다린다. 연결마다 별도 task라서 브라우저가 미리 열어 두는 빈 연결이나
/// 느린 연결이 진짜 요청을 막지 않는다. state가 맞는 요청이 올 때까지 계속 받는다.
pub async fn wait_callback(listener: TcpListener, state: &str, limit: Duration) -> Result<String, String> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Result<String, String>>();
    let state = state.to_string();
    let acceptor = tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else { continue };
            let (tx, state) = (tx.clone(), state.clone());
            tokio::spawn(async move {
                let mut buf = Vec::with_capacity(2048);
                let mut chunk = [0u8; 2048];
                // 요청 헤더 끝(\r\n\r\n)까지 읽되, 5초 안에 오지 않는 빈 연결은 버린다.
                let _ = tokio::time::timeout(Duration::from_secs(5), async {
                    while buf.len() < 16384 && !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        match s.read(&mut chunk).await { Ok(0) | Err(_) => break, Ok(n) => buf.extend_from_slice(&chunk[..n]) }
                    }
                }).await;
                let Some((code, st, err)) = parse_callback(&String::from_utf8_lossy(&buf)) else {
                    let status = if buf.is_empty() { "preconnect" } else { "ignored" };
                    eprintln!("[login] 무시한 연결({status})");
                    return respond(&mut s, "404 Not Found", "").await;
                };
                if st.as_deref() != Some(state.as_str()) { eprintln!("[login] state 불일치 요청 무시"); return respond(&mut s, "400 Bad Request", "").await; }
                if err.is_some() {
                    respond(&mut s, "200 OK", &page("로그인이 취소되었습니다", "런처로 돌아가 다시 시도해 주세요.")).await;
                    let _ = tx.send(Err("로그인이 취소되었습니다. 다시 시도해 주세요.".into()));
                } else if let Some(c) = code {
                    respond(&mut s, "200 OK", &page("로그인 완료", "런처로 돌아가세요. 이 창은 닫아도 됩니다.")).await;
                    let _ = tx.send(Ok(c));
                } else { respond(&mut s, "400 Bad Request", "").await; }
            });
        }
    });
    let r = tokio::time::timeout(limit, rx.recv()).await;
    acceptor.abort();
    match r {
        Ok(Some(v)) => v,
        Ok(None) => Err("로그인 대기 중 오류가 발생했습니다.".into()),
        Err(_) => Err("로그인 시간이 초과되었습니다(10분). 다시 시도해 주세요.".into()),
    }
}

pub async fn login() -> Result<Me, String> {
    let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|_| "로컬 포트를 열 수 없습니다.".to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{port}");
    let (verifier, challenge) = pkce();
    let state = rand_state();
    let url = reqwest::Url::parse_with_params("https://accounts.google.com/o/oauth2/v2/auth", [
        ("client_id", GOOGLE_CLIENT_ID), ("redirect_uri", &redirect), ("response_type", "code"),
        ("scope", "openid email profile"), ("code_challenge", &challenge), ("code_challenge_method", "S256"), ("state", &state),
    ]).unwrap();
    open_browser(url.as_str())?;

    eprintln!("[login] 브라우저를 열었고 127.0.0.1:{port} 에서 대기합니다");
    let code = wait_callback(listener, &state, Duration::from_secs(600)).await?;
    eprintln!("[login] 콜백 수신, 서버에 코드 교환 요청");

    let r = client().post(format!("{}/api/auth/google/desktop", api_base()))
        .json(&serde_json::json!({ "code": code, "code_verifier": verifier, "redirect_uri": redirect }))
        .send().await.map_err(net_err)?;
    match r.status().as_u16() {
        200 => {}
        401 => return Err("로그인에 실패했습니다. 다시 시도해 주세요.".into()),
        503 => return Err("서버의 데스크탑 로그인이 아직 설정되지 않았습니다.".into()),
        s => return Err(format!("서버 오류({s}). 잠시 후 다시 시도해 주세요.")),
    }
    let v: serde_json::Value = r.json().await.map_err(|_| "서버 응답을 읽지 못했습니다.".to_string())?;
    save_token(v["session_token"].as_str().ok_or("서버 응답이 올바르지 않습니다.")?)?;
    me().await?.ok_or_else(|| "로그인 확인에 실패했습니다.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pkce_challenge_matches_verifier() {
        let (v, c) = pkce();
        assert!(v.len() >= 43 && v.len() <= 128);
        assert_eq!(c, B64.encode(Sha256::digest(v.as_bytes())));
    }
    async fn get(port: u16, path: &str) -> String {
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes()).await.unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out).await;
        out
    }

    #[tokio::test]
    async fn callback_survives_preconnect_favicon_and_bad_state() {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        let h = tokio::spawn(wait_callback(l, "good", Duration::from_secs(20)));
        let _idle = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap(); // 브라우저의 빈 예비 연결(닫지 않음)
        let t = std::time::Instant::now();
        assert!(!get(port, "/favicon.ico").await.starts_with("HTTP/1.1 200")); // favicon은 무시
        assert!(get(port, "/?code=evil&state=bad").await.starts_with("HTTP/1.1 400"));
        let ok = get(port, "/?code=abc&state=good&iss=https://accounts.google.com&scope=openid").await;
        assert!(ok.contains("로그인 완료"));
        assert_eq!(h.await.unwrap().unwrap(), "abc");
        assert!(t.elapsed() < Duration::from_secs(3), "빈 연결이 요청을 막았다");
    }

    #[tokio::test]
    async fn callback_error_and_timeout() {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        let h = tokio::spawn(wait_callback(l, "s", Duration::from_secs(20)));
        get(port, "/?error=access_denied&state=s").await;
        assert!(h.await.unwrap().unwrap_err().contains("취소"));
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        assert!(wait_callback(l, "s", Duration::from_millis(100)).await.unwrap_err().contains("시간이 초과"));
    }

    #[test]
    fn callback_parse() {
        let (c, s, e) = parse_callback("GET /?code=4%2Fabc&state=xyz HTTP/1.1\r\nHost: x").unwrap();
        assert_eq!((c.as_deref(), s.as_deref(), e), (Some("4/abc"), Some("xyz"), None));
        assert!(parse_callback("GET /favicon.ico HTTP/1.1").unwrap().0.is_none());
    }
}
