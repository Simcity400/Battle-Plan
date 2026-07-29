# Entry/unwind instrumentation primitive — design

> **Read `documents/DECISIONS.md` first.** It is the target authority and outranks this file.
> This is a design for work that has not been built. Nothing here is a receipt.
>
> One claim in `DECISIONS.md` is contradicted by the evidence this design was built on; see
> §1.4. Where this file describes existing code it describes `production_hook_receipt` and the
> four consuming lanes as they stand on `codex/extractor-layer-recovery`.

Pinned program: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

`DECISIONS.md` allows executable instrumentation only "where a value is provably unsampleable,
through one audited primitive, and every point is registered and justified". This document
specifies that one primitive for **internal `hoi4.exe` functions**. It does not add a single new
instrumentation point.

---

## 1. The capability gap

### 1.1 What the two existing routes actually are

`extractor/native/lanes/production_hook_receipt/src/lib.rs` owns two routes.

**Route A — MinHook inline `JMP rel32`.** `ExpectedRvaHook` / `observe_enabled_minhook_rel5` /
`observe_released_set`. It does not install anything. It *observes* a hook the caller already
installed: it re-reads the five patched bytes, refuses `patchAbove`/split placement, decodes the
`E9 rel32` displacement to the relay, requires the relay to be the exact `FF 25 00000000` +
absolute-qword stub, and binds the decoded detour address to the one the caller declared. Custody
is a `(teardown_owner_token, requested_rva)` lease in a process-global registry; release requires
a `HookTeardownAttestation` with all three of `callbacks_quiesced`, `hook_removed`, `relay_freed`.

**Route B — PE64 import-address-slot CAS.** `ExpectedPe64ImportAddressSlot` /
`allocate_canonical_pe64_iat_shim` / `install_pe64_import_address_slot` / `quiesce_…` /
`restore_pe64_import_address_slot_process_lifetime`. It changes no code bytes and suspends no
thread. It allocates a **sealed counted shim** whose entry bytes are digest-pinned, registers
`RUNTIME_FUNCTION`/`UNWIND_INFO` for it with `RtlAddFunctionTable`, verifies the registration with
`RtlLookupFunctionEntry`, and then swaps the import slot with an aligned atomic CAS. Teardown is a
four-state lease (`Available → Pe64ImportSlotActive → …Terminalizing → …Terminal`), and the
allocation is **retained for process lifetime** because a target thread may already hold the old
pointer.

Route B's shim is the important prior art. Emitted by `allocate_canonical_pe64_iat_shim`, it is:

```
f0 48 ff 05 <rel32>     lock inc qword [rip+counter]
48 83 ec 58             sub  rsp, 0x58
   (0..7 stack-argument copies, per template)
ff 15 <rel32>           call qword [rip+dispatch]
f0 48 ff 0d <rel32>     lock dec qword [rip+counter]
48 83 c4 58             add  rsp, 0x58
c3                      ret
```

with `UNWIND_INFO = 11 0c 01 00 | 0c a2 00 00 | 00 01 00 00` — version 1, flags `UNW_FLAG_UHANDLER`,
prolog size `0x0c`, one `UWOP_ALLOC_SMALL(0x58)` at code offset `0x0c`, handler at allocation
`+0x100` — and a handler that tests `ExceptionFlags & 0x66`
(`UNWINDING|EXIT_UNWIND|TARGET_UNWIND|COLLIDED_UNWIND`), decrements the counter if any is set, and
returns `ExceptionContinueSearch`.

That is already an entry/unwind primitive. It is **balanced on the exceptional path**, and that is
exactly the property the four domains do not have.

### 1.2 What the four domains do today

All four install MinHook inline hooks on internal `hoi4.exe` functions and wrap the original call
inside a Rust `unsafe extern "system"` detour. Only d67 goes through `production_hook_receipt`;
d71, d95 and A29 call `minhook` directly with no custody receipt at all.

| | Lane file(s) | Points | Shape needed |
|---|---|---:|---|
| **D67** (A18) | `country_dynamic_modifiers_live/src/a729_hook.rs` | 2 | Outer bracket (before/after container snapshot) **plus** a nested inner hook whose observations must be correlated into the open outer bracket via `thread_local! OPEN_UPDATE` |
| **D71** (A28) | `dynamic_collection_trace_live/src/a729_named_overlap_hook.rs` and 6 siblings | **15** | Entry-only registration (record an argument pointer against an authored collection id) **plus** post-return-only observation for registered pointers |
| **D95** | `spotted_units_live/src/a729.rs` | 4 | Two frame brackets around a thread-local accumulator, **plus** two very hot inner return-value observers (per unit, per frame) |
| **A29** | `focus_research_decisions_depth_live/src/a729_research_ahead_hook.rs` | 4 | Entry + return-value capture **plus caller provenance** — the lane calls `RtlCaptureStackBackTrace` inside the detour and compares against pinned call-site RVAs (`UI_YEAR_HELPER_CALL_RVA`, `UI_YEAR_HELPER_RETURN_RVA`, …) |

Counts are `unsafe extern "system" fn hook_*` definitions per lane: **25 instrumentation points in
total**, of which D71 alone is 15 across seven hook modules. That D71 is by far the largest is not
visible from `RE_REMAINING.md` and matters twice over — for migration order (§1.3) and because
volume is the one variable measured to destabilise the process (§5).

### 1.3 Per-domain verdict

The question the brief asks is whether an existing route already serves each domain. The honest
answer for all four is **partly yes**, and it is worth stating plainly because it shrinks the work.

| Domain | Does route B (import slot) serve it? | Does route A (inline) serve it? | Verdict |
|---|---|---|---|
| **D71** | No — same reason. | **Yes, functionally**, and its needs are the weakest of the four *per point*: entry-only registration and post-return-only reads, no nesting, no return-value semantics. | **Served today.** D71 needs custody receipts and the shared teardown ledger, nothing else. But it is 15 points, not the 6 the named-overlap module suggests, so it is the **largest** migration and the last, not the first. It also owns the two widest arities in the product (§2.2). |
| **D95** | No — same reason. | **Yes, functionally**, but it is the domain where the mechanism costs most: `hook_intel_getter` runs once per unit per visibility frame on the renderer path. | **Served, but the wrong question is being asked.** See the open question in §8.3: if the POV-visible set is stored anywhere in `CGameState`, four hooks — including by far the hottest instrumentation point in the product — delete themselves. That is a bigger win than any primitive. |
| **A29** | No — same reason. | **Yes, but this is the one genuine capability gap.** A29 needs the *caller's* identity, and gets it by calling `RtlCaptureStackBackTrace` inside a hot detour. Neither existing route exposes an authenticated caller address. | **One new capability: authenticated caller RVA.** The shim already has the return address at `[rsp]` on entry, for free, before it touches the stack. Handing it to the observer as a validated RVA against the pinned base replaces a stack walk with a single `mov`. |

