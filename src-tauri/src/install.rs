use crate::auth::{client, load_token, net_err};
use crate::config::{api_base, platform_keys};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

#[derive(Deserialize, Clone, Debug)]
pub struct FileEntry { pub sha256: String, pub size: u64, pub url: String }
#[derive(Deserialize, Debug)]
struct Manifest { version: String, files: std::collections::HashMap<String, FileEntry> }

#[derive(Serialize, Clone)]
pub struct Latest { pub version: String, pub size: u64, pub installed: bool, pub has_other: bool }

pub struct Release { pub version: String, pub file: FileEntry }

/// Mac: ~/Library/Application Support/Sublevel, Windows: %LOCALAPPDATA%\Sublevel (dirs::data_local_dir).
pub fn root() -> PathBuf {
    if let Ok(p) = std::env::var("SUBLEVEL_HOME") { return PathBuf::from(p); } // 테스트용
    let base = if cfg!(target_os = "macos") { dirs::data_dir() } else { dirs::data_local_dir() };
    base.unwrap_or_else(std::env::temp_dir).join("Sublevel")
}
fn game_dir() -> PathBuf { root().join("game") }
fn version_dir(v: &str) -> PathBuf { game_dir().join(v) }
const MARK: &str = ".installed";
pub fn is_installed(v: &str) -> bool { version_dir(v).join(MARK).exists() }

fn safe_version(v: &str) -> bool {
    !v.is_empty() && v.len() <= 128 && v.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) && !v.starts_with('.')
}

pub async fn manifest() -> Result<Release, String> {
    let t = load_token().ok_or("로그인이 필요합니다.")?;
    let r = client().get(format!("{}/api/launcher/manifest", api_base())).bearer_auth(t).send().await.map_err(net_err)?;
    match r.status().as_u16() {
        200 => {}
        401 => return Err("로그인이 만료되었습니다. 다시 로그인해 주세요.".into()),
        404 => return Err("아직 배포된 게임 버전이 없습니다.".into()),
        s => return Err(format!("서버 오류({s}). 잠시 후 다시 시도해 주세요.")),
    }
    let m: Manifest = r.json().await.map_err(|_| "서버 응답을 읽지 못했습니다.".to_string())?;
    if !safe_version(&m.version) { return Err("서버 응답이 올바르지 않습니다.".into()); }
    let file = platform_keys().iter().find_map(|k| m.files.get(*k)).cloned().ok_or("이 운영체제용 게임이 아직 없습니다.")?;
    Ok(Release { version: m.version, file })
}

pub async fn latest() -> Result<Latest, String> {
    let r = manifest().await?;
    let has_other = std::fs::read_dir(game_dir()).map(|d| d.flatten().any(|e| e.path().join(MARK).exists())).unwrap_or(false);
    Ok(Latest { installed: is_installed(&r.version), has_other, size: r.file.size, version: r.version })
}

pub async fn sha256_file(p: &Path) -> std::io::Result<String> {
    use tokio::io::AsyncReadExt;
    let mut f = tokio::fs::File::open(p).await?;
    let (mut h, mut buf) = (Sha256::new(), vec![0u8; 1 << 20]);
    loop { let n = f.read(&mut buf).await?; if n == 0 { break; } h.update(&buf[..n]); }
    Ok(hex::encode(h.finalize()))
}

fn io_err(e: std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::StorageFull { "디스크 공간이 부족합니다. 공간을 확보한 뒤 다시 시도해 주세요.".into() }
    else { format!("파일을 쓰지 못했습니다({}).", e.kind()) }
}

