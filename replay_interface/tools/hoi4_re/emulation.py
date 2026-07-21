from __future__ import annotations

from pathlib import Path
from typing import Any


PAGE = 0x1000
STACK_BASE = 0x700000000000
STACK_SIZE = 0x200000
SENTINEL = 0x710000000000


def _align_up(value: int, alignment: int = PAGE) -> int:
    return (value + alignment - 1) & ~(alignment - 1)


def _register_map():
    from unicorn.x86_const import (
        UC_X86_REG_RAX, UC_X86_REG_RBX, UC_X86_REG_RCX, UC_X86_REG_RDX,
        UC_X86_REG_RSI, UC_X86_REG_RDI, UC_X86_REG_RBP, UC_X86_REG_RSP,
        UC_X86_REG_R8, UC_X86_REG_R9, UC_X86_REG_R10, UC_X86_REG_R11,
        UC_X86_REG_R12, UC_X86_REG_R13, UC_X86_REG_R14, UC_X86_REG_R15,
        UC_X86_REG_RIP,
    )
    return {
        "rax": UC_X86_REG_RAX, "rbx": UC_X86_REG_RBX, "rcx": UC_X86_REG_RCX,
        "rdx": UC_X86_REG_RDX, "rsi": UC_X86_REG_RSI, "rdi": UC_X86_REG_RDI,
        "rbp": UC_X86_REG_RBP, "rsp": UC_X86_REG_RSP, "r8": UC_X86_REG_R8,
        "r9": UC_X86_REG_R9, "r10": UC_X86_REG_R10, "r11": UC_X86_REG_R11,
        "r12": UC_X86_REG_R12, "r13": UC_X86_REG_R13, "r14": UC_X86_REG_R14,
        "r15": UC_X86_REG_R15, "rip": UC_X86_REG_RIP,
    }


def emulate_x64_blob(code: bytes, registers: dict[str, int] | None = None, max_instructions: int = 10000) -> dict[str, Any]:
    from unicorn import Uc, UC_ARCH_X86, UC_HOOK_CODE, UC_MODE_64

    uc = Uc(UC_ARCH_X86, UC_MODE_64)
    code_base = 0x1000000
    uc.mem_map(code_base, _align_up(len(code)))
    uc.mem_write(code_base, code)
    uc.mem_map(STACK_BASE, STACK_SIZE)
    uc.mem_map(SENTINEL, PAGE)
    stack_pointer = STACK_BASE + STACK_SIZE - 0x100
    uc.mem_write(stack_pointer, SENTINEL.to_bytes(8, "little"))
    regmap = _register_map()
    uc.reg_write(regmap["rsp"], stack_pointer)
    for name, value in (registers or {}).items():
        if name.lower() not in regmap or name.lower() in {"rsp", "rip"}:
            raise ValueError(f"unsupported or reserved input register: {name}")
        uc.reg_write(regmap[name.lower()], int(value))
    executed = 0

    def hook(emu, address, _size, _data):
        nonlocal executed
        executed += 1
        if address == SENTINEL:
            emu.emu_stop()

    uc.hook_add(UC_HOOK_CODE, hook)
    uc.emu_start(code_base, 0, count=max_instructions)
    return {
        "executed_instructions": executed,
        "registers": {name: f"0x{uc.reg_read(reg):x}" for name, reg in regmap.items()},
        "returned": uc.reg_read(regmap["rip"]) == SENTINEL or executed < max_instructions,
    }


def emulate_function(
    exe_path: Path, image_base: int, function_rva: int, function_call_count: int,
    inputs: dict[str, Any],
) -> dict[str, Any]:
    if function_call_count != 0:
        raise ValueError("initial emulator admits leaf functions only; this function contains calls")
    import pefile
    from unicorn import Uc, UcError, UC_ARCH_X86, UC_HOOK_CODE, UC_MODE_64

    pe = pefile.PE(str(exe_path), fast_load=False)
    preferred = int(pe.OPTIONAL_HEADER.ImageBase)
    if preferred != image_base:
        raise RuntimeError(f"registered image base 0x{image_base:x} does not match PE 0x{preferred:x}")
    uc = Uc(UC_ARCH_X86, UC_MODE_64)
    image_size = _align_up(int(pe.OPTIONAL_HEADER.SizeOfImage))
    uc.mem_map(image_base, image_size)
    headers = pe.__data__[: int(pe.OPTIONAL_HEADER.SizeOfHeaders)]
    uc.mem_write(image_base, headers)
    for section in pe.sections:
        data = section.get_data()
        if data:
            uc.mem_write(image_base + int(section.VirtualAddress), data)
    uc.mem_map(STACK_BASE, STACK_SIZE)
    uc.mem_map(SENTINEL, PAGE)
    stack_pointer = STACK_BASE + STACK_SIZE - 0x100
    uc.mem_write(stack_pointer, SENTINEL.to_bytes(8, "little"))
    regmap = _register_map()
    uc.reg_write(regmap["rsp"], stack_pointer)
    for name, value in inputs.get("registers", {}).items():
        lowered = str(name).lower()
        if lowered not in regmap or lowered in {"rsp", "rip"}:
            raise ValueError(f"unsupported or reserved input register: {name}")
        uc.reg_write(regmap[lowered], int(value, 0) if isinstance(value, str) else int(value))
    for region in inputs.get("memory", []):
        address = region.get("address")
        if address is None:
            address = image_base + int(str(region["rva"]), 0)
        elif isinstance(address, str):
            address = int(address, 0)
        data = bytes.fromhex(region["data_hex"])
        if not (image_base <= address and address + len(data) <= image_base + image_size):
            page = address & ~(PAGE - 1)
            size = _align_up(address + len(data) - page)
            try:
                uc.mem_map(page, size)
            except UcError:
                pass
        uc.mem_write(address, data)
    executed = 0
    returned = False

    def hook(emu, address, _size, _data):
        nonlocal executed, returned
        executed += 1
        if address == SENTINEL:
            returned = True
            emu.emu_stop()

    uc.hook_add(UC_HOOK_CODE, hook)
    max_instructions = inputs.get("max_instructions", 100000)
    if not isinstance(max_instructions, int) or not 1 <= max_instructions <= 1_000_000:
        raise ValueError("max_instructions must be from 1 through 1000000")
    error = None
    try:
        uc.emu_start(image_base + function_rva, 0, count=max_instructions)
    except UcError as exc:
        error = str(exc)
    watched = []
    for region in inputs.get("watch", []):
        address = region.get("address")
        if address is None:
            address = image_base + int(str(region["rva"]), 0)
        elif isinstance(address, str):
            address = int(address, 0)
        size = int(region["size"])
        watched.append({"address": f"0x{address:x}", "data_hex": bytes(uc.mem_read(address, size)).hex()})
    return {
        "schema": "hoi4-re-unicorn-function-v1",
        "mode": "deterministic leaf-function emulation",
        "entry_rva": f"0x{function_rva:x}",
        "returned": returned,
        "executed_instructions": executed,
        "error": error,
        "registers": {name: f"0x{uc.reg_read(reg):x}" for name, reg in regmap.items()},
        "watched_memory": watched,
        "promotion_allowed": False,
        "limitations": ["leaf functions only", "no import stubs", "no TLS or engine lifecycle"],
    }
