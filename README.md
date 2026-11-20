# sublevel-launcher

SUBLEVEL 데스크톱 런처(macOS · Windows). **구글 로그인 → 게임 내려받기·검증·설치 → 실행**과 런처 자체 업데이트를 맡는다.
Tauri 2(Rust) + 번들러 없는 웹 UI. 설계 이슈: junans0boi/sublevel#393.

## 다른 저장소와의 관계

```
sublevel-web (사이트)      '게임 시작' 버튼 → sublevel://start 딥링크로 런처를 연다
        │
sublevel-launcher (이것)   구글 PKCE 로그인 ─────────────→ sublevel-backend  POST /api/auth/google/desktop → 세션 토큰
                           최신 버전·서명 URL 조회 ──────→                   GET  /api/launcher/manifest
                           게임 zip 다운로드(Range) ─────→ Caddy /downloads/* (forward_auth → /api/launcher/verify)
                           게임 토큰 발급 ──────────────→                   POST /api/auth/game-token
                           게임 실행(env SUBLEVEL_GAME_TOKEN) ─→ sublevel(게임) 클라이언트가 토큰으로 매칭·접속
```
- 게임 zip과 `manifest.json`은 게임 저장소의 `tools/release.sh`가 만들고 `sublevel-web/scripts/release_upload.sh`가 서버에 올린다(게임 저장소 `docs/RELEASE.md`).
- 런처 설치 파일과 자체 업데이트 파일은 `sublevel-web/scripts/launcher_upload.sh`가 서버에 올리고 `/launcher/update.json`을 만든다.
- API 계약: `sublevel-backend/docs/launcher-api.md`, `docs/auth-api.md`.

## 구조

| 경로 | 내용 |
|---|---|
| `ui/` | 웹 UI(순수 HTML/CSS/JS). `mock.js`는 Rust 없이 UI를 띄우는 모의 백엔드 |
| `src-tauri/src/auth.rs` | 구글 데스크탑 PKCE + 루프백 리디렉트, 세션 토큰은 OS 키체인 |
| `src-tauri/src/install.rs` | manifest 비교, Range 재개 다운로드, sha256·크기 검증, 압축 해제, 파일 검사·복구, 실행 |
| `src-tauri/src/config.rs` | 공개 설정(API 주소, 구글 데스크탑 클라이언트 ID) |
| `src-tauri/tauri.conf.json` | 앱 ID `kr.steady2vivid.sublevel.launcher`, 딥링크 `sublevel`, 업데이터 공개키·엔드포인트 |
| `docs/SIGNING.md` | 업데이트 서명(적용)과 OS 코드 서명·공증(미적용) |
| `docs/captures/`, `docs/design/` | UI 상태별 캡처, 디자인 메모 |

앱 안에는 비밀이 없다. 구글 코드 교환은 백엔드가 하고, 앱에 들어가는 값(클라이언트 ID·업데이터 공개키)은 공개 값이다.

## 개발

```bash
source ~/.cargo/env                # rustup stable
npm install
npm run dev                        # 개발 실행(tauri dev)
npm run build                      # Mac: .app/.dmg (ad-hoc 서명)
cd src-tauri && cargo test         # 단위·통합 테스트(PKCE, Range 재개, 해시, 정리, 설치 e2e)
```

UI만(Rust 없이): `python3 -m http.server 1420 -d ui` 후 `http://localhost:1420/?state=<상태>` — Tauri가 없으면 `ui/mock.js`가 명령·이벤트를 흉내 낸다.
상태: `notinstalled` `update` `ready` `loggedout` `offline` `maint` `disk` `hash`, 옵션 `hold=0.4`(진행률 중간 정지) `click=main|verify`(자동 클릭) `bad=1` `start=1`.

## 환경 변수 (이름만, 값은 저장소에 두지 않음)

| 이름 | 언제 | 설명 |
|---|---|---|
| `SUBLEVEL_GOOGLE_CLIENT_ID` | 빌드 시(선택) | 구글 데스크탑 클라이언트 ID 덮어쓰기. 없으면 `config.rs`의 기본값(공개 값) |
| `SUBLEVEL_API_BASE` | 실행 시(개발) | 백엔드 주소를 모의 서버로 바꿈. 기본은 사이트 도메인 |
| `SUBLEVEL_HOME` | 실행 시(개발·테스트) | 설치 루트 대체 |
| `SUBLEVEL_KEY_SERVICE` | 실행 시(테스트) | 키체인 항목 이름 대체(시험이 실제 로그인을 건드리지 않게) |
| `SUBLEVEL_GAME_TOKEN` | 런처 → 게임 | 런처가 게임 프로세스에 넘기는 게임 토큰(사람이 설정하지 않음) |

