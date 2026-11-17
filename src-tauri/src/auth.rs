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
    keyring::Entry::new(KEY_SERVICE, KEY_USER).map_err(|_| "키체인을 열 수 없습니다.".to_string())
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

const PAGE: &str = "<!doctype html><meta charset=utf-8><title>Sublevel</title><body style=\"background:#0B0F14;color:#E6ECF2;font-family:sans-serif;text-align:center;padding-top:20vh\"><h2>로그인 완료</h2><p style=\"color:#98A5B3\">이 창을 닫고 런처로 돌아가세요.</p>";

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

    let code = tokio::time::timeout(Duration::from_secs(180), async {
        loop {
            let (mut s, _) = listener.accept().await.map_err(|e| e.to_string())?;
            let mut buf = vec![0u8; 4096];
            let n = tokio::time::timeout(Duration::from_secs(5), s.read(&mut buf)).await.ok().and_then(|r| r.ok()).unwrap_or(0);
            let Some((code, st, err)) = parse_callback(&String::from_utf8_lossy(&buf[..n])) else { continue }; // favicon 등
            let body = if code.is_some() { PAGE } else { "<meta charset=utf-8>로그인에 실패했습니다." };
            let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).as_bytes()).await;
            if err.is_some() { return Err("로그인이 취소되었습니다.".to_string()); }
            if st.as_deref() != Some(state.as_str()) { continue; } // 다른 곳에서 온 요청 무시
            if let Some(c) = code { return Ok(c); }
        }
    }).await.map_err(|_| "로그인 시간이 초과되었습니다. 다시 시도해 주세요.".to_string())??;

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
    #[test]
    fn callback_parse() {
        let (c, s, e) = parse_callback("GET /?code=4%2Fabc&state=xyz HTTP/1.1\r\nHost: x").unwrap();
        assert_eq!((c.as_deref(), s.as_deref(), e), (Some("4/abc"), Some("xyz"), None));
        assert!(parse_callback("GET /favicon.ico HTTP/1.1").unwrap().0.is_none());
    }
}
