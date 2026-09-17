# Job: hot (most-called unnamed functions)

Read `RULES.md` first. These are utility functions: strings, arrays, allocators, fixed-point maths, logging, locks, Lua
calls. Naming one makes thousands of other functions readable.

For each item from `hoi4re todo next hot`:
1. `hoi4re fn <addr> --max-lines 150`. Read the whole body.
2. `hoi4re callers <addr>` and read three or four callers with `fn`, looking only at how this function is used:
   what is passed in, what is done with the result, which literals sit next to the call.
3. Decide what it is. Typical answers: `CString::Assign(const char*)`, `CPdxArray::PushBack`, `PdxFree`, `CFixedPoint::Mul`,
   `lua_gettop`, `Log::Write`. Standard library and CRT functions get their standard names (`memcpy`, `std::string::~string`).
4. If the body is a thin wrapper, say what it wraps. If it takes `this`, record any offsets you understand as `field` lines.
5. Save. Use `medium` only when literals, asserts or the exact behaviour settle it.

## Idioms you will meet (MSVC x64, Clausewitz engine)

- `std::string`: 32 bytes; `+0x00` inline buffer or heap pointer, `+0x10` size, `+0x18` capacity; capacity `0xf` means inline.
  A test `0xf < p[3]` followed by a free, then `p[2]=0; p[3]=0xf; *p=0` is the string destructor.
- Large frees: `if (0xfff < size + 1) { real = ptr[-1]; ... }` is MSVC's aligned `_Deallocate`. It marks a container or string free.
- `std::vector`: three pointers `begin, end, capacity_end`; size is `(end - begin) / element size`.
- `std::shared_ptr` release: decrement at `+8`, call vtable slot 0 at zero, then decrement `+0xc` and call slot 1.
- Engine arrays (`CPdxArray`-like): pointer, then `u32 capacity`, then `u32 size`; some variants start with a vtable and end with an allocator pointer.
- Fixed point: the game stores decimals as integers scaled by 1000 (i32) or 100000 (i64); multiply then divide by that scale is a fixed-point multiply.
- `FUN(&DAT_x)` wrapped in `AcquireSRWLockExclusive`/`Release...` is a locked singleton or registry access.
- A function that only forwards to another with the same arguments is a thunk or inline wrapper: name it after its target.