**So the capability gap is one field, not a mechanism.** What the four domains actually share is not
a missing capability but a missing *contract*: three of them have no custody receipt, all four close
their per-invocation state only on the normal-return path, three of them spin unbounded in teardown,
and one of them (`d95::uninstall`) calls `MinHook::remove_hook`, which frees the trampoline.

That reframing is the main product of this analysis, and it makes the design smaller.

### 1.4 A premise correction — the crash-mode claim

`DECISIONS.md` §"What is now known" states: *"Three of the six full-instrumentation crashes were a
call through a null pointer — the half-written-detour signature. The other three faulted at
`hoi4.exe+0xC66F46`, in the game's own code rather than in `collector.dll`, with the collector
confirmed loaded."*

That 3/3 split is the **superseded** run. `extractor/native/runs/crashrate/trials.jsonl` — the valid
2026-07-26 run — is 4 × `collector_null_call` at `0x0`, 1 × `other_crash` at `hoi4.exe+0xC66F46`,
and 1 × `other_crash` at **`0x3F800000`**, an address `DECISIONS.md` does not mention. The 3/3 split
with the `+0xC66F46` triple appears in `trials_20260726_invalid_minimal_arm.jsonl`.

Reading the six minidumps under
`…\Paradox Interactive\Hearts of Iron IV\crashes\` changes the picture further. All six:

- crash on a **`tbb.dll` worker thread**, with the identical frame shape
  (`tbb::…internal_initialize +11562 / +8547` → two unnamed `hoi4.exe` frames → `hoi4.exe`
  `PHYSFS_swapULE64 +11013981` → `+10938637`);
- have `[RSP] == hoi4.exe+0xC66F4D` — the same slot in the same frame of the same code path;
- contain **no `collector.dll` return address and no private-executable (trampoline) return address
  anywhere on the crashing thread's stack**, although `collector.dll` is loaded (verified against the
  module list and the memory-info list in each dump);
- report `EXCEPTION_ACCESS_VIOLATION` with parameters `(0, 0xFFFFFFFFFFFFFFFF)` — read access,
  *faulting address not available*.

For the `+0xC66F46` case specifically, the reported faulting instruction is
`MOV RSI, qword ptr [RSP+0x60]`, and at crash time `RSP` is inside the thread's stack, the page is
`MEM_COMMIT` / `PAGE_READWRITE`, and the qword is present in the dump. **The instruction's operand
was readable.** The runtime bytes at `+0xC66F26 … +0xC66F66` were also read out of the dump and are
byte-identical to the static a729 image: that call site is not patched.

Three consequences for this design.

1. **The "half-written detour" attribution is not supported.** In the `rip == 0` dump the return
   address on the stack is `hoi4.exe+0xC66F4D`, not a `collector.dll` address. The game called
   through a null pointer; our detour did not.
2. **`0x3F800000` is `1.0f`.** A call target read out of memory that held float data is type
   confusion, a torn object, or a use-after-free — a *lifetime and concurrency* failure on the
   game's worker pool, not a code-patch failure.
3. **Therefore an entry/unwind primitive will not, on this evidence, fix the injection crashes.**
   The two mechanical hypotheses that a primitive *would* address — a blown worker stack from extra
   detour frames, and a corrupted `RSP` from an unbalanced detour — are both ruled out by the dumps.
   This design must not be sold as a crash fix. Its case is custody, teardown and exception
   correctness. The crash cause remains open; §8.1 names the step that closes it.

---

## 2. The primitive

### 2.1 Name and shape

`EntryUnwindProbe`: one admitted internal-function instrumentation point, consisting of

- a **pinned probe spec** (`ExpectedEntryProbe`) in a closed, compiled-in registry;
- a **bracket shim** — owned position-independent code with registered unwind info — that sits
  between MinHook's relay and the lane's observer;
- two **sealed dispatch slots** (entry, return) holding exact PE exports of `collector.dll`;
- a **five-phase custody ledger** with a crate-owned quiesce budget and process-lifetime retention.

The structural change is one line:

```
today:     patch → relay → Rust detour { observe; call original; observe }
proposed:  patch → relay → bracket shim (asm) { count++; call entry_obs; call original;
                                                call return_obs; count-- }
```

**The original is called from the shim, not from Rust.** That single move is what makes the
exception story correct (§4), and it is why this design is a generalisation of route B's shim rather
than a new invention.

### 2.2 Bracket shim, register-args class

For the `Win64RegisterArgsOnly` arity class (≤4 integer/pointer arguments, integer or void return,
no by-value aggregates, no FP arguments):

```
        ; entry: RCX,RDX,R8,R9 = args ; [RSP] = caller return address
        mov     rax, [rsp]                 ; caller return address, free, pre-adjustment
   lock inc     qword [rip+counter]
        sub     rsp, 0x58                  ; 0x20 home + saves + 16B alignment
        mov     [rsp+0x30], rcx
        mov     [rsp+0x38], rdx
        mov     [rsp+0x40], r8
        mov     [rsp+0x48], r9
        mov     [rsp+0x28], rax            ; caller return address into the frame
        lea     rcx, [rsp+0x28]            ; &ProbeFrame
        call    qword [rip+entry_dispatch]
        mov     rcx, [rsp+0x30]
        mov     rdx, [rsp+0x38]
        mov     r8,  [rsp+0x40]
        mov     r9,  [rsp+0x48]
        call    qword [rip+original]       ; MinHook trampoline
        mov     [rsp+0x50], rax
        lea     rcx, [rsp+0x28]
        call    qword [rip+return_dispatch]
        mov     rax, [rsp+0x50]
   lock dec     qword [rip+counter]
        add     rsp, 0x58                  ; epilogue starts here and must stay contiguous
        ret
