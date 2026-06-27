/* Battle Plan — Replay UI (offline, loopback-served).
 *
 * Consumes the canonical store (replay/SCHEMA.md v1): per-game index.json,
 * <hour>.snapshot.json, timeline.json — plus a compact /api/series/<root>/<gid>
 * the server precomputes so charts never fetch 5 MB snapshots (Codex #1).
 *
 * NO FABRICATION (DESIGN §1, SCHEMA §0.2): a field that is schema-present but
 * null/empty-unknown renders as "not captured" — never 0, never blank, never
 * invented. `0` is a measured zero. Every value carries a source badge (M / M* /
 * S / D) looked up from FIELD_SOURCES (the JS mirror of schema.FIELD_SOURCES) so
 * the viewer always knows reliability. Aggregates skip nulls and report coverage.
 */
"use strict";

// ---- FIELD_SOURCES (mirror of replay/schema.py) ---------------------------
// dotted path (concrete keys normalized to '*') -> source tag.
const FIELD_SOURCES = {
  "date":"S","hour_index":"D","save_version":"S",
  "countries.*.economy.factories.civ":"S","countries.*.economy.factories.mil":"S",
  "countries.*.economy.factories.dockyard":"S","countries.*.economy.factories.under_construction":"S",
  "countries.*.economy.construction_queue":"S","countries.*.economy.resources.produced":"S",
  "countries.*.economy.resources.exported":"S","countries.*.economy.resources.available_derived":"S/D",
  "countries.*.economy.resources.to_export":"S","countries.*.economy.resources.imported":"S",
  "countries.*.economy.fuel.current":"S/M*","countries.*.economy.fuel.max":"S/M*",
  "countries.*.economy.gross_ic_by_equipment":"S/D",
  "countries.*.military.num_ships":"M/S","countries.*.military.division_count":"M/D",
  "countries.*.military.manpower.available":"S/M*","countries.*.military.manpower.deployed":"S/D",
  "countries.*.military.xp.army":"S","countries.*.military.equipment_stockpile":"S",
  "countries.*.military.convoys":"S","countries.*.military.ships_by_hull":"S/D",
  "countries.*.military.casualties.killed":"S/D","countries.*.military.casualties.captured":"S/D",
  "countries.*.military.command_power":"M",
  "countries.*.politics.stability":"M","countries.*.politics.war_support":"M",
  "countries.*.politics.political_power":"M","countries.*.politics.command_power":"M",
  "countries.*.politics.ruling_party":"S","countries.*.politics.party_popularities":"S",
  "countries.*.politics.laws.economy":"S","countries.*.politics.laws.trade":"S",
  "countries.*.politics.laws.conscription":"S","countries.*.politics.ideas_all":"S",
  "countries.*.politics.ideas_other":"S","countries.*.politics.cabinet":"S",
  "countries.*.politics.is_human":"S/M","countries.*.politics.is_major":"S",
  "countries.*.politics.capital_state":"S","countries.*.politics.autonomy":"S",
  "countries.*.politics.faction":"S",
  "countries.*.research.tech_count":"S","countries.*.research.technologies":"S/D",
  "countries.*.research.in_progress":"S/M*","countries.*.research.research_slots":"S",
  "countries.*.research.original_research_slots":"S","countries.*.research.research_bonuses":"S",
  "countries.*.focus.focus_tree":"S","countries.*.focus.continuous_focus_palette":"S",
  "countries.*.focus.completed":"S/D","countries.*.focus.completed_count":"S",
  "countries.*.focus.current":"S/M*","countries.*.focus.progress":"S/M*","countries.*.focus.paused":"S",
  "countries.*.decisions":"S","countries.*.diplomacy.wars":"S","countries.*.diplomacy.opinion":"S",
  "countries.*.intel":"S",
  "states.*.owner":"M/S","states.*.controller":"M/S","states.*.is_core_of":"S",
  "states.*.buildings":"S/M*","states.*.province_buildings":"S",
  "states.*.victory_points_total":"S","states.*.victory_points_by_province":"S",
  "states.*.resources":"S","states.*.manpower_pool":"S",
  "states.*.category":"M/S","states.*.occupation":"S","states.*.demilitarized":"S",
  "states.*.supply":"S/M*",
  "provinces.*.controller":"S/D","provinces.*.state_id":"D","provinces.*.supply":"S",
  "divisions.*.div_id":"M/S","divisions.*.div_type":"M/S","divisions.*.owner":"M/S",
  "divisions.*.location_province":"M/S","divisions.*.state_id":"M/D",
  "divisions.*.template_id":"M/S","divisions.*.template_type":"M/S",
  "divisions.*.template_name":"M/S",
  "divisions.*.template_role":"S","divisions.*.organisation":"M","divisions.*.strength":"M/S",
  "divisions.*.equipment":"M/S","divisions.*.experience":"M*","divisions.*.moving":"M*",
  "divisions.*.in_combat":"M*","combats.*":"M*",
  "map.owner_by_state":"M/D","map.control_by_province":"S/D","map.vp_by_state":"S/D",
  "map.controlled_industry_share":"D","map.front_lines":"D","events.*":"D",
  // newer schema domains the rich store carries (mirror of schema.FIELD_SOURCES)
  "countries.*.economy.logistics":"S","countries.*.military.xp.army":"S",
  "countries.*.intel.agency":"S","countries.*.intel.operatives":"S",
  "countries.*.commanders.leader":"S","countries.*.commanders.generals":"S",
  "countries.*.commanders.admirals":"S","countries.*.commanders.advisors":"S",
  "countries.*.commanders.counts":"S/D","countries.*.mios":"S",
  "countries.*.extra.special_projects":"S","countries.*.extra.nukes_total":"S",
  "countries.*.extra.aces":"S","countries.*.extra.convoys_destroyed":"S",
  "countries.*.human_name":"S","countries.*.faction":"S",
  "diplomacy_world.wars":"S","diplomacy_world.factions":"S",
  "order_of_battle.army_groups":"M/S","order_of_battle.armies":"M/S",
  "order_of_battle.fleets":"M/S","order_of_battle.air_wings":"S",
  "order_of_battle.by_country":"S/D",
  "naval_air.region_control":"M","naval_air.air_superiority":"M",
  "supply.by_province":"M","world.scenario":"S","world_events.fired_events":"S",
};
const TAGRE = /^[A-Z]{2,3}$/;
function srcOf(dotted){
  const norm = dotted.split(".").map((s,i)=> (i>0 && (TAGRE.test(s)||/^\d+$/.test(s)||s==="*"))?"*":s).join(".");
  if(FIELD_SOURCES[norm]!==undefined) return FIELD_SOURCES[norm];
  const parts=norm.split(".");
  for(let cut=parts.length-1;cut>0;cut--){const p=parts.slice(0,cut).join(".");
    if(FIELD_SOURCES[p]!==undefined) return FIELD_SOURCES[p];}
  return null;
}

