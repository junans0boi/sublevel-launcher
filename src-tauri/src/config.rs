// 공개 값만 둔다. 비밀(client_secret)은 앱에 없다 — 코드 교환은 백엔드가 한다.
pub const DEFAULT_API_BASE: &str = "https://sublevel.steady2vivid.kro.kr";
// 구글 데스크탑 OAuth 클라이언트 ID(공개 값). 빌드 시 SUBLEVEL_GOOGLE_CLIENT_ID 로 덮어쓸 수 있다.
pub const GOOGLE_CLIENT_ID: &str = match option_env!("SUBLEVEL_GOOGLE_CLIENT_ID") {
    Some(v) => v,
    None => "586083056855-la8opascj6jcmm9b0kni2tk1u5ms7mkj.apps.googleusercontent.com",
};
// 개발·테스트용: 런타임 환경 변수로 모의 서버를 가리킬 수 있다.
pub fn api_base() -> String {
    std::env::var("SUBLEVEL_API_BASE").unwrap_or_else(|_| DEFAULT_API_BASE.to_string())
}
pub fn platform_keys() -> &'static [&'static str] {
    if cfg!(target_os = "macos") { &["mac", "macos", "darwin"] }
    else if cfg!(target_os = "windows") { &["win", "windows", "win64"] }
    else { &["linux"] }
}