/// Range 재개 다운로드 → sha256 검증. 불일치 시 파일 폐기.
pub async fn download(url: &str, dest: &Path, size: u64, sha: &str, mut on_progress: impl FnMut(u64, u64)) -> Result<(), String> {
    if let Some(d) = dest.parent() { tokio::fs::create_dir_all(d).await.map_err(io_err)?; }
    let part = dest.with_extension("part");
    let mut have = tokio::fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0);
    if have > size { let _ = tokio::fs::remove_file(&part).await; have = 0; }
    if have < size {
        let mut req = reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(15)).read_timeout(std::time::Duration::from_secs(30)).build().unwrap().get(url);
        if have > 0 { req = req.header("Range", format!("bytes={have}-")); }
        let mut r = req.send().await.map_err(net_err)?;
        match r.status().as_u16() {
            206 => {}
            200 => have = 0, // 서버가 Range를 무시 → 처음부터
            403 => return Err("다운로드 링크가 만료되었습니다. 다시 시도해 주세요.".into()),
            416 => { let _ = tokio::fs::remove_file(&part).await; return Err("이어받기에 실패했습니다. 다시 시도해 주세요.".into()); }
            s => return Err(format!("다운로드 서버 오류({s}).")),
        }
        let mut f = tokio::fs::OpenOptions::new().create(true).write(true).append(have > 0).truncate(have == 0).open(&part).await.map_err(io_err)?;
        on_progress(have, size);
        while let Some(c) = r.chunk().await.map_err(|_| "다운로드가 중단되었습니다. 다시 시도하면 이어받습니다.".to_string())? {
            f.write_all(&c).await.map_err(io_err)?;
            have += c.len() as u64;
            on_progress(have, size);
        }
        f.flush().await.map_err(io_err)?;
    }
    if tokio::fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0) != size {
        return Err("다운로드가 중단되었습니다. 다시 시도하면 이어받습니다.".into());
    }
    if sha256_file(&part).await.map_err(io_err)? != sha.to_lowercase() {
        let _ = tokio::fs::remove_file(&part).await;
        return Err("내려받은 파일이 손상되었습니다(해시 불일치). 파일을 폐기했으니 다시 시도해 주세요.".into());
    }
    tokio::fs::rename(&part, dest).await.map_err(io_err)
}

fn unzip(zip_path: &Path, out: &Path) -> Result<(), String> {
    let f = std::fs::File::open(zip_path).map_err(io_err)?;
    let mut z = zip::ZipArchive::new(f).map_err(|_| "압축 파일이 올바르지 않습니다.".to_string())?;
    z.extract(out).map_err(|e| match e { zip::result::ZipError::Io(e) => io_err(e), _ => "압축을 풀지 못했습니다.".into() })
}

pub async fn install(r: &Release, on_progress: impl FnMut(u64, u64)) -> Result<(), String> {
    if is_installed(&r.version) { return Ok(()); }
    let zip_path = game_dir().join(format!("{}.zip", r.version));
    download(&r.file.url, &zip_path, r.file.size, &r.file.sha256, on_progress).await?;
    let (tmp, dir) = (game_dir().join(format!("{}.tmp", r.version)), version_dir(&r.version));
    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_dir_all(&dir);
    let z = zip_path.clone();
    let t = tmp.clone();
    tauri::async_runtime::spawn_blocking(move || unzip(&z, &t)).await.map_err(|e| e.to_string())??;
    let t2 = tmp.clone();
    tauri::async_runtime::spawn_blocking(move || write_index(&t2)).await.map_err(|e| e.to_string())??;
    std::fs::write(tmp.join(MARK), b"").map_err(io_err)?;
    std::fs::rename(&tmp, &dir).map_err(io_err)?;
    let _ = std::fs::remove_file(&zip_path);
    cleanup(&game_dir(), 2, &r.version);
    Ok(())
}

/// 설치된 버전 폴더를 수정 시각 순으로 최근 keep개만 남긴다(현재 버전은 항상 유지).
pub fn cleanup(dir: &Path, keep: usize, current: &str) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut v: Vec<_> = rd.flatten().filter(|e| e.path().join(MARK).exists())
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path()))).collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, p) in v.into_iter().skip(keep) {
        if p.file_name().map(|n| n != current).unwrap_or(false) { let _ = std::fs::remove_dir_all(p); }
    }
}

/// 압축을 푼 폴더에서 실행 파일을 찾는다. Mac: *.app/Contents/MacOS/<첫 파일>, Windows: 최상위 *.exe.
pub fn find_executable(dir: &Path) -> Option<PathBuf> {
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        let name = p.file_name()?.to_string_lossy().to_string();
        if cfg!(target_os = "macos") && name.ends_with(".app") {
            return std::fs::read_dir(p.join("Contents/MacOS")).ok()?.flatten().map(|x| x.path()).next();
        }
        if cfg!(target_os = "windows") && name.ends_with(".exe") { return Some(p); }
        if cfg!(target_os = "linux") && p.is_file() && !name.starts_with('.') && std::fs::metadata(&p).map(|m| m.len() > 0).unwrap_or(false) && name != "manifest.json" { return Some(p); }
    }
    None
}

