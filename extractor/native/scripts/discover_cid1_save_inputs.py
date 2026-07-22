#!/usr/bin/env python3
"""Deterministically resolve retained CID1 S/D save candidates by file identity.

This is discovery only.  It joins the frozen candidate census by filename,
byte count, and 64-KiB prefix SHA-256, then adds the full-file SHA-256 and
absolute path.  Live endpoint admission still requires the save oracle and
same-hour memory transcript; this report cannot promote an arm.
"""
from __future__ import annotations
import argparse, hashlib, json, os, tempfile
from pathlib import Path

SCHEMA="hoi4.cid1.sd_save_discovery.v1"
D04_SHA="c8488b05a3b87c718eeff979976537b79e867634d0b4dccb1eef6c5a45362735"

def digest(path, limit=None):
    h=hashlib.sha256();remaining=limit
    with path.open("rb") as f:
        while True:
            n=(1<<20) if remaining is None else min(1<<20,remaining)
            if n<=0: break
            b=f.read(n)
            if not b: break
            h.update(b)
            if remaining is not None: remaining-=len(b)
    return h.hexdigest()

def atomic(path,value):
    path.parent.mkdir(parents=True,exist_ok=True)
    if path.exists(): raise ValueError(f"refusing to replace {path}")
    fd,tmp=tempfile.mkstemp(prefix=f".{path.name}.",suffix=".tmp",dir=path.parent)
    try:
        with os.fdopen(fd,"w",encoding="utf-8",newline="\n") as f: json.dump(value,f,indent=2,sort_keys=True);f.write("\n");f.flush();os.fsync(f.fileno())
        os.replace(tmp,path)
    finally:
        if os.path.exists(tmp): os.unlink(tmp)

def main():
    ap=argparse.ArgumentParser(description=__doc__);ap.add_argument("--save-root",type=Path,required=True);ap.add_argument("--candidate-census",type=Path,required=True);ap.add_argument("--out",type=Path,required=True);a=ap.parse_args()
    census=json.loads(a.candidate_census.read_text(encoding="utf-8-sig"));index={}
    for group in census.get("candidate_groups",[]):
        for source in group.get("sources",[]): index.setdefault((source["name"],source["size_bytes"],source["prefix_sha256"]),[]).append(group)
    rows=[];d04=[]
    for path in sorted(a.save_root.rglob("*.hoi4"),key=lambda p:str(p).lower()):
        size=path.stat().st_size;prefix=digest(path,65536);full=digest(path);matches=index.get((path.name,size,prefix),[])
        row={"path":str(path.resolve()),"filename":path.name,"bytes":size,"sha256":full,"prefix_sha256":prefix,"candidate_census_matches":len(matches)}
        if len(matches)==1: row.update({"candidate_campaign_id_v1":matches[0]["campaign_id_v1"],"candidate_game_unique_id":matches[0]["game_unique_id"],"candidate_game_unique_seed":matches[0]["game_unique_seed"],"candidate_only":True})
        rows.append(row)
        if full==D04_SHA:d04.append(row)
    groups={}
    for row in rows:
        if "candidate_campaign_id_v1" in row: groups.setdefault(row["candidate_campaign_id_v1"],[]).append(row)
    lineage=[{"candidate_campaign_id_v1":k,"ordered_saves":sorted(v,key=lambda r:r["filename"])} for k,v in sorted(groups.items()) if len(v)>=2]
    result={"schema":SCHEMA,"authority":"exact_file_discovery_only_live_oracle_and_endpoint_gate_required","save_root":str(a.save_root.resolve()),"candidate_census_path":str(a.candidate_census.resolve()),"candidate_census_sha256":digest(a.candidate_census),"d04_exact_matches":d04,"same_lineage_candidates":lineage,"distinct_campaign_candidate_count":len(groups),"all_saves":rows,"selection_rules":{"S1_S3_S4":"use the unique exact D04 match when present","S2":"choose two oracle-confirmed saves from one candidate lineage, then require second live raw hour lower","D2":"choose two oracle-confirmed distinct campaign ids and load them in one recorder-owned process","D1":"retained saves cannot prove fresh identical-start creation; use the authenticated new-campaign action profile"}}
    atomic(a.out.resolve(),result);print(a.out.resolve())
if __name__=="__main__":main()
