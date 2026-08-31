from __future__ import annotations

import json
import os
import time
from pathlib import Path
from typing import Any

from common import atomic_json
from registry import Registry


def _load_winmem(workspace: Path):
    """The read-only live-process memory reader, via the repository RE facade.

    The reader opens a running hoi4.exe, so it is acquisition code and stays in
    the acquisition tree. This broker is filed under the replay side and reaches
    it only through `<repo>/tools/hoi4_re`, which is owned by neither tree, so no
    file on the replay side names an acquisition path.
    """

    import sys

    facade_root = workspace / "tools"
    if str(facade_root) not in sys.path:
        sys.path.insert(0, str(facade_root))
    import hoi4_re

    # Two packages in this repository are named `hoi4_re`. Resolving the wrong
    # one would silently change which tree supplies the memory reader.
    located = Path(hoi4_re.__file__).resolve().parent.parent
    if located != facade_root.resolve():
        raise ImportError(
            f"the repository RE facade resolved to {located}, not {facade_root}"
        )
    return hoi4_re.load_winmem()


class RuntimeLease:
    def __init__(self, path: Path):
        self.path = path
        self.handle = None

    def __enter__(self):
        import msvcrt

        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.handle = self.path.open("a+b")
        self.handle.seek(0, os.SEEK_END)
        if self.handle.tell() == 0:
            self.handle.write(b"0")
            self.handle.flush()
        self.handle.seek(0)
        try:
            msvcrt.locking(self.handle.fileno(), msvcrt.LK_NBLCK, 1)
        except OSError as error:
            self.handle.close()
            self.handle = None
            raise RuntimeError("the live the game process is already leased by another RE job") from error
        return self

    def __exit__(self, *_: object) -> None:
        import msvcrt

        if self.handle is not None:
            self.handle.seek(0)
            try:
                msvcrt.locking(self.handle.fileno(), msvcrt.LK_UNLCK, 1)
            finally:
                self.handle.close()
                self.handle = None


def runtime_status(workspace: Path) -> dict[str, Any]:
    winmem = _load_winmem(workspace)
    pid = winmem.find_pid("hoi4.exe")
    if pid is None:
        return {"running": False, "process": "hoi4.exe"}
    with winmem.Process(pid, "hoi4.exe") as process:
        fingerprint = process.fingerprint()
    fingerprint["base"] = f"0x{int(fingerprint['base']):x}"
    fingerprint["size"] = f"0x{int(fingerprint['size']):x}"
    return {"running": True, "process": fingerprint}


def _read_value(process: Any, address: int, value_type: str, max_bytes: int) -> Any:
    readers = {
        "u8": process.u8, "i8": process.i8, "u16": process.u16, "i16": process.i16,
        "u32": process.u32, "i32": process.i32, "u64": process.u64, "i64": process.i64,
        "f32": process.f32, "f64": process.f64, "ptr": process.ptr,
    }
    if value_type in readers:
        return readers[value_type](address)
    if value_type == "cstr":
        return process.cstr(address, max_bytes)
    if value_type == "bytes":
        return process.read(address, max_bytes).hex()
    raise ValueError(f"unsupported memory path value_type: {value_type}")


