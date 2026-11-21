
const INSERTION_MAX: usize = 32;

pub fn sort<T: Copy>(values: &mut [T], mut less: impl FnMut(&T, &T) -> bool) {
    let count = values.len();
    if count <= INSERTION_MAX {
        insertion_sort(values, &mut less);
        return;
    }
    let half = count >> 1;
    let first = count - half;
    let mut buffer = Vec::with_capacity(first);
    buffered_merge_sort(&mut values[..first], &mut buffer, &mut less);
    buffered_merge_sort(&mut values[first..], &mut buffer, &mut less);
    inplace_merge(values, first, &mut buffer, &mut less);
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
    buffer: &mut Vec<T>,
    less: &mut impl FnMut(&T, &T) -> bool,
) {
    let count = values.len();
    for chunk in values.chunks_mut(INSERTION_MAX) {
        insertion_sort(chunk, less);
    }
    if count <= INSERTION_MAX {
        return;
    }
    buffer.clear();
    buffer.extend_from_slice(values);
    chunked_merge(values, buffer, INSERTION_MAX, less);
    let mut chunk = INSERTION_MAX * 2;
    loop {
        chunked_merge(buffer, values, chunk, less);
        chunk *= 2;
        if count <= chunk {
            return;
        }
        chunked_merge(values, buffer, chunk, less);
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
    buffer: &mut Vec<T>,
    less: &mut impl FnMut(&T, &T) -> bool,
) {
    let mut last = values.len();
    if mid == last {
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
    buffer.clear();
    if left <= right {
        merge_buffer_left(values, left, buffer, less);
    } else {
        merge_buffer_right(values, left, buffer, less);
    }
}

fn merge_buffer_left<T: Copy>(
    values: &mut [T],
    mid: usize,
    buffer: &mut Vec<T>,
    less: &mut impl FnMut(&T, &T) -> bool,
) {
    buffer.extend_from_slice(&values[..mid]);
    let left_last = buffer.len() - 1;
    let (mut out, mut next, mut left) = (0, mid, 0);
    values[out] = values[next];
    out += 1;
    next += 1;
    loop {
        if less(&values[next], &buffer[left]) {
            values[out] = values[next];
            out += 1;
            next += 1;
            if next == values.len() {
                values[out..].copy_from_slice(&buffer[left..]);
                return;
            }
        } else {
            values[out] = buffer[left];
            out += 1;
            left += 1;
            if left == left_last {
                values.copy_within(next.., out);
                let end = values.len() - 1;
                values[end] = buffer[left_last];
                return;
            }
        }
    }
}

fn merge_buffer_right<T: Copy>(
    values: &mut [T],
    mid: usize,
    buffer: &mut Vec<T>,
    less: &mut impl FnMut(&T, &T) -> bool,
) {
    buffer.extend_from_slice(&values[mid..]);
    let mut last = values.len() - 1;
    values[last] = values[mid - 1];
    let mut left = mid - 2;
    let mut right = buffer.len() - 1;
    loop {
        if less(&buffer[right], &values[left]) {
            last -= 1;
            values[last] = values[left];
            if left == 0 {
                last -= 1;
                values[last] = buffer[right];
                values[last - right..last].copy_from_slice(&buffer[..right]);
                return;
            }
            left -= 1;
        } else {
            last -= 1;
            values[last] = buffer[right];
            right -= 1;
            if right == 0 {
                last -= 1;
                values[last] = values[left];
                values.copy_within(0..left, last - left);
                values[0] = buffer[0];
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
}