/// 게임 토큰은 argv가 아니라 환경 변수로만 전달한다.
pub async fn game_token() -> Result<String, String> {
    let t = load_token().ok_or("로그인이 필요합니다.")?;
    // 12시간 범위 제한 토큰(game-token)을 우선 쓰고, 아직 없는 서버(404)면 예전 엔드포인트로 폴백한다.
    let post = |p: &str| client().post(format!("{}/api/auth/{}", api_base(), p)).bearer_auth(&t).send();
    let mut r = post("game-token").await.map_err(net_err)?;
    if r.status().as_u16() == 404 { r = post("token").await.map_err(net_err)?; }
    if !r.status().is_success() { return Err(format!("게임 토큰을 받지 못했습니다({}).", r.status().as_u16())); }
    let v: serde_json::Value = r.json().await.map_err(|_| "서버 응답을 읽지 못했습니다.".to_string())?;
    v["token"].as_str().map(String::from).ok_or_else(|| "서버 응답이 올바르지 않습니다.".into())
}

/// 0.5초 간격으로 `n`번 보는 동안 프로세스가 살아 있으면 true.
async fn still_running(child: &mut Option<std::process::Child>, n: u32) -> bool {
    for _ in 0..n {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if let Some(Ok(Some(_))) = child.as_mut().map(|c| c.try_wait()) { return false; }
    }
    true
}

pub async fn launch(version: &str, child: &mut Option<std::process::Child>) -> Result<(), String> {
    if let Some(c) = child { if c.try_wait().map(|s| s.is_none()).unwrap_or(false) { return Err("게임이 이미 실행 중입니다.".into()); } }
    let exe = find_executable(&version_dir(version)).ok_or("실행 파일을 찾을 수 없습니다. 게임을 다시 설치해 주세요.")?;
    let token = game_token().await?;
    let c = std::process::Command::new(&exe).env("SUBLEVEL_GAME_TOKEN", token).current_dir(exe.parent().unwrap_or(Path::new(".")))
        .spawn().map_err(|_| "게임을 실행하지 못했습니다.".to_string())?;
    *child = Some(c);
    // 실제로 떴는지 확인: 3초 안에 종료하면 실패로 본다(런처를 닫지 않고 오류를 보여 준다).
    if !still_running(child, 6).await {
        *child = None;
        return Err("게임이 바로 종료되었습니다. ‘파일 검사하기’로 설치를 확인하거나 다시 설치해 주세요.".into());
    }
    Ok(())
}

const INDEX: &str = ".files.json";

fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) -> std::io::Result<()> {
    for e in std::fs::read_dir(dir)?.flatten() {
        let p = e.path();
        let ft = e.file_type()?; // symlink는 따라가지 않는다
        if ft.is_dir() { walk(base, &p, out)?; }
        else if ft.is_file() {
            let rel = p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
            if rel != MARK && rel != INDEX { out.push(rel); }
        }
    }
    Ok(())
}
fn sha256_sync(p: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(p)?;
    let (mut h, mut buf) = (Sha256::new(), vec![0u8; 1 << 20]);
    loop { let n = f.read(&mut buf)?; if n == 0 { break; } h.update(&buf[..n]); }
    Ok(hex::encode(h.finalize()))
}

/// 설치 직후 모든 파일의 sha256을 기록한다. '파일 검사하기'가 이 목록과 대조한다.
pub fn write_index(dir: &Path) -> Result<(), String> {
    let mut files = vec![];
    walk(dir, dir, &mut files).map_err(io_err)?;
    let map: std::collections::BTreeMap<String, String> = files.into_iter()
        .map(|f| sha256_sync(&dir.join(&f)).map(|h| (f, h))).collect::<Result<_, _>>().map_err(io_err)?;
    std::fs::write(dir.join(INDEX), serde_json::to_vec(&map).unwrap()).map_err(io_err)
}