def observe_runtime(
    registry: Registry, workspace: Path, build_identifier: str,
    path_ids: list[str], lease_path: Path,
) -> dict[str, Any]:
    if not isinstance(path_ids, list) or not path_ids or len(path_ids) > 64:
        raise ValueError("path_ids must contain 1..64 registered path IDs")
    build = registry.get_build(build_identifier)
    winmem = _load_winmem(workspace)
    with RuntimeLease(lease_path):
        process = winmem.open_process("hoi4.exe", "hoi4.exe")
        if process is None:
            raise RuntimeError("hoi4.exe is not running")
        with process:
            fingerprint = process.fingerprint()
            if str(fingerprint.get("sha256", "")).lower() != build["sha256"]:
                raise RuntimeError(
                    f"live executable SHA-256 {fingerprint.get('sha256')} does not match build {build['sha256']}"
                )
            observations = []
            for path_id in path_ids:
                spec = registry.memory_path(path_id)
                if spec["build_id"] != build["build_id"]:
                    raise RuntimeError(f"memory path {path_id} belongs to a different build")
                address = process.module_base + int(spec["base_rva"])
                offsets = [int(value) for value in spec["offsets"]]
                if offsets:
                    address = process.chain(address, offsets, deref_base=spec["deref_base"])
                value = _read_value(process, address, spec["value_type"], int(spec["max_bytes"]))
                observations.append({
                    "path_id": path_id,
                    "value_type": spec["value_type"],
                    "value": value,
                    "resolved_address": f"0x{address:x}",
                    "resolved_module_rva": f"0x{address - process.module_base:x}" if process.in_module(address) else None,
                })
            return {
                "schema": "hoi4-re-runtime-observation-v1",
                "mode": "read-only ReadProcessMemory",
                "build_id": build["build_id"],
                "exe_sha256": fingerprint["sha256"],
                "pid": process.pid,
                "module_base": f"0x{process.module_base:x}",
                "observations": observations,
            }


_MODULE_ADDRESS_HELPER = """function moduleAddress(value) {
  const pointer = hx(value);
  if (pointer === null) return {pointer: null, rva: null, in_module: false};
  try {
    if (value.compare(module.base) >= 0 && value.compare(module.base.add(module.size)) < 0) {
      return {pointer: pointer, rva: hx(value.sub(module.base)), in_module: true};
    }
    return {pointer: pointer, rva: null, in_module: false};
  } catch (_) {
    return {pointer: pointer, rva: null, in_module: false};
  }
}
"""

# The caller, when a target asks for it. `Interceptor.attach` installs its own
# frame, so the qword at [rsp] on entry is the trampoline's return address and
# NOT the caller's; `this.returnAddress` is Frida's accessor for the real one.
# One pointer, read only, resolved to a module RVA by the same range compare the
# vtable-slot reader already uses.
_RETURN_ADDRESS_FIELD = """,
        return_address: target.capture_return_address
          ? moduleAddress(this.returnAddress) : null"""


def _trace_schema(targets: list[dict[str, Any]]) -> str:
    """The trace schema tag for a target list.

    The hit shape changes only when a target asked for the caller, so the tag
    moves only then: a trace whose enter records can carry `return_address` is
    v2, and one that cannot is the v1 it has always been.
    """
    if any(target.get("capture_return_address") for target in targets):
        return "hoi4-re-frida-call-trace-v2"
    return "hoi4-re-frida-call-trace-v1"