// ---- no-fabrication value helpers -----------------------------------------
// `null`/undefined -> not captured. A {} that should hold scalars (casualties) is
// also "not captured". 0 is a real measured value and is shown as 0.
function isCaptured(v){
  if(v===null||v===undefined) return false;
  if(typeof v==="object" && !Array.isArray(v) && Object.keys(v).length===0) return false;
  return true;
}
const NA = '<span class="na-inline">not captured</span>';
function fmtNum(v){
  if(!isCaptured(v)) return NA;
  if(typeof v!=="number") return esc(String(v));
  if(Math.abs(v)>=1000) return v.toLocaleString(undefined,{maximumFractionDigits:0});
  return (Number.isInteger(v)?v:v.toFixed(2)).toLocaleString?(Number.isInteger(v)?v.toLocaleString():v.toFixed(2)):String(v);
}
function fmtPct(v){ return isCaptured(v)? (v*100).toFixed(1)+"%" : NA; }
function esc(s){return String(s).replace(/[&<>"']/g,c=>({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[c]));}
function badge(src){ if(!src) return ""; return `<span class="srcbadge${src.indexOf("M*")>=0?" inferred":""}" data-src="${esc(src)}" title="source: ${esc(src)}">${esc(src)}</span>`; }
// A PROXY confidence travels WITH naval_air/supply data — never "exact" (Codex #4).
// Render it as a distinct estimate badge so the viewer can't mistake it for a live-exact read.
function proxyBadge(conf){ if(!conf) return ""; return `<span class="srcbadge proxy" data-src="${esc(conf)}" title="PROXY estimate — confidence: ${esc(conf)} (never exact)">~${esc(conf)}</span>`; }

// ---- asset fidelity (REAL game art where the data supports it) ------------
// Canonical the game ideology colours (engine constants — presentation styling, not
// captured data) so politics/party panels match the game's colour language.
const IDEOLOGY_COLORS = {
  fascism:"#8a7d39", nazism:"#8a7d39", communism:"#a23b2e", stalinism:"#a23b2e",
  democratic:"#3a6ea5", conservatism:"#3a6ea5", neutrality:"#5a7a4a",
  socialism:"#a23b2e", anarchist_communism:"#a23b2e",
};
function ideoColor(ide){ return IDEOLOGY_COLORS[(ide||"").toLowerCase()] || "#7a869a"; }
// token -> readable label WITHOUT inventing a localisation (Codex #2): a
// deterministic presentation transform of a REAL captured token. Raw token is
// preserved in the title attribute and the source badge is unchanged.
function humanizeToken(t){
  if(t==null) return null;
  return String(t).replace(/_/g," ")
    .replace(/([a-z])([0-9])/g,"$1 $2")           // excavation3 -> excavation 3
    .replace(/\b\w/g,m=>m.toUpperCase()).trim();
}
function tokenChip(raw, src){
  if(raw==null) return "";
  const h=humanizeToken(raw);
  return `<span class="chip" title="game token: ${esc(String(raw))}">${esc(h)}</span>`;
}
// real PORTRAIT (extracted PNG) for a portrait_gfx handle, else the handle as an
// honest monospace asset reference (no faked image — Codex #3).
function portraitImg(gfx, name){
  const p = gfx && S.assets && S.assets.portraits && S.assets.portraits[gfx];
  if(p) return `<img class="portrait" loading="lazy" src="/game_assets/${esc(p)}" `+
    `alt="${esc(name||gfx)}" title="${esc(gfx)}" onerror="this.classList.add('broken')">`;
  return gfx ? `<span class="assethandle" title="sprite (no extracted image)">${esc(gfx)}</span>` : "";
}
// real ruling-party FLAG (TAG_ideology.png) by convention; falls back to nothing.
function flagImg(tag, ruling){
  if(!tag) return "";
  const ide=(ruling||"").toLowerCase();
  const file = ide ? `${tag}_${ide}__medium.png` : `${tag}__medium.png`;
  return `<img class="flag" loading="lazy" src="/game_assets/assets/flags/${esc(file)}" `+
    `alt="${esc(tag)} flag" onerror="this.style.display='none'">`;
}

// ---- state ----------------------------------------------------------------
const S = {
  games:null, root:null, gameId:null,
  assets:null,            // /api/assets: real portrait map + country colors
  series:null,            // compact per-game series (/api/series)
  index:null,             // index.json snapshots[]
  timeline:null,          // timeline.json events[]
  frame:0,
  snapCache:{},           // file -> full snapshot (lazy)
  charts:{},              // canvas id -> Chart instance
  playing:false, playTimer:null,
  curScreen:"map", curCountry:null, milScope:"__all__",
};
const $=id=>document.getElementById(id);
const show=el=>el&&el.classList.remove("hidden");
const hide=el=>el&&el.classList.add("hidden");

async function jget(url){ const r=await fetch(url); if(!r.ok) throw new Error(url+" -> "+r.status); return r.json(); }

// ---- boot ------------------------------------------------------------------
async function boot(){
  try{
    S.games = await jget("/api/games");
    try{ S.assets = await jget("/api/assets"); }   // real-art index (best-effort)
    catch(_){ S.assets = {portraits:{}, country_colors:{}, has_assets:false}; }
    buildGameSelector();
    const first = pickDefaultGame();
    if(!first){ $("loading").textContent="No games found in the store. Run build_replay.py or use the demo."; return; }
    await loadGame(first.root, first.game_id);
    wireUI();
    hide($("loading"));
  }catch(e){ console.error(e); $("loading").textContent="ERROR: "+e.message; }
}

function pickDefaultGame(){
  // prefer a real multi-snapshot game, then any real, then the demo.
  const real=S.games.store||[], demo=S.games.demo||[];
  const multi=real.find(g=>g.n_snapshots>1);
  return multi || real[0] || demo[0] || null;
}

function buildGameSelector(){
  const sel=$("gameSel"); sel.innerHTML="";
  const add=(grp,arr)=>{ if(!arr||!arr.length) return;
    const og=document.createElement("optgroup"); og.label=grp;
    for(const g of arr){ const o=document.createElement("option");
      o.value=g.root+"/"+g.game_id;
      o.textContent=`${g.game_id} (${g.n_snapshots} snap${g.n_snapshots===1?"":"s"})`;
      og.appendChild(o);} sel.appendChild(og); };
  add("Replays", S.games.store);
  add("Demo (synthetic)", S.games.demo);
}

async function loadGame(root, gameId){
  S.root=root; S.gameId=gameId; S.frame=0; S.snapCache={};
  $("gameSel").value = root+"/"+gameId;
  S.series = await jget(`/api/series/${root}/${gameId}`);
  const base = `/${root==="store"?"replay/snapshots":"replay_ui/demo_snapshots"}/${gameId}`;
  S.storeBase = base;
  S.index = (await jget(`${base}/index.json`)).snapshots || [];
  try{ S.timeline = (await jget(`${base}/timeline.json`)).events || []; }
  catch(_){ S.timeline = []; }   // missing/empty timeline = "no events", not an error (Codex #6)

  // game meta line
  const n=S.series.n_frames;
  const sv=(S.series.save_versions||[]).join(", ")||"—";
  $("gameMeta").textContent = `${n} snapshot${n===1?"":"s"} · ${S.series.frames[0]?.date||"?"}` +
     (n>1?` → ${S.series.frames[n-1].date}`:"") + ` · ${sv}`;

  // scrubber range
  const scr=$("scrub"); scr.max=Math.max(0,n-1); scr.value=0;
  $("playBtn").disabled = n<=1;   // single frame -> no playback (Codex #6)

  buildCountrySelector();
  buildMilScope();
  buildTimelineFilters();
  setupMapFrame();
  setFrame(0);
}

// ---- global scrubber -------------------------------------------------------
function setFrame(i){
  const n=S.series.n_frames;
  S.frame=Math.max(0, Math.min(n-1, i));
  $("scrub").value=S.frame;
  const f=S.series.frames[S.frame];
  $("frameDate").textContent = f?.date || "—";
  $("frameProv").textContent = provLabel(f?.provenance);
  renderCurrentScreen();
}
function provLabel(p){
  if(!p) return "";
  const mem = p.memory && p.memory.captured ? "memory" : null;
  const sav = p.save && p.save.captured ? "save" : null;
  const ch=[mem,sav].filter(Boolean).join("+")||"no channel";
  return `channel: ${ch}`;
}
function play(){
  if(S.series.n_frames<=1) return;
  if(S.playing){ stopPlay(); return; }
  if(S.frame>=S.series.n_frames-1) S.frame=0;
  S.playing=true; $("playBtn").innerHTML="&#10073;&#10073;";
  S.playTimer=setInterval(()=>{
    if(S.frame>=S.series.n_frames-1){ stopPlay(); return; }
    setFrame(S.frame+1);
  }, 900);
}
function stopPlay(){ S.playing=false; $("playBtn").innerHTML="&#9658;";
  if(S.playTimer){ clearInterval(S.playTimer); S.playTimer=null; } }

// lazy full-snapshot load for the frame the user is parked on
async function curSnapshot(){
  const f=S.series.frames[S.frame];
  const file=f && f.file;
  if(!file){ return null; }
  if(S.snapCache[file]) return S.snapCache[file];
  const snap = await jget(`${S.storeBase}/${file}`);
  S.snapCache[file]=snap; return snap;
}

// ---- screen switching ------------------------------------------------------
function renderCurrentScreen(){
  const fns={map:renderMap, country:renderCountry, military:renderMilitary,
             combat:renderCombat, timeline:renderTimeline};
  (fns[S.curScreen]||(()=>{}))();
}
function switchScreen(name){
  S.curScreen=name;
  document.querySelectorAll("#tabs button").forEach(b=>b.classList.toggle("active",b.dataset.screen===name));
  document.querySelectorAll(".screen").forEach(s=>s.classList.toggle("active",s.id==="screen-"+name));
  renderCurrentScreen();
}

// ===========================================================================
// SCREEN 1 — MAP / TIME-LAPSE (embed the existing scrubber via iframe)
// ===========================================================================
function setupMapFrame(){
  // The renderer writes replay_render/output/<game_id>_scrubber.html. For the
  // demo there is none (replay_render is read-only / output is gitignored), so
  // we degrade with a clear note rather than a broken iframe.
  const fr=$("mapFrame"), note=$("mapNote");
  const candidate = `/replay_render/output/${S.gameId}_scrubber.html`;
  fetch(candidate,{method:"HEAD"}).then(r=>{
    if(r.ok){ fr.src=candidate; fr.style.display="block";
      note.innerHTML = `Embedded map time-lapse (owner/control + front lines + division markers). `+
        `Generated by <code>replay_render</code>; its own scrubber drives this frame. `+badge("M/S");
    } else { mapFallback(note,fr,candidate); }
  }).catch(()=>mapFallback(note,fr,candidate));
}
function mapFallback(note,fr,candidate){
  fr.style.display="none";
  note.innerHTML = `<div class="empty">No rendered time-lapse for <b>${esc(S.gameId)}</b> yet.<br>`+
    `Generate it with <code>python build_replay.py --game-id ${esc(S.gameId)}</code> `+
    `(writes <code>${esc(candidate)}</code>), then reopen this screen.<br>`+
    `The other four screens work from the snapshot store directly.</div>`;
}
function renderMap(){ /* iframe is self-driving; nothing per-frame here */ }

// ===========================================================================
// helpers for cards
// ===========================================================================
function card(label, valHtml, src){
  const na = (valHtml===NA);
  return `<div class="card"><div class="k"><span>${esc(label)}</span>${badge(src)}</div>`+
    `<div class="v${na?" na":""}">${na?"not captured":valHtml}</div></div>`;
}
function kvRow(key,val,src){
  const na=!isCaptured(val);
  const v = na?"not captured":(typeof val==="number"?fmtNum(val):esc(String(val)));
  return `<div class="key">${esc(key)} ${badge(src)}</div><div class="val${na?" na":""}">${v}</div>`;
}

// ===========================================================================
// SCREEN 2 — COUNTRY DETAIL
// ===========================================================================
function buildCountrySelector(){
  const sel=$("countrySel"); sel.innerHTML="";
  const tags=S.series.tags.slice().sort((a,b)=>{
    const ha=S.series.identity[a]?.is_human?0:1, hb=S.series.identity[b]?.is_human?0:1;
    return ha-hb || a.localeCompare(b);
  });
  for(const t of tags){ const o=document.createElement("option"); o.value=t;
    const id=S.series.identity[t]||{};
    o.textContent=t+(id.is_human?" (human)":"")+(id.is_major?" ★":"");
    sel.appendChild(o); }
  if(!S.curCountry || !tags.includes(S.curCountry))
    S.curCountry = tags.find(t=>S.series.identity[t]?.is_human) || tags[0];
  sel.value=S.curCountry;
}

async function renderCountry(){
  const tag=S.curCountry; if(!tag) return;
  const id=S.series.identity[tag]||{};
  // is_human is nullable (S/M). null = not captured — do NOT render it as "AI"
  // (that would fabricate a measured false). Only false renders "AI" (Codex #2).
  const humanChip = id.is_human===true ? `<span class="chip human">human player</span>`
    : id.is_human===false ? `<span class="chip">AI</span>`
    : `<span class="chip" title="is_human not captured">human? n/c</span>`;
  // NOTE: the ruling-party FLAG + ideology swatch are built in renderCountryDeep from
  // the loaded snapshot's PER-FRAME ruling_party — NOT from series.identity (which is
  // the last frame and would show a future flag on an earlier frame; Codex review #1).
  $("countryBadges").innerHTML =
    `<span class="chip">${esc(tag)}</span>`+ humanChip +
    (id.is_major===true?`<span class="chip">major</span>`:"");

  // cards from the series at the CURRENT frame (no-fabrication aware)
  const at=k=>S.series.series[tag]?.[k]?.[S.frame];
  const cards=[
    card("Civ factories", fmtNum(at("economy.factories.civ")), srcOf("countries.GER.economy.factories.civ")),
    card("Mil factories", fmtNum(at("economy.factories.mil")), srcOf("countries.GER.economy.factories.mil")),
    card("Dockyards", fmtNum(at("economy.factories.dockyard")), srcOf("countries.GER.economy.factories.dockyard")),
    card("Divisions", fmtNum(at("military.division_count")), srcOf("countries.GER.military.division_count")),
    card("Ships", fmtNum(at("military.num_ships")), srcOf("countries.GER.military.num_ships")),
    card("Manpower (avail)", fmtNum(at("military.manpower.available")), srcOf("countries.GER.military.manpower.available")),
    card("Fuel", fmtNum(at("economy.fuel.current")), srcOf("countries.GER.economy.fuel.current")),
    card("Stability", fmtPct(at("politics.stability")), srcOf("countries.GER.politics.stability")),
    card("War support", fmtPct(at("politics.war_support")), srcOf("countries.GER.politics.war_support")),
    card("Political power", fmtNum(at("politics.political_power")), srcOf("countries.GER.politics.political_power")),
    card("Casualties (killed)", fmtNum(at("military.casualties.killed")), srcOf("countries.GER.military.casualties.killed")),
    card("Technologies", fmtNum(at("research.tech_count")), srcOf("countries.GER.research.tech_count")),
    card("Focuses done", fmtNum(at("focus.completed_count")), srcOf("countries.GER.focus.completed_count")),
    card("States owned", fmtNum(at("derived.controlled_states")), "D"),
  ].join("");
  $("countryCards").innerHTML=cards;

  // time-series charts (collapse to a single point for one snapshot — Codex #6)
  lineChart("chEco","ecoCov",tag,[
    ["economy.factories.civ","Civ"],["economy.factories.mil","Mil"],
    ["economy.factories.dockyard","Dockyard"],["economy.factories.under_construction","Under constr."]]);
  lineChart("chFuel","fuelCov",tag,[["economy.fuel.current","Fuel"]]);
  lineChart("chMil","milCov",tag,[
    ["military.division_count","Divisions"],["military.num_ships","Ships"],
    ["military.casualties.killed","Casualties"]]);
  lineChart("chPol","polCov",tag,[
    ["politics.stability","Stability"],["politics.war_support","War support"]],{pct:true});
  lineChart("chRes","resCov",tag,[
    ["research.tech_count","Tech"],["focus.completed_count","Focuses"]]);

  // deep panels from the full snapshot (lazy) at the current frame
  const snap=await curSnapshot();
  const c=(snap && snap.countries && snap.countries[tag]) || null;
  renderCountryDeep(c, snap, tag);
}

function renderCountryDeep(c, snap, tag){
  const set=(id,html)=>{const el=$(id); if(el) el.innerHTML=html;};
  const PANELS=["cLaws","cFocus","cResearch","cDecisions","cStock","cDiplo",
                "cIntel","cLeaders","cProjects","cMios"];
  if(!c){ PANELS.forEach(id=>set(id,'<div class="empty">snapshot not loaded</div>')); return; }
  const pol=c.politics||{}, fc=c.focus||{}, res=c.research||{}, dip=c.diplomacy||{}, mil=c.military||{};
  // PER-FRAME ruling-party flag + ideology swatch from THIS frame's snapshot (so an
  // earlier frame never shows a later government; Codex review #1). Prepend the flag,
  // append the swatch + human_name to the header that renderCountry already drew.
  const bd=$("countryBadges");
  if(bd){
    const rp=pol.ruling_party;
    if(rp){
      const flag = (S.assets&&S.assets.has_assets) ? flagImg(tag, rp) : "";
      if(flag) bd.insertAdjacentHTML("afterbegin", flag);
      if(bd.querySelector(".ideoswatch")==null)
        bd.insertAdjacentHTML("beforeend",
          `<span class="chip ideoswatch" style="border-color:${ideoColor(rp)}">`+
          `<span class="swatch" style="background:${ideoColor(rp)}"></span>${esc(rp)}</span>`);
    }
    // human_name (MP persona, the new domain) — only when captured
    if(isCaptured(c.human_name) && bd.querySelector(".humanpersona")==null)
      bd.insertAdjacentHTML("beforeend",
        `<span class="chip humanpersona" title="MP lobby persona (human_name) ${esc(srcOf("countries.GER.human_name")||"S")}">👤 ${esc(c.human_name)}</span>`);
  }

  // laws & ideas
  const laws=pol.laws||{};
  let h='<div class="kv">'+
    kvRow("Ruling party",pol.ruling_party,"S")+
    kvRow("Economy law",laws.economy,"S")+
    kvRow("Trade law",laws.trade,"S")+
    kvRow("Conscription",laws.conscription,"S")+
    '</div>';
  if(isCaptured(pol.party_popularities)){
    h+='<div class="kv" style="margin-top:8px">'+Object.entries(pol.party_popularities)
      .map(([k,v])=>kvRow(k,fmtPct(v).replace(/<[^>]+>/g,"")==="not captured"?null:v*1,"S")).join("")+'</div>';
  } else { h+=`<div class="legendnote">party popularities ${badge("S")} ${NA}</div>`; }
  const ideas=(pol.ideas_other&&pol.ideas_other.length?pol.ideas_other:pol.ideas_all)||[];
  h+=`<div style="margin-top:8px"><div class="legendnote">spirits / ideas ${badge("S")}</div><div class="taglist">`+
    (ideas.length?ideas.slice(0,40).map(i=>`<span class="chip">${esc(i)}</span>`).join(""):NA)+`</div></div>`;
  set("cLaws",h);

  // focus
  let fh='<div class="kv">'+
    kvRow("Focus tree",fc.focus_tree,"S")+
    kvRow("Completed",fc.completed_count,"S")+
    kvRow("Current",fc.current,"S/M*")+
    '</div>';
  if(isCaptured(fc.progress)) fh+=`<div class="legendnote">progress on current: ${esc(String(fc.progress))} ${badge("S/M*")}</div>`;
  else fh+=`<div class="legendnote">progress ${badge("S/M*")} ${NA}</div>`;
  const comp=fc.completed||[];
  fh+=`<div style="margin-top:8px"><div class="legendnote">recently completed ${badge("S/D")}</div><div class="taglist">`+
    (comp.length?comp.slice(-24).map(f=>`<span class="chip">${esc(f)}</span>`).join(""):NA)+`</div></div>`;
  set("cFocus",fh);

  // research
  const techs=res.technologies||[], prog=res.in_progress||[], bon=res.research_bonuses||[];
  let rh='<div class="kv">'+kvRow("Tech count",res.tech_count,"S")+kvRow("Research slots",res.research_slots,"S")+'</div>';
  rh+=`<div class="legendnote" style="margin-top:8px">in progress ${badge("S/M*")} (progress% is M* — ${NA})</div>`;
  // in_progress carries only a raw token (e.g. "excavation3"); humanize for display,
  // keep the raw token in the chip title (Codex #2 — presentation transform, not invention).
  rh+='<div class="taglist">'+(prog.length?prog.map(p=>tokenChip(p.name,"S/M*")).join(""):NA)+'</div>';
  rh+=`<div class="legendnote" style="margin-top:8px">bonuses ${badge("S")}</div><div class="taglist">`+
    (bon.length?bon.slice(0,20).map(b=>`<span class="chip" title="game token: ${esc(String(b.name))}">${esc(humanizeToken(b.name))} ×${esc(String(b.bonus))}</span>`).join(""):NA)+'</div>';
  set("cResearch",rh);

  // decisions
  const dec=c.decisions||{};
  const dl=(arr,lbl)=>`<div class="legendnote">${lbl} ${badge("S")}</div><div class="taglist">`+
    ((arr&&arr.length)?arr.slice(0,30).map(x=>`<span class="chip">${esc(typeof x==="string"?x:(x.id||x.name||JSON.stringify(x)))}</span>`).join(""):'<span class="na-inline">none</span>')+`</div>`;
  set("cDecisions", dl(dec.active,"active")+dl(dec.timed,"timed")+dl(dec.missions,"missions"));

  // equipment stockpile (aggregate by archetype)
  const stock=mil.equipment_stockpile||[];
  if(stock.length){
    // only sum captured amounts; a null/missing amount is NOT silently added as 0
    // (Codex #5). Aggregates skip nulls — matching the no-fabrication rule.
    const agg={}; for(const e of stock){ const a=e.archetype||e.equip_type||"?";
      if(isCaptured(e.amount) && typeof e.amount==="number") agg[a]=(agg[a]||0)+e.amount; }
    const rows=Object.entries(agg).sort((x,y)=>y[1]-x[1]).slice(0,25);
    set("cStock",'<div class="kv">'+rows.map(([a,v])=>kvRow(a,Math.round(v),"S")).join("")+'</div>');
  } else set("cStock",`<div class="empty">stockpile ${NA}</div>`);

  // diplomacy
  // SCHEMA §3.7 documents wars[].against as a list; the store currently emits one
  // entry per opponent with a scalar `against`. Handle BOTH (Codex #7) — flatten to
  // a unique opponent set either way.
  const wars=dip.wars||[];
  const enemies=new Set();
  for(const w of wars){ const a=w&&w.against;
    if(Array.isArray(a)) a.forEach(x=>x&&enemies.add(x));
    else if(a) enemies.add(a); }
  const elist=[...enemies];
  let dh=`<div class="legendnote">at war with ${badge("S")}</div><div class="taglist">`+
    (elist.length?elist.map(t=>`<span class="chip">${esc(t)}</span>`).join(""):'<span class="na-inline">at peace</span>')+`</div>`;
  const fac=pol.faction;
  dh+=`<div class="kv" style="margin-top:8px">`+kvRow("Faction",fac?fac.name:null,"S")+
      kvRow("Faction leader",fac?(fac.is_leader?"yes":"no"):null,"S")+`</div>`;
  // world-scoped wars (diplomacy_world.wars) carry CASUALTIES + war score for this
  // tag's conflicts — surface them (the per-country wars[] only had the opponent set).
  const dw=(snap&&snap.diplomacy_world)||{}; const wmap=dw.wars||{};
  const myWars=Object.entries(wmap).filter(([_,w])=>w&&(w.a===tag||w.b===tag));
  if(myWars.length){
    dh+=`<div class="legendnote" style="margin-top:10px">wars — casualties &amp; score ${badge("S")}</div>`+
      '<div class="kv">';
    for(const [wid,w] of myWars.slice(0,12)){
      const us=(w.a===tag)?"a":"b", them=(w.a===tag)?w.b:w.a;
      const ourCas=w[us+"_casualties"], theirCas=w[(us==="a"?"b":"a")+"_casualties"];
      const score=w["war_score_"+us+"_vs_"+(us==="a"?"b":"a")];
      dh+=`<div class="key" title="${esc(wid)}">vs ${esc(String(them))} ${badge("S")}</div>`+
        `<div class="val">${isCaptured(ourCas)?fmtNum(ourCas):"n/c"} lost · `+
        `${isCaptured(theirCas)?fmtNum(theirCas):"n/c"} infl. · `+
        `score ${isCaptured(score)?(+score).toFixed(1):"n/c"}</div>`;
    }
    dh+='</div>';
  }
  set("cDiplo",dh);

  // ---- NEW PANEL: Intel agency (S) -------------------------------------
  const intel=c.intel||{};
  const ag=intel.agency;
  if(isCaptured(ag)){
    const cap=ag.capacity||{};
    let ih='<div class="kv">'+kvRow("Agency",ag.name,"S")+
      kvRow("Created",ag.is_created===true?"yes":(ag.is_created===false?"no":null),"S")+
      kvRow("Upgrade progress",isCaptured(ag.upgrade_progress)?ag.upgrade_progress+"%":null,"S")+
      kvRow("Current upgrade",ag.current_upgrade?humanizeToken(ag.current_upgrade):null,"S")+
      kvRow("Max operatives",cap.max_operatives,"S")+
      kvRow("Usable slots",cap.usable_slots,"S")+
      kvRow("Counter-intel",cap.defense,"S")+'</div>';
    const ups=ag.upgrades&&Object.keys(ag.upgrades).length?ag.upgrades:null;
    if(ups) ih+=`<div class="legendnote" style="margin-top:8px">upgrades ${badge("S")}</div><div class="taglist">`+
      Object.entries(ups).map(([k,v])=>`<span class="chip" title="game token: ${esc(k)}">${esc(humanizeToken(k))} ${esc(String(v))}</span>`).join("")+'</div>';
    const ops=(intel.operatives||[]);
    if(ops.length) ih+=`<div class="legendnote" style="margin-top:8px">operatives ${badge("S")}</div><div class="taglist">`+
      ops.slice(0,10).map(o=>`<span class="chip">${esc(o.name||("op "+o.op_id))}${isCaptured(o.skill)?(" · skill "+o.skill):""}</span>`).join("")+'</div>';
    set("cIntel",ih);
  } else set("cIntel",`<div class="empty">intelligence agency ${badge("S")} ${NA}</div>`);

  // ---- NEW PANEL: Leadership & commanders (S) — real portraits ---------
  const cmd=c.commanders||{};
  const lead=cmd.leader;
  let lh="";
  if(isCaptured(lead)){
    lh+=`<div class="leaderrow">${portraitImg(lead.portrait_gfx, lead.name)}`+
      `<div><div class="leadername">${esc(humanizeToken((lead.name||"").replace(/^[A-Z]{2,3}_/,"")))}</div>`+
      `<div class="legendnote">leader ${badge("S")}${lead.ideology?` · ${esc(lead.ideology)}`:""}</div>`+
      (lead.portrait_gfx?`<div class="assethandle" title="portrait sprite">${esc(lead.portrait_gfx)}</div>`:"")+
      `</div></div>`;
  } else lh+=`<div class="legendnote">country leader ${badge("S")} ${NA}</div>`;
  const cn=cmd.counts;
  if(isCaptured(cn)){
    lh+='<div class="kv" style="margin-top:8px">'+
      kvRow("Field marshals",cn.field_marshals,"S/D")+kvRow("Generals",cn.generals,"S/D")+
      kvRow("Admirals",cn.admirals,"S/D")+kvRow("Advisors",cn.advisors,"S/D")+'</div>';
  }
  // a few named commanders w/ portraits + assignment
  const milLeaders=[].concat(cmd.field_marshals||[],cmd.generals||[],cmd.admirals||[]);
  if(milLeaders.length){
    lh+=`<div class="legendnote" style="margin-top:8px">commanders ${badge("S")}</div><div class="portraitgrid">`+
      milLeaders.slice(0,8).map(g=>{
        const nm=humanizeToken((g.name||g.token||"").replace(/^[A-Z]{2,3}_/,""));
        const asn=g.assignment?esc(g.assignment.name||g.assignment.kind):'<span class="na-inline">reserve</span>';
        return `<div class="pcard">${portraitImg(g.portrait_gfx,g.name)}`+
          `<div class="pname">${esc(nm)}</div>`+
          `<div class="legendnote">${esc(g.role||"")}${isCaptured(g.level)?(" · skill "+g.level):""}</div>`+
          `<div class="legendnote">${asn}</div></div>`;
      }).join("")+'</div>';
  }
  set("cLeaders",lh);

  // ---- NEW PANEL: Special projects / extra (S) — the new domain --------
  const ex=c.extra;
  if(isCaptured(ex)){
    const sp=ex.special_projects||{};
    let xh='<div class="kv">'+
      kvRow("Nukes (total)",ex.nukes_total,"S")+
      kvRow("Nukes ready",ex.nukes_ready_total,"S")+
      kvRow("Projects started",sp.projects_started,"S")+
      kvRow("Projects completed",sp.projects_completed,"S")+
      kvRow("Convoys destroyed",ex.convoys_destroyed,"S")+
      kvRow("Aces",ex.ace_count,"S")+'</div>';
    const prog=(sp.active_programs||[]);
    if(prog.length) xh+=`<div class="legendnote" style="margin-top:8px">active programs ${badge("S")}</div><div class="taglist">`+
      prog.slice(0,12).map(p=>`<span class="chip">${esc(humanizeToken(p.building||"?"))}${p.province?(" @"+p.province):""}</span>`).join("")+'</div>';
    const aces=(ex.aces||[]);
    if(aces.length) xh+=`<div class="legendnote" style="margin-top:8px">aces ${badge("S")}</div><div class="taglist">`+
      aces.slice(0,10).map(a=>`<span class="chip">${esc(((a.name||"")+" "+(a.surname||"")).trim())}${a.callsign?` "${esc(a.callsign)}"`:""}</span>`).join("")+'</div>';
    set("cProjects",xh);
  } else set("cProjects",`<div class="empty">special projects / country extras ${badge("S")} ${NA}<br><span class="legendnote">(this snapshot predates the <code>extra</code> domain)</span></div>`);

  // ---- NEW PANEL: MIOs (S) ---------------------------------------------
  const mios=c.mios;
  if(Array.isArray(mios)&&mios.length){
    let mh=`<div class="legendnote">military-industrial organisations ${badge("S")}</div><div class="kv">`;
    for(const m of mios.slice(0,12)){
      mh+=`<div class="key" title="${esc(m.key||"")}">${esc(m.name||humanizeToken(m.key||"MIO"))} ${badge("S")}</div>`+
        `<div class="val">lvl ${isCaptured(m.level)?m.level:"n/c"}`+
        `${isCaptured(m.research_bonus)?` · bonus ×${m.research_bonus}`:""}`+
        `${isCaptured(m.funds)?` · funds ${fmtNum(m.funds)}`:""}</div>`;
    }
    mh+='</div>';
    set("cMios",mh);
  } else set("cMios",`<div class="empty">MIOs ${badge("S")} ${(Array.isArray(mios)&&!mios.length)?"none":NA}</div>`);
}

// ---- generic line chart from the compact series ---------------------------
const PALETTE=["#6ea8ff","#5ad1a0","#e0b15a","#e06a6a","#c79bff","#ff9bce","#7ee0d8","#b6c34a"];
function lineChart(canvasId, covId, tag, metrics, opts){
  opts=opts||{};
  const frames=S.series.frames, labels=frames.map(f=>f.date||"?");
  const single = frames.length<=1;
  const datasets=[]; let covParts=[];
  metrics.forEach(([key,label],i)=>{
    const arr=(S.series.series[tag]&&S.series.series[tag][key])||[];
    const data=arr.map(v=> (v===null||v===undefined)?null:(opts.pct?v*100:v));
    const cov=arr.filter(v=>v!==null&&v!==undefined).length;
    covParts.push(`${label}: ${cov}/${frames.length}`);
    datasets.push({label, data, borderColor:PALETTE[i%PALETTE.length],
      backgroundColor:PALETTE[i%PALETTE.length], spanGaps:true,
      pointRadius: single?5:2, tension:.2, borderWidth:2});
  });
  drawChart(canvasId,{type:"line",data:{labels,datasets},
    options:chartOpts({pct:opts.pct, single})});
  $(covId).innerHTML = `coverage — ${covParts.join(" · ")}` +
    (single?`  ·  single snapshot: charts show one point`:"");
}
function chartOpts(o){o=o||{};
  return {responsive:true,maintainAspectRatio:false,interaction:{mode:"index",intersect:false},
    plugins:{legend:{labels:{color:"#9fb2cc",boxWidth:12,font:{size:11}}},
      tooltip:{callbacks:{label:c=>{const v=c.parsed.y; if(v===null) return c.dataset.label+": not captured";
        return c.dataset.label+": "+(o.pct?v.toFixed(1)+"%":v.toLocaleString());}}}},
    scales:{x:{ticks:{color:"#8aa0bd",font:{size:10}},grid:{color:"#22293a"}},
      y:{ticks:{color:"#8aa0bd",font:{size:10}},grid:{color:"#22293a"},
        beginAtZero:!o.pct, suggestedMax:o.pct?100:undefined}}};
}
function drawChart(id,cfg){ if(S.charts[id]) S.charts[id].destroy();
  S.charts[id]=new Chart($(id).getContext("2d"),cfg); }

// ===========================================================================
// SCREEN 3 — MILITARY / DIVISIONS  (full snapshot, lazy)
// ===========================================================================
function buildMilScope(){
  const sel=$("milScope"); sel.innerHTML='<option value="__all__">All countries</option>';
  for(const t of S.series.tags){ const o=document.createElement("option"); o.value=t; o.textContent=t; sel.appendChild(o); }
  if(![...sel.options].some(o=>o.value===S.milScope)) S.milScope="__all__";
  sel.value=S.milScope;
}
async function renderMilitary(){
  const snap=await curSnapshot();
  const divs=(snap && snap.divisions)||[];
  const scope=S.milScope;
  const set=(scope==="__all__")?divs:divs.filter(d=>d.owner===scope);

  // cards
  const byTag={}; set.forEach(d=>{ if(d.owner) byTag[d.owner]=(byTag[d.owner]||0)+1; });
  const orgs=set.map(d=>d.organisation).filter(isCaptured);
  const strs=set.map(d=>d.strength).filter(isCaptured);
  const avg=a=>a.length? (a.reduce((x,y)=>x+y,0)/a.length):null;
  $("milCards").innerHTML=[
    card("Divisions", fmtNum(set.length), "M/S"),
    card("Countries with divisions", fmtNum(Object.keys(byTag).length), "D"),
    card("Avg organisation", isCaptured(avg(orgs))?avg(orgs).toFixed(1):NA, "M"),
    card("Avg strength", isCaptured(avg(strs))?avg(strs).toFixed(1):NA, "M/S"),
    card("Org coverage", `${orgs.length}/${set.length}`, "M"),
  ].join("");

  // div count by country (top 15)
  const tagRows=Object.entries(byTag).sort((a,b)=>b[1]-a[1]).slice(0,15);
  drawChart("chDivByTag",{type:"bar",
    data:{labels:tagRows.map(r=>r[0]),datasets:[{label:"Divisions",data:tagRows.map(r=>r[1]),
      backgroundColor:"#6ea8ff"}]},options:barOpts()});

  // roles
  // template_role is S; a null role is "not captured", not an invented category (Codex #6)
  const roleCount={}; set.forEach(d=>{const r=isCaptured(d.template_role)?d.template_role:"role not captured";roleCount[r]=(roleCount[r]||0)+1;});
  const roleRows=Object.entries(roleCount).sort((a,b)=>b[1]-a[1]);
  drawChart("chRoles",{type:"bar",
    data:{labels:roleRows.map(r=>r[0]),datasets:[{label:"Templates",data:roleRows.map(r=>r[1]),
      backgroundColor:"#5ad1a0"}]},options:barOpts()});

  // org / strength histograms
  drawChart("chOrg",histCfg(orgs,0,100,10,"#e0b15a","Organisation"));
  drawChart("chStr",histCfg(strs,0,Math.max(100,...(strs.length?strs:[100])),10,"#c79bff","Strength"));

  // table
  renderDivTable(set);

  // OOB command tree (order_of_battle): army_groups -> armies -> divisions,
  // fleets -> task_forces, air_wings -> planes. Scope-aware. (Codew #3 surface)
  renderOOB(snap, scope);
}

// ---- OOB command tree ------------------------------------------------------
function renderOOB(snap, scope){
  const el=$("oobTree"); if(!el) return;
  const oob=(snap && snap.order_of_battle)||{};
  const want = t => scope==="__all__" || t===scope;
  const ags=(oob.army_groups||[]).filter(a=>want(a.owner));
  const armies=(oob.armies||[]).filter(a=>want(a.owner));
  const fleets=(oob.fleets||[]).filter(f=>want(f.owner));
  const wings=(oob.air_wings||[]).filter(w=>want(w.owner));
  const byc=oob.by_country||{};
  if(!ags.length && !armies.length && !fleets.length && !wings.length){
    el.innerHTML=`<div class="empty">order-of-battle command tree ${badge("M/S")} `+
      ((snap&&snap.order_of_battle)?"— nothing for this scope/frame":NA)+`</div>`;
    return;
  }
  // resolve unit_leader_id -> commander name from the country's commanders block
  const cmds={}; const cc=(snap&&snap.countries)||{};
  for(const [tg,c] of Object.entries(cc)){
    for(const g of [].concat((c.commanders||{}).field_marshals||[],(c.commanders||{}).generals||[],(c.commanders||{}).admirals||[])){
      if(g.unit_leader_id!=null) cmds[tg+":"+g.unit_leader_id]=g.name;
    }
  }
  const leaderName=(owner,ref)=>{ // ref like "12:4713"
    if(!ref) return "";
    const id=String(ref).split(":")[0];
    const nm=cmds[owner+":"+id];
    return nm?` · <span class="leadtag">${esc(humanizeToken(nm.replace(/^[A-Z]{2,3}_/,"")))}</span>`:"";
  };
  const armyById={}; armies.forEach(a=>armyById[a.id]=a);
  let h="";
  // group army_groups by owner
  const owners=[...new Set([...ags,...armies,...fleets,...wings].map(x=>x.owner))].sort();
  for(const owner of owners){
    const r=byc[owner]||{};
    h+=`<div class="oobowner"><span class="chip">${esc(owner)}</span>`+
      `<span class="legendnote">${fmtNum(r.armies)} armies · ${fmtNum(r.divisions_in_armies)} divisions · `+
      `${fmtNum(r.fleets)} fleets · ${fmtNum(r.ships)} ships · ${fmtNum(r.air_wings)} wings · ${fmtNum(r.planes)} planes ${badge("S/D")}</span></div>`;
    // army groups for this owner
    for(const ag of ags.filter(a=>a.owner===owner)){
      h+=`<details class="oobnode" open><summary>${esc(ag.name||ag.id)}${leaderName(owner,ag.leader)} `+
        `<span class="legendnote">army group · ${fmtNum(ag.army_refs_resolved)} armies</span></summary>`;
      for(const ref of (ag.army_refs||[])){ const a=armyById[ref];
        if(a) h+=`<div class="oobleaf">▸ ${esc(a.name||a.id)}${leaderName(owner,a.leader)} `+
          `<span class="legendnote">${fmtNum(a.member_count)} divisions</span></div>`;
      }
      h+=`</details>`;
    }
    // armies NOT under a listed army group
    const grouped=new Set(); ags.filter(a=>a.owner===owner).forEach(ag=>(ag.army_refs||[]).forEach(r=>grouped.add(r)));
    const loose=armies.filter(a=>a.owner===owner && !grouped.has(a.id));
    for(const a of loose){
      h+=`<div class="oobleaf indep">▸ ${esc(a.name||a.id)}${leaderName(owner,a.leader)} `+
        `<span class="legendnote">independent army · ${fmtNum(a.member_count)} divisions</span></div>`;
    }
    // fleets
    for(const f of fleets.filter(x=>x.owner===owner)){
      const tf=(f.task_forces||[]); const ships=tf.reduce((n,t)=>n+((t.ships||[]).length),0);
      h+=`<details class="oobnode"><summary>${esc(f.name||f.id)}${leaderName(owner,f.leader)} `+
        `<span class="legendnote">fleet · ${tf.length} task force(s) · ${ships} ships</span></summary>`;
      for(const t of tf) h+=`<div class="oobleaf">▸ ${esc(t.name||t.id)} `+
        `<span class="legendnote">${(t.ships||[]).length} ships${t.mission?(" · "+esc(humanizeToken(t.mission))):""}</span></div>`;
      h+=`</details>`;
    }
    // air wings (summary line)
    const ow=wings.filter(x=>x.owner===owner);
    if(ow.length){
      const planes=ow.reduce((n,w)=>n+(w.count||0),0);
      h+=`<div class="oobleaf air">✈ ${ow.length} air wing(s) · ${fmtNum(planes)} planes `+
        `<span class="legendnote">${badge("S")}</span></div>`;
    }
  }
  el.innerHTML=h;
}
function barOpts(){ return {responsive:true,maintainAspectRatio:false,
  plugins:{legend:{display:false}},
  scales:{x:{ticks:{color:"#8aa0bd",font:{size:10}},grid:{display:false}},
    y:{beginAtZero:true,ticks:{color:"#8aa0bd",font:{size:10}},grid:{color:"#22293a"}}}}; }
function histCfg(vals,lo,hi,bins,color,label){
  const edges=[],counts=new Array(bins).fill(0),w=(hi-lo)/bins;
  for(let i=0;i<bins;i++) edges.push(`${Math.round(lo+i*w)}–${Math.round(lo+(i+1)*w)}`);
  vals.forEach(v=>{let b=Math.floor((v-lo)/w); if(b<0)b=0; if(b>=bins)b=bins-1; counts[b]++;});
  return {type:"bar",data:{labels:edges,datasets:[{label,data:counts,backgroundColor:color}]},options:barOpts()};
}
let divSort={key:"owner",dir:1};
function renderDivTable(divs){
  const cols=[["owner","Owner","M/S"],["template_name","Template","M/S"],["template_role","Role","S"],
    ["location_province","Prov","M/S"],["state_id","State","M/D"],
    ["organisation","Org","M"],["strength","Str","M/S"],["div_id","ID","M/S"]];
  const thead=$("divTable").tHead, tbody=$("divTable").tBodies[0];
  thead.innerHTML="<tr>"+cols.map(c=>`<th data-k="${c[0]}">${esc(c[1])} ${badge(c[2])}</th>`).join("")+"</tr>";
  thead.querySelectorAll("th").forEach(th=>th.onclick=()=>{
    const k=th.dataset.k; divSort.dir=(divSort.key===k)?-divSort.dir:1; divSort.key=k; renderDivTable(divs);});
  const sorted=divs.slice().sort((a,b)=>{
    const x=a[divSort.key],y=b[divSort.key];
    if(x===y) return 0; if(x===null||x===undefined) return 1; if(y===null||y===undefined) return -1;
    return (x>y?1:-1)*divSort.dir;});
  const cap=2000; // guard the DOM on 1654+ rows
  const rows=sorted.slice(0,cap).map(d=>"<tr>"+cols.map(c=>{
    const v=d[c[0]]; const num=(c[0]==="organisation"||c[0]==="strength"||c[0]==="location_province"||c[0]==="state_id"||c[0]==="div_id");
    if(!isCaptured(v)) return `<td class="na">n/c</td>`;
    const disp = (typeof v==="number" && (c[0]==="organisation"||c[0]==="strength"))? v.toFixed(1):esc(String(v));
    return `<td class="${num?"num":""}">${disp}</td>`;
  }).join("")+"</tr>").join("");
  tbody.innerHTML = rows || `<tr><td colspan="${cols.length}" class="empty">no divisions for this scope/frame</td></tr>`;
  $("divTblCount").textContent = `${divs.length} division${divs.length===1?"":"s"}`+(divs.length>cap?` (showing ${cap})`:"");
}

// ===========================================================================
// SCREEN 4 — COMBAT LOG
// ===========================================================================
async function renderCombat(){
  const snap=await curSnapshot();
  const combats=(snap && snap.combats)||[];
  // combats is canonical M* (SCHEMA §7) — the domain may come from memory OR save.
  // "captured" means SOME channel listed the combats domain this snapshot. When it
  // was NOT captured at all, a count of 0 would be a FABRICATED zero, so the cards
  // must say "not captured" rather than 0 (Codex review #1).
  const prov=(snap && snap.provenance)||{};
  const memDom=(prov.memory&&prov.memory.domains)||[];
  const savDom=(prov.save&&prov.save.domains)||[];
  const captured = memDom.includes("combats") || savDom.includes("combats");
  $("combatNote").innerHTML = `Active engagements at the snapshot instant `+badge("M*")+
    ` — the combats domain is memory-planned/inferred and active-only (no stored history). `+
    `Each engagement's lifecycle is reconstructed from the event timeline below.`+
    (captured?"":` <b>(combats domain not captured this snapshot.)</b>`);

  // not captured -> null (so the card shows "not captured", not a fabricated 0)
  const kindCount = pred => captured ? combats.filter(pred).length : null;
  $("combatCards").innerHTML=[
    card("Active combats", fmtNum(captured?combats.length:null), "M*"),
    card("Land", fmtNum(kindCount(c=>c.kind==="land")), "M*"),
    card("Naval", fmtNum(kindCount(c=>c.kind==="naval")), "M*"),
    card("Air", fmtNum(kindCount(c=>c.kind==="air")), "M*"),
  ].join("");

  // table — show captured fields; missing ones render "n/c" (Codex #5: don't blanket-NA non-empty combats)
  const cols=[["combat_id","Combat"],["kind","Kind"],["location_province","Prov"],
    ["terrain","Terrain"],["progress","Progress"],["combat_width","Width"],
    ["__atk","Attackers"],["__def","Defenders"]];
  const thead=$("combatTable").tHead, tbody=$("combatTable").tBodies[0];
  thead.innerHTML="<tr>"+cols.map(c=>`<th>${esc(c[1])}</th>`).join("")+"</tr>";
  const cnt=arr=>Array.isArray(arr)?arr.length:null;
  tbody.innerHTML = combats.length? combats.map(c=>"<tr>"+cols.map(col=>{
    let v;
    if(col[0]==="__atk") v=cnt(c.attackers);
    else if(col[0]==="__def") v=cnt(c.defenders);
    else v=c[col[0]];
    if(col[0]==="progress" && isCaptured(v)) return `<td class="num">${(+v).toFixed(1)}</td>`;
    if(!isCaptured(v)) return `<td class="na">n/c</td>`;
    const num=(col[0]==="location_province"||col[0]==="__atk"||col[0]==="__def");
    return `<td class="${num?"num":""}">${esc(String(v))}${(col[0]==="__atk"||col[0]==="__def")?" units":""}</td>`;
  }).join("")+"</tr>").join("")
    : `<tr><td colspan="${cols.length}" class="empty">${captured?"no active combats this snapshot":"combats not captured this snapshot"}</td></tr>`;

  // PROXY sidecars (naval_air region control + air superiority): region-granularity
  // LIVE estimates that carry their OWN confidence label (never "exact" — Codex #4).
  renderProxySidecars(snap);

  // lifecycle from timeline combat_* events
  const ce=(S.timeline||[]).filter(e=>e.type==="combat_started"||e.type==="combat_ended");
  $("combatLifecycle").innerHTML = ce.length? ce.map(e=>
    `<div class="ev"><span class="when">${esc(e.date||"")}</span>`+
    `<span class="type">${esc(e.type)}</span>`+
    `<span class="desc">combat ${esc(e.key||"?")}${e.detail?.casualties?(" · casualties "+esc(String(e.detail.casualties))):""}</span>`+
    `<span class="tags">${badge(e.source)}<span class="conf ${esc(e.confidence)}">${esc(e.confidence)}</span></span></div>`).join("")
    : `<div class="empty">No combat lifecycle events in the timeline.<br>`+
      `(Combat start/end are inferred from high-frequency memory sampling — not yet emitted by the diff engine for this replay.)</div>`;
}

// PROXY estimates (naval_air air-superiority + supply) — strictly badged as
// region/division PROXY, never "exact". The confidence travels with the data.
function renderProxySidecars(snap){
  const el=$("proxyPanel"); if(!el) return;
  const na=(snap&&snap.naval_air)||{}, sup=(snap&&snap.supply)||{};
  const haveNA = isCaptured(na.air_superiority)||isCaptured(na.region_control);
  const haveSup= isCaptured(sup.by_province);
  if(!haveNA && !haveSup){
    el.innerHTML=`<div class="empty">live naval/air &amp; supply PROXY sidecars `+
      `${badge("M")} not captured this snapshot <span class="legendnote">(memory channel absent, or this snapshot predates the sidecars)</span></div>`;
    return;
  }
  let h="";
  // the PROXY confidence is rendered VERBATIM from the snapshot — never defaulted to a
  // label the data didn't carry (Codex review #2). Absent confidence => "not captured".
  const confBadge=conf=> conf ? proxyBadge(conf)
    : `<span class="srcbadge" title="confidence not captured">~conf n/c</span>`;
  if(haveNA){
    const conf=na.confidence;
    h+=`<div class="legendnote">air superiority / sea-zone control — region granularity ${confBadge(conf)}</div><div class="kv">`;
    const air=na.air_superiority||{};
    for(const [rid,v] of Object.entries(air).slice(0,12)){
      const ctrl=(v&&v.controller)||null;
      h+=`<div class="key">region ${esc(rid)} ${confBadge(conf)}</div><div class="val${ctrl?"":" na"}">${ctrl?esc(ctrl):"contested / n/c"}</div>`;
    }
    h+='</div>';
  }
  if(haveSup){
    const conf=sup.confidence;
    h+=`<div class="legendnote" style="margin-top:10px">division-province supply ${confBadge(conf)} <b>estimate, not the exact tooltip number</b></div><div class="kv">`;
    for(const [pid,v] of Object.entries(sup.by_province||{}).slice(0,12)){
      h+=`<div class="key">prov ${esc(pid)} ${confBadge(conf)}</div>`+
        `<div class="val">${fmtNum(v.current)} / ${fmtNum(v.capacity)} cap · ${fmtNum(v.divisions)} div</div>`;
    }
    h+='</div>';
  }
  el.innerHTML=h;
}

// ===========================================================================
// SCREEN 5 — EVENT TIMELINE
// ===========================================================================
function buildTimelineFilters(){
  const types=[...new Set((S.timeline||[]).map(e=>e.type))].sort();
  const tsel=$("evType"); tsel.innerHTML='<option value="">All types</option>'+
    types.map(t=>`<option value="${esc(t)}">${esc(t)} (${(S.timeline||[]).filter(e=>e.type===t).length})</option>`).join("");
  const tags=new Set();
  (S.timeline||[]).forEach(e=>{ [e.subject,e.object].forEach(x=>{ if(x&&TAGRE.test(x)) tags.add(x); }); });
  const csel=$("evCountry"); csel.innerHTML='<option value="">All countries</option>'+
    [...tags].sort().map(t=>`<option value="${esc(t)}">${esc(t)}</option>`).join("");
}
function renderTimeline(){
  const tFilter=$("evType").value, cFilter=$("evCountry").value, upto=$("evUpToFrame").checked;
  const curHour=S.series.frames[S.frame]?.hour_index;
  let evs=(S.timeline||[]).slice();
  if(tFilter) evs=evs.filter(e=>e.type===tFilter);
  if(cFilter) evs=evs.filter(e=>e.subject===cFilter||e.object===cFilter||e.from===cFilter||e.to===cFilter);
  if(upto && curHour!=null) evs=evs.filter(e=>e.hour_index==null||e.hour_index<=curHour);

  $("evCount").textContent = `${evs.length} of ${(S.timeline||[]).length} event${(S.timeline||[]).length===1?"":"s"}`;
  const feed=$("eventFeed");
  if(!evs.length){ feed.innerHTML=`<div class="empty">`+
    ((S.timeline||[]).length? "No events match the filter."
      : "No timeline events.<br>A single-snapshot replay has no adjacent pair to diff — add more saves (same game_id) and re-run build_replay.py.")+`</div>`;
    return; }
  feed.innerHTML = evs.map(e=>{
    return `<div class="ev"><span class="when">${esc(e.date||"?")}</span>`+
      `<span class="type">${esc(e.type)}</span>`+
      `<span class="desc">${describeEvent(e)}</span>`+
      `<span class="tags">${badge(e.source)}<span class="conf ${esc(e.confidence||"")}" title="confidence">${esc(e.confidence||"")}</span></span></div>`;
  }).join("");
}
function describeEvent(e){
  const s=e.subject?`<b>${esc(e.subject)}</b>`:"", o=e.object?esc(e.object):"", k=e.key!=null?esc(String(e.key)):"";
  const fromto=(e.from!=null||e.to!=null)?` ${e.from!=null?esc(String(e.from)):"∅"} → ${e.to!=null?esc(String(e.to)):"∅"}`:"";
  switch(e.type){
    case "state_owner_change": return `state ${k} owner${fromto}`;
    case "state_controller_change": return `state ${k} controller${fromto}`;
    case "province_controller_change": return `province ${k} controller${fromto}`;
    case "division_created": return `${s} built division ${k}`;
    case "division_destroyed": return `lost division ${k} (was ${esc(String(e.from||"?"))})`;
    case "division_moved": return `division ${k} moved${fromto}`;
    case "division_template_changed": return `division ${k} re-templated${fromto}`;
    case "focus_completed": return `${s} completed focus <i>${k}</i>`;
    case "law_changed": return `${s} ${k} law${fromto}`;
    case "idea_gained": return `${s} gained idea <i>${k}</i>`;
    case "idea_lost": return `${s} lost idea <i>${k}</i>`;
    case "government_changed": return `${s} government${fromto}`;
    case "tech_researched": return `${s} researched <i>${k}</i>${e.to!=null?` (lvl ${esc(String(e.to))})`:""}`;
    case "tech_lost": return `${s} lost tech <i>${k}</i>`;
    case "research_slot_gained": return `${s} research slots${fromto}`;
    case "war_declared": return `${s} ⚔ war on ${o}`;
    case "peace": return `${s} ⟷ peace with ${o}`;
    case "faction_joined": return `${s} joined faction ${o}`;
    case "faction_left": return `${s} left faction ${o}`;
    case "subject_created": return `${s} became subject of ${o}${e.to?` (${esc(String(e.to))})`:""}`;
    case "subject_freed": return `${s} freed from ${o}`;
    case "autonomy_changed": return `${s} autonomy${fromto}`;
    case "capitulated": return `${s} capitulated`;
    case "production_milestone": return `${s} production milestone: ${k}`;
    case "factory_milestone": return `${s} ${k} factories → ${esc(String(e.to))}`;
    case "combat_started": return `combat ${k} started`;
    case "combat_ended": return `combat ${k} ended`;
    default: return `${s} ${esc(e.type)} ${k}${fromto}`;
  }
}

// ---- UI wiring -------------------------------------------------------------
function wireUI(){
  document.querySelectorAll("#tabs button").forEach(b=>b.onclick=()=>switchScreen(b.dataset.screen));
  $("scrub").oninput=e=>{ stopPlay(); setFrame(+e.target.value); };
  $("playBtn").onclick=play;
  $("gameSel").onchange=async e=>{ stopPlay(); const [r,g]=e.target.value.split("/");
    show($("loading")); $("loading").textContent="loading "+g+"…";
    await loadGame(r,g); switchScreen(S.curScreen); hide($("loading")); };
  $("countrySel").onchange=e=>{ S.curCountry=e.target.value; renderCountry(); };
  $("milScope").onchange=e=>{ S.milScope=e.target.value; renderMilitary(); };
  $("evType").onchange=renderTimeline; $("evCountry").onchange=renderTimeline;
  $("evUpToFrame").onchange=renderTimeline;
}

window.addEventListener("DOMContentLoaded", boot);
