#!/usr/bin/env python3
"""
Battle Plan - local web dashboard for the "Battle Plan" the game mod.

The mod writes one "HOI4MPSTAT;..." line per country per month into
  ...\the game\logs\game.log
This tool serves a live dashboard on localhost that parses those lines. Stdlib
only - no pip installs.

Usage:
    python battle_plan.py [path/to/game.log] [--port 8077] [--no-open]
    python battle_plan.py [path/to/game.log] --static dashboard.html   # offline file instead

If no log path is given it looks in the default (and OneDrive) the game logs folder.
While a game runs the page auto-refreshes (re-parses the log) every 15 seconds,
so it doubles as a live caster view. Bound to 127.0.0.1 only.
"""

import argparse
import json
import sys
import webbrowser
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

# v1 schema: column order the mod emits after "HOI4MPSTAT;v1;DATA;". Fallback if the
# HEADER line isn't present in the log.
DEFAULT_COLUMNS = [
    "tick", "tag", "icout", "constr", "mp", "cas",
    "civ_fac", "mil_fac", "dock",
    "p_inf", "p_art", "p_sup", "p_at", "p_aa", "p_rocket", "p_mot", "p_mech",
    "p_armor", "p_fighter", "p_cas", "p_hfighter", "p_tac", "p_nav", "p_strat",
    "p_transport", "p_convoy", "p_sub", "p_screen", "p_capital", "p_carrier",
]

# per-type: column key -> (nice label, representative build_cost_ic)
TYPES = [
    ("p_inf", "Infantry", 0.43), ("p_art", "Artillery", 3.5),
    ("p_sup", "Support", 4), ("p_at", "Anti-tank", 4), ("p_aa", "Anti-air", 4),
    ("p_rocket", "Rocket arty", 4), ("p_mot", "Motorized", 2.5),
    ("p_mech", "Mechanized", 8), ("p_armor", "Armor", 7),
    ("p_fighter", "Fighter", 36), ("p_cas", "CAS", 36),
    ("p_hfighter", "Heavy fighter", 40), ("p_tac", "Tac bomber", 40),
    ("p_nav", "Naval bomber", 40), ("p_strat", "Strat bomber", 63),
    ("p_transport", "Transport", 38), ("p_convoy", "Convoy", 70),
    ("p_sub", "Submarine", 320), ("p_screen", "Screen ship", 400),
    ("p_capital", "Capital ship", 3000), ("p_carrier", "Carrier", 2094),
]

# scalar metrics offered in the line chart (key -> label)
SCALARS = {
    "icout": "Mil IC output / mo (mod)", "icout_web": "Mil IC output / mo (web weights)",
    "constr": "Construction power", "mp": "Army manpower", "cas": "Casualties",
    "civ_fac": "Civilian factories", "mil_fac": "Military factories",
    "dock": "Dockyards", "total_fac": "Total factories",
}


def default_log_path():
    """Best guess at game.log. Returns a path to WATCH even if it doesn't exist
    yet, so the dashboard can be opened before the game is launched and fill in live."""
    rel = ("Documents", "Paradox Interactive", "Hearts of Iron IV", "logs", "game.log")
    one = Path.home() / "OneDrive" / Path(*rel)
    doc = Path.home() / Path(*rel)
    for p in (one, doc):           # prefer one that already has the log
        if p.exists():
            return p
    for p in (one, doc):           # else prefer whichever logs folder exists
        if p.parent.exists():
            return p
    return one


def num(s):
    s = s.strip().replace(",", "").replace(" ", "")
    if s == "":
        return 0.0
    try:
        return float(s)
    except ValueError:
        return 0.0


def parse_log(path):
    columns = None
    rows = []
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as fh:
            for line in fh:
                i = line.find("HOI4MPSTAT;")
                if i < 0:
                    continue
                parts = line[i:].strip().split(";")
                if len(parts) < 4:
                    continue
                kind = parts[2]
                payload = parts[3:]
                if kind == "HEADER":
                    columns = payload
                elif kind == "DATA":
                    cols = columns or DEFAULT_COLUMNS
                    rec = {}
                    for idx, colname in enumerate(cols):
                        val = payload[idx] if idx < len(payload) else ""
                        rec[colname] = val if colname == "tag" else num(val)
                    rec["tick"] = int(rec.get("tick", 0))
                    rows.append(rec)
    except FileNotFoundError:
        pass
    return rows