def _trace_script(targets: list[dict[str, Any]], max_hits: int) -> str:
    # Emitted ONLY when some target asked. A spec written before this knob
    # existed therefore injects a byte-identical script into a shared process.
    wants_return_address = any(target.get("capture_return_address") for target in targets)
    module_address_helper = _MODULE_ADDRESS_HELPER if wants_return_address else ""
    return_address_field = _RETURN_ADDRESS_FIELD if wants_return_address else ""
    return f"""
'use strict';
const module = Process.getModuleByName('hoi4.exe');
const targets = {json.dumps(targets)};
const maxHits = {max_hits};
const counts = {{}};
const encounters = {{}};
function hx(value) {{ try {{ return value.toString(); }} catch (_) {{ return null; }} }}
function receiver(value) {{
  const pointer = hx(value);
  if (pointer === null || value.isNull()) return {{pointer: pointer, readable: false}};
  try {{
    const vtable = value.readPointer();
    let vtableRva = null;
    if (vtable.compare(module.base) >= 0 && vtable.compare(module.base.add(module.size)) < 0) {{
      vtableRva = hx(vtable.sub(module.base));
    }}
    return {{pointer: pointer, readable: true, vtable_ptr: hx(vtable), vtable_rva: vtableRva}};
  }} catch (_) {{
    return {{pointer: pointer, readable: false}};
  }}
}}
{module_address_helper}function receiverQwords(value, offsets) {{
  const rows = [];
  for (const offset of offsets) {{
    try {{
      const field = value.add(ptr(offset)).readPointer();
      let pointeeVtable = null;
      let pointeeVtableRva = null;
      if (!field.isNull()) {{
        try {{
          pointeeVtable = field.readPointer();
          if (pointeeVtable.compare(module.base) >= 0 &&
              pointeeVtable.compare(module.base.add(module.size)) < 0) {{
            pointeeVtableRva = hx(pointeeVtable.sub(module.base));
          }}
        }} catch (_) {{ pointeeVtable = null; }}
      }}
      rows.push({{offset: offset, value: hx(field), readable: true,
        pointee_vtable_ptr: pointeeVtable === null ? null : hx(pointeeVtable),
        pointee_vtable_rva: pointeeVtableRva}});
    }} catch (_) {{
      rows.push({{offset: offset, value: null, readable: false,
        pointee_vtable_ptr: null, pointee_vtable_rva: null}});
    }}
  }}
  return rows;
}}
function receiverPointeeQwords(value, specs) {{
  const rows = [];
  for (const spec of specs) {{
    try {{
      const pointee = value.add(ptr(spec.field_offset)).readPointer();
      rows.push({{field_offset: spec.field_offset, pointer: hx(pointee), readable: true,
        qwords: receiverQwords(pointee, spec.pointee_qword_offsets)}});
    }} catch (_) {{
      rows.push({{field_offset: spec.field_offset, pointer: null, readable: false, qwords: []}});
    }}
  }}
  return rows;
}}
function receiverPointerChains(value, specs) {{
  const rows = [];
  for (const spec of specs) {{
    let current = value;
    const steps = [];
    let readable = true;
    for (const offset of spec.pointer_offsets) {{
      try {{
        current = current.add(ptr(offset)).readPointer();
        steps.push({{offset: offset, pointer: hx(current), readable: true}});
      }} catch (_) {{
        steps.push({{offset: offset, pointer: null, readable: false}});
        readable = false;
        break;
      }}
    }}
    rows.push({{pointer_offsets: spec.pointer_offsets, steps: steps,
      pointer: readable ? hx(current) : null, readable: readable,
      qwords: readable ? receiverQwords(current, spec.qword_offsets) : []}});
  }}
  return rows;
}}
function argumentPointerArrays(context, specs) {{
  const arrays = [];
  for (const spec of specs) {{
    const root = context[spec.register];
    const row = {{register: spec.register, root: hx(root), readable: false,
      declared_count: null, captured_count: 0, items: []}};
    try {{
      const data = root.add(ptr(spec.data_offset)).readPointer();
      const count = root.add(ptr(spec.count_offset)).readU32();
      row.readable = true;
      row.data = hx(data);
      row.declared_count = count;
      const available = Math.max(0, count - spec.start_index);
      const limit = Math.min(available, spec.max_items);
      for (let relativeIndex = 0; relativeIndex < limit; relativeIndex++) {{
        const index = spec.start_index + relativeIndex;
        try {{
          const item = data.add(index * Process.pointerSize).readPointer();
          const info = receiver(item);
          const slots = [];
          const qwords = receiverQwords(item, spec.item_qword_offsets);
          if (info.readable) {{
            const vtable = item.readPointer();
            for (const slotOffset of spec.item_vtable_slot_offsets) {{
              try {{
                const target = vtable.add(ptr(slotOffset)).readPointer();
                let targetRva = null;
                if (target.compare(module.base) >= 0 &&
                    target.compare(module.base.add(module.size)) < 0) {{
                  targetRva = hx(target.sub(module.base));
                }}
                slots.push({{offset: slotOffset, target: hx(target), target_rva: targetRva,
                  readable: true}});
              }} catch (_) {{
                slots.push({{offset: slotOffset, target: null, target_rva: null,
                  readable: false}});
              }}
            }}
          }}
          row.items.push({{index: index, pointer: hx(item), receiver: info,
            qwords: qwords, vtable_slots: slots}});
        }} catch (_) {{
          row.items.push({{index: index, pointer: null,
            receiver: {{pointer: null, readable: false}}, qwords: [], vtable_slots: []}});
        }}
      }}
      row.captured_count = row.items.length;
    }} catch (_) {{}}
    arrays.push(row);
  }}
  return arrays;
}}
for (const target of targets) {{
  const address = module.base.add(ptr(target.rva));
  counts[target.symbol_id] = 0;
  encounters[target.symbol_id] = 0;
  try {{
    Interceptor.attach(address, {{
    onEnter(args) {{
      const receiverInfo = receiver(this.context.rcx);
      if (target.receiver_vtable_rvas.length !== 0 &&
          !target.receiver_vtable_rvas.includes(receiverInfo.vtable_rva)) {{
        this.capture = false;
        return;
      }}
      const encountered = encounters[target.symbol_id];
      encounters[target.symbol_id] = encountered + 1;
      if (encountered < target.skip_hits) {{ this.capture = false; return; }}
      const seen = counts[target.symbol_id];
      if (seen >= maxHits) {{ this.capture = false; return; }}
      counts[target.symbol_id] = seen + 1;
      this.capture = true;
      this.hit = seen + 1;
      send({{
        type: 'enter', symbol_id: target.symbol_id, rva: target.rva,
        hit: this.hit, thread_id: this.threadId,
        registers: {{ rip: hx(this.context.rip), rsp: hx(this.context.rsp),
          rcx: hx(this.context.rcx), rdx: hx(this.context.rdx),
          r8: hx(this.context.r8), r9: hx(this.context.r9) }},
        receiver: receiverInfo,
        receiver_qwords: receiverQwords(this.context.rcx, target.receiver_qword_offsets),
        receiver_pointee_qwords: receiverPointeeQwords(
          this.context.rcx, target.receiver_pointee_qword_offsets),
        receiver_pointer_chains: receiverPointerChains(
          this.context.rcx, target.receiver_pointer_chains),
        argument_pointer_arrays: argumentPointerArrays(this.context, target.argument_pointer_arrays){return_address_field}
      }});
    }},
    onLeave(retval) {{
      if (!this.capture) return;
      send({{type: 'leave', symbol_id: target.symbol_id, rva: target.rva,
        hit: this.hit, thread_id: this.threadId, retval: hx(retval)}});
    }}
    }});
  }} catch (error) {{
    send({{type: 'target_error', symbol_id: target.symbol_id, rva: target.rva,
      description: String(error)}});
  }}
}}
send({{type: 'ready', targets: targets.length}});
"""


