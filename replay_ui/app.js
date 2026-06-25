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

// ---- state ----------------------------------------------------------------
const S = {
  games:null, root:null, gameId:null,
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
  $("countryBadges").innerHTML =
    `<span class="chip">${esc(tag)}</span>`+ humanChip +
    (id.is_major===true?`<span class="chip">major</span>`:"")+
    (id.ruling_party?`<span class="chip">${esc(id.ruling_party)}</span>`:"");

  // cards from the series at the CURRENT frame (no-fabrication aware)
  const at=k=>S.series.series[tag]?.[k]?.[S.frame];
  const cards=[
    card("Civ factories", fmtNum(at("economy.factories.civ")), srcOf("countries.GER.economy.factories.civ")),
    card("Mil factories", fmtNum(at("economy.factories.mil")), srcOf("countries.GER.economy.factories.mil")),
    card("Dockyards", fmtNum(at("economy.factories.dockyard")), srcOf("countries.GER.economy.factories.dockyard")),
    card("Divisions", fmtNum(at("military.division_count")), srcOf("countries.GER.military.division_count")),
    card("Ships", fmtNum(at("military.num_ships")), srcOf("countries.GER.military.num_ships")),
    card("Manpower (avail)", fmtNum(at("military.manpower.available")), srcOf("countries.GER.military.manpower.available")),
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
  renderCountryDeep(c);
}

function renderCountryDeep(c){
  const set=(id,html)=>{$(id).innerHTML=html;};
  if(!c){ ["cLaws","cFocus","cResearch","cDecisions","cStock","cDiplo"]
      .forEach(id=>set(id,'<div class="empty">snapshot not loaded</div>')); return; }
  const pol=c.politics||{}, fc=c.focus||{}, res=c.research||{}, dip=c.diplomacy||{}, mil=c.military||{};

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
  rh+='<div class="taglist">'+(prog.length?prog.map(p=>`<span class="chip">${esc(p.name)}</span>`).join(""):NA)+'</div>';
  rh+=`<div class="legendnote" style="margin-top:8px">bonuses ${badge("S")}</div><div class="taglist">`+
    (bon.length?bon.slice(0,20).map(b=>`<span class="chip">${esc(b.name)} ×${esc(String(b.bonus))}</span>`).join(""):NA)+'</div>';
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
  set("cDiplo",dh);
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