```

**The epilogue must be contiguous.** `lock dec` is placed *before* `add rsp, 0x58`, not between it
and the `ret`. A Windows x64 epilogue may contain only the stack deallocation, optional non-volatile
pops, and the `ret`; anything in between makes the sequence unrecognisable to the unwinder's
epilogue scan, and a thread interrupted at that instruction would have `RSP` already restored while
`RtlVirtualUnwind` applies `UWOP_ALLOC_SMALL` a second time. Route B's shipped template already has
the correct order; an earlier draft of this document inverted it.

**Frame layout, exactly.** `RSP` is `≡ 8 (mod 16)` on entry; `0x58` is `≡ 8 (mod 16)`, so `RSP` is
16-byte aligned at every `call` — the same arithmetic route B's template relies on, and any future
frame size must preserve `≡ 8 (mod 16)`.

| Range | Contents |
|---|---|
| `[rsp+0x00 … +0x1F]` | callee home space (owned by whatever we call) |
| `[rsp+0x20 … +0x27]` | padding (register-only class); see below for the stack-argument classes |
| `[rsp+0x28]` | caller return address |
| `[rsp+0x30 … +0x48]` | saved `RCX`, `RDX`, `R8`, `R9` |
| `[rsp+0x50]` | saved return value |

That is six qwords, and **that is the whole of `ProbeFrame`.** `caller_rva`, `depth` and
`invocation_id` are *not* materialised by the shim — there is no storage for them and no
instruction that would fill them. They are derived in the observer's prologue from the raw slice
(`resolve_caller_rva` in §7, the primitive's TLS depth, and one relaxed atomic), which is also
where they belong: the shim stays branch-free and its byte digest stays pinned. The observer-side
struct is `ProbeContext`; `ProbeFrame` is only the six raw slots above.

**Stack-argument classes.** The outgoing stack slots must sit immediately above the home space, at
`[rsp+0x20 + i*8]`, because that is where the callee will look for them. The `ProbeFrame` therefore
moves up rather than sharing that range: the frame grows by `8 × stack_argument_count` rounded up to
keep `≡ 8 (mod 16)`, `ProbeFrame` starts at `[rsp+0x20 + 8*stack_argument_count]`, and the six
qwords above keep their relative order from that new base. Concretely the census needs `0x58`
(0 stack args), `0x68` (1), `0x68` (2, using the existing padding) and `0x78` (3).

The stack arguments are copied **once**, from the caller's frame into their outgoing positions,
before the entry dispatch — route B's emitter already does exactly this copy, from
`[rsp + frame_size + 0x28 + i*8]` to `[rsp+0x20 + i*8]`. `ProbeFrame` records the base of the copy,
so the observer reads the same bytes the original will, and no second copy exists to drift.
`UWOP_ALLOC_SMALL` covers allocations of 8–128 bytes, so every class in the census stays a single
small-alloc unwind code.

`UNWIND_INFO` is emitted, not hardcoded: `SizeOfProlog` = the byte length through the `sub rsp`
(`0x10` for this layout — the `mov rax,[rsp]` is four bytes ahead of route B's `0x0c`), one
`UWOP_ALLOC_SMALL` at that code offset, `UNW_FLAG_UHANDLER`, handler RVA = allocation `+0x100`.
The argument stores need no unwind codes: they touch no non-volatile register and do not move
`RSP`. The handler is route B's existing 23-byte handler, **unchanged and doing exactly what it
does today**: on `ExceptionFlags & 0x66`, `lock dec` the counter, return `ExceptionContinueSearch`.
It touches no TLS and sets no flag, and Section 4.2 is written to that limit.

Three deliberate differences from route B's shim, each because route B's shim is a *pure forwarder*
and this one is not:

1. **Argument registers are saved and restored** across the observer call. Route B never needed
   this: it calls exactly one callee and never touches `RCX/RDX/R8/R9/XMM0-3`.
2. **The return value is preserved** across `return_dispatch`.
3. **`ProbeFrame` is the shim's own stack slice**, so the observers receive arguments, the caller
   return address and the return value by pointer, with no allocation and no TLS on the hot path.

**Arity census of the 25 points, taken from the declared detour signatures — not assumed.**

| Class | Points | Examples |
|---|---:|---|
| ≤4 integer/pointer args | 22 | `hook_daily_update(usize) -> i64`, `hook_entry_erase(usize,i32,i32) -> i32`, `hook_intel_getter(usize) -> u32`, `hook_ui_producer(usize,usize,usize) -> usize`, `hook_localization(usize,usize,usize,u32) -> usize`, `hook_peace(usize,usize,usize,u32) -> usize` |
| 5 args → **1 stack arg** | 2 | D71 `hook_controller(usize,usize,u8,u32,usize)`; A29 `hook_ui_year_helper(usize,*mut i64,usize,i64,i64) -> *mut i64` |
| 6 args → **2 stack args** | 1 | A29 `hook_effective_producer(usize,*mut i64,usize,usize,i64,i64) -> *mut i64` |
| 7 args → **3 stack args** | 1 | D71 `hook_upgrade(usize×7) -> usize` |

So the register-only class does **not** cover the set, and an earlier draft of this document was
wrong to say it did. Three of the four arity classes needed are stack-argument classes. This is not
a problem: route B's emitter is already parameterised by `stack_argument_count` and ships 6- and
7-argument variants, copying from `[rsp+0x80 + i*8]` (the caller's first stack slot, given the
`0x58` frame) to `[rsp+0x20 + i*8]`. What changes is that the bracket shim must copy the stack
arguments **twice** — once for the entry observer's `ProbeFrame` view and once for the real call —
or, more cheaply, copy once into the outgoing slots and have `ProbeFrame` point at them. The latter
is preferred and means the observer sees stack arguments at their final location.

**Floating-point arguments are excluded, and the exclusion is now load-bearing.** Route B's shim
never needed to preserve `XMM0–5` because it is a pure forwarder that calls exactly one callee. A
bracket shim calls the observer *first*, and `XMM0–5` are volatile, so an FP argument would be
destroyed before the original ever saw it. No point among the 25 takes an FP argument — but
`focus_research_decisions_depth_live` does contain FP-taking detours belonging to other domains
(`hook_item_state_producer(…, delta: f32, …)`, `hook_dynamic_refresh(…, delta: f32, …)`), so the
class exists in the product and will be requested. Admitting it requires adding `movaps` saves of
`XMM0–3` (and `XMM0` across the return dispatch) to the template and a new pinned digest. Until
then the arity validator must **reject** any spec declaring FP arguments rather than silently
emitting the integer template.

Hidden-return-pointer aggregates need no special handling: under Win64 the hidden pointer is an
ordinary first integer argument and is returned in `RAX`, so `hook_ui_year_helper` and
`hook_effective_producer` are covered by the 1- and 2-stack-argument classes above.

### 2.3 Observer contract

Both observers are `unsafe extern "system" fn(*mut ProbeFrame)` exported by name from
`collector.dll`, admitted through the existing `ExternalCallbackBinaryIdentity` machinery
(module file SHA-256, PE identity SHA-256, `SizeOfImage`, callback RVA, entry-prefix digest,
and `GetProcAddress` equality). They must:

- **never unwind.** Every observer body is wrapped in `catch_unwind`; a caught panic sets a poison
  flag and the domain's hour becomes a typed gap. A29 already does this and is the model.
- **never block.** No unbounded lock acquisition. `try_lock` with a `dropped_lock_busy` counter, the
  pattern d67 already uses.
- **never allocate on the hot path** where the target's cadence class says so (d95's intel getter).
- **never call back into the target.**

Two structs, kept apart on purpose:

- **`ProbeFrame`** is the shim's raw stack slice and nothing more — the six qwords tabulated in
  §2.2, plus the outgoing stack-argument slots for the wider classes. Every field is materialised by
  an instruction in the listing. It has no derived fields, because the shim has no storage for them.
- **`ProbeContext`** is what the observer prologue builds and hands the lane: the `ProbeFrame`
  contents, plus `caller_rva` (from `resolve_caller_rva`, `None` when the return address is outside
  `hoi4.exe`'s executable sections), `depth` and the open-bracket `invocation_id` from the
  primitive's TLS, and the probe's logical identity.

Keeping the derivation on the Rust side is what lets `resolve_caller_rva` be a pure, tested function
(§7) instead of three more instructions inside a byte-pinned template.

`caller_rva` is the one new capability, and it retires A29's `RtlCaptureStackBackTrace` call.

### 2.4 Two-slot dispatch is the operational win

Because the observers are reached through slots, a domain can be **disarmed by two atomic pointer
CAS operations to a sealed no-op** — no thread suspension, no code-byte churn, no `Freeze()` over
the 117 threads this process actually has (verified in the minidump thread list). Route A cannot do
this today; disabling a domain means `MinHook::disable_hook`, which suspends every thread in the
process.

Given §1.4 says the crashes are a concurrency/lifetime perturbation, the ability to turn a domain's
observation off *without* another whole-process freeze is likely to matter more than anything else
in this document.

---

## 3. Custody and teardown

### 3.1 The correctness argument, stated as an obligation

Route B retains its allocation for process lifetime because *a target thread may already have loaded
the old pointer out of the import slot*. The equivalent obligation here is stronger, because there
are three separately-owned executable regions, not one:

| Region | Owner | May a thread be inside it after we stop observing? | Disposition |
|---|---|---|---|
| The 5 patched bytes at the target | the game's `.text` | `MH_DisableHook` *attempts* it. `Freeze()` skips the calling thread, tries `OpenThread`/`SuspendThread` on the rest and **silently records a failure as "not suspended" rather than failing the call**, and `ProcessThreadIPs`/`FindOldIP` remaps only exact recorded trampoline boundaries, the relay address and the `patchAbove` slot — not an arbitrary address inside the five bytes. Best-effort, not a guarantee. | **Restored byte-for-byte and attested.** Safety does not rest on `Freeze`; see below. |
| MinHook's trampoline | MinHook's internal buffer pool | **Yes.** A thread inside our shim holds the trampoline address and will call it. | **Never freed.** See §3.2. |
| Our bracket shim + its `RUNTIME_FUNCTION` | this primitive | **Yes**, and also: a thread may be *unwinding through* it. | **Retained until process exit.** `RtlDeleteFunctionTable` is never called. |

The third row is the one that is easy to get wrong. Deleting the `RUNTIME_FUNCTION` while a thread
is unwinding through the shim turns it into a leaf frame in `RtlLookupFunctionEntry`'s eyes mid-pass;
the unwinder then computes the wrong `RSP` for the caller and either lands on a fabricated frame or
raises `STATUS_BAD_STACK`, which is not survivable. There is no cheap evidence that no thread is
unwinding. So we never delete it: an unused `RUNTIME_FUNCTION` for an unreachable shim costs 12
bytes and one entry in the process's dynamic function table.

### 3.2 `MinHook::remove_hook` must not be called

Verified against the vendored C source in `minhook-0.9.0`:

- `MH_RemoveHook` → `FreeBuffer(pHook->pTrampoline)` → the slot is pushed back on the block free
  list, and **if the block's `usedCount` reaches zero the block is `VirtualFree`d**
  (`minhook/src/buffer.c`, `FreeBuffer`).
- `Freeze()` / `ProcessThreadIPs` only relocate a thread's *instruction pointer*. They do not and
  cannot fix a thread that is about to `call` the trampoline through a pointer it already loaded.
- `Freeze()` excludes the calling thread, and a thread it fails to open or suspend is simply marked
  unsuspended while the disable proceeds. **`MH_DisableHook` returning `MH_OK` is therefore not
  evidence that no thread was in the patched region.**

This is why the phase order in Section 3.3 is `disarm → quiesce → restore` and never
`restore → quiesce`. The "no thread is inside our code" property comes from **our** shim counter,
which the unwind handler keeps balanced; it never comes from MinHook. MinHook is used only to put
the original five bytes back so the restoration attestation can be minted, and by the time it is
called the dispatch slots already point at a no-op and the counter has already been observed at
zero — so a thread MinHook failed to suspend can at worst run a pass-through shim into the
original.

`d95::uninstall` calls `remove_hook` today. It happens to be protected, because its unbounded
`while ACTIVE_CALLBACKS != 0 { yield_now() }` runs first and the lease is taken before
`ORIGINAL_*` is read — but the protection depends entirely on that lease being decremented on
*every* exit path, which is exactly what today's Rust detours do not guarantee (§4). Under the
proposed design the lease is the shim's asm counter, which is balanced on both paths, and
`remove_hook` is still never called: the trampoline is retained for process lifetime alongside the
shim.

### 3.3 Phases

```
Armed ──disarm──▶ Disarmed ──quiesce──▶ Quiesced ──restore──▶ BytesRestored ──seal──▶ Retained
```

| Phase | Action | Evidence minted |
|---|---|---|
| `arm` | emit shim → `RtlAddFunctionTable` → verify with `RtlLookupFunctionEntry` → `PAGE_EXECUTE_READ` → `FlushInstructionCache` → publish `original` and both dispatch slots **before** MinHook enables the patch | `ArmedEntryProbeReceipt`: module identity, probe spec digest, target RVA, observed prologue sentinel, shim identity + normalized template digest + unwind-handler/info/runtime-function digests, both callback binary identities, patch class |
| `disarm` | two atomic CAS of the dispatch slots to the sealed no-op; expected-value checked, never blind stores | `DisarmedEntryProbeReceipt`: observed before/after slot values, `observation_source` |
| `quiesce` | bounded observation of the shim counter reaching zero under a **crate-owned** budget | `QuiescedEntryProbePermit`: observed count, attempts used, budget |
| `restore` | `MH_DisableHook`, then re-read the five bytes and require equality with the pinned prologue sentinel | `RestoredEntryProbeReceipt`: restored-prefix digest vs pinned digest |
| `seal` | mark retained; never remove, never free, never delete the function table | `RetainedEntryProbeCustodyReceipt` with `retained_until_process_exit: true`, custody digest, promotion digest |

Ordering at `arm` is load-bearing. Today's lanes publish the patch and the `ORIGINAL_*` pointer as
two separate steps, and every detour has a `if original == 0 { … }` early-out precisely because that
window exists. Here the shim reads `original` from its own slot, which is written and made
`PAGE_EXECUTE_READ` *before* MinHook is asked to enable anything, so the window does not exist and
the early-out is unnecessary.

### 3.4 Reconciliation with the in-flight custody fixes

A sibling lane is repairing two defects in `production_hook_receipt` that this design depends on.
Both are load-bearing here, so the contract is stated explicitly rather than inherited:

- **No fabricated observations on idempotent-retry branches.** Every receipt in §3.3 carries
  `observation_source: FirstPass | ReplayedFromCustody`. A receipt replayed out of the custody
  registry is byte-identical to the original *and says so*; it can never be presented as a fresh
  live observation. `restore_pe64_import_address_slot_process_lifetime` currently synthesises an
  `AtomicPointerCasObservation` on the `current == expected_original_address` branch without having
  performed a CAS; the equivalent branch here must be labelled, not synthesised.
- **The quiesce budget belongs to the crate, not the caller.** `quiesce_pe64_import_address_slot`
  takes `max_yields: u32` from its caller and yields under the global custody mutex. Here,
  `quiesce_decision` (§6) is a pure function of `(observed_count, attempt, budget)` where `budget`
  is a crate constant, the loop runs with **no lock held**, and exhausting the budget returns
  resumable custody rather than failing or hanging.

If the sibling lane's fixes land differently, §3.3's receipts are the part that must be re-derived;
the phase machine and the retention policy are independent of them.

---

## 4. Unwind correctness

### 4.1 What an entry hook owes a passing exception

Three obligations, all currently unmet by the four lanes.

**(a) It owes the unwinder a description of its own frame.** x64 Windows has no frame pointer chain;
`RtlVirtualUnwind` computes the caller's context from the `UNWIND_INFO` that
`RtlLookupFunctionEntry` returns for the current RIP. Dynamically generated code that is not
registered with `RtlAddFunctionTable` returns `NULL` from the lookup, and the unwinder then treats
the frame as a **leaf** — it simulates a `ret`: `RIP = [RSP]`, `RSP += 8`, and nothing else. For our
shim, `[RSP]` at the point of the `call original` is inside our own `0x58`-byte frame, not a return
address, and `RSP += 8` does not begin to undo the allocation. The unwind then continues from a fabricated context: in the good case
it terminates the process with `STATUS_BAD_STACK`; in the bad case it "succeeds" onto a wrong frame
and runs the wrong `catch`. Route B already gets this right, which is why its template registers
unwind info and then *proves* the registration by requiring `RtlLookupFunctionEntry` to return the
exact table and base it registered. This design keeps that check verbatim.

**(b) It owes balanced accounting on the exceptional path.** If the counter is incremented on entry
and decremented only after the normal return, an exception through the frame permanently inflates
it. The consequence is not a leak but a **hang**: teardown waits for the counter to reach zero, and
`d95::uninstall`, `d67`, `direct_frd_transitions`, `naval_combat_live` and others all spin on that
condition with no bound. Route B's `UNW_FLAG_UHANDLER` handler exists for exactly this. Reusing it
unchanged is the single highest-value part of this design.

**(c) It owes not being a `nounwind` frame.** The toolchain is pinned at Rust 1.96.0 with the
default `panic = "unwind"`. Every current detour is `unsafe extern "system"`, a **non-unwinding
ABI**, and every one of them calls the original from inside that frame. A native C++ exception
raised by the original and propagating through the Rust frame is **undefined behaviour**, and the
useful precision is that *no particular behaviour is guaranteed*: the "unwinding out of a
non-unwinding ABI aborts" rule is about a **Rust panic** reaching the boundary, not about a foreign
unwind entering one. Neither an abort nor the execution of `Drop` glue may be relied on, which is
exactly why the active-callback lease must not be a Rust `Drop` guard. `catch_unwind` does not help
either: it catches Rust panics and cannot make a foreign unwind through a `nounwind` frame defined.

Moving the call to `original` out of Rust and into the asm shim removes the Rust frame from the
unwind path entirely, retiring the undefined behaviour rather than betting on how it manifests. That
is a stronger fix than switching the detours to `extern "system-unwind"`: the observers stay
`nounwind`, which is what we want, because they must never unwind.

### 4.2 What the unwind handler may do

Exactly one thing: `lock dec` the counter. **No lane code, no TLS write, no allocation, no lock
acquisition and no observer dispatch runs on the unwind path.** That is not an aspiration; it is
what route B's shipped handler already does, and keeping it there is what lets the handler bytes
stay digest-pinned.

So who closes the lane's per-invocation bracket when an exception skips the return dispatch? **The
next entry on the same thread does**, lazily. The primitive's TLS slot holds the open bracket's
`invocation_id`; if the entry observer finds a bracket already open at the same depth with a
*different* `invocation_id`, the previous invocation left without a return dispatch, so the lane
discards the stale bracket and poisons that domain's hour into a typed gap. d67 already has the
shape of this check (`OPEN_UPDATE.is_some()` becomes `DROPPED_REENTRANT`); it gains a distinct
`dropped_unwound` counter with a distinct meaning.

Lazy closure is deliberately preferred to an extended handler. Running lane code during phase-2
unwind runs it at a point where the lane cannot know which of the game's invariants the
partially-destroyed stack still holds, and any lock taken there can deadlock against a lock held by
a frame that is being unwound. The cost is that a thread which unwinds and then never re-enters
leaves one stale TLS bracket until thread exit — a leaked struct, not a correctness failure, and
never a teardown blocker, because teardown waits on the shim counter and the handler keeps *that*
balanced.

### 4.3 Known residual: multiply-invoked handlers

`UNW_FLAG_UHANDLER` means the handler is skipped during the search pass and invoked during unwind,
so it fires once per unwind through the frame. `EXCEPTION_COLLIDED_UNWIND` (`0x40`) is in the tested
mask, which means a collided unwind would decrement a second time. Whether that is reachable in
practice is not established. This is a real hole in the *existing* route B template as well as in
the proposed one, and §8.4 names the test that closes it.

---

## 5. Re-entrancy and thread safety

**The shim is fully re-entrant.** The *shim's* only mutable state is one process-global `AtomicU64`
counter mutated with `lock inc`/`lock dec`; everything else it touches lives in the invocation's own
stack frame. It is correct under recursion and under any number of concurrent threads by
construction. This is a
material improvement on the current lanes, whose per-invocation state is a `thread_local!
RefCell<Option<…>>` that each lane hand-rolls differently: d67 detects nesting and drops
(`DROPPED_REENTRANT`), d95 saves and restores a `prior` slot to build a manual stack, A29 uses
`RefCell` plus `catch_unwind`.

The TLS depth and bracket below are **observer-side, not shim-side**. The distinction is
load-bearing: it is why the handler in §4.2 has nothing to clean up, and why the shim's pinned byte
digest does not change when a lane changes its bracket policy.

**Recursion.** The primitive maintains a TLS depth and an open-bracket `invocation_id` per probe,
and the observer prologue derives both before handing the lane a `ProbeContext`. The lane keeps its
own policy — d67's "drop the nested one" and d95's "stack them" are both expressible — but it no
longer builds the mechanism, and `RefCell` double-borrow panics on a recursive path stop being
possible. That same TLS bracket performs the lazy close of an unwound invocation (§4.2), so
recursion and exception handling share one mechanism instead of two.

**Concurrency.** The four target sets are not known to be single-threaded, and the crash evidence in
§1.4 shows the game running the relevant code on a `tbb.dll` worker pool with 117 threads in the
process. The contract is therefore: **an observer must assume it runs concurrently with itself on
many threads**, and the primitive gives it no mutual exclusion. Cross-invocation state stays behind
`try_lock` with a busy counter. Which threads each of the four target sets actually runs on is not
established; §8.2 names the step.

**Cadence class.** Each probe spec declares one of `Rare` / `PerFrame` / `PerFrameFanout`.
`PerFrameFanout` (d95's intel getter) forbids allocation, locking and any syscall in the observer;
the type system enforces it by giving that class an observer signature that only permits writes into
a preallocated per-thread ring. Volume is the one thing that is *measured* to destabilise this
process, so it is a declared, checkable property of every point, not a review note.

---

## 6. Identity gate

### 6.1 The gate

Arming refuses unless **all** of the following hold. Each is an existing check in
`production_hook_receipt`, plus one addition.

1. `GetModuleHandleW(NULL)`'s file name is `hoi4.exe` (case-insensitive).
2. The file's SHA-256 equals `A729_EXE_SHA256`, read through a `File` handle that is **held open**
   for the process lifetime so the bytes cannot be swapped after validation
   (`ObservedMainModule::_stable_file`).
3. The mapped PE identity (machine, section count, timestamp, optional-header size and magic,
   `SizeOfImage`, `SizeOfHeaders`, headers digest) equals the on-disk identity, re-checked on
   *every* `observe_main_module()` call, not just at initialisation.
4. The module base has not changed since initialisation.
5. The probe's `logical_name`/`requested_rva`/arity class/cadence class appear in the compiled-in
   admitted registry, and the spec digest matches.
6. **New:** the 16 bytes at the target RVA equal the probe's pinned prologue sentinel *before*
   arming and *after* restore. The lanes all do this today
   (`DAILY_UPDATE_SENTINEL`, `GETTER_SENTINEL`, `PEACE_SENTINEL`, …) but each in its own code; it
   belongs in the audited primitive.
7. Both dispatch callbacks are exact named PE exports of an admitted `collector.dll` identity.

### 6.2 Why the sentinel matters more than the SHA

The SHA-256 gate makes 6 redundant *today*. It will not stay redundant.
`MOD_AND_UPDATE_ROBUSTNESS_PLAN.md` U4 requires the build profile to become data keyed by exe hash,
so that supporting a new patch is a profile entry rather than a code edit. The moment that lands,
the SHA gate becomes "is this hash in the table" and the RVA becomes profile data — at which point
the prologue sentinel is the *only* thing standing between a stale table row and a five-byte `JMP`
written into the middle of an unrelated instruction. Putting the sentinel in the primitive now means
the U4 change touches one admission function instead of eight lanes.

### 6.3 Blast radius if the gate is wrong

Ordered worst-first.

1. **Wrong RVA, gate passes.** Five bytes of `E9 rel32` land mid-instruction somewhere in a 58 MB
   image. The game then executes a deterministic garbage instruction stream. The failure is silent
   at capture time and presents as a game crash with **no `collector.dll` frame on the stack** —
   which is precisely the signature we are already staring at in §1.4 and cannot currently
   attribute. This is the reason the sentinel check is non-negotiable: a wrong instrumentation point
   is indistinguishable from a game bug from the outside.
2. **Wrong arity class, gate passes.** The shim restores four argument registers a target never had,
   or fails to preserve `XMM0` for an FP return. Corrupted values flow into the game's own logic;
   the recorder observes plausible-looking numbers. This is the worst *data* outcome, because
   fail-closed does not trigger — nothing looks wrong.
3. **Wrong callback identity.** The dispatch slot points at code we did not author. Contained by the
   export-equality and entry-prefix-digest checks route B already performs.
4. **Gate too strict.** The domain refuses to arm, the hour becomes a typed gap, playback shows the
   last known state marked stale. This is the designed failure and needs no mitigation.

The asymmetry between 1–2 and 4 is the whole argument for fail-closed, and it is why §7 refuses to
weaken any of these checks for testability.

---

## 7. Test seam

`production_hook_receipt` currently contains **zero `#[test]` functions**. Every entry point is
`#[cfg(windows)]` behind `observe_main_module()`, so none of it is reachable from `cargo test`. The
decision logic is therefore entirely unverified except by running the real game.

