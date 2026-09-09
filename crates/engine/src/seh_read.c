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
    return 1;
}

int seh_guarded_copy(unsigned char *dst, const unsigned char *src, size_t len) {
    __try {
        memcpy(dst, src, len);
        return 1;
    } __except (copy_fault(GetExceptionInformation(), src, len, dst)
                    ? EXCEPTION_EXECUTE_HANDLER : EXCEPTION_CONTINUE_SEARCH) {
        return 0;
    }
}
