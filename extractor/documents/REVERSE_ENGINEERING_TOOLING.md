# Reverse Engineering Tooling

**Purpose:** Inventory the Windows-only local reverse-engineering toolchain and the evidence rules
for safely promoting findings into the extractor.
**Verification:** Command/path visibility was re-verified 2026-07-14; version labels remain the
2026-06-29 inventory snapshot and should be rechecked before version-sensitive work.
**Completion status:** Extractor progress is tracked only in
`extractor/documents/MASTER_EXTRACTION_CONTRACT.md`.

This machine has a local reverse-engineering toolchain for the Battle Plan DLL
work. Prefer scriptable tools first, and use GUI tools only when a task really
needs live visual inspection or breakpoint work.

## Agent-First Tools

These are the best tools for automated evidence because they produce command
line output or files agents can inspect.

### Ghidra Headless

- GUI launcher: `ghidraRun.bat`
- Headless analyzer: `analyzeHeadless.bat`
- Installed under: `%USERPROFILE%\Downloads\ghidra_12.1.2_PUBLIC_20260605\ghidra_12.1.2_PUBLIC`
- Headless support path is in the user `PATH`.

Use for static analysis of `hoi4.exe`: vtables, function xrefs, string xrefs,
struct layout hints, build-to-build diff inputs, and named analysis scripts.

Example:

```powershell
analyzeHeadless.bat C:\Tools\ghidra_projects hoi4_latest -import "<BATTLE_PLAN_INSTALL>\hoi4.exe" -analysisTimeoutPerFile 1800
```

### Python Binary Analysis Stack

Installed for Python 3.11:

- `pefile`
- `lief`
- `capstone`
- `keystone-engine`
- `unicorn`
- `frida`
- `frida-tools`

Frida scripts are in `%APPDATA%\Python\Python311\Scripts`, which is in the user
`PATH`.

Use Python scripts for repeatable PE inspection, signature extraction,
disassembly checks, pattern searches, hook prototypes, and evidence dumps.

Example:

```powershell
@'
import pefile
pe = pefile.PE(r"<BATTLE_PLAN_INSTALL>\hoi4.exe")
print(hex(pe.OPTIONAL_HEADER.ImageBase))
'@ | python -
```

### Frida

- Commands: `frida`, `frida-ps`, `frida-trace`
- Version verified: `17.15.3`

Use for live, scriptable process inspection and temporary hooks. Keep probes
bounded and evidence-focused. Do not leave persistent hooks or modify gameplay
state.

Examples:

```powershell
frida-ps -a | Select-String the game
frida -p <pid> -l .\scratch\probe.js
```

### Windows Debugging Tools

- Command-line debugger: `cdb`
- Path: `C:\Program Files (x86)\Windows Kits\10\Debuggers\x64\cdb.exe`
- Version verified: `10.0.18362.1`

Use `cdb` for scriptable breakpoint/call-path proof. Attaching a debugger can
pause the game; only do this when the task explicitly allows live debug
interruption.

Example:

```powershell
cdb -p <pid> -c "lm m the game; q"
```

### LLVM

- Commands: `llvm-objdump`, `llvm-readobj`, `llvm-strings`, `llvm-nm`,
  `llvm-pdbutil`, `clang`, `clang-cl`
- Version installed by winget: LLVM `22.1.8`
- Path: `C:\Program Files\LLVM\bin`

Use for PE/object inspection, imports/exports, strings, and native build/debug
helpers when MSVC tools are not loaded.

Examples:

```powershell
llvm-readobj --file-headers "<BATTLE_PLAN_INSTALL>\hoi4.exe"
llvm-strings "<BATTLE_PLAN_INSTALL>\hoi4.exe" | rg "production|combat|focus"
```

### Rizin And Radare2

Rizin:

- Commands: `rizin`, `rz-bin`, `rz-find`, `rz-hash`
- Version verified: `0.8.2`
- Path: `C:\Program Files\Rizin\bin`

Radare2:

- Commands: `radare2`, `rabin2`, `rahash2`
- Version verified: `6.1.8`
- Path: `C:\Tools\radare2-6.1.8\radare2-6.1.8-w64\bin`

Use for quick scripted disassembly, binary metadata, strings, and xref-oriented
experiments when Ghidra would be too heavy.

Example:

```powershell
rabin2 -I "<BATTLE_PLAN_INSTALL>\hoi4.exe"
rz-bin -I "<BATTLE_PLAN_INSTALL>\hoi4.exe"
```

### Sysinternals Suite

Commands available through winget aliases include:

- `sigcheck`
- `Listdlls`
- `handle`
- `procexp`
- `vmmap`
- `procdump`
- `Procmon`

Use for process/module inventory, memory-map sanity checks, loaded DLLs,
signature checks, and diagnostic dumps. GUI tools such as Process Explorer,
VMMap, and ProcMon are useful, but prefer CLI/export output when possible.

Examples:

```powershell
sigcheck -nobanner "<BATTLE_PLAN_INSTALL>\hoi4.exe"
Listdlls the game
vmmap <pid>
```

### 7-Zip

- Command: `7z`
- Version verified: `26.01`
- Path: `C:\Program Files\7-Zip`

Use for deterministic extraction of release archives and tool bundles.

## GUI / Human-Assisted Tools

These are installed and useful, but agents should treat them as human-assisted
or evidence-export tools unless a task explicitly calls for GUI control.

### ReClass.NET

- Launcher: `C:\Tools\ReClass.NET-v1.2\ReClass.NET_Launcher.exe`
- x64 binary: `C:\Tools\ReClass.NET-v1.2\x64\ReClass.NET.exe`
- Version bundle: `v1.2`

Use for live class/struct layout exploration after an address/root is known.
Save ReClass project files or screenshots as evidence. Do not use a live layout
as proof until it has been cross-checked against static analysis or memory
reader tests.

### x64dbg

- Launcher alias: `x96dbg`
- x64 binary:
  `%LOCALAPPDATA%\Microsoft\WinGet\Packages\x64dbg.x64dbg_Microsoft.Winget.Source_8wekyb3d8bbwe\release\x64\x64dbg.exe`
- Version installed by winget: `2026.05.27`

Use for live breakpoint validation, function call paths, and confirming
read/write sites. Attaching can pause or perturb the live game; get explicit
permission before using breakpoints in a running session that matters.

### Cheat Engine

- Installed path: `C:\Program Files\Cheat Engine\Cheat Engine.exe`
- Existing installer in Downloads verified with a valid Authenticode signature.

Use only as a scout for value scans and pointer relationship discovery. Do not
build production extraction around Cheat Engine tables or pointer scans; promote
only independently validated offsets/roots into code.

### Dependencies

- Command/GUI alias: `DependenciesGui`
- Installed by winget: `lucasg.Dependencies` `2.2.0`

Use for DLL imports/exports and dependency checks, especially the replay
collector DLL and any injected/native helper binaries.

## Native Build Tools

Rust is available via `cargo` and `rustc`.

Visual Studio Build Tools are installed at:

```text
C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools
```

MSVC commands such as `cl`, `link`, and `dumpbin` are not available in a normal
PowerShell until the VC environment is loaded. Use:

```powershell
cmd /c '"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" && dumpbin /headers extractor\native\target\debug\collector.dll'
```

Prefer `llvm-readobj`, `llvm-objdump`, or `DependenciesGui` when a full MSVC
developer shell is not necessary.

## Operating Rules For Agents

- Prefer repeatable command-line evidence over screenshots.
- Keep reverse-engineering outputs under ignored scratch/output folders unless
  the user asks for a checked-in artifact.
- Do not commit local machine paths, home-directory paths, downloaded binaries,
  ReClass projects, Ghidra projects, memory dumps, or raw process dumps.
- For live the game work, state whether the probe is passive, debugger-attached, or
  hook-based before running it.
- Do not use heap scans in the DLL hot loop. Use these tools to prove bounded
  roots, bridges, lifecycle hooks, and validation contracts.
- Any offset promoted into code needs at least two forms of evidence, such as
  Ghidra xrefs plus live ReClass/CDB/Frida proof, or memory-reader proof plus
  save-oracle validation.

## Quick Capability Check

Run this in a fresh shell to confirm the key tools are visible:

```powershell
$tools = @(
  "analyzeHeadless.bat", "cdb.exe", "frida.exe", "llvm-objdump.exe",
  "radare2.exe", "rizin.exe", "sigcheck.exe", "7z.exe"
)
foreach ($tool in $tools) {
  Get-Command $tool -ErrorAction SilentlyContinue |
    Select-Object Name, Source
}
```