def build_series(rows):
    by_tag = {}
    for r in rows:
        by_tag.setdefault(r["tag"], []).append(r)
    out = {}
    all_ticks = set()
    for tag, recs in by_tag.items():
        recs.sort(key=lambda r: r["tick"])
        prev = {k: 0.0 for k, _, _ in TYPES}
        seen_first = False
        clean = []
        for r in recs:
            all_ticks.add(r["tick"])
            row = dict(r)
            row["total_fac"] = r.get("civ_fac", 0) + r.get("mil_fac", 0) + r.get("dock", 0)
            ic_web = 0.0
            for k, _, w in TYPES:
                cum = r.get(k, 0.0)
                delta = cum - prev[k]
                if delta < 0 or not seen_first:
                    delta = 0.0
                row["out_" + k] = delta
                ic_web += delta * w
                prev[k] = cum
            row["icout_web"] = ic_web
            seen_first = True
            clean.append(row)
        out[tag] = clean
    return out, sorted(all_ticks)


def make_payload(rows):
    series, ticks = build_series(rows)
    return {
        "series": series,
        "ticks": ticks,
        "types": [{"key": k, "label": lbl} for k, lbl, _ in TYPES],
        "scalars": SCALARS,
    }


_CHARTJS = None

def chartjs_tag():
    """Inline the vendored Chart.js so the dashboard is FULLY OFFLINE (no CDN).
    Falls back to the CDN only if the local file is missing."""
    global _CHARTJS
    if _CHARTJS is None:
        f = Path(__file__).resolve().parent / "chart.umd.min.js"
        if f.exists():
            _CHARTJS = "<script>\n" + f.read_text(encoding="utf-8") + "\n</script>"
        else:
            print("WARNING: chart.umd.min.js missing next to battle_plan.py -> using CDN (needs internet).")
            _CHARTJS = '<script src="https://cdn.jsdelivr.net/npm/chart.js@4"></script>'
    return _CHARTJS


