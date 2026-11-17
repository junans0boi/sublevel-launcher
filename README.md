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

## UI 개발(Rust 없이)
`npm run ui` 후 `http://localhost:1420/?state=<상태>` — 모의 백엔드(`ui/mock.js`)가 Tauri 명령·이벤트를 흉내 낸다.
상태: `notinstalled` `update` `ready` `loggedout` `offline` `maint` `disk` `hash`, 옵션 `hold=0.4`(진행률 중간 정지) `click=main|verify`(자동 클릭) `bad=1` `start=1`.
상태별 캡처: `docs/captures/`.

## 파일 검사하기
설치 직후 모든 파일의 sha256을 `.files.json`에 기록한다. 검사는 이 목록과 대조해 손상·누락 파일만 찾아 **해당 파일만 교체**한다.
알려진 한계: manifest에 파일별 해시가 없어 복구 시 zip을 (재개 가능하게) 받은 뒤 손상 항목만 풀어 쓴다. L0이 파일별 해시/Range를 주면 그 파일만 받도록 개선한다.

## 딥링크 `sublevel://start`
사이트의 '게임 시작'이 런처를 연다. 허용 URL은 정확히 `sublevel://start`뿐이며(경로·쿼리·포트 등이 붙으면 무시), URL 값은 경로나 명령으로 쓰이지 않는다. 이미 실행 중이면 `single-instance`가 기존 창을 앞으로 가져온다.
- Mac: 번들 `Info.plist`의 `CFBundleURLTypes`(`tauri.conf.json`의 `plugins.deep-link.desktop.schemes`에서 빌드 시 생성). 설치된 `.app`을 한 번 열면 등록된다. 시험: `open "sublevel://start"`
- Windows(L3에서 실기 확인): 설치 관리자가 레지스트리 `HKCU\Software\Classes\sublevel`(`URL Protocol`, `shell\open\command` = `"설치경로\Sublevel.exe" "%1"`)를 등록한다. 개발 실행은 `register_all()`로 등록한다.

## 이미지 출처
히어로·공지 타일 이미지는 이 게임(junans0boi/sublevel)의 자체 캡처(`docs/bible/img/ui/`, `docs/tracks/interact_captures/`)를 잘라 썼다. 외부 상표·로고·이미지는 없다. 배틀그라운드·서든어택 런처는 레이아웃(구조)만 참고했다.

## 설치 위치
Mac `~/Library/Application Support/Sublevel/game/<버전>/`, Windows `%LOCALAPPDATA%\Sublevel\game\<버전>\`. 최근 2개 버전만 유지.
