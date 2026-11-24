
const INSERTION_MAX: usize = 32;
const STACK_SCRATCH_MAX: usize = 512;

pub fn sort<T: Copy>(values: &mut [T], less: impl FnMut(&T, &T) -> bool) {
    sort_with_scratch_alloc(values, true, try_reserve_scratch, less);
}

#[cfg(any(test, feature = "test-support"))]
pub fn sort_with_scratch<T: Copy>(
    values: &mut [T],
    try_alloc: impl FnMut(usize) -> Option<Vec<T>>,
    less: impl FnMut(&T, &T) -> bool,
) {
    sort_with_scratch_alloc(values, true, try_alloc, less);
}

enum Scratch<T> {
    Stack {
        storage: [T; STACK_SCRATCH_MAX],
        len: usize,
    },
    Heap(Vec<T>),
}

impl<T: Copy> Scratch<T> {
    fn stack(seed: T) -> Self {
        Self::Stack {
            storage: [seed; STACK_SCRATCH_MAX],
            len: 0,
        }
    }

    fn capacity(&self) -> usize {
        match self {
            Self::Stack { .. } => STACK_SCRATCH_MAX,
            Self::Heap(buffer) => buffer.capacity(),
        }
    }

    fn as_slice(&self) -> &[T] {
        match self {
            Self::Stack { storage, len } => &storage[..*len],
            Self::Heap(buffer) => buffer.as_slice(),
        }
    }

    fn as_mut_slice(&mut self) -> &mut [T] {
        match self {
            Self::Stack { storage, len } => &mut storage[..*len],
            Self::Heap(buffer) => buffer.as_mut_slice(),
        }
    }

    fn copy_from_slice(&mut self, source: &[T]) {
        match self {
            Self::Stack { storage, len } => {
                storage[..source.len()].copy_from_slice(source);
                *len = source.len();
            }
            Self::Heap(buffer) => {
                buffer.clear();
                buffer.extend_from_slice(source);
            }
        }
    }
}

fn try_reserve_scratch<T: Copy>(capacity: usize) -> Option<Vec<T>> {
    let mut buffer = Vec::new();
    buffer.try_reserve_exact(capacity).ok()?;
    Some(buffer)
}

fn acquire_scratch<T: Copy>(
    requested: usize,
    seed: T,
    use_stack_floor: bool,
    mut try_alloc: impl FnMut(usize) -> Option<Vec<T>>,
) -> Scratch<T> {
    if use_stack_floor && requested <= STACK_SCRATCH_MAX {
        return Scratch::stack(seed);
    }
    let min_heap = if use_stack_floor { STACK_SCRATCH_MAX } else { 0 };
    let mut capacity = requested;
    while capacity > min_heap {
        if let Some(mut buffer) = try_alloc(capacity) {
            buffer.clear();
            return Scratch::Heap(buffer);
        }
        capacity >>= 1;
    }
    if use_stack_floor {
        Scratch::stack(seed)
    } else {
        Scratch::Heap(Vec::new())
    }
}

fn sort_with_scratch_alloc<T: Copy>(
    values: &mut [T],
    use_stack_floor: bool,
    try_alloc: impl FnMut(usize) -> Option<Vec<T>>,
    mut less: impl FnMut(&T, &T) -> bool,
) {
    let count = values.len();
    if count <= INSERTION_MAX {
        insertion_sort(values, &mut less);
        return;
    }
    let half = count >> 1;
    let first = count - half;
    let mut buffer = acquire_scratch(first, values[0], use_stack_floor, try_alloc);
    stable_sort_with_buffer(values, &mut buffer, &mut less);
}

fn stable_sort_with_buffer<T: Copy>(
    values: &mut [T],
    buffer: &mut Scratch<T>,
    less: &mut impl FnMut(&T, &T) -> bool,
) {
    let count = values.len();
    if count <= INSERTION_MAX {
        insertion_sort(values, less);
        return;
    }
    let half = count >> 1;
    let first = count - half;
    if first <= buffer.capacity() {
        buffered_merge_sort(&mut values[..first], buffer, less);
        buffered_merge_sort(&mut values[first..], buffer, less);
    } else {
        stable_sort_with_buffer(&mut values[..first], buffer, less);
        stable_sort_with_buffer(&mut values[first..], buffer, less);
    }
    inplace_merge(values, first, buffer, less);
}