업데이터 개인키는 저장소·CI 밖에 있다(`docs/SIGNING.md`).

## 동작 메모

- **설치 위치**: Mac `~/Library/Application Support/Sublevel/game/<버전>/`, Windows `%LOCALAPPDATA%\Sublevel\game\<버전>\`. 최근 2개 버전만 유지.
- **파일 검사**: 설치 직후 모든 파일의 sha256을 `.files.json`에 기록하고, 검사 때 대조해 손상·누락 파일만 교체한다. 한계: manifest에 파일별 해시가 없어 복구 때 zip을(재개 가능하게) 다시 받은 뒤 손상 항목만 푼다.
- **딥링크 `sublevel://start`**: 허용 URL은 정확히 이것 하나(경로·쿼리·포트가 붙으면 무시), 값은 경로나 명령으로 쓰지 않는다. 실행 중이면 `single-instance`가 기존 창을 앞으로.
  - Mac: `tauri.conf.json`의 `plugins.deep-link.desktop.schemes` → 번들 `Info.plist`. 설치된 `.app`을 한 번 열면 등록. 시험: `open "sublevel://start"`
  - Windows: 설치 관리자가 `HKCU\Software\Classes\sublevel`을 등록. 개발 실행은 `register_all()`.
- **게임 실행 후 런처 닫기**: 설정(기본 켬).

## 빌드·배포

| 플랫폼 | 어디서 | 산출물 |
|---|---|---|
| Windows | GitHub Actions `build.yml`(windows-latest, main push·수동·태그) | NSIS 설치 파일 `.exe` (아티팩트) |
| macOS | 로컬 `npm run build` | `.app`, `.dmg` |

배포 순서(버전 올릴 때):
1. `package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`의 버전을 같이 올린다.
2. Windows exe는 Actions 아티팩트(또는 아래 Release)에서 받고, Mac은 로컬 빌드.
3. 업데이트 서명(`npx tauri signer sign`) → `sublevel-web/scripts/launcher_upload.sh <버전 폴더>`로 서버에 올림(절차: `docs/SIGNING.md`).

## 릴리스 (GitHub Releases)

- `vX.Y.Z` 주석 태그를 push하면 `build.yml`이 Windows NSIS를 빌드하고 **GitHub Release를 만들어 설치 파일을 첨부**한다. 태그는 `package.json` 버전과 같아야 한다(다르면 실패).
- 노트 = 태그 메시지 + 이전 태그 이후 커밋. 비밀 없음(`GITHUB_TOKEN`만). Release에는 서명 안 된(OS 코드 서명 없는) exe가 붙는다 — 사용자 배포는 계속 서버(`launcher_upload.sh`) 경로.

```bash
git tag -a v0.1.8 -m "요약"
git push origin v0.1.8
```

## 이미지 출처
히어로·공지 타일 이미지는 게임(junans0boi/sublevel)의 자체 캡처(`docs/bible/img/ui/`, `docs/tracks/interact_captures/`)를 잘라 썼다. 외부 상표·로고·이미지는 없다. 다른 게임 런처는 레이아웃 구조만 참고했다.

## 측정값 (2026-11-20, Mac M4 Pro, 런처 0.1.7)
- 앱 용량: `Sublevel.app` 4.8 MB.
- 메모리: 대기 화면에서 런처 본 프로세스 RSS 85 MB(물리 사용량 24 MB) + 시스템 WebKit 보조 프로세스(공유라 따로 나눠 재지 않음).
- 종단: 로그인 → v0.2.0(507 MiB) 다운로드 약 3.0 MiB/s → sha256 일치 → 설치 → 게임 실행 확인(junans0boi/sublevel#393). `cargo test` 13개 통과(해시 불일치·Range 재개 포함).
- Windows: Actions에서 NSIS 빌드만 확인, **실기 미검증**.
