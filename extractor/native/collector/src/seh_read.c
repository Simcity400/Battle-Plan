/*
 * SEH-guarded crash-safe copy of foreign (live-game) heap memory from inside
 * the injected collector DLL.
 *
 * Rationale (see runs/perf_rearch_20260706/ plan v2): the shipped read
 * primitive copies each scalar via ReadProcessMemory(GetCurrentProcess()),
 * which is one syscall per field read. Under ~30 concurrent reader threads at
 * game speed 5 the syscall + kernel MM-lock dominates warm frame cost. A plain
 * in-process memcpy is far cheaper, but a copy out of a page the game decommits
 * mid-read raises an access violation that, with no handler, terminates the
 * whole game process. Wrapping the copy in a structured-exception frame
 * (__try/__except, table-based on x64 => zero happy-path cost) makes the fault
 * recoverable: the filter returns fail (0) and the caller treats the
 * destination as unspecified, exactly like RPM's all-or-nothing short read.
 *
 * SAFETY / SCOPE (Codex-reviewed constraints):
 *   - The __try body contains ONLY the copy. No allocation, no locks, no
 *     logging, no CRT calls that could themselves fault or unwind oddly.
 *   - The filter catches ONLY access-violation / guard-page / in-page-error,
 *     and ONLY when the recorded fault address lies inside the source or
 *     destination span. Anything else (a genuine bug elsewhere on the stack, a
 *     stack-overflow guard-page trip unrelated to our span, etc.) is passed
 *     through with EXCEPTION_CONTINUE_SEARCH so we never swallow unrelated
 *     faults.
 *   - Callers MUST still gate the source range with VirtualQuery for the
 *     current frame epoch (guard-page consumption is a hazard shared with RPM;
 *     the VirtualQuery gate is the mitigation for both designs).
 */

#include <stddef.h>
#include <string.h>
#include <windows.h>

/* Decide whether an exception raised inside the copy is one we own. */
static int seh_read_fault_is_ours(EXCEPTION_POINTERS *ep,
                                  const unsigned char *src, size_t len,
                                  const unsigned char *dst) {
    DWORD code = ep->ExceptionRecord->ExceptionCode;
    if (code != EXCEPTION_ACCESS_VIOLATION &&
        code != STATUS_GUARD_PAGE_VIOLATION &&
        code != EXCEPTION_IN_PAGE_ERROR) {
        return 0; /* not a memory fault -> not ours */
    }
    /* ExceptionInformation[1] = the virtual address that was accessed, for
     * AV / in-page / guard-page records (NumberParameters >= 2). If the fault
     * address is inside the span we are copying, it is our decommit race;
     * otherwise let it propagate. */
    if (ep->ExceptionRecord->NumberParameters >= 2) {
        ULONG_PTR fa = (ULONG_PTR)ep->ExceptionRecord->ExceptionInformation[1];
        ULONG_PTR s = (ULONG_PTR)src;
        ULONG_PTR d = (ULONG_PTR)dst;
        if ((fa >= s && fa < s + len) || (fa >= d && fa < d + len)) {
            return 1;
        }
        return 0;
    }
    /* No fault address recorded: conservatively treat a memory fault as ours
     * (the copy is the only memory op in the frame). */
    return 1;
}

/*
 * Crash-safe copy of `len` bytes from `src` (live foreign heap) to `dst`.
 * Returns 1 iff the full range copied without fault; 0 on any fault (dst is
 * then unspecified — a prefix may have been written). All-or-nothing at the
 * caller contract level, identical to rpm_exact.
 */
int seh_guarded_copy(unsigned char *dst, const unsigned char *src, size_t len) {
    __try {
        memcpy(dst, src, len);
        return 1;
    } __except (seh_read_fault_is_ours(GetExceptionInformation(), src, len, dst)
                    ? EXCEPTION_EXECUTE_HANDLER
                    : EXCEPTION_CONTINUE_SEARCH) {
        return 0;
    }
}