The fix is not to weaken the gate. It is to put the decisions in pure functions and test those.

### 7.1 Pure helpers, no `cfg(windows)`, no process state

| Helper | Signature | What it decides |
|---|---|---|
| `validate_probe_spec` | `fn(&ExpectedEntryProbe) -> Result<(), SpecError>` | arity/cadence/name/RVA/sentinel-length admissibility |
| `probe_spec_sha256` | `fn(&ExpectedEntryProbe) -> [u8;32]` | spec identity |
| `emit_bracket_shim` | `fn(ArityClass, ShimLayout) -> Result<Vec<u8>, EmitError>` | the shim bytes, with `base`-relative RIP displacements passed in, not read from the process |
| `normalized_shim_digest` | `fn(ArityClass, &[u8]) -> [u8;32]` | displacement-blanked template digest |
| `encode_unwind_info` | `fn(prolog_len: u8, alloc_bytes: u16, handler_rva: u32) -> Result<[u8;12], _>` | the `UNWIND_INFO` blob |
| `encode_runtime_function` | `fn(begin: u32, end: u32, unwind_rva: u32) -> [u8;12]` | the `RUNTIME_FUNCTION` |
| `classify_patch_bytes` | `fn(&[u8;5]) -> PatchClass` | `NormalJmpRel32` / `PatchAboveSplit` / `CallRel32` / `Unpatched` / `Foreign` |
| `decode_relay_stub` | `fn(&[u8;14]) -> Result<usize, RelayError>` | the `FF 25 00000000` + absolute-qword decode |
| `resolve_caller_rva` | `fn(ret_addr: usize, base: usize, sections: &[SectionRange]) -> Option<u64>` | A29's provenance, without a stack walk |
| `quiesce_decision` | `fn(count: u64, attempt: u32, budget: u32) -> QuiesceStep` | `Done` / `Yield` / `RetainResumable` |
| `ProbeCustodyLedger::step` | `fn(Phase, Event) -> Result<Phase, LedgerError>` | the whole §3.3 machine as a pure transition |
| `receipt_digest` / `promotion_digest` | `fn(&Receipt) -> [u8;32]` | serialisation-stable digests |

