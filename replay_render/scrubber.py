"""HTML time-lapse scrubber generator.

Emits a single self-contained HTML file with a timeline slider that redraws the
map at each timestamp, entirely client-side. It mirrors ``map_view/app.js``'s
recolor pipeline (decode the province-id PNG ONCE into a Uint32 raster, then a
single packed-colour pixel loop per repaint) and extends app.js's documented
``TODO: SCRUBBER`` to a real multi-snapshot timeline.

Two asset-loading modes:
  * ``embed_raster=False`` (default): the page fetches the province-id PNG and
    ``province_to_state.json`` from the repo-relative ``/map_pipeline/assets/``
    path -- exactly like map_view, so serve it from the same stdlib loopback
    server (``map_view/serve.py`` roots at the repo). Smallest HTML.
  * ``embed_raster=True``: the id-PNG + province_to_state are inlined as base64
    / JSON so the file is portable (open from anywhere via a tiny server). Bigger.

The snapshot SERIES (per-timestamp {state:{owner,controller}}) and the optional
per-frame division markers are always embedded as JSON. Colours come from
``country_colors.json`` (embedded). Styling matches the dashboard theme
(dark #10141c / panels #161b26 / text #dfe6f0, Segoe UI).
"""

from __future__ import annotations

import base64
import json
from pathlib import Path
from typing import Dict, List, Optional

from .frames import Frame


def _assets_dir() -> Path:
    return Path(__file__).resolve().parent.parent / "map_pipeline" / "assets"


def _frame_to_snapshot(fr: Frame) -> dict:
    """Frame -> the JS snapshot shape.

    {date, ownership:{state:{owner,controller}}, pctrl:{prov:TAG}, divisions:[...]}
    ``pctrl`` (province-level control, SCHEMA §5/§8) is emitted only when the
    frame carries it; the JS overrides the state controller per province with it.
    """
    ownership: Dict[str, dict] = {}
    ctrl = fr.controller or {}
    for sid, own in fr.owner.items():
        if not isinstance(own, str):
            # literal colour -> synthesise a pseudo-tag so JS can look it up
            own_tag = f"#{own[0]:02x}{own[1]:02x}{own[2]:02x}"
        else:
            own_tag = own
        c = ctrl.get(sid, own_tag)
        ownership[str(sid)] = {"owner": own_tag, "controller": c}
    snap = {"date": fr.timestamp, "ownership": ownership}
    if fr.control_by_prov:
        snap["pctrl"] = {str(p): t for p, t in fr.control_by_prov.items()}
    if fr.divisions:
        snap["divisions"] = [{"p": d.province_id, "o": d.owner,
                              "s": round(d.strength, 1)} for d in fr.divisions]
    return snap


def _literal_colors(frames: List[Frame]) -> Dict[str, list]:
    """Collect any literal-colour pseudo-tags used by the frames (demo path)."""
    extra: Dict[str, list] = {}
    for fr in frames:
        for own in fr.owner.values():
            if not isinstance(own, str):
                tag = f"#{own[0]:02x}{own[1]:02x}{own[2]:02x}"
                extra[tag] = [int(own[0]), int(own[1]), int(own[2])]
    return extra