fn insertion_sort<T: Copy>(values: &mut [T], less: &mut impl FnMut(&T, &T) -> bool) {
    for next in 1..values.len() {
        let value = values[next];
        if less(&value, &values[0]) {
            values.copy_within(0..next, 1);
            values[0] = value;
        } else {
            let mut hole = next;
            while less(&value, &values[hole - 1]) {
                values[hole] = values[hole - 1];
                hole -= 1;
            }
            values[hole] = value;
        }
    }
}

fn buffered_merge_sort<T: Copy>(
    values: &mut [T],
    buffer: &mut Scratch<T>,
    less: &mut impl FnMut(&T, &T) -> bool,
) {
    let count = values.len();
    for chunk in values.chunks_mut(INSERTION_MAX) {
        insertion_sort(chunk, less);
    }
    if count <= INSERTION_MAX {
        return;
    }
    buffer.copy_from_slice(values);
    chunked_merge(values, buffer.as_mut_slice(), INSERTION_MAX, less);
    let mut chunk = INSERTION_MAX * 2;
    loop {
        chunked_merge(buffer.as_slice(), values, chunk, less);
        chunk *= 2;
        if count <= chunk {
            return;
        }
        chunked_merge(values, buffer.as_mut_slice(), chunk, less);
        chunk *= 2;
    }
}

fn chunked_merge<T: Copy>(
    source: &[T],
    dest: &mut [T],
    chunk: usize,
    less: &mut impl FnMut(&T, &T) -> bool,
) {
    let mut first = 0;
    let mut remaining = source.len();
    while chunk < remaining {
        remaining -= chunk;
        let second = chunk.min(remaining);
        remaining -= second;
        let last = first + chunk + second;
        merge_move(&source[first..last], chunk, &mut dest[first..last], less);
        first = last;
    }
    dest[first..].copy_from_slice(&source[first..]);
}

fn merge_move<T: Copy>(
    source: &[T],
    mid: usize,
    dest: &mut [T],
    less: &mut impl FnMut(&T, &T) -> bool,
) {
    let (mut left, mut right, mut out) = (0, mid, 0);
    loop {
        if less(&source[right], &source[left]) {
            dest[out] = source[right];
            out += 1;
            right += 1;
            if right == source.len() {
                dest[out..].copy_from_slice(&source[left..mid]);
                return;
            }
        } else {
            dest[out] = source[left];
            out += 1;
            left += 1;
            if left == mid {
                dest[out..].copy_from_slice(&source[right..]);
                return;
            }
        }
    }
}

fn inplace_merge<T: Copy>(
    values: &mut [T],
    mid: usize,
    buffer: &mut Scratch<T>,
    less: &mut impl FnMut(&T, &T) -> bool,
) {
    let mut last = values.len();
    if mid == 0 || mid == last {
        return;
    }
    let mut first = 0;
    loop {
        if first == mid {
            return;
        }
        if less(&values[mid], &values[first]) {
            break;
        }
        first += 1;
    }
    let highest = mid - 1;
    loop {
        last -= 1;
        if mid == last {
            values[first..=mid].rotate_right(1);
            return;
        }
        if less(&values[last], &values[highest]) {
            break;
        }
    }
    last += 1;
    let (left, right) = (mid - first, last - mid);
    if left == 1 {
        values[first..last].rotate_left(1);
        return;
    }
    let values = &mut values[first..last];
    let capacity = buffer.capacity();
    if left <= right && left <= capacity {
        merge_buffer_left(values, left, buffer, less);
    } else if right < left && right <= capacity {
        merge_buffer_right(values, left, buffer, less);
    } else {
        let (left_split, right_split) = if left <= right {
            let left_split = left >> 1;
            let pivot = values[left_split];
            let right_split = lower_bound(&values[left..], &pivot, less);
            (left_split, right_split)
        } else {
            let right_split = right >> 1;
            let pivot = values[left + right_split];
            let left_split = upper_bound(&values[..left], &pivot, less);
            (left_split, right_split)
        };
        values[left_split..left + right_split].rotate_right(right_split);
        let merged_mid = left_split + right_split;
        inplace_merge(&mut values[..merged_mid], left_split, buffer, less);
        inplace_merge(&mut values[merged_mid..], left - left_split, buffer, less);
    }
}

fn lower_bound<T>(values: &[T], target: &T, less: &mut impl FnMut(&T, &T) -> bool) -> usize {
    let mut first = 0;
    let mut count = values.len();
    while count > 0 {
        let step = count >> 1;
        let mid = first + step;
        if less(&values[mid], target) {
            first = mid + 1;
            count -= step + 1;
        } else {
            count = step;
        }
    }
    first
}