### 7.2 Tests those enable, that cannot be written today

- **Emitter round-trip.** `emit_bracket_shim` output hashes to a pinned constant per arity class,
  and `normalized_shim_digest` is invariant under changing the allocation base. Route B's three
  admitted entry digests (`d1d8825314…`, `272f0c711f…`, `c931ce51ba…`) become the first three test
  vectors, so the refactor is provably byte-identical to the code that ships today.
- **Unwind-info encoder.** `encode_unwind_info(0x0c, 0x58, 0x100) == [0x11,0x0c,0x01,0x00,
  0x0c,0xa2,0x00,0x00,0x00,0x01,0x00,0x00]` — the currently-hardcoded array becomes an assertion.
  Then: prolog length that does not match the emitted prologue is rejected; alloc sizes that need
  `UWOP_ALLOC_LARGE` are rejected rather than silently truncated into `ALLOC_SMALL`.
- **Patch classification.** `EB F9` prefix ⇒ `PatchAboveSplit` ⇒ refuse. `E9` ⇒ decode. `E8` ⇒
  `CallRel32` and only admissible for the D90 primitive. Displacement arithmetic overflow at the
  `i32`/`isize` boundary. These are the checks in `observe_enabled_minhook_rel5` today, untested.
- **Ledger.** Table-driven over every `(Phase, Event)` pair: exactly the legal transitions succeed;
  `restore` before `quiesce`, double-`seal`, `arm` on a `Retained` probe, and `disarm` on a probe
  owned by a different teardown token all produce distinct typed errors. Today the equivalent
  (`SealedShimLeasePhase`, `Pe64ImportSlotCustodyState`) can only be exercised against a live game.
