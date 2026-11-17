// 브라우저 개발용 모의 백엔드. Tauri 안에서는 window.__TAURI__ 가 이미 있으므로 아무것도 하지 않는다.
// 사용: ?state=ready|notinstalled|update|loggedout|offline|disk|hash|maint  (&hold=0.4 중간 정지, &at=0.3 시작 위치, &bad=1, &start=1)
if (!window.__TAURI__) {
  const q = new URLSearchParams(location.search), st = q.get("state") || "notinstalled";
  const ls = {}; const GB = 1073741824;
  const me = { id: 1, display_name: "준환", nickname: "해무", needs_nickname: false };
  let loggedIn = st !== "loggedout", installed = st === "ready";
  const err = { offline: "서버에 연결할 수 없습니다. 인터넷 연결을 확인해 주세요.", maint: "서버 점검 중입니다. 잠시 후 다시 시도해 주세요." };
  const emit = (n, p) => (ls[n] || []).forEach((f) => f({ payload: p }));
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const cmds = {
    me: async () => (loggedIn ? me : null),
    login: async () => { await sleep(600); loggedIn = true; return me; },
    logout: async () => { loggedIn = false; },
    latest: async () => {
      if (err[st]) throw err[st];
      return { version: "0.2.0", size: 2.4 * GB, installed, has_other: st === "update" };
    },
    install_game: async () => {
      if (st === "disk") { await sleep(500); throw "디스크 공간이 부족합니다. 공간을 확보한 뒤 다시 시도해 주세요."; }
      const total = 2.4 * GB; let done = q.get("at") ? total * +q.get("at") : 0;
      while (done < total) {
        done = Math.min(total, done + 28 * 1048576); emit("progress", [done, total]); await sleep(120);
        if (q.get("hold") && done >= total * +q.get("hold")) await new Promise(() => {});
      }
      if (st === "hash") throw "내려받은 파일이 손상되었습니다(해시 불일치). 파일을 폐기했으니 다시 시도해 주세요.";
      installed = true;
    },
    verify_files: async () => { if (q.get("hold")) await new Promise(() => {}); await sleep(800); return { checked: 1284, bad: q.get("bad") ? 1 : 0 }; },
    launch_game: async () => { await sleep(300); },
    take_start: async () => q.get("start") === "1",
    news: async () => [{ title: "게임 설치 및 실행 안내", url: "" }, { title: "무기 밸런스 및 탄약 업데이트", url: "" }, { title: "계정 보안 안내", url: "" }],
    open_link: async () => {},
  };
  window.__TAURI__ = {
    core: { invoke: (c, a) => cmds[c](a) },
    event: { listen: (n, f) => { (ls[n] = ls[n] || []).push(f); return Promise.resolve(() => {}); } },
    window: { getCurrentWindow: () => ({ minimize() {}, close() {} }) },
  };
  // 캡처용: ?click=main|verify 이면 로드 후 해당 버튼을 자동으로 누른다.
  const c = q.get("click");
  if (c) addEventListener("load", () => setTimeout(() => document.getElementById(c).click(), 400));
}