fn upper_bound<T>(values: &[T], target: &T, less: &mut impl FnMut(&T, &T) -> bool) -> usize {
    let mut first = 0;
    let mut count = values.len();
    while count > 0 {
        let step = count >> 1;
        let mid = first + step;
        if less(target, &values[mid]) {
            count = step;
        } else {
            first = mid + 1;
            count -= step + 1;
        }
    }
    first
}

fn merge_buffer_left<T: Copy>(
    values: &mut [T],
    mid: usize,
    buffer: &mut Scratch<T>,
    less: &mut impl FnMut(&T, &T) -> bool,
) {
    buffer.copy_from_slice(&values[..mid]);
    let saved = buffer.as_slice();
    let left_last = saved.len() - 1;
    let (mut out, mut next, mut left) = (0, mid, 0);
    values[out] = values[next];
    out += 1;
    next += 1;
    loop {
        if less(&values[next], &saved[left]) {
            values[out] = values[next];
            out += 1;
            next += 1;
            if next == values.len() {
                values[out..].copy_from_slice(&saved[left..]);
                return;
            }
        } else {
            values[out] = saved[left];
            out += 1;
            left += 1;
            if left == left_last {
                values.copy_within(next.., out);
                let end = values.len() - 1;
                values[end] = saved[left_last];
                return;
            }
        }
    }
}