/// 색인과 대조해 (검사한 개수, 손상·누락된 상대 경로들)를 돌려준다.
pub fn verify_dir(dir: &Path, mut on_progress: impl FnMut(u64, u64)) -> Result<(usize, Vec<String>), String> {
    let raw = std::fs::read(dir.join(INDEX)).map_err(|_| "검사 정보가 없습니다. 게임을 다시 설치해 주세요.".to_string())?;
    let map: std::collections::BTreeMap<String, String> = serde_json::from_slice(&raw).map_err(|_| "검사 정보가 손상되었습니다. 게임을 다시 설치해 주세요.".to_string())?;
    let (total, mut bad) = (map.len() as u64, vec![]);
    for (i, (f, h)) in map.iter().enumerate() {
        if !safe_rel(f) || sha256_sync(&dir.join(f)).map(|x| x != *h).unwrap_or(true) { bad.push(f.clone()); }
        on_progress(i as u64 + 1, total);
    }
    Ok((map.len(), bad))
}

fn safe_rel(f: &str) -> bool { !f.is_empty() && !f.starts_with('/') && !f.contains(':') && !f.split('/').any(|c| c == ".." || c.is_empty()) }

/// zip에서 지정한 항목만 out에 다시 풀어 쓴다(경로 검증, 실행 권한 유지).
pub fn extract_entries(zip_path: &Path, out: &Path, names: &[String]) -> Result<(), String> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(zip_path).map_err(io_err)?).map_err(|_| "압축 파일이 올바르지 않습니다.".to_string())?;
    for n in names {
        if !safe_rel(n) { continue; }
        let mut e = z.by_name(n).map_err(|_| "복구할 파일을 압축 파일에서 찾지 못했습니다.".to_string())?;
        let dest = out.join(n);
        if let Some(d) = dest.parent() { std::fs::create_dir_all(d).map_err(io_err)?; }
        let mut f = std::fs::File::create(&dest).map_err(io_err)?;
        std::io::copy(&mut e, &mut f).map_err(io_err)?;
        #[cfg(unix)] if let Some(m) = e.unix_mode() { use std::os::unix::fs::PermissionsExt; let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(m & 0o7777)); }
    }
    Ok(())
}

#[derive(Serialize)]
pub struct VerifyResult { pub checked: usize, pub bad: usize }

