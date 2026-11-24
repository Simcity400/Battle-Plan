#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include <windows.h>
#include <xmmintrin.h>
#include <emmintrin.h>

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

static __forceinline void copy_other_bytes(unsigned char *dst,
                                          const unsigned char *src, size_t len) {
    if (len == 16) {
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
    } else if (len == 26) {
        memcpy(dst, src, 26);
    } else if (len == 64) {
        memcpy(dst, src, 64);
    } else {
        memcpy(dst, src, len);
    }
}

int seh_guarded_copy4(unsigned char *dst, const unsigned char *src) {
    __try {
        memcpy(dst, src, 4);
        return 1;
    } __except (copy_fault(GetExceptionInformation(), src, 4, dst)
                    ? EXCEPTION_EXECUTE_HANDLER : EXCEPTION_CONTINUE_SEARCH) {
        return 0;
    }
}

uint64_t seh_guarded_read4(const unsigned char *src) {
    __try {
        uint32_t value;
        memcpy(&value, src, 4);
        return ((uint64_t)1 << 32) | (uint64_t)value;
    } __except (copy_fault(GetExceptionInformation(), src, 4, src)
                    ? EXCEPTION_EXECUTE_HANDLER : EXCEPTION_CONTINUE_SEARCH) {
        return 0;
    }
}

static __forceinline void copy_bytes(unsigned char *dst,
                                    const unsigned char *src, size_t len) {
    if (len == 8) {
        memcpy(dst, src, 8);
    } else if (len == 4) {
        memcpy(dst, src, 4);
    } else {
        copy_other_bytes(dst, src, len);
    }
}

int seh_guarded_copy8(unsigned char *dst, const unsigned char *src) {
    __try {
        memcpy(dst, src, 8);
        return 1;
    } __except (copy_fault(GetExceptionInformation(), src, 8, dst)
                    ? EXCEPTION_EXECUTE_HANDLER : EXCEPTION_CONTINUE_SEARCH) {
        return 0;
    }
}

int seh_guarded_copy(unsigned char *dst, const unsigned char *src, size_t len) {
    __try {
        copy_other_bytes(dst, src, len);
        return 1;
    } __except (copy_fault(GetExceptionInformation(), src, len, dst)
                    ? EXCEPTION_EXECUTE_HANDLER : EXCEPTION_CONTINUE_SEARCH) {
        return 0;
    }
}

typedef struct {
    uintptr_t address;
    uint32_t destination_offset;
    uint32_t len;
} PlannedRead;

typedef struct {
    uint32_t after_reads;
    uint32_t offset;
    uint32_t len;
    uint32_t expected_offset;
} PlannedControl;

typedef struct {
    uintptr_t address;
    uint32_t after_reads;
    uint32_t len;
} PlannedHint;

static __forceinline int same_control(const unsigned char *a,
                                      const unsigned char *b, size_t len) {
    if (len == 8) return memcmp(a, b, 8) == 0;
    if (len == 4) return memcmp(a, b, 4) == 0;
    if (len == 26) {
        __m128i equal = _mm_cmpeq_epi8(_mm_loadu_si128((const __m128i *)a),
                                       _mm_loadu_si128((const __m128i *)b));
        return _mm_movemask_epi8(equal) == 0xffff &&
               memcmp(a + 16, b + 16, 8) == 0 && memcmp(a + 24, b + 24, 2) == 0;
    }
    if (len == 16 || len == 32 || len == 64) {
        __m128i different = _mm_xor_si128(_mm_loadu_si128((const __m128i *)a),
                                         _mm_loadu_si128((const __m128i *)b));
        if (len >= 32) {
            different = _mm_or_si128(different,
                _mm_xor_si128(_mm_loadu_si128((const __m128i *)(a + 16)),
                              _mm_loadu_si128((const __m128i *)(b + 16))));
        }
        if (len == 64) {
            different = _mm_or_si128(different,
                _mm_xor_si128(_mm_loadu_si128((const __m128i *)(a + 32)),
                              _mm_loadu_si128((const __m128i *)(b + 32))));
            different = _mm_or_si128(different,
                _mm_xor_si128(_mm_loadu_si128((const __m128i *)(a + 48)),
                              _mm_loadu_si128((const __m128i *)(b + 48))));
        }
        return _mm_movemask_epi8(_mm_cmpeq_epi8(different, _mm_setzero_si128())) == 0xffff;
    }
    if (len == 1) return *a == *b;
    return memcmp(a, b, len) == 0;
}