def _build_targets(
    registry: Registry, build_id: str, symbol_ids: list[str], max_hits: int,
    raw_filters: dict, raw_offsets: dict, raw_arrays: dict, raw_pointee_offsets: dict,
    raw_pointer_chains: dict, raw_skip_hits: dict, raw_return_addresses: dict,
) -> list[dict[str, Any]]:
    """The per-symbol target list, and the capture budgets it has to fit.

    Extracted from `trace_calls` so a test can drive the REAL construction
    rather than a hand-built copy of it. What an old job spec records in its
    artifact and injects into a shared process is decided here, so this is
    the function that has to stay byte-compatible.
    """
    targets = []
    total_pointer_items = 0
    total_pointer_qwords = 0
    total_pointee_qwords = 0
    total_chain_qwords = 0
    for symbol_id in symbol_ids:
        instance = registry.resolve_symbol(symbol_id, build_id)
        skip_value = raw_skip_hits.get(symbol_id, 0)
        if not isinstance(skip_value, int) or skip_value < 0 or skip_value > 65535:
            raise ValueError("skip_hits values must be integers from 0 through 65535")
        values = raw_filters.get(symbol_id, [])
        if not isinstance(values, list) or len(values) > 32:
            raise ValueError("each receiver_vtable_rvas entry must contain at most 32 RVAs")
        parsed_filters = []
        for value in values:
            parsed = int(value, 0) if isinstance(value, str) else int(value)
            if parsed <= 0:
                raise ValueError("receiver vtable RVAs must be positive")
            parsed_filters.append(f"0x{parsed:x}")
        if len(set(parsed_filters)) != len(parsed_filters):
            raise ValueError("receiver vtable RVA filters must be unique")
        offsets = raw_offsets.get(symbol_id, [])
        if not isinstance(offsets, list) or len(offsets) > 32:
            raise ValueError("each receiver_qword_offsets entry must contain at most 32 offsets")
        parsed_offsets = []
        for value in offsets:
            parsed = int(value, 0) if isinstance(value, str) else int(value)
            if parsed < 0 or parsed > 0x10000 or parsed % 8 != 0:
                raise ValueError("receiver qword offsets must be aligned and within 0x0..0x10000")
            parsed_offsets.append(f"0x{parsed:x}")
        if len(set(parsed_offsets)) != len(parsed_offsets):
            raise ValueError("receiver qword offsets must be unique")
        pointee_specs = raw_pointee_offsets.get(symbol_id, [])
        if not isinstance(pointee_specs, list) or len(pointee_specs) > 8:
            raise ValueError("each receiver_pointee_qword_offsets entry must contain at most 8 pointees")
        parsed_pointees = []
        for spec in pointee_specs:
            if not isinstance(spec, dict) or set(spec) != {"field_offset", "pointee_qword_offsets"}:
                raise ValueError("receiver pointee qword specifications require field_offset and pointee_qword_offsets")
            field_value = spec["field_offset"]
            field_offset = int(field_value, 0) if isinstance(field_value, str) else int(field_value)
            if field_offset < 0 or field_offset > 0x10000 or field_offset % 8 != 0:
                raise ValueError("receiver pointee field offsets must be aligned within 0x0..0x10000")
            qword_values = spec["pointee_qword_offsets"]
            if not isinstance(qword_values, list) or len(qword_values) > 64:
                raise ValueError("each pointee_qword_offsets list must contain at most 64 offsets")
            qword_offsets = []
            for value in qword_values:
                parsed = int(value, 0) if isinstance(value, str) else int(value)
                if parsed < 0 or parsed > 0x10000 or parsed % 8 != 0:
                    raise ValueError("pointee qword offsets must be aligned within 0x0..0x10000")
                qword_offsets.append(f"0x{parsed:x}")
            if len(set(qword_offsets)) != len(qword_offsets):
                raise ValueError("pointee qword offsets must be unique")
            total_pointee_qwords += len(qword_offsets)
            parsed_pointees.append({
                "field_offset": f"0x{field_offset:x}",
                "pointee_qword_offsets": qword_offsets,
            })
        chain_specs = raw_pointer_chains.get(symbol_id, [])
        if not isinstance(chain_specs, list) or len(chain_specs) > 8:
            raise ValueError("each receiver_pointer_chains entry must contain at most 8 chains")
        parsed_chains = []
        for spec in chain_specs:
            if not isinstance(spec, dict) or set(spec) != {"pointer_offsets", "qword_offsets"}:
                raise ValueError("receiver pointer chains require pointer_offsets and qword_offsets")
            raw_chain_offsets = spec["pointer_offsets"]
            if not isinstance(raw_chain_offsets, list) or not raw_chain_offsets or len(raw_chain_offsets) > 4:
                raise ValueError("pointer_offsets must contain 1..4 offsets")
            chain_offsets = []
            for value in raw_chain_offsets:
                parsed = int(value, 0) if isinstance(value, str) else int(value)
                if parsed < 0 or parsed > 0x10000 or parsed % 8 != 0:
                    raise ValueError("pointer chain offsets must be aligned within 0x0..0x10000")
                chain_offsets.append(f"0x{parsed:x}")
            raw_chain_qwords = spec["qword_offsets"]
            if not isinstance(raw_chain_qwords, list) or len(raw_chain_qwords) > 64:
                raise ValueError("chain qword_offsets must contain at most 64 offsets")
            chain_qwords = []
            for value in raw_chain_qwords:
                parsed = int(value, 0) if isinstance(value, str) else int(value)
                if parsed < 0 or parsed > 0x10000 or parsed % 8 != 0:
                    raise ValueError("chain qword offsets must be aligned within 0x0..0x10000")
                chain_qwords.append(f"0x{parsed:x}")
            if len(set(chain_qwords)) != len(chain_qwords):
                raise ValueError("chain qword offsets must be unique")
            total_chain_qwords += len(chain_qwords)
            parsed_chains.append({"pointer_offsets": chain_offsets, "qword_offsets": chain_qwords})
        array_specs = raw_arrays.get(symbol_id, [])
        if not isinstance(array_specs, list) or len(array_specs) > 4:
            raise ValueError("each argument_pointer_arrays entry must contain at most 4 arrays")
        parsed_arrays = []
        for spec in array_specs:
            if not isinstance(spec, dict):
                raise ValueError("argument pointer array specifications must be objects")
            allowed = {"register", "data_offset", "count_offset", "start_index", "max_items", "item_qword_offsets", "item_vtable_slot_offsets"}
            if set(spec) - allowed:
                raise ValueError("unsupported argument pointer array field")
            register = str(spec.get("register", "")).lower()
            if register not in {"rcx", "rdx", "r8", "r9"}:
                raise ValueError("argument pointer array register must be rcx, rdx, r8, or r9")
            data_offset = int(spec.get("data_offset", 0), 0) if isinstance(spec.get("data_offset", 0), str) else int(spec.get("data_offset", 0))
            count_offset = int(spec.get("count_offset", 0), 0) if isinstance(spec.get("count_offset", 0), str) else int(spec.get("count_offset", 0))
            start_index = int(spec.get("start_index", 0))
            max_items_value = int(spec.get("max_items", 0))
            if data_offset < 0 or data_offset > 0x1000 or data_offset % 8 != 0:
                raise ValueError("argument pointer array data_offset must be aligned within 0x0..0x1000")
            if count_offset < 0 or count_offset > 0x1000 or count_offset % 4 != 0:
                raise ValueError("argument pointer array count_offset must be 4-byte aligned within 0x0..0x1000")
            if max_items_value < 1 or max_items_value > 4096:
                raise ValueError("argument pointer array max_items must be from 1 through 4096")
            if start_index < 0 or start_index > 65535:
                raise ValueError("argument pointer array start_index must be from 0 through 65535")
            raw_item_qwords = spec.get("item_qword_offsets", [])
            if not isinstance(raw_item_qwords, list) or len(raw_item_qwords) > 16:
                raise ValueError("item_qword_offsets must contain at most 16 offsets")
            item_qwords = []
            for value in raw_item_qwords:
                parsed = int(value, 0) if isinstance(value, str) else int(value)
                if parsed < 0 or parsed > 0x10000 or parsed % 8 != 0:
                    raise ValueError("item qword offsets must be aligned within 0x0..0x10000")
                item_qwords.append(f"0x{parsed:x}")
            if len(set(item_qwords)) != len(item_qwords):
                raise ValueError("item qword offsets must be unique")
            raw_slots = spec.get("item_vtable_slot_offsets", [])
            if not isinstance(raw_slots, list) or len(raw_slots) > 16:
                raise ValueError("item_vtable_slot_offsets must contain at most 16 offsets")
            slots = []
            for value in raw_slots:
                parsed = int(value, 0) if isinstance(value, str) else int(value)
                if parsed < 0 or parsed > 0x1000 or parsed % 8 != 0:
                    raise ValueError("item vtable slot offsets must be aligned within 0x0..0x1000")
                slots.append(f"0x{parsed:x}")
            if len(set(slots)) != len(slots):
                raise ValueError("item vtable slot offsets must be unique")
            total_pointer_items += max_items_value
            total_pointer_qwords += max_items_value * len(item_qwords)
            parsed_arrays.append({
                "register": register,
                "data_offset": f"0x{data_offset:x}",
                "count_offset": f"0x{count_offset:x}",
                "start_index": start_index,
                "max_items": max_items_value,
                "item_qword_offsets": item_qwords,
                "item_vtable_slot_offsets": slots,
            })
        wants_return_address = raw_return_addresses.get(symbol_id, False)
        target = {
            "symbol_id": symbol_id,
            "rva": f"0x{int(instance['rva']):x}",
            "receiver_vtable_rvas": parsed_filters,
            "receiver_qword_offsets": parsed_offsets,
            "receiver_pointee_qword_offsets": parsed_pointees,
            "receiver_pointer_chains": parsed_chains,
            "argument_pointer_arrays": parsed_arrays,
            "skip_hits": skip_value,
        }
        # ABSENT rather than false when unasked, so a job spec written before
        # this knob existed produces a byte-identical target list, a
        # byte-identical injected script, and the same schema tag.
        if wants_return_address:
            target["capture_return_address"] = True
        targets.append(target)
    if total_pointer_items * max_hits > 8192:
        raise ValueError("argument pointer array capture exceeds the 8192-item trace budget")
    if total_pointer_qwords * max_hits > 8192:
        raise ValueError("argument pointer array qword capture exceeds the 8192-qword trace budget")
    if total_pointee_qwords * max_hits > 4096:
        raise ValueError("receiver pointee qword capture exceeds the 4096-qword trace budget")
    if total_chain_qwords * max_hits > 4096:
        raise ValueError("receiver pointer-chain qword capture exceeds the 4096-qword trace budget")
    return targets