- **Quiesce budget.** A property test that `quiesce_decision` reaches a terminal step within
  `budget + 1` calls for every `(count, budget)` — i.e. that the unbounded-spin defect class cannot
  reappear. This is the test whose absence let the current unbounded loops ship.
- **Caller RVA.** Addresses outside the module, inside a non-executable section, and exactly at a
  section boundary all resolve correctly or to `None`.
- **Epilogue contiguity.** A decoder over the emitted bytes asserts that the tail is exactly
  `add rsp, imm8` followed immediately by `ret`, with nothing between them — the property that makes
  the sequence recognisable to the unwinder's epilogue scan, and the one an earlier draft of this
  document got wrong. Every arity class is checked.
- **Frame budget.** For each arity class: the emitted `sub rsp, N` is `≡ 8 (mod 16)`; `N` is within
  `UWOP_ALLOC_SMALL`'s 8–128 range; and every offset the listing writes to lies inside `[0, N)`.
  This is the check that would have caught `ProbeFrame` overrunning its allocation.
- **Arity validator.** A spec declaring a floating-point argument is rejected rather than emitting
  the integer template (§2.2); a spec whose declared stack-argument count disagrees with its
  declared arity is rejected.
- **Lazy bracket close.** Given a TLS bracket left open with `invocation_id = N`, an entry at the
  same depth with `invocation_id = N+1` discards the stale bracket and increments `dropped_unwound`
  exactly once — the observer-side half of §4.2, testable without an exception.