HTML = r"""<!doctype html>
<html><head><meta charset="utf-8"><title>Battle Plan</title>
__CHARTJS__
<style>
  body{font-family:Segoe UI,Arial,sans-serif;margin:0;background:#10141c;color:#dfe6f0}
  header{padding:12px 18px;background:#161b26;border-bottom:1px solid #2a3344;display:flex;
         justify-content:space-between;align-items:center}
  h1{font-size:18px;margin:0}
  #status{font-size:12px;opacity:.7}
  .wrap{display:flex;gap:18px;padding:18px;flex-wrap:wrap}
  .panel{background:#161b26;border:1px solid #2a3344;border-radius:8px;padding:14px;flex:1;min-width:480px}
  .controls{display:flex;gap:14px;align-items:center;flex-wrap:wrap;margin-bottom:10px}
  select{background:#222b3a;color:#dfe6f0;border:1px solid #38465e;border-radius:5px;padding:5px 8px}
  .tags{display:flex;flex-wrap:wrap;gap:4px 14px;max-height:90px;overflow:auto;margin-bottom:10px;font-size:13px}
  .tags label{white-space:nowrap}
  table{border-collapse:collapse;width:100%;font-size:13px}
  th,td{border-bottom:1px solid #2a3344;padding:4px 8px;text-align:right}
  th:first-child,td:first-child{text-align:left}
  th{cursor:pointer;position:sticky;top:0;background:#161b26}
  canvas{max-height:340px}
</style></head>
<body>
<header><h1>Battle Plan — live dashboard</h1><span id="status">loading…</span></header>
<div class="wrap">
  <div class="panel">
    <div class="controls"><label>Metric <select id="metric"></select></label>
      <span style="opacity:.6">pick countries:</span></div>
    <div class="tags" id="tags"></div>
    <canvas id="lineChart"></canvas>
  </div>
  <div class="panel">
    <div class="controls"><label>Production mix for <select id="mixCountry"></select></label>
      <span style="opacity:.6">(units/mo, stacked)</span></div>
    <canvas id="mixChart"></canvas>
  </div>
  <div class="panel" style="min-width:100%">
    <div class="controls"><b>Latest-tick snapshot</b><span style="opacity:.6">(click a header to sort)</span></div>
    <div style="max-height:340px;overflow:auto"><table id="snap"></table></div>
  </div>
</div>
<script>
const BOOT = __DATA__;            // non-null only in --static mode
const palette=["#e6194B","#3cb44b","#4363d8","#f58231","#42d4f4","#f032e6","#bfef45",
 "#fabed4","#9A6324","#fffac8","#800000","#aaffc3","#808000","#ffd8b1","#000075",
 "#a9a9a9","#ffffff","#e6beff"];
const color=i=>palette[i%palette.length];
let D=null, tags=[], built=false, sortKey="icout", sortDir=-1, lineChart, mixChart;

async function fetchData(){ if(BOOT) return BOOT;
  const r=await fetch("/data.json",{cache:"no-store"}); return await r.json(); }

function buildControls(){
  const m=document.getElementById("metric");
  m.innerHTML=Object.entries(D.scalars).map(([k,v])=>`<option value="${k}">${v}</option>`).join("");
  const tg=document.getElementById("tags");
  tg.innerHTML=tags.map((t,i)=>`<label><input type="checkbox" class="tagchk" value="${t}" `+
    `${i<8?"checked":""}>${t}</label>`).join("");
  document.getElementById("mixCountry").innerHTML=tags.map(t=>`<option value="${t}">${t}</option>`).join("");
  m.onchange=drawLine;
  document.getElementById("mixCountry").onchange=drawMix;
  tg.querySelectorAll(".tagchk").forEach(c=>c.onchange=drawLine);
  built=true;
}
const selectedTags=()=>[...document.querySelectorAll(".tagchk:checked")].map(c=>c.value);

function drawLine(){
  const metric=document.getElementById("metric").value, sel=selectedTags();
  const ds=sel.map(t=>({label:t,borderColor:color(tags.indexOf(t)),backgroundColor:color(tags.indexOf(t)),
    data:(D.series[t]||[]).map(r=>({x:r.tick,y:r[metric]??0})),tension:.2,pointRadius:0,borderWidth:2}));
  const cfg={type:"line",data:{datasets:ds},options:{responsive:true,animation:false,parsing:false,
    scales:{x:{type:"linear",title:{display:true,text:"month tick"},grid:{color:"#243047"}},y:{grid:{color:"#243047"}}},
    plugins:{legend:{labels:{color:"#dfe6f0"}}}}};
  if(lineChart)lineChart.destroy(); lineChart=new Chart(document.getElementById("lineChart"),cfg);
}
function drawMix(){
  const t=document.getElementById("mixCountry").value, recs=D.series[t]||[];
  const ds=D.types.map((ty,i)=>({label:ty.label,data:recs.map(r=>({x:r.tick,y:r["out_"+ty.key]??0})),
    borderColor:color(i),backgroundColor:color(i),fill:true,stack:"s",pointRadius:0,borderWidth:1}));
  const cfg={type:"line",data:{datasets:ds},options:{responsive:true,animation:false,parsing:false,
    scales:{x:{type:"linear",title:{display:true,text:"month tick"},grid:{color:"#243047"}},
            y:{stacked:true,grid:{color:"#243047"}}},
    plugins:{legend:{labels:{color:"#dfe6f0",boxWidth:10,font:{size:10}}}}}};
  if(mixChart)mixChart.destroy(); mixChart=new Chart(document.getElementById("mixChart"),cfg);
}
function drawTable(){
  const cols=[["tag","Country"],["icout","IC out/mo"],["icout_web","IC out (web)"],["mil_fac","Mil fac"],
    ["civ_fac","Civ fac"],["dock","Dock"],["mp","Manpower"],["cas","Casualties"]];
  const last={};
  for(const t of tags){const a=D.series[t]; if(a&&a.length)last[t]=a[a.length-1];}
  let rows=Object.values(last);
  rows.sort((a,b)=>{const x=a[sortKey],y=b[sortKey];return (x<y?-1:x>y?1:0)*sortDir;});
  let h="<tr>"+cols.map(c=>`<th data-k="${c[0]}">${c[1]}</th>`).join("")+"</tr>";
  for(const r of rows){h+="<tr>"+cols.map(c=>`<td>${c[0]==="tag"?r[c[0]]:Math.round(r[c[0]]).toLocaleString()}</td>`).join("")+"</tr>";}
  const tb=document.getElementById("snap"); tb.innerHTML=h;
  tb.querySelectorAll("th").forEach(th=>th.onclick=()=>{const k=th.dataset.k;
    if(k===sortKey)sortDir*=-1; else{sortKey=k;sortDir=-1;} drawTable();});
}
function drawAll(){ if(!tags.length){document.getElementById("status").textContent="waiting for HOI4MPSTAT data…";return;}
  drawLine();drawMix();drawTable();
  document.getElementById("status").textContent=`${tags.length} countries · ${D.ticks.length} months · updated `+new Date().toLocaleTimeString();
}
async function refresh(){ D=await fetchData(); const t=Object.keys(D.series).sort();
  if(!built || t.length!==tags.length){tags=t; if(tags.length)buildControls();} else {tags=t;}
  drawAll(); }
async function init(){ await refresh(); if(!BOOT) setInterval(refresh,15000); }
init();
</script>
</body></html>
"""


