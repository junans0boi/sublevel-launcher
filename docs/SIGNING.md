# 서명·공증 정리 (런처)

두 가지는 별개다.

| 구분 | 무엇 | 현재 |
|---|---|---|
| 업데이트 서명 | 런처 자체 갱신 파일이 우리가 만든 것인지 확인(minisign, Tauri updater) | **적용됨, 무료** |
| OS 서명·공증 | macOS Gatekeeper / Windows SmartScreen 경고를 없앰 | **미적용(유료, 준환 결정)** |

## 1. 업데이트 서명 (적용됨)

- 공개키: `src-tauri/tauri.conf.json`의 `plugins.updater.pubkey` (공개해도 안전).
- 개인키: 저장소 밖 `~/…/Key/sublevel-launcher-updater/updater.key`. 저장소·CI·로그에 없다. 분실하면 기존 설치본은 자동 갱신을 받을 수 없으므로(새 키로는 검증 실패) 백업할 것.
- 서명 절차(릴리스마다):
  1. Mac: `npx tauri build --bundles app,dmg --config '{"bundle":{"createUpdaterArtifacts":true}}'` 로 `.app.tar.gz` 생성.
  2. Windows: GitHub Actions가 NSIS exe를 만든다(키 없음). 내려받은 exe를 로컬에서 서명한다.
  3. 둘 다 `npx tauri signer sign -f <개인키> -p "" <파일>` 로 `.sig` 생성.
  4. `sublevel-web/scripts/launcher_upload.sh <버전 폴더>` 가 파일·`.sig`를 올리고 `/launcher/update.json`을 만든다.
- 런처는 시작할 때 `/launcher/update.json`을 확인하고, 공개키로 검증된 파일만 설치한다.

## 2. OS 서명·공증 (미적용, 비용 결정 필요)

서명이 없으면 첫 실행 때 경고가 뜬다. 다운로드 페이지의 "런처 안내"(`/download/launcher/`)가 우회 방법을 안내한다.

- macOS: 첫 실행 때 앱을 **우클릭 → 열기**.
- Windows: SmartScreen에서 **추가 정보 → 실행**.

경고를 없애려면:

| OS | 필요한 것 | 대략 비용(변동 가능, 가입 전 확인) |
|---|---|---|
| macOS | Apple Developer Program 가입 → Developer ID 서명 + 공증(notarytool) | 연 약 99달러 |
| Windows | 코드서명 인증서(OV/EV) 또는 클라우드 서명 서비스 | 연 수십~수백 달러 수준, 서비스·인증서 종류에 따라 다름 |

도입 시 바뀌는 것: 빌드 때 인증서/자격 증명을 CI 비밀 또는 로컬 키체인으로 주입, Mac은 `tauri build`에 서명 신원·공증 환경변수 설정. 지금 구조(Windows는 Actions, Mac은 로컬)에서는 Mac 공증은 로컬 빌드에 그대로 붙일 수 있다.

현재 단계(클로즈드 테스트)에서는 미적용으로 두고 안내 문구로 대처한다.