def build_scrubber_html(frames: List[Frame],
                        out_path: Path,
                        title: str = "Battle Plan time-lapse",
                        embed_raster: bool = False,
                        show_divisions: bool = False,
                        half_res: bool = True) -> Path:
    """Write a self-contained HTML scrubber for ``frames`` to ``out_path``."""
    frames = list(frames)
    assets = _assets_dir()

    colors = json.loads((assets / "country_colors.json").read_text(encoding="utf-8"))
    colors.update(_literal_colors(frames))

    snaps = [_frame_to_snapshot(fr) for fr in frames]

    cfg = {
        "title": title,
        "snaps": snaps,
        "colors": colors,
        "showDivisions": show_divisions,
        "embedRaster": embed_raster,
        "pngUrl": ("/map_pipeline/assets/"
                   + ("province_ids_half.png" if half_res else "province_ids.png")),
        "p2sUrl": "/map_pipeline/assets/province_to_state.json",
    }

    if embed_raster:
        png_name = "province_ids_half.png" if half_res else "province_ids.png"
        png_b64 = base64.b64encode((assets / png_name).read_bytes()).decode("ascii")
        cfg["pngDataUri"] = "data:image/png;base64," + png_b64
        cfg["p2s"] = json.loads((assets / "province_to_state.json").read_text(encoding="utf-8"))

    html = _TEMPLATE.replace("/*__CONFIG__*/null",
                             json.dumps(cfg, separators=(",", ":")))
    out_path = Path(out_path)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(html, encoding="utf-8")
    return out_path


# --------------------------------------------------------------------------
# The HTML/JS template. Recolor mirrors map_view/app.js; the scrubber wires the
# documented SNAPS timeline (app.js TODO: SCRUBBER) for real.
# --------------------------------------------------------------------------

