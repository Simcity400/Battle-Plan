#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include <windows.h>

/* Ported from the old collector's seh_read.c. Only faults within this copy
   are handled; unrelated exceptions continue through the game's handlers. */
static int copy_fault(EXCEPTION_POINTERS *ep, const unsigned char *src,
                      size_t len, const unsigned char *dst) {
    DWORD code = ep->ExceptionRecord->ExceptionCode;
    if (code != EXCEPTION_ACCESS_VIOLATION && code != STATUS_GUARD_PAGE_VIOLATION &&
        code != EXCEPTION_IN_PAGE_ERROR) {
        return 0;
    }
    if (ep->ExceptionRecord->NumberParameters >= 2) {
        uintptr_t address = (uintptr_t)ep->ExceptionRecord->ExceptionInformation[1];
        return address - (uintptr_t)src < len || address - (uintptr_t)dst < len;
    }
    return 0;
}

/* Fixed-size memcpy remains valid for unaligned fields and gives the compiler
   exact copy widths for the common pointer/scalar and array-header reads. */
static __forceinline void copy_bytes(unsigned char *dst,
                                    const unsigned char *src, size_t len) {
    if (len == 8) {
        memcpy(dst, src, 8);
    } else if (len == 4) {
        memcpy(dst, src, 4);
    } else if (len == 16) {
        memcpy(dst, src, 16);
    } else {
        memcpy(dst, src, len);
    }
}

int seh_guarded_copy(unsigned char *dst, const unsigned char *src, size_t len) {
    __try {
        copy_bytes(dst, src, len);
        return 1;
    } __except (copy_fault(GetExceptionInformation(), src, len, dst)
                    ? EXCEPTION_EXECUTE_HANDLER : EXCEPTION_CONTINUE_SEARCH) {
        return 0;
    }
}

typedef struct {
    unsigned char *dst;
    const unsigned char *src;
    size_t len;
} CopySpan;

size_t seh_guarded_copy_scattered(const CopySpan *spans, size_t count) {
    volatile size_t completed = 0;
    __try {
        for (; completed < count; ++completed) {
            const CopySpan *span = &spans[completed];
            if (span->len != 0) {
                copy_bytes(span->dst, span->src, span->len);
            }
        }
    } __except (completed < count &&
                        copy_fault(GetExceptionInformation(), spans[completed].src,
                                   spans[completed].len, spans[completed].dst)
                    ? EXCEPTION_EXECUTE_HANDLER : EXCEPTION_CONTINUE_SEARCH) {
        /* The current span may be partial. No later span has been accessed. */
    }
    return completed;
}
