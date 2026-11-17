const T = window.__TAURI__;
const { invoke } = T.core;
const $ = (id) => document.getElementById(id);
let me = null, latest = null, busy = false, error = false;
let autoRun = false; // 딥링크 시작이면 설치 후 바로 실행

const size = (n) => (n >= 1073741824 ? (n / 1073741824).toFixed(2) + " GB" : (n / 1048576).toFixed(1) + " MB");
const clock = (s) => [s / 3600, (s % 3600) / 60, s % 60].map((x) => String(Math.floor(x)).padStart(2, "0")).join(":");
const say = (t, err) => { $("status").textContent = t || ""; $("status").className = "status" + (err ? " err" : ""); error = !!err; };

function setProgress(done, total, speed) {
  const p = total ? Math.min(100, Math.floor((done / total) * 100)) : 0;
  $("pct").textContent = p + "%"; $("fill").style.width = p + "%"; $("barA11y").setAttribute("aria-valuenow", p);
  $("sTotal").textContent = total ? size(total) : "–"; $("sDone").textContent = total ? size(done) : "–";
  $("sSpeed").textContent = speed ? `${(speed / 1048576).toFixed(2)} MB/s` : "–";
  $("sEta").textContent = speed && total ? clock(Math.max(0, (total - done) / speed)) : "–";
}

// 버튼·검사 버튼 상태를 한 곳에서 계산한다.
function render() {
  const b = $("main");
  let label, on = false;
  if (busy) { label = busy === "verify" ? "검사 중…" : busy === "install" ? (latest?.has_other ? "업데이트 중…" : "설치 중…") : "실행 중…"; }
  else if (!me) { label = "로그인 후 시작"; on = true; }
  else if (!latest) { label = error ? "다시 시도" : "확인 중…"; on = error; }
  else { label = latest.installed ? "게임 실행" : latest.has_other ? "업데이트" : "설치"; on = true; }
  b.textContent = label; b.disabled = !on;
  $("verify").disabled = !!busy || !me || !latest?.installed; $("verifyMenu").disabled = $("verify").disabled;
  $("acct").hidden = !me; if (me) $("nick").textContent = me.nickname || me.display_name || "플레이어";
}

async function refresh() {
  latest = null; render();
  try {
    latest = await invoke("latest");
    if (latest.installed) { setProgress(1, 1, 0); say("‘게임 실행’ 버튼을 눌러 게임을 실행해 주세요."); }
    else { setProgress(0, latest.size, 0); say(`최신 버전 ${latest.version} · ${size(latest.size)} 을(를) ${latest.has_other ? "업데이트" : "설치"}합니다.`); }
  } catch (e) { say(e, true); setProgress(0, 0, 0); }
  render();
}

async function enter(m) { me = m; render(); await refresh(); if (await invoke("take_start")) startFlow(); }

// 진행률 이벤트 → 최근 4초 이동 평균 속도
let samples = [];
function onProgress([done, total]) {
  const now = performance.now(); samples.push([now, done]); samples = samples.filter((s) => now - s[0] < 4000);
  const [t0, d0] = samples[0]; const speed = now > t0 ? ((done - d0) / (now - t0)) * 1000 : 0;
  setProgress(done, total, speed);
}

async function install() {
  busy = "install"; samples = []; say("내려받는 중입니다. 창을 닫지 마세요."); render();
  try { await invoke("install_game"); await refresh(); }
  finally { busy = false; }
}
async function launch() {
  busy = "launch"; render();
  try { await invoke("launch_game"); say("게임을 실행했습니다."); }
  finally { busy = false; }
}
async function run() {
  if (busy) return;
  try {
    if (!me) { say("브라우저에서 로그인을 완료해 주세요…"); return await enter(await invoke("login")); }
    if (!latest) return refresh();
    if (!latest.installed) { await install(); if (!($("auto").checked || autoRun)) return; }
    await launch();
  } catch (e) { say(e, true); setProgress(0, 0, 0); }
  finally { busy = false; autoRun = false; render(); }
}

async function verify() {
  if (busy) return; busy = "verify"; samples = []; say("파일을 검사하는 중…"); render();
  try {
    const r = await invoke("verify_files");
    say(r.bad === 0 ? `파일 ${r.checked}개를 검사했습니다. 모두 정상입니다.` : `손상된 파일 ${r.bad}개를 찾아 다시 받아 복구했습니다.`);
    setProgress(1, 1, 0);
  } catch (e) { say(e, true); }
  finally { busy = false; render(); }
}

// 사이트 '게임 시작'(sublevel://start)
function startFlow() {
  if (!me) { say("로그인한 뒤 ‘로그인 후 시작’을 눌러 주세요."); return; }
  autoRun = true; run();
}

const closeMenu = () => { $("menu").hidden = true; $("gear").setAttribute("aria-expanded", "false"); };
$("main").onclick = run;
$("verify").onclick = verify; $("verifyMenu").onclick = () => { closeMenu(); verify(); };
$("logout").onclick = async () => { closeMenu(); await invoke("logout"); me = null; latest = null; setProgress(0, 0, 0); say("로그아웃했습니다."); render(); };
$("min").onclick = () => T.window.getCurrentWindow().minimize();
$("close").onclick = () => T.window.getCurrentWindow().close();
$("gear").onclick = () => { const h = !$("menu").hidden; $("menu").hidden = h; $("gear").setAttribute("aria-expanded", String(!h)); };
document.addEventListener("keydown", (e) => { if (e.key === "Escape") closeMenu(); });
document.querySelectorAll("[data-link]").forEach((el) => (el.onclick = () => invoke("open_link", { url: null })));
$("auto").checked = localStorage.getItem("autoRun") === "1";
$("auto").onchange = () => localStorage.setItem("autoRun", $("auto").checked ? "1" : "0");

T.event.listen("progress", (ev) => onProgress(ev.payload));
T.event.listen("deeplink", async () => { if (await invoke("take_start")) startFlow(); });

(async () => {
  setProgress(0, 0, 0); render();
  invoke("news").then((items) => {
    $("tiles").replaceChildren(...items.slice(0, 3).map((n, i) => {
      const b = document.createElement("button"); b.className = "tile"; b.style.backgroundImage = `url(img/t${i + 1}.jpg)`;
      const s = document.createElement("span"); s.textContent = n.title; b.append(s); b.onclick = () => invoke("open_link", { url: n.url }); return b;
    }));
  }).catch(() => {});
  try { const m = await invoke("me"); if (m) await enter(m); else { say("구글 계정으로 로그인하고 게임을 시작하세요."); render(); } }
  catch (e) { say(e, true); render(); }
})();
