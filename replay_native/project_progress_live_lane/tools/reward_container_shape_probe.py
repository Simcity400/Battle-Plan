"""Bound CProgramStatus+0x58 reward-container candidates.

Read-only development probe. This intentionally does not feed the production
hot loop. It tests whether the currently observed CPrototypeReward references
under CProgramStatus+0x58 can be treated as fired reward-history rows.
"""

import argparse
import json
import os
import sys
import time
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path

THIS_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(THIS_DIR))

import residual_blocker_probe as rb  # noqa: E402

pp = rb.pp
winmem = rb.winmem


def hx(value):
    return pp.hx(value)


def counter_to_json(counter):
    return {str(key): value for key, value in sorted(counter.items())}


def load_save_counts(path):
    truth = rb.load_save_truth(path)
    if not truth:
        return None
    per_tag = Counter(truth["country_history_counts"])
    per_project = Counter()
    for event in truth["history_events"]:
        per_project[(event["tag"], event["project_id"])] += 1
    return {
        "truth": truth,
        "total": len(truth["history_events"]),
        "per_tag": per_tag,
        "per_project": per_project,
    }


def scan_status_container(p, container_ptr, reward_set, max_container_bytes):
    if not pp.is_heap_ptr(container_ptr):
        return {
            "readable": False,
            "reason": "container pointer invalid",
            "refs": [],
        }
    blob = p.try_read(container_ptr, max_container_bytes)
    if not blob:
        return {
            "readable": False,
            "reason": "container unreadable",
            "refs": [],
        }
    refs = []
    for off in range(0, max(0, len(blob) - 7), 8):
        value = rb.read_u64_blob(blob, off)
        if value in reward_set:
            refs.append({"offset": off, "reward_object": value})
    return {
        "readable": True,
        "bytes_read": len(blob),
        "refs": refs,
    }