static int plan_copy_fault(EXCEPTION_POINTERS *ep, const PlannedRead *reads,
                           size_t read_count, unsigned char *destination,
                           size_t completed, int active_read) {
    if (!active_read || completed >= read_count) {
        return 0;
    }
    const PlannedRead *read = &reads[completed];
    return copy_fault(ep, (const unsigned char *)read->address, read->len,
                      destination + read->destination_offset);
}

int seh_execute_read_plan(const PlannedRead *reads, size_t read_count,
                         const PlannedControl *controls, size_t control_count,
                         const PlannedHint *hints, size_t hint_count,
                         const unsigned char *expected, unsigned char *destination,
                         size_t *completed_out) {
    volatile size_t completed = 0;
    volatile int active_read = 0;
    size_t control_index = 0;
    size_t hint_index = 0;
    int result = 0;
    __try {
        if (control_count == 0 && hint_count == 0) {
            active_read = 1;
            for (size_t index = 0; index < read_count; ++index) {
                const PlannedRead *read = &reads[index];
                if (read->len != 0) {
                    copy_bytes(destination + read->destination_offset,
                               (const unsigned char *)read->address, read->len);
                }
                completed = index + 1;
            }
            active_read = 0;
        } else {
            for (size_t index = 0;;) {
                while (control_index < control_count &&
                       controls[control_index].after_reads == index) {
                    const PlannedControl *control = &controls[control_index];
                    if (control->len != 0 &&
                        !same_control(destination + control->offset,
                                      expected + control->expected_offset, control->len)) {
                        result = 1;
                        goto plan_finished;
                    }
                    ++control_index;
                }
                while (hint_index < hint_count && hints[hint_index].after_reads == index) {
                    const PlannedHint *hint = &hints[hint_index++];
                    if (hint->len != 0) {
                        _mm_prefetch((const char *)hint->address, _MM_HINT_T0);
                        uintptr_t last = hint->address + hint->len - 1;
                        if ((last >> 6) != (hint->address >> 6)) {
                            _mm_prefetch((const char *)last, _MM_HINT_T0);
                        }
                    }
                }
                if (index == read_count) {
                    break;
                }
                size_t stop = read_count;
                if (control_index < control_count && controls[control_index].after_reads < stop) {
                    stop = controls[control_index].after_reads;
                }
                if (hint_index < hint_count && hints[hint_index].after_reads < stop) {
                    stop = hints[hint_index].after_reads;
                }
                if (stop <= index) {
                    stop = index + 1;
                }
                active_read = 1;
                while (index < stop) {
                    const PlannedRead *read = &reads[index];
                    if (read->len != 0) {
                        copy_bytes(destination + read->destination_offset,
                                   (const unsigned char *)read->address, read->len);
                    }
                    completed = ++index;
                }
                active_read = 0;
            }
        }
    } __except (plan_copy_fault(GetExceptionInformation(), reads, read_count,
                               destination, completed, active_read)
                    ? EXCEPTION_EXECUTE_HANDLER : EXCEPTION_CONTINUE_SEARCH) {
        result = 2;
    }
plan_finished:
    *completed_out = completed;
    return result;
}

typedef struct {
    unsigned char *dst;
    const unsigned char *src;
    size_t len;
} CopySpan;

size_t seh_guarded_copy_scattered(const CopySpan *spans, size_t count) {
    volatile size_t completed = 0;
    __try {
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
    }
    return completed;
}

static __forceinline void hint_copy_span(const CopySpan *span) {
    if (span->len != 0) {
        uintptr_t first = (uintptr_t)span->src;
        uintptr_t last = first + span->len - 1;
        _mm_prefetch((const char *)first, _MM_HINT_T0);
        if ((first >> 6) != (last >> 6)) {
            _mm_prefetch((const char *)last, _MM_HINT_T0);
        }
    }
}