def trace_calls(
    registry: Registry, workspace: Path, build_identifier: str,
    symbol_ids: list[str], duration_s: float, max_hits: int,
    lease_path: Path, partial_path: Path,
    receiver_vtable_rvas: dict[str, list[str | int]] | None = None,
    receiver_qword_offsets: dict[str, list[str | int]] | None = None,
    argument_pointer_arrays: dict[str, list[dict[str, Any]]] | None = None,
    receiver_pointee_qword_offsets: dict[str, list[dict[str, Any]]] | None = None,
    receiver_pointer_chains: dict[str, list[dict[str, Any]]] | None = None,
    skip_hits: dict[str, int] | None = None,
    capture_return_address: dict[str, bool] | None = None,
) -> dict[str, Any]:
    if not isinstance(symbol_ids, list) or not symbol_ids or len(symbol_ids) > 32:
        raise ValueError("symbol_ids must contain 1..32 logical symbol IDs")
    if not isinstance(duration_s, (int, float)) or not 0.1 <= float(duration_s) <= 30.0:
        raise ValueError("duration_s must be from 0.1 through 30.0")
    if not isinstance(max_hits, int) or not 1 <= max_hits <= 64:
        raise ValueError("max_hits must be from 1 through 64")
    # Validated up here, BEFORE the Frida import, so a malformed request fails
    # with the reason rather than with ModuleNotFoundError on a machine that has
    # no Frida. `is None` rather than `or {}`: an empty list or 0 is a malformed
    # request, not an absent one. The knobs below keep their existing `or {}`
    # because changing their behaviour is not this lane's to make.
    raw_return_addresses = {} if capture_return_address is None else capture_return_address
    if not isinstance(raw_return_addresses, dict) or any(
        key not in symbol_ids for key in raw_return_addresses
    ):
        raise ValueError("capture_return_address keys must be requested symbol IDs")
    if any(value is not True and value is not False for value in raw_return_addresses.values()):
        raise ValueError("capture_return_address values must be booleans")
    import frida

    build = registry.get_build(build_identifier)
    raw_filters = receiver_vtable_rvas or {}
    if not isinstance(raw_filters, dict) or any(key not in symbol_ids for key in raw_filters):
        raise ValueError("receiver_vtable_rvas keys must be requested symbol IDs")
    raw_offsets = receiver_qword_offsets or {}
    if not isinstance(raw_offsets, dict) or any(key not in symbol_ids for key in raw_offsets):
        raise ValueError("receiver_qword_offsets keys must be requested symbol IDs")
    raw_arrays = argument_pointer_arrays or {}
    if not isinstance(raw_arrays, dict) or any(key not in symbol_ids for key in raw_arrays):
        raise ValueError("argument_pointer_arrays keys must be requested symbol IDs")
    raw_pointee_offsets = receiver_pointee_qword_offsets or {}
    if not isinstance(raw_pointee_offsets, dict) or any(key not in symbol_ids for key in raw_pointee_offsets):
        raise ValueError("receiver_pointee_qword_offsets keys must be requested symbol IDs")
    raw_pointer_chains = receiver_pointer_chains or {}
    if not isinstance(raw_pointer_chains, dict) or any(key not in symbol_ids for key in raw_pointer_chains):
        raise ValueError("receiver_pointer_chains keys must be requested symbol IDs")
    raw_skip_hits = skip_hits or {}
    if not isinstance(raw_skip_hits, dict) or any(key not in symbol_ids for key in raw_skip_hits):
        raise ValueError("skip_hits keys must be requested symbol IDs")
    targets = _build_targets(
        registry, build['build_id'], symbol_ids, max_hits,
        raw_filters, raw_offsets, raw_arrays, raw_pointee_offsets,
        raw_pointer_chains, raw_skip_hits, raw_return_addresses,
    )
    winmem = _load_winmem(workspace)
    with RuntimeLease(lease_path):
        process = winmem.open_process("hoi4.exe", "hoi4.exe")
        if process is None:
            raise RuntimeError("hoi4.exe is not running")
        with process:
            fingerprint = process.fingerprint()
            pid = process.pid
        if str(fingerprint.get("sha256", "")).lower() != build["sha256"]:
            raise RuntimeError("live executable does not match the requested build; no force override is available")
        messages: list[dict[str, Any]] = []
        errors: list[dict[str, Any]] = []

        def on_message(message: dict[str, Any], data: bytes | None) -> None:
            if message.get("type") == "send" and isinstance(message.get("payload"), dict):
                messages.append(message["payload"])
            else:
                errors.append({"message": message, "data_bytes": 0 if data is None else len(data)})

        session = frida.attach(pid)
        script = None
        cleanup_errors: list[str] = []
        try:
            script = session.create_script(_trace_script(targets, max_hits))
            script.on("message", on_message)
            script.load()
            time.sleep(float(duration_s))
        finally:
            if script is not None:
                try:
                    script.unload()
                except Exception as error:
                    cleanup_errors.append(f"script.unload: {type(error).__name__}: {error}")
            try:
                session.detach()
            except Exception as error:
                cleanup_errors.append(f"session.detach: {type(error).__name__}: {error}")
        result = {
            "schema": _trace_schema(targets),
            "mode": "hook-based observational instrumentation",
            "build_id": build["build_id"],
            "exe_sha256": build["sha256"],
            "pid": pid,
            "duration_s": float(duration_s),
            "max_hits": max_hits,
            "targets": targets,
            "messages": messages,
            "errors": errors,
            "cleanup_errors": cleanup_errors,
            "promotion_allowed": False,
        }
        atomic_json(partial_path, result)
        if cleanup_errors:
            raise RuntimeError("Frida cleanup failed; the runtime session is tainted: " + "; ".join(cleanup_errors))
        return result
