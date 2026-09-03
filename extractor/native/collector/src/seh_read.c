
#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include <windows.h>

static int seh_read_fault_is_ours(EXCEPTION_POINTERS *ep, const unsigned char *src,
                                  size_t len, const unsigned char *dst) {
    DWORD code = ep->ExceptionRecord->ExceptionCode;
    if (code != EXCEPTION_ACCESS_VIOLATION && code != STATUS_GUARD_PAGE_VIOLATION &&
        code != EXCEPTION_IN_PAGE_ERROR) {
        return 0;
    }
    if (ep->ExceptionRecord->NumberParameters >= 2) {
        uintptr_t fa = (uintptr_t)ep->ExceptionRecord->ExceptionInformation[1];
        uintptr_t s = (uintptr_t)src;
        uintptr_t d = (uintptr_t)dst;
        if ((fa - s) < (uintptr_t)len || (fa - d) < (uintptr_t)len) {
            return 1;
        }
        return 0;
    }
    return 1;
}

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

/* One planned range of a domain-53 snapshot batch. Layout-identical to the
   Rust `Domain53SnapshotCopySpec`; the caller passes its array straight
   through. */
typedef struct {
    uint64_t src;
    uint64_t dst_offset;
    uint64_t len;
} SehCopySpec;

/* Copies specs[0..count) from `src` into `dst_base + dst_offset`, all under
   ONE structured-exception frame: a scattered plan pays the frame once per
   confirmed region instead of once per range, which is the whole reason the
   snapshot can afford exact ranges rather than whole pages.

   Answers how many LEADING specs copied. On a fault that answer is the index
   of the spec that faulted: everything before it holds its bytes, and that
   spec and every later one hold nothing the caller may trust (the faulting
   spec's destination may have been partly written). The caller must have
   confirmed `dst_base + dst_offset .. + len` in bounds for every spec. */
size_t seh_guarded_copy_specs(unsigned char *dst_base, const SehCopySpec *specs,
                              size_t count) {
    volatile size_t done = 0;
    __try {
        while (done < count) {
            const SehCopySpec *spec = &specs[done];
            memcpy(dst_base + (size_t)spec->dst_offset, (const unsigned char *)(uintptr_t)spec->src,
                   (size_t)spec->len);
            done = done + 1;
        }
    } __except (done < count &&
                        seh_read_fault_is_ours(GetExceptionInformation(),
                                               (const unsigned char *)(uintptr_t)specs[done].src,
                                               (size_t)specs[done].len,
                                               dst_base + (size_t)specs[done].dst_offset)
                    ? EXCEPTION_EXECUTE_HANDLER
                    : EXCEPTION_CONTINUE_SEARCH) {
    }
    return done;
}