def probe(args):
    save_counts = load_save_counts(args.save_projects_json)
    with winmem.Process(args.pid, "hoi4.exe") as p:
        fp = p.fingerprint()
        sha = str(fp.get("sha256") or "").lower()
        if sha != pp.EXPECTED_SHA256 and not args.force:
            raise SystemExit(
                "live hoi4.exe sha256 %s != expected %s; pass --force to probe anyway"
                % (sha, pp.EXPECTED_SHA256)
            )

        context = rb.collect_anchored_context(p, args.max_projects)
        rtti_vtables = rb.find_msvc_rtti_vtables(p, rb.REWARD_RTTI_NAME)
        candidates = rb.current_reward_candidates(p.module_base, rtti_vtables)
        heap_hits, truncated, heap_scan = rb.heap_scan_vtable_hits(
            p, candidates, args.max_vtable_hits
        )
        rtti_labels = [label for label, _addr in candidates if label.startswith("rtti_")]
        selected_labels = rtti_labels or [label for label, _addr in candidates]
        reward_objects = sorted(
            {
                addr
                for label in selected_labels
                for addr in heap_hits.get(label, [])
            }
        )
        reward_set = set(reward_objects)

        per_tag_refs = Counter()
        per_tag_unique = Counter()
        object_ref_count = Counter()
        object_tags = defaultdict(set)
        offset_counts = Counter()
        offset_mod_0x30 = Counter()
        status_examples = []
        unreadable_containers = 0
        statuses_with_refs = 0

        for status in context["status_rows"]:
            status_ptr = status["status_ptr"]
            tag = status.get("tag")
            container_ptr = pp.try_u64(p, status_ptr + 0x58)
            scanned = scan_status_container(
                p, container_ptr, reward_set, args.max_container_bytes
            )
            if not scanned["readable"]:
                unreadable_containers += 1
                continue
            refs = scanned["refs"]
            if refs:
                statuses_with_refs += 1
            seen_for_tag = set()
            for ref in refs:
                obj = ref["reward_object"]
                off = ref["offset"]
                per_tag_refs[tag] += 1
                seen_for_tag.add(obj)
                object_ref_count[obj] += 1
                object_tags[obj].add(tag)
                offset_counts[off] += 1
                offset_mod_0x30[off % 0x30] += 1
            per_tag_unique[tag] += len(seen_for_tag)
            if refs and len(status_examples) < args.example_statuses:
                status_examples.append(
                    {
                        "tag": tag,
                        "tag_db_id": status["tag_db_id"],
                        "status_ptr": hx(status_ptr),
                        "container_ptr": hx(container_ptr),
                        "ref_count": len(refs),
                        "unique_reward_objects": len(seen_for_tag),
                        "first_refs": [
                            {
                                "offset": hx(ref["offset"]),
                                "offset_mod_0x30": hx(ref["offset"] % 0x30),
                                "reward_object": hx(ref["reward_object"]),
                            }
                            for ref in refs[: args.example_refs]
                        ],
                    }
                )

        live_total_refs = sum(per_tag_refs.values())
        live_unique_objects = len(object_ref_count)
        reused_objects = sum(1 for count in object_ref_count.values() if count > 1)
        multi_tag_objects = sum(1 for tags in object_tags.values() if len(tags) > 1)
        max_tags_per_object = max((len(tags) for tags in object_tags.values()), default=0)
        max_refs_per_object = max(object_ref_count.values() or [0])

        save_compare = None
        if save_counts:
            all_tags = sorted(set(save_counts["per_tag"]) | set(per_tag_refs))
            ref_mismatches = []
            unique_mismatches = []
            for tag in all_tags:
                save_count = save_counts["per_tag"].get(tag, 0)
                ref_count = per_tag_refs.get(tag, 0)
                unique_count = per_tag_unique.get(tag, 0)
                if ref_count != save_count and (ref_count or save_count):
                    ref_mismatches.append(
                        {
                            "tag": tag,
                            "live_ref_count": ref_count,
                            "save_history_count": save_count,
                        }
                    )
                if unique_count != save_count and (unique_count or save_count):
                    unique_mismatches.append(
                        {
                            "tag": tag,
                            "live_unique_reward_objects": unique_count,
                            "save_history_count": save_count,
                        }
                    )
            save_compare = {
                "save_history_total": save_counts["total"],
                "live_total_refs": live_total_refs,
                "live_unique_reward_objects": live_unique_objects,
                "total_ref_match": live_total_refs == save_counts["total"],
                "total_unique_match": live_unique_objects == save_counts["total"],
                "per_tag_ref_mismatches": len(ref_mismatches),
                "per_tag_unique_mismatches": len(unique_mismatches),
                "ref_mismatch_examples": ref_mismatches[:32],
                "unique_mismatch_examples": unique_mismatches[:32],
            }

        accepted_as_history_root = bool(
            save_compare
            and save_compare["total_ref_match"]
            and save_compare["per_tag_ref_mismatches"] == 0
            and multi_tag_objects == 0
            and reused_objects == 0
        )

        return {
            "schema": "project_progress_reward_container_shape_probe_v1",
            "utc": datetime.now(timezone.utc).isoformat(),
            "pid": args.pid,
            "module_path": fp.get("path"),
            "module_base": hx(p.module_base),
            "sha256": sha,
            "container_candidate": "CProgramStatus+0x58 pointer payload",
            "scan_limit_bytes": hx(args.max_container_bytes),
            "status_rows": len(context["status_rows"]),
            "statuses_with_refs": statuses_with_refs,
            "unreadable_containers": unreadable_containers,
            "reward_rtti_vtables": [
                {
                    "vtable_rva": hx(row["vtable_rva"]),
                    "complete_object_locator_rva": hx(row["complete_object_locator_rva"]),
                    "type_desc_rva": hx(row["type_desc_rva"]),
                    "first_function_rva": hx(row["first_function_rva"]),
                }
                for row in rtti_vtables
            ],
            "candidate_vtable_hits": {
                label: {
                    "vtable": hx(addr),
                    "vtable_rva": hx(addr - p.module_base),
                    "heap_objects": len(heap_hits.get(label, [])),
                    "truncated": bool(truncated.get(label)),
                }
                for label, addr in candidates
            },
            "heap_scan": heap_scan,
            "live_ref_summary": {
                "total_refs": live_total_refs,
                "unique_reward_objects_referenced": live_unique_objects,
                "reused_reward_objects": reused_objects,
                "multi_tag_reward_objects": multi_tag_objects,
                "max_tags_per_reward_object": max_tags_per_object,
                "max_refs_per_reward_object": max_refs_per_object,
                "offset_mod_0x30_counts": counter_to_json(offset_mod_0x30),
                "top_absolute_offsets": [
                    {"offset": hx(off), "count": count}
                    for off, count in offset_counts.most_common(24)
                ],
            },
            "save_compare": save_compare,
            "status_examples": status_examples,
            "accepted_as_fired_reward_history_root": accepted_as_history_root,
            "decision": (
                "reject CProgramStatus+0x58 CPrototypeReward references as fired "
                "reward-history rows"
                if not accepted_as_history_root
                else "candidate needs per-row field pinning before production use"
            ),
        }


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Bound CProgramStatus+0x58 reward container shape"
    )
    parser.add_argument("--pid", type=int, default=None, help="live hoi4.exe PID")
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--max-projects", type=int, default=30000)
    parser.add_argument("--max-vtable-hits", type=int, default=10000)
    parser.add_argument("--max-container-bytes", type=lambda x: int(x, 0), default=0x4000)
    parser.add_argument("--example-statuses", type=int, default=16)
    parser.add_argument("--example-refs", type=int, default=12)
    parser.add_argument("--save-projects-json", required=True)
    parser.add_argument("--out", default=None)
    args = parser.parse_args(argv)
    if args.pid is None:
        args.pid = winmem.find_pid("hoi4.exe")
    if not args.pid:
        raise SystemExit("hoi4.exe PID not found")
    started = time.perf_counter()
    summary = probe(args)
    summary["elapsed_ms"] = round((time.perf_counter() - started) * 1000, 3)
    if args.out:
        os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
        with open(args.out, "w", encoding="utf-8") as fh:
            json.dump(summary, fh, indent=2, sort_keys=True)
            fh.write("\n")
    print(json.dumps(summary, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