### 7.3 What the seam deliberately does not offer

There is no way to arm a probe against anything but the pinned binary, no injectable "module
authority", no `#[cfg(test)]` bypass of `observe_main_module()`, and no test-only relaxation of the
sentinel check. `mint_test_released_receipt` already exists behind a `test-support` feature for
*dependent* lanes and stays exactly as scoped: it fabricates a released receipt so downstream
custody flows can be exercised, and it cannot arm anything.

The rule this encodes: **pure helpers decide, `cfg(windows)` code observes.** A test may drive any
decision; it may never make the process believe it is attached to a binary it is not.

---

## 8. Open questions

Each names the one concrete step that closes it.

### 8.1 What actually causes the injection crashes

Not settled, and §1.4 rules out the two hypotheses this primitive would have addressed. Both the
`0x0` and `0x3F800000` faults are the game calling through a garbage function pointer, with no
collector frame on the stack and the reported call site unpatched.

**Next step:** re-run `Measure-InjectionCrashRate.ps1` with (a) `MiniDumpWithFullMemory` so the
faulting object and its vtable can be typed instead of guessed, and (b) a domain-group bisect —
`production_render_submit_live::staged_hook_count_for_group_mask` already provides the mask
mechanism — to find the smallest scheduled set that reproduces. Six trials per arm at the existing
protocol. That converts "volume causes crashes" into "these N points cause crashes", which is the
difference between a rewrite and a deletion.

### 8.2 Which threads the four target sets run on

The primitive's concurrency contract (§5) assumes the worst. Whether it needs to is unknown. It
matters concretely: a target reached only from the main render thread can use far cheaper observer
state than one reached from the TBB pool.

