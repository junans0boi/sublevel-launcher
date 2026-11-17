const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = (id) => document.getElementById(id);
const show = (id) => ["loading", "out", "in"].forEach((s) => ($(s).hidden = s !== id));
const say = (t, err) => { $("msg").textContent = t || ""; $("msg").className = "msg" + (err ? " err" : ""); };
const mb = (n) => (n / 1048576).toFixed(0) + "MB";
let latest = null, busy = false;

function setPlay(text, disabled) { $("play").textContent = text; $("play").disabled = disabled; }

async function enter(me) {
  $("nick").textContent = me.nickname || me.display_name || "플레이어";
  show("in");
  say(me.needs_nickname ? "닉네임은 사이트에서 정할 수 있습니다." : "");
  await refresh();
}

async function refresh() {
  setPlay("확인 중…", true);
  try {
    latest = await invoke("latest");
    $("ver").textContent = `최신 버전 ${latest.version}` + (latest.installed ? " · 설치됨" : ` · ${mb(latest.size)}`);
    setPlay(latest.installed ? "게임 시작" : "설치하고 시작", false);
  } catch (e) { latest = null; $("ver").textContent = ""; say(e, true); setPlay("다시 확인", false); }
}

$("login").onclick = async () => {
  $("login").disabled = true; say("브라우저에서 로그인을 완료해 주세요…");
  try { await enter(await invoke("login")); } catch (e) { say(e, true); }
  $("login").disabled = false;
};
$("logout").onclick = async () => { await invoke("logout"); say(""); show("out"); };

$("play").onclick = async () => {
  if (busy) return; busy = true;
  try {
    if (!latest) { await refresh(); return; }
    if (!latest.installed) {
      setPlay("설치 중…", true); $("bar").hidden = false; say("");
      await invoke("install_game");
      $("bar").hidden = true; latest.installed = true;
    }
    setPlay("실행 중…", true);
    await invoke("launch_game");
    say("게임을 실행했습니다.");
    await refresh();
  } catch (e) { say(e, true); $("bar").hidden = true; await refresh(); }
  finally { busy = false; }
};

listen("progress", (ev) => {
  const [done, total] = ev.payload;
  $("fill").style.width = (total ? (done / total) * 100 : 0) + "%";
  $("play").textContent = `내려받는 중 ${mb(done)} / ${mb(total)}`;
});

(async () => {
  try { const me = await invoke("me"); me ? await enter(me) : show("out"); }
  catch (e) { show("out"); say(e, true); } // 오프라인 등: 토큰은 유지, 로그인 화면만 보임
})();