def make_handler(log_path):
    class Handler(BaseHTTPRequestHandler):
        def _send(self, body, ctype):
            self.send_response(200)
            self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            if self.path.startswith("/data.json"):
                payload = make_payload(parse_log(log_path))
                self._send(json.dumps(payload, separators=(",", ":")).encode("utf-8"),
                           "application/json")
            elif self.path in ("/", "/index.html"):
                page = HTML.replace("__CHARTJS__", chartjs_tag()).replace("__DATA__", "null")
                self._send(page.encode("utf-8"), "text/html; charset=utf-8")
            else:
                self.send_error(404)

        def log_message(self, *a):  # keep the console quiet
            pass
    return Handler


def serve(log_path, port, open_browser):
    httpd = ThreadingHTTPServer(("127.0.0.1", port), make_handler(log_path))
    url = f"http://127.0.0.1:{port}"
    print(f"Battle Plan dashboard serving at {url}")
    print(f"Reading: {log_path}" + ("" if log_path.exists() else "   (not created yet - will fill in once the game logs)"))
    print("(page auto-refreshes every 15s; Ctrl+C to stop)")
    if open_browser:
        webbrowser.open(url)
    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        print("\nstopped.")


def write_static(log_path, out):
    rows = parse_log(log_path)
    if not rows:
        sys.exit("No HOI4MPSTAT lines found in the log yet.")
    payload = make_payload(rows)
    html = HTML.replace("__CHARTJS__", chartjs_tag()).replace("__DATA__", json.dumps(payload, separators=(",", ":")))
    Path(out).write_text(html, encoding="utf-8")
    print(f"Wrote {Path(out).resolve()} ({len(rows)} rows, {len(payload['series'])} countries)")


def main():
    ap = argparse.ArgumentParser(description="Serve the Battle Plan dashboard on localhost.")
    ap.add_argument("log", nargs="?", help="path to game.log (default: the game logs folder)")
    ap.add_argument("--port", type=int, default=8077, help="localhost port (default 8077)")
    ap.add_argument("--no-open", action="store_true", help="don't auto-open the browser")
    ap.add_argument("--static", metavar="FILE", help="write a self-contained HTML file instead of serving")
    args = ap.parse_args()

    log_path = Path(args.log) if args.log else default_log_path()

    if args.static:
        if not log_path.exists():
            sys.exit(f"game.log not found: {log_path}\n(pass the path explicitly for --static)")
        write_static(log_path, args.static)
    else:
        # serve even if the log doesn't exist yet - the page waits and fills in live
        serve(log_path, args.port, not args.no_open)


if __name__ == "__main__":
    main()