**Next step:** statically, walk callers of each of the four lanes' target RVAs in Ghidra until the
chain reaches either a `tbb::parallel_for`-style dispatch or a known single-threaded entry —
`ghidra_export_functions` with `depth=3` on the 25 target RVAs is one job. The crash evidence
already proves the pool is used for *some* of this call graph, so the answer is not "obviously the
UI thread".

### 8.3 Whether d95 needs hooks at all

`spotted_units_live` installs four hooks including `hook_intel_getter`, which runs once per unit per
visibility frame — the hottest instrumentation point in the product, and therefore the largest single
contributor to the one variable that is *measured* to destabilise the process. The lane's own comment
says these are "complete current map-visibility frames, not one-shot events", which is exactly what a
*stored* structure would also give.

**Next step:** in Ghidra, find the writers of the per-unit intel field the lane reads
(`unit+0x20C`, and the raw intel byte behind `INTEL_GETTER_RVA = +0x16068C0`) and determine whether
any per-country container holds the resolved visible set — the way D94's FoW state is held. If one
exists, d95 becomes an ordinary read domain and four hooks including the hottest disappear. This is a
larger win than the entire primitive and should be answered **before** d95 is migrated.

### 8.4 Collided-unwind double decrement

§4.3. The existing handler decrements on `ExceptionFlags & 0x66`, which includes
`EXCEPTION_COLLIDED_UNWIND`. Whether a collided unwind can reach the shim, and whether it would
decrement twice, is not established — and it applies to the *shipping* route B shim, not only to
this design.

**Next step:** a standalone Windows test executable (not the game) that installs the shim over a
C++ function and throws through it under four scenarios — normal return, single unwind, unwind with
an intervening `catch(...)` that rethrows, and a destructor that throws during unwind — asserting the
counter returns to its pre-call value in every case. This is runnable today against the code that
already ships.

### 8.5 Can the targets throw at all

If none of the 25 admitted targets can propagate a C++ exception, §4's obligation (c) is a
theoretical hazard rather than a live one, and the migration can be sequenced accordingly. It does
not change the design — the shim is correct either way — but it changes the urgency.

**Next step:** for each target, check whether its `RUNTIME_FUNCTION` in the a729 `.pdata` carries
`UNW_FLAG_EHANDLER`, and whether `__CxxThrowException` is reachable in its call graph. Both are
mechanical from the existing Ghidra export tooling.

### 8.6 The retained-allocation ceiling

Every armed probe retains one shim allocation (`0x2000`), one MinHook trampoline slot, and one
dynamic function-table entry, for the process lifetime, across arm/disarm cycles. At 25 probes
this is roughly 200 KB and 25 function-table entries, which is trivial. If a future design arms and disarms probes repeatedly per campaign, it is not.

**Next step:** decide whether re-arming a previously-retained probe reuses its shim (safe: the shim
is stateless apart from its slots, and re-pointing the slots is the same CAS as disarming) or
allocates a new one. The reuse path looks correct and would make the ceiling a constant; it needs
one review pass, not an experiment.

---

## 9. Deliberately excluded

| Excluded | Why |
|---|---|
| **Any new instrumentation point.** | The primitive re-homes the 25 existing points. `DECISIONS.md` permits a hook only where a value is provably unsampleable; adding points is per-domain work with its own evidence bar, and §8.3 argues the count should go *down*. |
| **Replacing MinHook's patcher/trampoline.** | `observe_enabled_minhook_rel5` already attests the patch and relay exactly, and `Freeze`/`FindOldIP` correctly relocate IPs on disable. Writing our own inline patcher would re-derive that for no gain. The one real MinHook hazard — `FreeBuffer` on `remove_hook` — is avoided by never calling it (§3.2). |
| **The import-slot route for these four.** | Structurally impossible: the targets are internal `hoi4.exe` functions with no import slot. |
| **VEH / hardware-breakpoint / page-guard hooking.** | Debug registers are per-thread and there are four of them against 117 threads; page-guard trapping costs a kernel transition per access, which is unthinkable at d95's call rate. |
| **FP-argument arity class.** | No point among the 25 takes an FP argument, and admitting the class needs `XMM0–3` saves plus a new pinned template digest (§2.2). The 1/2/3-stack-argument classes are **not** excluded — the census says they are required. |
| **Lane code on the unwind path.** | §4.2. |
| **Any mod/patch-day generalisation.** | `MOD_AND_UPDATE_ROBUSTNESS_PLAN.md` U1–U5 own build-profile-as-data. §6.2 only ensures the primitive presents one admission function for U4 to change. |
| **Cadence, scheduling and gap policy.** | The primitive declares a cadence *class* per probe so volume is checkable; it does not decide when a domain samples. |
| **A crash fix.** | §1.4. The dump evidence does not support the claim that unwind bookkeeping causes the observed crashes, and this design will not be presented as fixing them. |
| **Migrating all four at once.** | The correct order is **D67** (2 points, already the only lane with custody receipts, and its bracket is the canonical shape), then **A29** (4 points; proves the caller-RVA capability and the 1- and 2-stack-argument classes), then **D95** — and only after §8.3 is answered, because the answer may delete it — then **D71** last (15 points, and the only 3-stack-argument class). |

---

## 10. Evidence

- Hook crate: `extractor/native/lanes/production_hook_receipt/src/lib.rs`.
- Lanes: `country_dynamic_modifiers_live/src/a729_hook.rs`,
  `dynamic_collection_trace_live/src/a729_named_overlap_hook.rs`,
  `spotted_units_live/src/a729.rs`,
  `focus_research_decisions_depth_live/src/a729_research_ahead_hook.rs`.
- Crash ledger: `extractor/native/runs/crashrate/trials.jsonl` (valid) and
  `trials_20260726_invalid_minimal_arm.jsonl` (superseded).
- Crash dumps: `…\Paradox Interactive\Hearts of Iron IV\crashes\hoi4_20260726_{184725,185003,
  185241,093040,093834,094112}\minidump.dmp`. Machine-local, outside the repo.
- Crash-site static evidence: Ghidra job
  `20260729T020739Z-entry-unwind-design-crash-site-ab864a9f`, manifest SHA-256
  `dfe27a055a27a06c610625bc5eb936272209d4384d333b773a502846029f2715`, resolving
- MinHook behaviour: vendored C in `minhook-0.9.0`, `minhook/src/hook.c`
  (`MH_RemoveHook`, `ProcessThreadIPs`, `FindOldIP`) and `minhook/src/buffer.c` (`FreeBuffer`).
