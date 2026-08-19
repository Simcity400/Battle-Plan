
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