fn merge_buffer_right<T: Copy>(
    values: &mut [T],
    mid: usize,
    buffer: &mut Scratch<T>,
    less: &mut impl FnMut(&T, &T) -> bool,
) {
    buffer.copy_from_slice(&values[mid..]);
    let saved = buffer.as_slice();
    let mut last = values.len() - 1;
    values[last] = values[mid - 1];
    let mut left = mid - 2;
    let mut right = saved.len() - 1;
    loop {
        if less(&saved[right], &values[left]) {
            last -= 1;
            values[last] = values[left];
            if left == 0 {
                last -= 1;
                values[last] = saved[right];
                values[last - right..last].copy_from_slice(&saved[..right]);
                return;
            }
            left -= 1;
        } else {
            last -= 1;
            values[last] = saved[right];
            right -= 1;
            if right == 0 {
                last -= 1;
                values[last] = values[left];
                values.copy_within(0..left, last - left);
                values[0] = saved[0];
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(count: usize, range: u64, seed: u64) -> Vec<(u64, usize)> {
        let mut state = seed;
        (0..count)
            .map(|index| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                ((state >> 33) % range, index)
            })
            .collect()
    }

    #[test]
    fn a_strict_weak_order_sorts_as_any_stable_sort() {
        for count in [
            0, 1, 2, 31, 32, 33, 64, 65, 127, 128, 129, 200, 255, 256, 257, 1_000, 5_000,
        ] {
            for range in [1, 3, 50, u64::MAX] {
                let mut ours = keys(count, range, (count as u64).wrapping_add(range));
                let mut expected = ours.clone();
                expected.sort_by_key(|key| key.0);
                sort(&mut ours, |a, b| a.0 < b.0);
                assert_eq!(ours, expected, "{count} items below {range}");
            }
        }
    }

    #[test]
    fn a_wrapping_comparator_orders_as_the_games_insertion_sort() {
        let less = |a: &i32, b: &i32| a.wrapping_sub(*b) < 0;
        let (low, mid, high) = (i32::MIN + 10, 0, i32::MAX - 10);
        assert!(less(&low, &mid) && less(&mid, &high) && less(&high, &low));
        let mut values = [mid, high, low];
        sort(&mut values, less);
        assert_eq!(values, [low, mid, high]);
        let mut values = [high, low, mid];
        sort(&mut values, less);
        assert_eq!(values, [mid, high, low]);
    }

    #[test]
    fn a_wrapping_comparator_through_the_merges_keeps_every_item() {
        let less = |a: &(i32, usize), b: &(i32, usize)| a.0.wrapping_sub(b.0) < 0;
        for count in [33, 100, 1_000, 5_000] {
            let mut values: Vec<(i32, usize)> = keys(count, u64::from(u32::MAX), 7)
                .into_iter()
                .map(|(key, index)| (key as u32 as i32, index))
                .collect();
            let mut again = values.clone();
            sort(&mut values, less);
            sort(&mut again, less);
            assert_eq!(values, again);
            let mut indices: Vec<usize> = values.iter().map(|value| value.1).collect();
            indices.sort_unstable();
            assert_eq!(indices, (0..count).collect::<Vec<_>>());
        }
    }

    #[test]
    fn constrained_scratch_halves_fallibly_and_matches_full_stable_sort() {
        for count in [0, 1, 32, 33, 64, 65, 129, 513] {
            let input = keys(count, 17, 99 + count as u64);
            let mut expected = input.clone();
            sort(&mut expected, |a, b| a.0 < b.0);
            for limit in [0, 1, 2, 7, 16, 32, 33, 64, 128] {
                let mut requests = Vec::new();
                let mut ours = input.clone();
                sort_with_scratch_alloc(
                    &mut ours,
                    false,
                    |requested| {
                        requests.push(requested);
                        if requested > limit {
                            return None;
                        }
                        let mut buffer = Vec::new();
                        buffer.try_reserve_exact(requested).ok()?;
                        Some(buffer)
                    },
                    |a, b| a.0 < b.0,
                );
                assert_eq!(ours, expected, "count {count} with scratch limit {limit}");
                for pair in requests.windows(2) {
                    assert_eq!(pair[1], pair[0] >> 1);
                }
                if count > INSERTION_MAX && limit == 0 {
                    assert_eq!(requests.last().copied(), Some(1));
                }
            }
        }
        let large = keys(2_200, 31, 42);
        let mut expected = large.clone();
        sort(&mut expected, |a, b| a.0 < b.0);
        let mut requests = Vec::new();
        let mut fallback = large;
        sort_with_scratch(
            &mut fallback,
            |requested| {
                requests.push(requested);
                None
            },
            |a, b| a.0 < b.0,
        );
        assert_eq!(fallback, expected);
        assert_eq!(requests, vec![1_100, 550]);
    }

    #[test]
    fn stack_floor_and_msvc_pivot_side_match_native_cyclic_fleet_permutations() {
        const A: u64 = 1;
        const B: u64 = 0x6000_0001;
        const C: u64 = 0xc000_0001;
        assert!(fleet_difference(A, B) < 0);
        assert!(fleet_difference(B, C) < 0);
        assert!(fleet_difference(C, A) < 0);

        let mut fixture = Vec::with_capacity(34);
        fixture.extend(std::iter::repeat_n(A, 17));
        fixture.push(A);
        fixture.extend(std::iter::repeat_n(B, 14));
        fixture.push(C);
        fixture.push(C);

        let mut expected_buffered = Vec::with_capacity(34);
        expected_buffered.push(C);
        expected_buffered.extend(std::iter::repeat_n(A, 17));
        expected_buffered.extend(std::iter::repeat_n(B, 14));
        expected_buffered.push(C);
        expected_buffered.push(A);

        let mut expected_zero_scratch = Vec::with_capacity(34);
        expected_zero_scratch.push(C);
        expected_zero_scratch.extend(std::iter::repeat_n(A, 18));
        expected_zero_scratch.extend(std::iter::repeat_n(B, 14));
        expected_zero_scratch.push(C);

        let mut heap_calls = 0usize;
        let mut production_oom = fixture.clone();
        sort_with_scratch(
            &mut production_oom,
            |_| {
                heap_calls += 1;
                None
            },
            |&a, &b| fleet_difference(a, b) < 0,
        );
        assert_eq!(heap_calls, 0, "34-entry list must use the 512-element stack buffer");
        assert_eq!(production_oom, expected_buffered);

        let mut zero_scratch = fixture.clone();
        sort_with_scratch_alloc(
            &mut zero_scratch,
            false,
            |_| None,
            |&a, &b| fleet_difference(a, b) < 0,
        );
        assert_eq!(zero_scratch, expected_zero_scratch);

        let mut pivot_fixture = Vec::with_capacity(24);
        pivot_fixture.extend(std::iter::repeat_n(A, 12));
        pivot_fixture.extend([C, A, B, C, A, B, B, C, A, B, B, C]);
        let expected_pivot = [
            C, A, A, A, A, A, A, A, B, C, A, A, A, A, A, A, A, B, B, C, A, B, B, C,
        ];
        let mut empty = Scratch::Heap(Vec::new());
        inplace_merge(
            &mut pivot_fixture,
            12,
            &mut empty,
            &mut |&a, &b| fleet_difference(a, b) < 0,
        );
        assert_eq!(pivot_fixture.iter().position(|&v| v == B), Some(8));
        assert_eq!(pivot_fixture, expected_pivot);
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct NavalEntry {
        id: usize,
        own: bool,
        full_visibility: bool,
        refused: bool,
        reserve: bool,
        fleet: u64,
    }

    fn fleet_difference(a: u64, b: u64) -> i32 {
        let ((a_low, a_high), (b_low, b_high)) = (
            (a as u32 as i32, (a >> 32) as u32 as i32),
            (b as u32 as i32, (b >> 32) as u32 as i32),
        );
        if a_low == b_low {
            a_high.wrapping_sub(b_high)
        } else {
            a_low.wrapping_sub(b_low)
        }
    }

    fn compare_naval(a: &NavalEntry, b: &NavalEntry) -> i32 {
        let first = |a_first: bool| if a_first { -1 } else { 1 };
        if a.own != b.own {
            return first(a.own);
        }
        if !a.own {
            return if a.full_visibility == b.full_visibility {
                0
            } else {
                first(a.full_visibility)
            };
        }
        if a.refused != b.refused {
            return first(a.refused);
        }
        if a.reserve != b.reserve {
            return first(b.reserve);
        }
        if !a.reserve && a.fleet != b.fleet {
            return fleet_difference(a.fleet, b.fleet);
        }
        0
    }

    fn naval_list(count: usize, wrap_handles: bool) -> Vec<NavalEntry> {
        let mut state = 0x5eed_cafe_u64;
        let handles: [u64; 6] = if wrap_handles {
            [
                u64::from(10u32),
                u64::from(0x7fff_fff0u32),
                u64::from(0x8000_0010u32),
                (1u64 << 32) | u64::from(0x7fff_fff0u32),
                (0x8000_0020u64 << 32) | u64::from(0x7fff_fff0u32),
                u64::from(0xffff_fff0u32),
            ]
        } else {
            [10, 20, 30, 40, 50, 60]
        };
        (0..count)
            .map(|id| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let bits = state >> 32;
                NavalEntry {
                    id,
                    own: bits & 3 != 0,
                    full_visibility: bits & 4 != 0,
                    refused: bits & 8 != 0,
                    reserve: bits & 16 != 0,
                    fleet: handles[(bits as usize >> 5) % handles.len()],
                }
            })
            .collect()
    }

    #[test]
    fn fault_injected_naval_list_preserves_stable_wrapping_order() {
        let strict_list = naval_list(200, false);
        let mut strict_full = strict_list.clone();
        sort(&mut strict_full, |a, b| compare_naval(a, b) < 0);
        for limit in [0, 1, 8, 32, 64] {
            let mut constrained = strict_list.clone();
            sort_with_scratch(
                &mut constrained,
                |requested| (requested <= limit).then(|| Vec::with_capacity(requested)),
                |a, b| compare_naval(a, b) < 0,
            );
            assert_eq!(constrained, strict_full, "naval list scratch limit {limit}");
        }

        let wrapping_list = naval_list(200, true);
        let tier = |entry: &NavalEntry| match (entry.own, entry.full_visibility, entry.refused, entry.reserve) {
            (true, _, true, false) => 0u8,
            (true, _, true, true) => 1,
            (true, _, false, false) => 2,
            (true, _, false, true) => 3,
            (false, true, _, _) => 4,
            (false, false, _, _) => 5,
        };
        for limit in [0, 1, 8, 32, 64, 200] {
            let mut first = wrapping_list.clone();
            let mut again = wrapping_list.clone();
            let alloc = |requested: usize| (requested <= limit).then(|| Vec::with_capacity(requested));
            sort_with_scratch(&mut first, alloc, |a, b| compare_naval(a, b) < 0);
            sort_with_scratch(&mut again, alloc, |a, b| compare_naval(a, b) < 0);
            assert_eq!(first, again, "wrapping naval sort must be deterministic at limit {limit}");
            let mut ids: Vec<usize> = first.iter().map(|entry| entry.id).collect();
            ids.sort_unstable();
            assert_eq!(ids, (0..wrapping_list.len()).collect::<Vec<_>>());
            for pair in first.windows(2) {
                assert!(
                    tier(&pair[0]) <= tier(&pair[1]),
                    "naval tier order broke at limit {limit}: {:?} before {:?}",
                    pair[0],
                    pair[1]
                );
                if tier(&pair[0]) == tier(&pair[1]) && (pair[0].reserve || !pair[0].own || pair[0].fleet == pair[1].fleet) {
                    assert!(
                        pair[0].id < pair[1].id,
                        "equal naval entries lost stability at limit {limit}"
                    );
                }
            }
        }
    }
}
