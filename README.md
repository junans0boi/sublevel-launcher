# Sublevel 런처

Tauri 2(Rust + 웹 UI) 런처. 구글 로그인 → 게임 내려받기·검증·설치 → 실행. 설계: junans0boi/sublevel#393.

## 구조
- `ui/` — 웹 UI(번들러 없음, 순수 HTML/CSS/JS)
- `src-tauri/src/auth.rs` — 구글 데스크탑 PKCE + 루프백, 세션 토큰은 OS 키체인
- `src-tauri/src/install.rs` — manifest 비교, Range 재개 다운로드, sha256, 압축 해제, 실행(게임 토큰은 환경 변수 `SUBLEVEL_GAME_TOKEN`)
- `src-tauri/src/config.rs` — 공개 설정(API 주소, 구글 데스크탑 클라이언트 ID)

앱 안에는 비밀이 없다. 구글 코드 교환은 백엔드(`POST /api/auth/google/desktop`)가 한다.

## 빌드
```bash
source ~/.cargo/env                # rustup stable
npm install
SUBLEVEL_GOOGLE_CLIENT_ID=<데스크탑 클라이언트 ID> npm run build   # Mac: .app/.dmg
npm run dev                        # 개발 실행
cd src-tauri && cargo test         # 단위 테스트(PKCE, Range 재개, 해시, 정리)
```
클라이언트 ID(공개 값)는 `src-tauri/src/config.rs`의 상수에 직접 써도 된다.

## 개발용 환경 변수
`SUBLEVEL_API_BASE`(모의 서버 주소), `SUBLEVEL_HOME`(설치 루트 대체).

## 설치 위치
Mac `~/Library/Application Support/Sublevel/game/<버전>/`, Windows `%LOCALAPPDATA%\Sublevel\game\<버전>\`. 최근 2개 버전만 유지.