/// 설치된 최신 버전을 검사하고, 손상된 파일만 다시 풀어 복구한다.
/// shortcut: manifest에 파일별 해시가 없어 zip 전체를 (재개 가능하게) 받은 뒤 손상 항목만 교체한다. L0이 파일별 해시/Range zip을 제공하면 해당 파일만 받도록 개선.
pub async fn verify(r: &Release, mut on_verify: impl FnMut(u64, u64) + Send + 'static, on_progress: impl FnMut(u64, u64)) -> Result<VerifyResult, String> {
    if !is_installed(&r.version) { return Err("먼저 게임을 설치해 주세요.".into()); }
    let dir = version_dir(&r.version);
    let d = dir.clone();
    let (checked, bad) = tauri::async_runtime::spawn_blocking(move || verify_dir(&d, &mut on_verify)).await.map_err(|e| e.to_string())??;
    if bad.is_empty() { return Ok(VerifyResult { checked, bad: 0 }); }
    let zip_path = game_dir().join(format!("{}.zip", r.version));
    download(&r.file.url, &zip_path, r.file.size, &r.file.sha256, on_progress).await?;
    let (z, d, b) = (zip_path.clone(), dir.clone(), bad.clone());
    let res = tauri::async_runtime::spawn_blocking(move || extract_entries(&z, &d, &b)).await.map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&zip_path);
    res?;
    Ok(VerifyResult { checked, bad: bad.len() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[cfg(unix)]
    #[tokio::test]
    async fn detects_early_exit_and_running() {
        let sh = |s: &str| Some(std::process::Command::new("sh").args(["-c", s]).spawn().unwrap());
        let mut dead = sh("exit 1");
        assert!(!still_running(&mut dead, 4).await);
        let mut alive = sh("sleep 5");
        assert!(still_running(&mut alive, 4).await);
        alive.unwrap().kill().unwrap();
    }

    /// 최소 Range 지원 HTTP 서버. cut_at이 Some이면 첫 응답을 그 바이트에서 끊는다.
    async fn serve(data: Vec<u8>, cut_at: Option<usize>) -> String {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/g.zip", l.local_addr().unwrap());
        tokio::spawn(async move {
            let mut first = true;
            loop {
                let (mut s, _) = l.accept().await.unwrap();
                let mut b = vec![0u8; 4096];
                let n = s.read(&mut b).await.unwrap();
                let req = String::from_utf8_lossy(&b[..n]).to_lowercase();
                let start = req.split("range: bytes=").nth(1).and_then(|r| r.split('-').next()).and_then(|x| x.trim().parse::<usize>().ok()).unwrap_or(0);
                let body = &data[start..];
                let send = if first { cut_at.map(|c| c.min(body.len())).unwrap_or(body.len()) } else { body.len() };
                first = false;
                let head = if start > 0 { format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{}/{}\r\n", data.len() - 1, data.len()) } else { "HTTP/1.1 200 OK\r\n".into() };
                let _ = s.write_all(format!("{head}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await;
                let _ = s.write_all(&body[..send]).await; let _ = s.shutdown().await;
            }
        });
        url
    }

    /// 설치→실행 종단: 모의 서버(manifest·game-token·zip) + 가짜 게임(스크립트).
    /// 깨끗한 폴더에 설치하고, 재실행 시 다운로드를 건너뛰며, 토큰은 환경변수로만 가고 argv에는 없음을 확인한다.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn e2e_install_run_token_hygiene() {
        use std::io::Write;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        // 가짜 게임 zip: Fake.app/Contents/MacOS/fake — 받은 argv와 토큰 환경변수를 파일에 적고 잠시 살아 있는다.
        let work = tempfile::tempdir().unwrap();
        let out = work.path().join("seen.txt");
        let script = format!("#!/bin/sh\necho \"ARGC=$#\" > '{o}'\necho \"ARGV=$*\" >> '{o}'\necho \"TOKEN=$SUBLEVEL_GAME_TOKEN\" >> '{o}'\necho \"SESSION=${{SUBLEVEL_SESSION_TOKEN:-none}}\" >> '{o}'\nsleep 5\n", o = out.display());
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
            w.start_file("Fake.app/Contents/MacOS/fake", o).unwrap();
            w.write_all(script.as_bytes()).unwrap();
            w.finish().unwrap();
        }
        let zip_bytes = buf.into_inner();
        let sha = hex::encode(Sha256::digest(&zip_bytes));
        let hits = Arc::new(AtomicUsize::new(0));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let (b2, h2, z2, sha2) = (base.clone(), hits.clone(), zip_bytes.clone(), sha.clone());
        tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = l.accept().await else { return };
                let (b, h, z, sha) = (b2.clone(), h2.clone(), z2.clone(), sha2.clone());
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096]; let n = s.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let (status, ctype, body): (&str, &str, Vec<u8>) = if req.starts_with("GET /api/launcher/manifest") && req.contains("TESTSESSION") {
                        ("200 OK", "application/json", format!("{{\"version\":\"9.9.9\",\"files\":{{\"mac\":{{\"sha256\":\"{sha}\",\"size\":{},\"url\":\"{b}/game.zip\"}}}}}}", z.len()).into_bytes())
                    } else if req.starts_with("POST /api/auth/game-token") && req.contains("TESTSESSION") {
                        ("200 OK", "application/json", br#"{"token":"sg_TESTGAMETOKEN"}"#.to_vec())
                    } else if req.starts_with("GET /game.zip") {
                        h.fetch_add(1, Ordering::SeqCst); ("200 OK", "application/zip", z)
                    } else { ("404 Not Found", "text/plain", vec![]) };
                    let _ = s.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await;
                    let _ = s.write_all(&body).await; let _ = s.shutdown().await;
                });
            }
        });
        let home = tempfile::tempdir().unwrap(); // 깨끗한 설치 폴더
        std::env::set_var("SUBLEVEL_HOME", home.path());
        std::env::set_var("SUBLEVEL_API_BASE", &base);
        std::env::set_var("SUBLEVEL_KEY_SERVICE", "kr.steady2vivid.sublevel.launcher.test");
        keyring::Entry::new("kr.steady2vivid.sublevel.launcher.test", "session").unwrap().set_password("TESTSESSION").unwrap();

        let r = manifest().await.unwrap();
        assert!(!is_installed(&r.version));
        install(&r, |_, _| {}).await.unwrap();
        assert!(is_installed("9.9.9") && hits.load(Ordering::SeqCst) == 1);
        install(&manifest().await.unwrap(), |_, _| {}).await.unwrap(); // 재실행: 다운로드 생략
        assert_eq!(hits.load(Ordering::SeqCst), 1, "재실행 시 다시 받지 않아야 한다");

        let mut child = None;
        launch("9.9.9", &mut child).await.unwrap();
        let seen = std::fs::read_to_string(&out).unwrap();
        assert!(seen.contains("ARGC=0"), "{seen}");
        assert!(seen.contains("TOKEN=sg_TESTGAMETOKEN"), "{seen}");
        assert!(seen.contains("SESSION=none"), "세션 토큰은 게임에 넘기지 않는다: {seen}");
        assert!(!seen.contains("TESTSESSION"));
        child.unwrap().kill().unwrap();
        keyring::Entry::new("kr.steady2vivid.sublevel.launcher.test", "session").unwrap().delete_credential().unwrap();
    }

    #[tokio::test]
    async fn resume_and_verify() {
        let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let sha = hex::encode(Sha256::digest(&data));
        let url = serve(data.clone(), Some(100_000)).await;
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("g.zip");
        assert!(download(&url, &dest, data.len() as u64, &sha, |_, _| {}).await.is_err()); // 중단
        let got = std::fs::metadata(dest.with_extension("part")).unwrap().len(); assert!(got > 0 && got < data.len() as u64, "partial {got}");
        download(&url, &dest, data.len() as u64, &sha, |_, _| {}).await.unwrap(); // 재개
        assert_eq!(std::fs::read(&dest).unwrap(), data);
    }

    #[tokio::test]
    async fn hash_mismatch_discards() {
        let data = vec![7u8; 50_000];
        let url = serve(data.clone(), None).await;
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("g.zip");
        let e = download(&url, &dest, data.len() as u64, &"0".repeat(64), |_, _| {}).await.unwrap_err();
        assert!(e.contains("해시"));
        assert!(!dest.exists() && !dest.with_extension("part").exists());
    }

    #[test]
    fn cleanup_keeps_two_newest() {
        let d = tempfile::tempdir().unwrap();
        for (i, v) in ["a", "b", "c", "d"].iter().enumerate() {
            let p = d.path().join(v);
            std::fs::create_dir(&p).unwrap();
            std::fs::write(p.join(MARK), b"").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20 + i as u64));
        }
        cleanup(d.path(), 2, "d");
        let mut left: Vec<_> = std::fs::read_dir(d.path()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left, ["c", "d"]);
    }

    #[test]
    fn version_guard() { assert!(safe_version("2026-11-18_abc1")); assert!(!safe_version("../x")); assert!(!safe_version(".hid")); }

    #[test]
    fn verify_finds_only_corrupted_and_repair_restores() {
        use std::io::Write;
        let d = tempfile::tempdir().unwrap();
        let (zp, game) = (d.path().join("g.zip"), d.path().join("game"));
        {
            let mut w = zip::ZipWriter::new(std::fs::File::create(&zp).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            for (n, c) in [("a.bin", &b"aaaa"[..]), ("sub/b.bin", b"bbbb"), ("c.bin", b"cccc")] { w.start_file(n, o).unwrap(); w.write_all(c).unwrap(); }
            w.finish().unwrap();
        }
        unzip(&zp, &game).unwrap();
        write_index(&game).unwrap();
        assert_eq!(verify_dir(&game, |_, _| {}).unwrap(), (3, vec![]));
        std::fs::write(game.join("sub/b.bin"), b"XXXX").unwrap(); // 손상
        std::fs::remove_file(game.join("c.bin")).unwrap(); // 누락
        let (n, bad) = verify_dir(&game, |_, _| {}).unwrap();
        assert_eq!((n, bad.clone()), (3, vec!["c.bin".to_string(), "sub/b.bin".to_string()]));
        extract_entries(&zp, &game, &bad).unwrap();
        assert_eq!(verify_dir(&game, |_, _| {}).unwrap().1.len(), 0);
        assert_eq!(std::fs::read(game.join("a.bin")).unwrap(), b"aaaa");
    }

    #[test]
    fn rel_path_guard() { assert!(safe_rel("a/b.bin")); assert!(!safe_rel("../x")); assert!(!safe_rel("/etc/passwd")); assert!(!safe_rel("a//b")); }
}