size_t seh_guarded_copy_scattered_prefetched(const CopySpan *spans, size_t count) {
    volatile size_t completed = 0;
    __try {
        size_t hinted = 0;
        while (hinted < 4 && hinted < count) {
            hint_copy_span(&spans[hinted++]);
        }
        for (size_t index = 0; index < count; ++index) {
            if (hinted < count) {
                hint_copy_span(&spans[hinted++]);
            }
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
    }
    return completed;
}

typedef struct {
    size_t source_offset;
    size_t destination_offset;
    size_t len;
} RecordField;

static __forceinline uintptr_t record_pointer(const uintptr_t *pointers,
                                               uintptr_t base, size_t stride,
                                               size_t row) {
    return pointers != NULL ? pointers[row] : base + row * stride;
}

static __forceinline void hint_record_fields(const uintptr_t *pointers,
                                             uintptr_t base, size_t stride,
                                             size_t row,
                                             const RecordField *fields,
                                             size_t count) {
    uintptr_t pointer = 0;
    int have_pointer = 0;
    for (size_t index = 0; index < count; ++index) {
        const RecordField *field = &fields[index];
        if (field->len != 0) {
            if (!have_pointer) {
                pointer = record_pointer(pointers, base, stride, row);
                have_pointer = 1;
            }
            uintptr_t first = pointer + field->source_offset;
            uintptr_t last = first + field->len - 1;
            _mm_prefetch((const char *)first, _MM_HINT_T0);
            if ((first >> 6) != (last >> 6)) {
                _mm_prefetch((const char *)last, _MM_HINT_T0);
            }
        }
    }
}

static __forceinline void hint_pointer_record(uintptr_t pointer) {
    uintptr_t last = pointer + 7;
    _mm_prefetch((const char *)pointer, _MM_HINT_T0);
    if ((pointer >> 6) != (last >> 6)) {
        _mm_prefetch((const char *)last, _MM_HINT_T0);
    }
}

static __forceinline void hint_packed_field(uintptr_t address, size_t len) {
    uintptr_t last = address + len - 1;
    _mm_prefetch((const char *)address, _MM_HINT_T0);
    if (len > 128) {
        _mm_prefetch((const char *)(address + 64), _MM_HINT_T0);
    }
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

static __forceinline void hint_packed_two_record(uintptr_t pointer,
                                               size_t source0, size_t source1) {
    hint_packed_field(pointer + source0, 2);
    hint_packed_field(pointer + source1, 8);
}

static __forceinline int is_packed_two_layout(const RecordField *fields) {
    return fields[0].len == 2 && fields[0].destination_offset == 0 &&
           fields[1].len == 8 && fields[1].destination_offset == 8;
}

static int record_copy_fault(EXCEPTION_POINTERS *ep, const uintptr_t *pointers,
                             uintptr_t source_base, size_t source_stride,
                             size_t validated_field_count, const RecordField *fields,
                             size_t field_count, size_t row_stride,
                             unsigned char *destination, size_t completed) {
    if (field_count == 0 || completed >= validated_field_count) {
        return 0;
    }
    size_t row = completed / field_count;
    const RecordField *field = &fields[completed % field_count];
    if (field->len == 0) {
        return 0;
    }
    uintptr_t pointer = record_pointer(pointers, source_base, source_stride, row);
    if (pointer == 0 ||
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

size_t seh_guarded_copy_records(const uintptr_t *pointers,
                               uintptr_t source_base, size_t source_stride,
                               size_t validated_field_count,
                               const RecordField *fields, size_t field_count,
                               size_t row_stride, unsigned char *destination) {
    volatile size_t completed = 0;
    __try {
        if (validated_field_count != 0 && field_count == 1 && row_stride == 8 &&
            fields[0].source_offset == 0 && fields[0].destination_offset == 0 &&
            fields[0].len == 8) {
            size_t hinted = 0;
            while (hinted < 16 && hinted < validated_field_count) {
                hint_pointer_record(record_pointer(pointers, source_base, source_stride, hinted));
                ++hinted;
            }
            for (size_t index = 0; index < validated_field_count; ++index) {
                if (hinted < validated_field_count) {
                    hint_pointer_record(record_pointer(pointers, source_base, source_stride, hinted));
                    ++hinted;
                }
                memcpy(destination + index * 8,
                       (const unsigned char *)record_pointer(pointers, source_base, source_stride, index), 8);
                completed = index + 1;
            }
        } else if (validated_field_count != 0 && field_count == 1 &&
                   fields[0].len != 0 && fields[0].destination_offset == 0) {
            const size_t source_offset = fields[0].source_offset;
            const size_t len = fields[0].len;
            size_t hinted = 0;
            while (hinted < 16 && hinted < validated_field_count) {
                hint_packed_field(
                    record_pointer(pointers, source_base, source_stride, hinted) + source_offset,
                    len);
                ++hinted;
            }
            for (size_t index = 0; index < validated_field_count; ++index) {
                if (hinted < validated_field_count) {
                    hint_packed_field(
                        record_pointer(pointers, source_base, source_stride, hinted) + source_offset,
                        len);
                    ++hinted;
                }
                copy_bytes(
                    destination + index * row_stride,
                    (const unsigned char *)(record_pointer(pointers, source_base, source_stride,
                                                           index) +
                                            source_offset),
                    len);
                completed = index + 1;
            }
        } else if (validated_field_count != 0 && field_count == 5 &&
                   row_stride == 52 && validated_field_count % 5 == 0 &&
                   is_packed_five_layout(fields)) {
            const size_t source0 = fields[0].source_offset;
            const size_t source1 = fields[1].source_offset;
            const size_t source2 = fields[2].source_offset;
            const size_t source3 = fields[3].source_offset;
            const size_t source4 = fields[4].source_offset;
            const size_t rows = validated_field_count / 5;
            size_t hinted = 0;
            while (hinted < 16 && hinted < rows) {
                hint_packed_five_record(record_pointer(pointers, source_base, source_stride, hinted), source0, source1,
                                       source2, source3, source4);
                ++hinted;
            }
            size_t index = 0;
            for (size_t row = 0; row < rows; ++row) {
                if (hinted < rows) {
                    hint_packed_five_record(record_pointer(pointers, source_base, source_stride, hinted), source0, source1,
                                           source2, source3, source4);
                    ++hinted;
                }
                uintptr_t pointer = record_pointer(pointers, source_base, source_stride, row);
                unsigned char *target = destination + row * 52;
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
        } else if (validated_field_count != 0 && field_count == 2 &&
                   row_stride == 16 && validated_field_count % 2 == 0 &&
                   is_packed_two_layout(fields)) {
            const size_t source0 = fields[0].source_offset;
            const size_t source1 = fields[1].source_offset;
            const size_t rows = validated_field_count / 2;
            size_t hinted = 0;
            while (hinted < 16 && hinted < rows) {
                hint_packed_two_record(record_pointer(pointers, source_base, source_stride, hinted),
                                       source0, source1);
                ++hinted;
            }
            size_t index = 0;
            for (size_t row = 0; row < rows; ++row) {
                if (hinted < rows) {
                    hint_packed_two_record(record_pointer(pointers, source_base, source_stride, hinted),
                                           source0, source1);
                    ++hinted;
                }
                uintptr_t pointer = record_pointer(pointers, source_base, source_stride, row);
                unsigned char *target = destination + row * 16;
                memcpy(target, (const unsigned char *)(pointer + source0), 2);
                completed = ++index;
                memcpy(target + 8, (const unsigned char *)(pointer + source1), 8);
                completed = ++index;
            }
        } else {
            size_t hinted_fields = 0;
            size_t hinted_rows = 0;
            while (hinted_rows < 16 && hinted_fields < validated_field_count) {
                size_t remaining = validated_field_count - hinted_fields;
                size_t count = remaining < field_count ? remaining : field_count;
                hint_record_fields(pointers, source_base, source_stride, hinted_rows, fields, count);
                hinted_fields += count;
                ++hinted_rows;
            }
            size_t index = 0;
            for (size_t row = 0; index < validated_field_count; ++row) {
                if (hinted_fields < validated_field_count) {
                    size_t remaining = validated_field_count - hinted_fields;
                    size_t count = remaining < field_count ? remaining : field_count;
                    hint_record_fields(pointers, source_base, source_stride, hinted_rows, fields, count);
                    hinted_fields += count;
                    ++hinted_rows;
                }
                uintptr_t pointer = 0;
                int have_pointer = 0;
                unsigned char *target = destination + row * row_stride;
                size_t remaining = validated_field_count - index;
                size_t row_fields = remaining < field_count ? remaining : field_count;
                for (size_t field_index = 0; field_index < row_fields; ++field_index) {
                    const RecordField *field = &fields[field_index];
                    if (field->len != 0) {
                        if (!have_pointer) {
                            pointer = record_pointer(pointers, source_base, source_stride, row);
                            have_pointer = 1;
                        }
                        uintptr_t address = pointer + field->source_offset;
                        copy_bytes(target + field->destination_offset,
                                   (const unsigned char *)address, field->len);
                    }
                    completed = ++index;
                }
            }
        }
    } __except (record_copy_fault(GetExceptionInformation(), pointers, source_base, source_stride, validated_field_count,
                                  fields, field_count, row_stride, destination,
                                  completed)
                    ? EXCEPTION_EXECUTE_HANDLER : EXCEPTION_CONTINUE_SEARCH) {
    }
    return completed;
}
