import json
import sys

SRC = r"C:\hoi4_evidence\agent_re\temporary\a36_d04_units_mine\divisions.json"
OUT = sys.argv[1]

d = json.load(open(SRC, encoding="utf-8"))
divs = d["divisions"]
owners = sorted({x["owner"] for x in divs if x.get("owner")})
owner_idx = {t: i + 1 for i, t in enumerate(owners)}

rows = 0
with open(OUT, "w", encoding="ascii") as f:
    for x in divs:
        div_id = x["div_id"]
        div_type = x["div_type"]
        owner = owner_idx.get(x.get("owner"), 0)
        loc = x["location_province"]
        tpl = x["template_id"] if x.get("template_id") is not None else -1
        org = x["organisation"]
        stg = x["strength"]
        f.write(f"{div_id} {div_type} {owner} {loc} {tpl} {org!r} {stg!r}\n")
        rows += 1

print(f"{rows} divisions, {len(owners)} distinct owners")
print(f"save_date={d['meta']['save_date']} version={d['meta']['save_version']}")
