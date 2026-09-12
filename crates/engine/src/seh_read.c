#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include <windows.h>

/* Ported from the old collector's seh_read.c. Only faults within this copy
   are handled; unrelated exceptions continue through the game's handlers.
   Windows reports a fault on a noncanonical x86-64 source
   (bits 63:48 != bit 47, e.g. ASCII bytes such as "_factor" read as a
   pointer) with ExceptionInformation[1] == -1. The guarded instruction only
   touches the requested source range (the destination is valid pool memory),
   so -1 during the copy must come from that source range and is handled
   gracefully. Any other address still uses the exact range check, so a
   canonical out-of-range fault elsewhere keeps returning
   EXCEPTION_CONTINUE_SEARCH for real crash reporting. The Rust
   is_canonical_range gate makes this -1 path unreachable in normal walks; it
   remains as containment for any direct FFI caller bypassing that gate. */
static int copy_fault(EXCEPTION_POINTERS *ep, const unsigned char *src,
                      size_t len, const unsigned char *dst) {
    DWORD code = ep->ExceptionRecord->ExceptionCode;
    if (code != EXCEPTION_ACCESS_VIOLATION && code != STATUS_GUARD_PAGE_VIOLATION &&
        code != EXCEPTION_IN_PAGE_ERROR) {
        return 0;
    }
    if (ep->ExceptionRecord->NumberParameters >= 2) {
        uintptr_t address = (uintptr_t)ep->ExceptionRecord->ExceptionInformation[1];
        if (address == (uintptr_t)-1) {
            return 1;
        }
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
    } else if (len == 1) {
        memcpy(dst, src, 1);
    } else if (len == 2) {
        memcpy(dst, src, 2);
    } else if (len == 12) {
        memcpy(dst, src, 12);
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
        /* The normal loop index can stay in a register. Publish completion only
           after each exact span; the exception filter needs that volatile
           witness, not repeated volatile reloads for indexing and bounds. */
        for (size_t index = 0; index < count; ++index) {
            const CopySpan *span = &spans[index];
            if (span->len != 0) {
                copy_bytes(span->dst, span->src, span->len);
            }
            completed = index + 1;
        }
    } __except (completed < count &&
                        copy_fault(GetExceptionInformation(), spans[completed].src,
                                   spans[completed].len, spans[completed].dst)
                    ? EXCEPTION_EXECUTE_HANDLER : EXCEPTION_CONTINUE_SEARCH) {
        /* The current span may be partial. No later span has been accessed. */
    }
    return completed;
}

typedef struct {
    size_t source_offset;
    size_t destination_offset;
    size_t len;
} RecordField;

static int record_copy_fault(EXCEPTION_POINTERS *ep, const uintptr_t *pointers,
                             size_t row_count, const RecordField *fields,
                             size_t field_count, size_t row_stride,
                             unsigned char *destination, size_t completed) {
    if (field_count == 0 || completed >= row_count * field_count) {
        return 0;
    }
    size_t row = completed / field_count;
    const RecordField *field = &fields[completed % field_count];
    uintptr_t pointer = pointers[row];
    if (field->len == 0 || pointer == 0 ||
        field->source_offset > UINTPTR_MAX - pointer) {
        return 0;
    }
    uintptr_t address = pointer + field->source_offset;
    if (field->len > UINTPTR_MAX - address) {
        return 0;
    }
    return copy_fault(ep, (const unsigned char *)address, field->len,
                      destination + row * row_stride + field->destination_offset);
}

/* Rust validates immutable metadata, total field count, and every destination
   bound before entry. Source ranges are checked in field order, so a later bad
   source cannot conceal an earlier completed field or change its precedence. */
size_t seh_guarded_copy_records(const uintptr_t *pointers, size_t row_count,
                               const RecordField *fields, size_t field_count,
                               size_t row_stride, unsigned char *destination) {
    volatile size_t completed = 0;
    __try {
        size_t index = 0;
        for (size_t row = 0; row < row_count; ++row) {
            uintptr_t pointer = pointers[row];
            unsigned char *target = destination + row * row_stride;
            for (size_t field_index = 0; field_index < field_count; ++field_index) {
                const RecordField *field = &fields[field_index];
                if (field->len != 0) {
                    if (pointer == 0 || field->source_offset > UINTPTR_MAX - pointer) {
                        return completed;
                    }
                    uintptr_t address = pointer + field->source_offset;
                    if (field->len > UINTPTR_MAX - address) {
                        return completed;
                    }
                    copy_bytes(target + field->destination_offset,
                               (const unsigned char *)address, field->len);
                }
                completed = ++index;
            }
        }
    } __except (record_copy_fault(GetExceptionInformation(), pointers, row_count,
                                  fields, field_count, row_stride, destination,
                                  completed)
                    ? EXCEPTION_EXECUTE_HANDLER : EXCEPTION_CONTINUE_SEARCH) {
        /* The current field may be partial. Later fields are untouched. */
    }
    return completed;
}