_TEMPLATE = r"""<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Battle Plan time-lapse</title>
<style>
  :root{--bg:#10141c;--panel:#161b26;--border:#2a3344;--text:#dfe6f0;--accent:#6ea8ff}
  html,body{margin:0;background:var(--bg);color:var(--text);font-family:Segoe UI,Arial,sans-serif}
  header{padding:10px 16px;background:var(--panel);border-bottom:1px solid var(--border);
         display:flex;align-items:center;gap:16px;flex-wrap:wrap}
  header h1{font-size:16px;margin:0;font-weight:600}
  .ctrls{display:flex;align-items:center;gap:10px;flex-wrap:wrap}
  button,select{background:#222b3a;color:var(--text);border:1px solid #38465e;
         border-radius:5px;padding:5px 10px;cursor:pointer}
  button:hover{border-color:var(--accent)}
  #scrub{width:min(60vw,640px)}
  #date{font-variant-numeric:tabular-nums;min-width:120px;color:var(--accent)}
  main{display:flex;gap:12px;padding:12px;align-items:flex-start;flex-wrap:wrap}
  #mapwrap{flex:3;min-width:520px;background:var(--panel);border:1px solid var(--border);
           border-radius:8px;padding:8px;overflow:auto}
  canvas{width:100%;height:auto;image-rendering:pixelated;display:block}
  aside{flex:1;min-width:240px;display:flex;flex-direction:column;gap:12px}
  .panel{background:var(--panel);border:1px solid var(--border);border-radius:8px;padding:12px}
  .panel h2{font-size:13px;margin:0 0 8px;color:#9fb2cc;text-transform:uppercase;letter-spacing:.04em}
  ul{list-style:none;margin:0;padding:0}
  li{display:flex;align-items:center;gap:8px;font-size:13px;padding:2px 0}
  .sw{width:14px;height:14px;border-radius:3px;border:1px solid #0006;flex:none}
  #status{font-size:12px;color:#8aa0bd;padding:0 16px 8px}
  label.tog{font-size:13px;display:flex;align-items:center;gap:5px;cursor:pointer}
  #tip{position:fixed;pointer-events:none;background:#0b0e14;border:1px solid var(--border);
       padding:4px 7px;border-radius:4px;font-size:12px;display:none;z-index:9}
</style></head>
<body>
<header>
  <h1 id="title">Battle Plan time-lapse</h1>
  <div class="ctrls">
    <button id="play">&#9658; Play</button>
    <input id="scrub" type="range" min="0" max="0" value="0" step="1">
    <span id="date">--</span>
    <select id="mode"><option value="owner">Owner</option><option value="control">Control</option></select>
    <label class="tog"><input type="checkbox" id="fronts" checked> Front lines</label>
    <label class="tog"><input type="checkbox" id="divs"> Divisions</label>
  </div>
</header>
<div id="status">loading...</div>
<main>
  <div id="mapwrap"><canvas id="map"></canvas></div>
  <aside>
    <div class="panel"><h2 id="legtitle">Top owners</h2><ul id="legend"></ul></div>
    <div class="panel"><h2>Selected</h2><div id="info" style="font-size:13px;line-height:1.5">Hover or click a province.</div></div>
  </aside>
</main>
<div id="tip"></div>
<script>
const CFG = /*__CONFIG__*/null;
const WATER=[38,50,66], NO_OWNER=[120,120,120], UNKNOWN=[70,60,60], FRONT=[12,12,14];
const M={width:0,height:0,idRaster:null,stateByProvince:null,colors:CFG.colors,
         snaps:CFG.snaps,frame:0,mode:'owner',fronts:true,divs:CFG.showDivisions,
         packed:{},ctx:null,img:null,canvas:null,selProv:null,raf:false,playing:false,timer:null,
         primary:null,alt:null,contested:null,ctrlCode:null};
const $=id=>document.getElementById(id);
const setStatus=m=>{const e=$('status'); if(e)e.textContent=m;};
function pack(c){return (0xff<<24|c[2]<<16|c[1]<<8|c[0])>>>0;}
function clamp8(v){return v<0?0:v>255?255:v|0;}

function decodeRaster(img){
  const w=img.naturalWidth,h=img.naturalHeight;
  const c=document.createElement('canvas');c.width=w;c.height=h;
  const ctx=c.getContext('2d',{willReadFrequently:true});ctx.imageSmoothingEnabled=false;
  ctx.drawImage(img,0,0);
  const d=ctx.getImageData(0,0,w,h).data,n=w*h,ids=new Uint32Array(n);
  for(let i=0,p=0;i<n;i++,p+=4) ids[i]=d[p]|(d[p+1]<<8)|(d[p+2]<<16);
  return {width:w,height:h,idRaster:ids};
}
function loadImage(src){return new Promise((res,rej)=>{const im=new Image();
  im.onload=()=>res(im);im.onerror=()=>rej(new Error('img '+src));im.src=src;});}

function buildStateByProvince(p2s,maxId){
  const a=new Int32Array(maxId+1).fill(-1);
  for(const k in p2s){const v=+k; if(v>=0&&v<=maxId)a[v]=p2s[k]|0;} return a;}

// per-province colour tables (mirror app.js buildColorTables + a controller code
// array for front-line derivation). owner-fill + contested hatch toward controller.
function buildTables(){
  const sb=M.stateByProvince,maxId=sb.length-1;
  const primary=new Uint32Array(maxId+1),alt=new Uint32Array(maxId+1);
  const contested=new Uint8Array(maxId+1),ctrlCode=new Int32Array(maxId+1).fill(-1);
  const wP=pack(WATER),nP=pack(NO_OWNER),uP=pack(UNKNOWN);
  const frame=M.snaps[M.frame], snap=frame.ownership, pctrl=frame.pctrl||null, codes={};
  const ctrlMode=(M.mode==='control');
  primary[0]=wP;alt[0]=wP;
  for(let prov=1;prov<=maxId;prov++){
    const st=sb[prov];
    if(st<0){primary[prov]=wP;alt[prov]=wP;continue;}
    const o=snap[st];
    if(!o||!o.owner){primary[prov]=nP;alt[prov]=nP;continue;}
    // province-level control (SCHEMA §5/§8) overrides the state controller
    const ct=(pctrl&&pctrl[prov])?pctrl[prov]:(o.controller||o.owner);
    const oc=M.packed[o.owner]||uP, cc=M.packed[ct]||uP;
    if(ctrlMode){
      primary[prov]=cc;
      if(ct!==o.owner){contested[prov]=1;const orgb=M.colors[o.owner]||UNKNOWN;
        alt[prov]=pack([clamp8(orgb[0]*.55),clamp8(orgb[1]*.55),clamp8(orgb[2]*.55)]);}
      else alt[prov]=cc;
    } else {
      primary[prov]=oc;
      if(ct!==o.owner){contested[prov]=1;alt[prov]=cc;} else alt[prov]=oc;
    }
    const key=ct; if(!(key in codes))codes[key]=Object.keys(codes).length+1;
    ctrlCode[prov]=codes[key];
  }
  M.primary=primary;M.alt=alt;M.contested=contested;M.ctrlCode=ctrlCode;
}

function recolor(){
  buildTables();
  const w=M.width,h=M.height,ids=M.idRaster,out=new Uint32Array(M.img.data.buffer);
  const pr=M.primary,al=M.alt,co=M.contested,n=w*h;
  for(let i=0;i<n;i++){
    const prov=ids[i];
    if(co[prov]){const x=i%w,y=(i/w)|0; out[i]=(((x+y)&7)<4)?pr[prov]:al[prov];}
    else out[i]=pr[prov];
  }
  if(M.fronts) drawFronts(out);
  M.ctx.putImageData(M.img,0,0);
  if(M.divs) drawDivisions();
}

function drawFronts(out){
  const w=M.width,h=M.height,ids=M.idRaster,cc=M.ctrlCode,fr=pack(FRONT);
  // right + down neighbour controller mismatch -> dark edge
  for(let y=0;y<h;y++){
    const row=y*w;
    for(let x=0;x<w-1;x++){
      const a=cc[ids[row+x]],b=cc[ids[row+x+1]];
      if(a>0&&b>0&&a!==b){out[row+x]=fr;out[row+x+1]=fr;}
    }
  }
  for(let y=0;y<h-1;y++){
    const row=y*w,nrow=row+w;
    for(let x=0;x<w;x++){
      const a=cc[ids[row+x]],b=cc[ids[nrow+x]];
      if(a>0&&b>0&&a!==b){out[row+x]=fr;out[nrow+x]=fr;}
    }
  }
}

// representative IN-province points for markers (actual province pixel nearest
// the centroid -> always on land, never the sea). Computed lazily, once, in two
// linear passes, mirroring geometry._compute_centroids in Python.
let PROV_POINTS=null;
function provPoints(){
  if(PROV_POINTS) return PROV_POINTS;
  const w=M.width,h=M.height,ids=M.idRaster;
  const sx={},sy={},cn={};
  for(let y=0;y<h;y++){const row=y*w;
    for(let x=0;x<w;x++){const p=ids[row+x]; if(!p)continue;
      sx[p]=(sx[p]||0)+x; sy[p]=(sy[p]||0)+y; cn[p]=(cn[p]||0)+1;}}
  const cx={},cy={}; for(const p in cn){cx[p]=sx[p]/cn[p];cy[p]=sy[p]/cn[p];}
  const bestd={},pts={};
  for(let y=0;y<h;y++){const row=y*w;
    for(let x=0;x<w;x++){const p=ids[row+x]; if(!p)continue;
      const dx=x-cx[p],dy=y-cy[p],d=dx*dx+dy*dy;
      if(bestd[p]===undefined||d<bestd[p]){bestd[p]=d;pts[p]=[x,y];}}}
  PROV_POINTS=pts; return pts;
}
function drawDivisions(){
  const snap=M.snaps[M.frame]; if(!snap.divisions) return;
  const cen=provPoints(), ctx=M.ctx;
  const agg={}; // prov -> {s, owners}
  for(const d of snap.divisions){const a=agg[d.p]||(agg[d.p]={s:0,o:{}});
    a.s+=d.s; a.o[d.o]=(a.o[d.o]||0)+d.s;}
  for(const p in agg){const c=cen[p]; if(!c)continue;
    const a=agg[p]; let best=null,bv=-1;
    for(const o in a.o){if(a.o[o]>bv){bv=a.o[o];best=o;}}
    const rgb=M.colors[best]||UNKNOWN, r=Math.max(2,Math.min(9,1.5+Math.sqrt(a.s)*0.25));
    ctx.beginPath();ctx.arc(c[0],c[1],r,0,7);
    ctx.fillStyle='rgb('+rgb[0]+','+rgb[1]+','+rgb[2]+')';ctx.fill();
    ctx.lineWidth=1;ctx.strokeStyle='#0c0c0e';ctx.stroke();}
}

function schedule(){if(M.raf)return;M.raf=true;requestAnimationFrame(()=>{M.raf=false;recolor();});}
function apply(){buildLegend();schedule();$('date').textContent=M.snaps[M.frame].date;
  if(M.selProv!=null) renderInfo(describe(M.selProv));}

function buildLegend(){
  const fr=M.snaps[M.frame],snap=fr.ownership,pctrl=fr.pctrl||null,counts={};
  let unit='states';
  if(M.mode==='control'&&pctrl){
    // count by PROVINCE (the unit actually controlled) so province-level
    // overrides are reflected -- a state-count legend would hide intra-state
    // splits. Effective controller = pctrl[prov] || state controller || owner.
    unit='provinces';
    const sb=M.stateByProvince;
    for(let prov=1;prov<sb.length;prov++){const st=sb[prov];if(st<0)continue;
      const o=snap[st];if(!o||!o.owner)continue;
      const k=pctrl[prov]||o.controller||o.owner;if(!k)continue;
      counts[k]=(counts[k]||0)+1;}
  } else {
    for(const st in snap){const k=(M.mode==='control')?(snap[st].controller||snap[st].owner):snap[st].owner;
      if(!k)continue;counts[k]=(counts[k]||0)+1;}
  }
  const sorted=Object.entries(counts).sort((a,b)=>b[1]-a[1]).slice(0,14);
  const ul=$('legend');ul.textContent='';
  for(const [tag,c] of sorted){const li=document.createElement('li');
    const sw=document.createElement('span');sw.className='sw';
    const rgb=M.colors[tag]||UNKNOWN;sw.style.background='rgb('+rgb[0]+','+rgb[1]+','+rgb[2]+')';
    const lb=document.createElement('span');lb.textContent=tag+'  ('+c+')';
    li.appendChild(sw);li.appendChild(lb);ul.appendChild(li);}
  $('legtitle').textContent=(M.mode==='control'?'Top controllers ('+unit+')':'Top owners');
}

function provAt(cx,cy){const r=M.canvas.getBoundingClientRect();
  const x=Math.floor((cx-r.left)*M.width/r.width),y=Math.floor((cy-r.top)*M.height/r.height);
  if(x<0||y<0||x>=M.width||y>=M.height)return null;return M.idRaster[y*M.width+x];}
function describe(prov){if(prov===0)return{water:true};
  const sb=M.stateByProvince,st=(prov<sb.length)?sb[prov]:-1;
  if(st<0)return{prov,sea:true};
  const fr=M.snaps[M.frame],o=fr.ownership[st]||{};
  // province-level control overrides the state controller for this province
  const pc=(fr.pctrl&&fr.pctrl[prov])?fr.pctrl[prov]:null;
  const ctrl=pc||o.controller||o.owner||'(none)';
  return{prov,state:st,owner:o.owner||'(none)',controller:ctrl,
    contested:!!(o.owner&&ctrl&&ctrl!==o.owner)};}
function renderInfo(info){const el=$('info');el.textContent='';
  if(!info){el.textContent='Hover or click a province.';return;}
  if(info.water){el.textContent='Open water.';return;}
  if(info.sea){el.textContent='Sea / lake province '+info.prov+'.';return;}
  const rows=[['State','#'+info.state],['Province',String(info.prov)],
    ['Owner',info.owner],['Controller',info.controller+(info.contested?'  [CONTESTED]':'')]];
  for(const [k,v] of rows){const d=document.createElement('div');
    d.textContent=k+': '+v;el.appendChild(d);} }

function setFrame(i){M.frame=Math.max(0,Math.min(M.snaps.length-1,i));
  $('scrub').value=M.frame;apply();}
// rAF-based playback with elapsed-time stepping: advances by wall-clock, so it
// never piles up frames under load (a slow recolor just means fewer steps), and
// it coalesces with the scrubber's own rAF gate.
const FRAME_MS=700;
function play(){if(M.playing){stop();return;}
  if(M.frame>=M.snaps.length-1)M.frame=0;
  M.playing=true;$('play').innerHTML='&#10073;&#10073; Pause';
  M.lastTick=performance.now();
  const loop=(t)=>{if(!M.playing)return;
    if(t-M.lastTick>=FRAME_MS){M.lastTick=t;
      if(M.frame>=M.snaps.length-1){stop();return;}setFrame(M.frame+1);}
    M.timer=requestAnimationFrame(loop);};
  M.timer=requestAnimationFrame(loop);}
function stop(){M.playing=false;$('play').innerHTML='&#9658; Play';
  if(M.timer){cancelAnimationFrame(M.timer);M.timer=null;}}

function wire(){
  const s=$('scrub');s.max=M.snaps.length-1;s.value=0;
  s.addEventListener('input',e=>{stop();setFrame(+e.target.value);});
  $('play').addEventListener('click',play);
  $('mode').addEventListener('change',e=>{M.mode=e.target.value;apply();});
  $('fronts').addEventListener('change',e=>{M.fronts=e.target.checked;schedule();});
  $('divs').addEventListener('change',e=>{M.divs=e.target.checked;schedule();});
  $('divs').checked=M.divs;
  M.canvas.addEventListener('mousemove',e=>{const p=provAt(e.clientX,e.clientY),t=$('tip');
    if(p==null){t.style.display='none';return;}const i=describe(p);
    t.textContent=i.water?'open water':i.sea?('sea '+i.prov):(i.owner+' / '+i.controller+(i.contested?' *':''));
    t.style.left=(e.clientX+14)+'px';t.style.top=(e.clientY+14)+'px';t.style.display='block';});
  M.canvas.addEventListener('mouseleave',()=>{$('tip').style.display='none';});
  M.canvas.addEventListener('click',e=>{const p=provAt(e.clientX,e.clientY);
    M.selProv=p;renderInfo(p==null?null:describe(p));});
}

async function boot(){
  try{
    $('title').textContent=CFG.title;document.title=CFG.title;
    M.divs=!!CFG.showDivisions;
    for(const t in M.colors)M.packed[t]=pack(M.colors[t]);
    setStatus('decoding province raster...');
    const img=await loadImage(CFG.embedRaster?CFG.pngDataUri:CFG.pngUrl);
    const r=decodeRaster(img);M.width=r.width;M.height=r.height;M.idRaster=r.idRaster;
    const p2s=CFG.embedRaster?CFG.p2s:await (await fetch(CFG.p2sUrl)).json();
    let maxP=0;for(const k in p2s){const v=+k;if(v>maxP)maxP=v;}
    for(let i=0;i<M.idRaster.length;i++)if(M.idRaster[i]>maxP)maxP=M.idRaster[i];
    M.stateByProvince=buildStateByProvince(p2s,maxP);
    M.canvas=$('map');M.canvas.width=M.width;M.canvas.height=M.height;
    M.ctx=M.canvas.getContext('2d',{willReadFrequently:false});M.ctx.imageSmoothingEnabled=false;
    M.img=M.ctx.createImageData(M.width,M.height);
    wire();setFrame(0);
    setStatus('ready - '+M.snaps.length+' frames, '+M.width+'x'+M.height+' raster.');
  }catch(e){console.error(e);setStatus('ERROR: '+e.message+'  (serve via map_view/serve.py unless embed_raster).');}
}
window.addEventListener('DOMContentLoaded',boot);
</script></body></html>
"""
