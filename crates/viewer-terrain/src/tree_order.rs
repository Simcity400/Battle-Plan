
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Entry {
    pub palette: u8,
    pub distance: f32,
    pub bucket: usize,
}

impl Entry {
    fn before(self, other: Self) -> bool {
        self.palette < other.palette
            || (self.palette == other.palette && self.distance < other.distance)
    }
}

pub(super) fn sort(rows: &mut [Entry]) {
    sort_budget(rows, rows.len());
}

fn sort_budget(mut rows: &mut [Entry], mut budget: usize) {
    loop {
        if rows.len() <= 32 {
            for next in 1..rows.len() {
                let value = rows[next];
                let mut hole = next;
                while hole > 0 && value.before(rows[hole - 1]) {
                    rows[hole] = rows[hole - 1];
                    hole -= 1;
                }
                rows[hole] = value;
            }
            return;
        }
        if budget == 0 {
            heap_sort(rows);
            return;
        }
        let (first, last) = partition(rows);
        budget = (budget >> 1) + (budget >> 2);
        if first < rows.len() - last {
            sort_budget(&mut rows[..first], budget);
            rows = &mut rows[last..];
        } else {
            sort_budget(&mut rows[last..], budget);
            rows = &mut rows[..first];
        }
    }
}

fn median(rows: &mut [Entry], first: usize, middle: usize, last: usize) {
    if rows[middle].before(rows[first]) {
        rows.swap(first, middle);
    }
    if rows[last].before(rows[middle]) {
        rows.swap(middle, last);
        if rows[middle].before(rows[first]) {
            rows.swap(first, middle);
        }
    }
}

fn partition(rows: &mut [Entry]) -> (usize, usize) {
    let length = rows.len();
    let middle = length / 2;
    if length >= 42 {
        let step = length / 8;
        median(rows, 0, step, 2 * step);
        median(rows, middle - step, middle, middle + step);
        median(rows, length - 1 - 2 * step, length - 1 - step, length - 1);
        median(rows, step, middle, length - 1 - step);
    } else {
        median(rows, 0, middle, length - 1);
    }
    let mut first = middle;
    let mut last = middle + 1;
    while first > 0 && !rows[first - 1].before(rows[first]) && !rows[first].before(rows[first - 1])
    {
        first -= 1;
    }
    while last < length && !rows[last].before(rows[first]) && !rows[first].before(rows[last]) {
        last += 1;
    }
    let mut right = last;
    let mut left = first;
    loop {
        while right < length {
            if !rows[first].before(rows[right]) {
                if rows[right].before(rows[first]) {
                    break;
                }
                rows.swap(last, right);
                last += 1;
            }
            right += 1;
        }
        while left > 0 {
            if !rows[left - 1].before(rows[first]) {
                if rows[first].before(rows[left - 1]) {
                    break;
                }
                first -= 1;
                rows.swap(first, left - 1);
            }
            left -= 1;
        }
        if left == 0 && right == length {
            return (first, last);
        }
        if left == 0 {
            if last != right {
                rows.swap(first, last);
            }
            last += 1;
            rows.swap(first, right);
            first += 1;
            right += 1;
        } else if right == length {
            left -= 1;
            first -= 1;
            rows.swap(left, first);
            last -= 1;
            rows.swap(first, last);
        } else {
            left -= 1;
            rows.swap(right, left);
            right += 1;
        }
    }
}

fn adjust(rows: &mut [Entry], top: usize, value: Entry) {
    let length = rows.len();
    let bottom_parent = (length - 1) / 2;
    let mut hole = top;
    while hole < bottom_parent {
        let right = 2 * hole + 2;
        let child = if rows[right].before(rows[right - 1]) {
            right - 1
        } else {
            right
        };
        rows[hole] = rows[child];
        hole = child;
    }
    if hole == bottom_parent && length.is_multiple_of(2) {
        rows[hole] = rows[length - 1];
        hole = length - 1;
    }
    while hole > top {
        let parent = (hole - 1) / 2;
        if !rows[parent].before(value) {
            break;
        }
        rows[hole] = rows[parent];
        hole = parent;
    }
    rows[hole] = value;
}

