#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include <windows.h>
#include <xmmintrin.h>

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
    } else if (len == 32) {
        memcpy(dst, src, 32);
    } else if (len == 1) {
        memcpy(dst, src, 1);
    } else if (len == 2) {
        memcpy(dst, src, 2);
    } else if (len == 12) {
        memcpy(dst, src, 12);
    } else if (len == 24) {
        memcpy(dst, src, 24);
    } else if (len == 64) {
        memcpy(dst, src, 64);
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

/* Hints only the endpoint cache lines of already-validated exact fields.
   PREFETCHT0 observes no value and does not replace any guarded load. Pointer
   metadata is caller-owned; no source pointer is followed here. The caller
   passes only fields inside the flattened validated prefix, including a
   possible partial final row. Zero-width fields never evaluate their source. */
static __forceinline void hint_record_fields(uintptr_t pointer,
                                             const RecordField *fields,
                                             size_t count) {
    for (size_t index = 0; index < count; ++index) {
        const RecordField *field = &fields[index];
        if (field->len != 0) {
            uintptr_t first = pointer + field->source_offset;
            uintptr_t last = first + field->len - 1;
            _mm_prefetch((const char *)first, _MM_HINT_T0);
            if ((first >> 6) != (last >> 6)) {
                _mm_prefetch((const char *)last, _MM_HINT_T0);
            }
        }
    }
}

/* The exact one-field pointer layout has the same endpoint hints without
   reloading its invariant field metadata for each row. Unaligned pointers can
   cross a cache line, so the second endpoint must still be considered. */
static __forceinline void hint_pointer_record(uintptr_t pointer) {
    uintptr_t last = pointer + 7;
    _mm_prefetch((const char *)pointer, _MM_HINT_T0);
    if ((pointer >> 6) != (last >> 6)) {
        _mm_prefetch((const char *)last, _MM_HINT_T0);
    }
}

/* Fixed packed widths with caller-selected, already-validated source offsets.
   These hints retain the generic path's exact endpoints and row lead; only
   invariant metadata/width interpretation moves outside the row loops. */
static __forceinline void hint_packed_field(uintptr_t address, size_t len) {
    uintptr_t last = address + len - 1;
    _mm_prefetch((const char *)address, _MM_HINT_T0);
    if ((address >> 6) != (last >> 6)) {
        _mm_prefetch((const char *)last, _MM_HINT_T0);
    }
}

static __forceinline void hint_packed_five_record(uintptr_t pointer,
                                                size_t source0, size_t source1,
                                                size_t source2, size_t source3,
                                                size_t source4) {
    hint_packed_field(pointer + source0, 12);
    hint_packed_field(pointer + source1, 16);
    hint_packed_field(pointer + source2, 8);
    hint_packed_field(pointer + source3, 8);
    hint_packed_field(pointer + source4, 8);
}

static __forceinline int is_packed_five_layout(const RecordField *fields) {
    return fields[0].len == 12 && fields[0].destination_offset == 0 &&
           fields[1].len == 16 && fields[1].destination_offset == 12 &&
           fields[2].len == 8 && fields[2].destination_offset == 28 &&
           fields[3].len == 8 && fields[3].destination_offset == 36 &&
           fields[4].len == 8 && fields[4].destination_offset == 44;
}

static int record_copy_fault(EXCEPTION_POINTERS *ep, const uintptr_t *pointers,
                             size_t validated_field_count, const RecordField *fields,
                             size_t field_count, size_t row_stride,
                             unsigned char *destination, size_t completed) {
    if (field_count == 0 || completed >= validated_field_count) {
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

/* Rust validates immutable metadata, total field count, every destination
   bound, and each nonempty source in the flattened validated_field_count
   prefix before entry. field_count is nonzero, and the prefix fits the checked
   row_count * field_count. Only that prefix may be evaluated here; it can end in
   the middle of a row. Source validity means nonnull base, nonoverflowing
   arithmetic, and a canonical range, not readability. An earlier unreadable
   field still faults through the exact completed-field witness/filter. */
size_t seh_guarded_copy_records(const uintptr_t *pointers, size_t validated_field_count,
                               const RecordField *fields, size_t field_count,
                               size_t row_stride, unsigned char *destination) {
    volatile size_t completed = 0;
    __try {
        /* Specialize only the fully matching layout. An empty prefix must not
           inspect either metadata array. All source/destination validation
           remains in Rust, and the shared exception filter uses the same
           completed-field witness for both paths. */
        if (validated_field_count != 0 && field_count == 1 && row_stride == 8 &&
            fields[0].source_offset == 0 && fields[0].destination_offset == 0 &&
            fields[0].len == 8) {
            size_t hinted = 0;
            while (hinted < 4 && hinted < validated_field_count) {
                hint_pointer_record(pointers[hinted]);
                ++hinted;
            }
            for (size_t index = 0; index < validated_field_count; ++index) {
                if (hinted < validated_field_count) {
                    hint_pointer_record(pointers[hinted]);
                    ++hinted;
                }
                /* One exact unaligned-safe observation per row; never widen,
                   combine duplicates, or publish a partially copied field. */
                memcpy(destination + index * 8,
                       (const unsigned char *)pointers[index], 8);
                completed = index + 1;
            }
        } else if (validated_field_count != 0 && field_count == 5 &&
                   row_stride == 52 && validated_field_count % 5 == 0 &&
                   is_packed_five_layout(fields)) {
            /* A partial validated row uses the untouched generic path. A
               complete prefix may still stop before a later invalid pointer;
               no pointer/hint beyond these rows is evaluated. Source offsets
               remain dynamic and may overlap or repeat, just as in generic
               metadata; their immutability is part of the existing contract. */
            const size_t source0 = fields[0].source_offset;
            const size_t source1 = fields[1].source_offset;
            const size_t source2 = fields[2].source_offset;
            const size_t source3 = fields[3].source_offset;
            const size_t source4 = fields[4].source_offset;
            const size_t rows = validated_field_count / 5;
            size_t hinted = 0;
            while (hinted < 4 && hinted < rows) {
                hint_packed_five_record(pointers[hinted], source0, source1,
                                       source2, source3, source4);
                ++hinted;
            }
            size_t index = 0;
            for (size_t row = 0; row < rows; ++row) {
                if (hinted < rows) {
                    hint_packed_five_record(pointers[hinted], source0, source1,
                                           source2, source3, source4);
                    ++hinted;
                }
                uintptr_t pointer = pointers[row];
                unsigned char *target = destination + row * 52;
                /* Preserve every exact observation and its completed-field
                   witness before beginning the next field. No source gap is
                   read and no failing field is published as complete. */
                memcpy(target, (const unsigned char *)(pointer + source0), 12);
                completed = ++index;
                memcpy(target + 12, (const unsigned char *)(pointer + source1), 16);
                completed = ++index;
                memcpy(target + 28, (const unsigned char *)(pointer + source2), 8);
                completed = ++index;
                memcpy(target + 36, (const unsigned char *)(pointer + source3), 8);
                completed = ++index;
                memcpy(target + 44, (const unsigned char *)(pointer + source4), 8);
                completed = ++index;
            }
        } else {
            /* Start four rows ahead, then hint one further row per copied row.
               Counters advance by the actual validated prefix length, not by a
               rounded-up row count; neither the final pointer nor its field
               metadata can be read beyond the supplied prefix. */
            size_t hinted_fields = 0;
            size_t hinted_rows = 0;
            while (hinted_rows < 4 && hinted_fields < validated_field_count) {
                size_t remaining = validated_field_count - hinted_fields;
                size_t count = remaining < field_count ? remaining : field_count;
                hint_record_fields(pointers[hinted_rows], fields, count);
                hinted_fields += count;
                ++hinted_rows;
            }
            size_t index = 0;
            for (size_t row = 0; index < validated_field_count; ++row) {
                if (hinted_fields < validated_field_count) {
                    size_t remaining = validated_field_count - hinted_fields;
                    size_t count = remaining < field_count ? remaining : field_count;
                    hint_record_fields(pointers[hinted_rows], fields, count);
                    hinted_fields += count;
                    ++hinted_rows;
                }
                uintptr_t pointer = pointers[row];
                unsigned char *target = destination + row * row_stride;
                size_t remaining = validated_field_count - index;
                size_t row_fields = remaining < field_count ? remaining : field_count;
                for (size_t field_index = 0; field_index < row_fields; ++field_index) {
                    const RecordField *field = &fields[field_index];
                    if (field->len != 0) {
                        uintptr_t address = pointer + field->source_offset;
                        copy_bytes(target + field->destination_offset,
                                   (const unsigned char *)address, field->len);
                    }
                    completed = ++index;
                }
            }
        }
    } __except (record_copy_fault(GetExceptionInformation(), pointers, validated_field_count,
                                  fields, field_count, row_stride, destination,
                                  completed)
                    ? EXCEPTION_EXECUTE_HANDLER : EXCEPTION_CONTINUE_SEARCH) {
        /* The current field may be partial. Later fields are untouched. */
    }
    return completed;
}