fn heap_sort(rows: &mut [Entry]) {
    for top in (0..rows.len() / 2).rev() {
        adjust(rows, top, rows[top]);
    }
    for last in (1..rows.len()).rev() {
        let value = rows[last];
        rows[last] = rows[0];
        adjust(&mut rows[..last], 0, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tie_permutations_match_msvc_insertion_partition_and_heap_paths() {
        let cases: &[(usize, usize, usize, u64)] = &[
            (0, 0, 0, 0xcbf29ce484222325),
            (0, 0, 1, 0xcbf29ce484222325),
            (0, 1, 0, 0xcbf29ce484222325),
            (0, 1, 1, 0xcbf29ce484222325),
            (0, 2, 0, 0xcbf29ce484222325),
            (0, 2, 1, 0xcbf29ce484222325),
            (0, 3, 0, 0xcbf29ce484222325),
            (0, 3, 1, 0xcbf29ce484222325),
            (1, 0, 0, 0xaf63bd4c8601b7df),
            (1, 0, 1, 0xaf63bd4c8601b7df),
            (1, 1, 0, 0xaf63bd4c8601b7df),
            (1, 1, 1, 0xaf63bd4c8601b7df),
            (1, 2, 0, 0xaf63bd4c8601b7df),
            (1, 2, 1, 0xaf63bd4c8601b7df),
            (1, 3, 0, 0xaf63bd4c8601b7df),
            (1, 3, 1, 0xaf63bd4c8601b7df),
            (2, 0, 0, 0x082f2207b4e88cc4),
            (2, 0, 1, 0x082f2207b4e88cc4),
            (2, 1, 0, 0x08328707b4eb6e3a),
            (2, 1, 1, 0x08328707b4eb6e3a),
            (2, 2, 0, 0x082f2207b4e88cc4),
            (2, 2, 1, 0x082f2207b4e88cc4),
            (2, 3, 0, 0x08328707b4eb6e3a),
            (2, 3, 1, 0x08328707b4eb6e3a),
            (31, 0, 0, 0xe11bcf94eea18f40),
            (31, 0, 1, 0xe11bcf94eea18f40),
            (31, 1, 0, 0x3169064b80f155b8),
            (31, 1, 1, 0x3169064b80f155b8),
            (31, 2, 0, 0x94aba342e6cdef2e),
            (31, 2, 1, 0x94aba342e6cdef2e),
            (31, 3, 0, 0xc8492aa3896d345a),
            (31, 3, 1, 0xc8492aa3896d345a),
            (32, 0, 0, 0x4e8648ae2ada18cb),
            (32, 0, 1, 0x4e8648ae2ada18cb),
            (32, 1, 0, 0xe6cb594c1a148ac5),
            (32, 1, 1, 0xe6cb594c1a148ac5),
            (32, 2, 0, 0xbd394fb3699e7133),
            (32, 2, 1, 0xbd394fb3699e7133),
            (32, 3, 0, 0xc165bee28472f50b),
            (32, 3, 1, 0xc165bee28472f50b),
            (33, 0, 0, 0xf7cef4966b2010cd),
            (33, 0, 1, 0xf7cef4966b2010cd),
            (33, 1, 0, 0x4013a15050e8031f),
            (33, 1, 1, 0x64d1775f49735df7),
            (33, 2, 0, 0x6f5a1bfa1d1fe381),
            (33, 2, 1, 0xafd80a7ffd5a6d25),
            (33, 3, 0, 0x12d885e70f569811),
            (33, 3, 1, 0x9b929dfe21590b9b),
            (41, 0, 0, 0x7457a54037130593),
            (41, 0, 1, 0x7acce7eb6bb77fef),
            (41, 1, 0, 0xa528b53cafb6ab0f),
            (41, 1, 1, 0xc70f1429ccacd6b7),
            (41, 2, 0, 0x6c8046894d85ade3),
            (41, 2, 1, 0xdf2069bc7cf9afc9),
            (41, 3, 0, 0x654e1035e2befecd),
            (41, 3, 1, 0xb33388d33771d12b),
            (42, 0, 0, 0x6aa6d6ac017bb136),
            (42, 0, 1, 0x542849ef4606f75a),
            (42, 1, 0, 0x5ad71c1e9364d192),
            (42, 1, 1, 0x876b38dc829bb50c),
            (42, 2, 0, 0x95382da497796ab6),
            (42, 2, 1, 0xc0b2aa4aec13c13a),
            (42, 3, 0, 0xe0cbbbaf95f730f0),
            (42, 3, 1, 0xa761659815aa29c6),
            (63, 0, 0, 0x3dac506d59553c88),
            (63, 0, 1, 0xda41bf7094af0d04),
            (63, 1, 0, 0x867ed6dd75f9e138),
            (63, 1, 1, 0xa02efb507893c7a0),
            (63, 2, 0, 0x1faa94acf1f629a4),
            (63, 2, 1, 0xa4c9c08587d9c2a6),
            (63, 3, 0, 0x5d446b21ad297562),
            (63, 3, 1, 0x1d2d270f8b88a7be),
            (127, 0, 0, 0xb5bf223c9e0c4810),
            (127, 0, 1, 0x430d95b3c6cb5364),
            (127, 1, 0, 0x25e407261fd2f438),
            (127, 1, 1, 0xb5879734d9e4bc56),
            (127, 2, 0, 0x441712379edcd878),
            (127, 2, 1, 0x26b107cfdcfd0dec),
            (127, 3, 0, 0x9b7189755e0fd1da),
            (127, 3, 1, 0x8ade5769a6137982),
            (256, 0, 0, 0x29b5918aaef7c2ed),
            (256, 0, 1, 0x623ca66c9e69f639),
            (256, 1, 0, 0x4242dc5249c33625),
            (256, 1, 1, 0xbf63abd6a831027f),
            (256, 2, 0, 0xe83e78213d4968ab),
            (256, 2, 1, 0xf75f21f9586e31d3),
            (256, 3, 0, 0x0d431c9de83363c1),
            (256, 3, 1, 0xe0c415ddde03b3a5),
            (1024, 0, 0, 0x768d887a3ef48fbd),
            (1024, 0, 1, 0x81db0b5fe2d2af5b),
            (1024, 1, 0, 0x80615f6b2bf74f25),
            (1024, 1, 1, 0x2f89f10ad4f773ff),
            (1024, 2, 0, 0x38938c8910f3d797),
            (1024, 2, 1, 0xa033e207dc15243d),
            (1024, 3, 0, 0x4e0c8ad340225d65),
            (1024, 3, 1, 0xcb955dc4f81fc115),
        ];
        for &(length, mode, heap, expected) in cases {
            let mut rows: Vec<_> = (0..length)
                .map(|i| Entry {
                    palette: match mode {
                        0 => ((i * 17 + length * 3) % 7) as u8,
                        1 => 1,
                        _ => (i / 19) as u8,
                    },
                    distance: match mode {
                        0 => ((i * 13 + length) % 5) as f32,
                        1 => 4.0,
                        2 => ((length - i) % 11) as f32,
                        _ => (i % 11) as f32,
                    },
                    bucket: i,
                })
                .collect();
            sort_budget(&mut rows, if heap == 1 { 0 } else { length });
            assert!(rows.windows(2).all(|pair| !pair[1].before(pair[0])));
            let hash = rows.iter().fold(0xcbf29ce484222325u64, |hash, row| {
                (hash ^ row.bucket as u64).wrapping_mul(0x100000001b3)
            });
            assert_eq!(hash, expected, "length={length}, mode={mode}, heap={heap}");
        }
    }
}
