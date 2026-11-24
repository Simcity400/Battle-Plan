use super::*;
use std::cell::{Cell, RefCell};

fn staging(paths: usize) -> Vec<u8> {
    Vec::with_capacity(paths * 4)
}

struct Observed<'a> {
    source: &'a PresentationMemory,
    reads: RefCell<Vec<(usize, usize)>>,
    batches: Cell<usize>,
    fault: Option<usize>,
    changing: bool,
}

impl<'a> Observed<'a> {
    fn new(source: &'a PresentationMemory, fault: Option<usize>) -> Self {
        Self {
            source,
            reads: RefCell::new(Vec::new()),
            batches: Cell::new(0),
            fault,
            changing: false,
        }
    }
}

impl engine_native_memory::Memory for Observed<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let ordinal = self.reads.borrow().len();
        self.reads.borrow_mut().push((address, output.len()));
        if self.fault == Some(ordinal) {
            let partial = output.len().div_ceil(2);
            output[..partial].fill(0xfe);
            return false;
        }
        let success = self.source.copy(address, output);
        if success && self.changing {
            output[0] = output[0].wrapping_add(ordinal as u8);
        }
        success
    }

    fn copy_records(
        &self,
        sources: engine_native_memory::memory::RecordSources<'_>,
        layout: &engine_native_memory::memory::RecordLayout<'_>,
        output: &mut [u8],
    ) -> usize {
        self.batches.set(self.batches.get() + 1);
        struct Scalar<'a, 'b>(&'a Observed<'b>);
        impl engine_native_memory::Memory for Scalar<'_, '_> {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                self.0.copy(at, out)
            }
        }
        engine_native_memory::Memory::copy_records(&Scalar(self), sources, layout, output)
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.batches.set(self.batches.get() + 1);
        struct Scalar<'a, 'b>(&'a Observed<'b>);
        impl engine_native_memory::Memory for Scalar<'_, '_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.0.copy(address, output)
            }
        }
        engine_native_memory::Memory::copy_scattered(&Scalar(self), addresses, destinations)
    }
}

fn scalar_prefixes(
    memory: &impl engine_native_memory::Memory,
    routes: [engine_native_memory::cpdx_array::Array; 2],
    outputs: [&mut [u32]; 2],
) -> Result<(), &'static str> {
    for (route, output) in routes.into_iter().zip(outputs) {
        let mut bytes = vec![0; output.len() * 4];
        let bytes = bytes.as_mut_slice();
        if !engine_native_memory::read_slice(memory, route.data, bytes) {
            return Err("movement route unreadable");
        }
        for (slot, word) in output.iter_mut().zip(bytes.chunks_exact(4)) {
            *slot = u32::from_le_bytes(word.try_into().unwrap());
        }
    }
    Ok(())
}

#[test]
fn prefixes_preserve_empty_spans_order_duplicates_and_partial_failures() {
    use engine_native_memory::cpdx_array::Array;
    let source = shared_geometry_fixture();
    for lengths in [[0, 0], [0, 1], [1, 0], [1, 1], [3, 2], [256, 256]] {
        for addresses in [
            [0x16000, 0x17000],
            [0x16000, 0x16000],
            [0, 0x17000],
            [0x16000, 0],
            [0x16000, usize::MAX - 1],
            [usize::MAX, 0x17000],
        ] {
            let routes = std::array::from_fn(|i| Array {
                data: addresses[i],
                count: lengths[i] as u32,
            });
            for fault in [None, Some(0), Some(1)] {
                let mut actual = Observed::new(&source, fault);
                let mut expected = Observed::new(&source, fault);
                actual.changing = true;
                expected.changing = true;
                let mut a = [0xdead_beef; 256];
                let mut b = a;
                let mut x = a;
                let mut y = a;
                let result = carmy::movement_route_prefixes(
                    &actual,
                    routes[0],
                    routes[1],
                    &mut staging(lengths[0] + lengths[1]),
                    &mut a[..lengths[0]],
                    &mut b[..lengths[1]],
                );
                let reference = scalar_prefixes(
                    &expected,
                    routes,
                    [&mut x[..lengths[0]], &mut y[..lengths[1]]],
                );
                assert_eq!((result, a, b), (reference, x, y));
                assert_eq!(*actual.reads.borrow(), *expected.reads.borrow());
                assert_eq!(actual.batches.get(), usize::from(lengths != [0, 0]));
            }
        }
    }
}

#[cfg(all(windows, target_env = "msvc"))]
#[test]
fn native_pairs_keep_ordered_failure_prefixes() {
    use engine_native_memory::cpdx_array::Array;
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let values = [3_u32, 9, 27];
    let route = Array {
        data: values.as_ptr() as usize,
        count: 3,
    };
    let bad = Array { data: 1, count: 1 };
    for (first, second, expected) in [(route, bad, [3, 9]), (bad, route, [0xdead_beef; 2])] {
        let mut path = [0xdead_beef; 2];
        let mut alternate = [0xdead_beef];
        assert_eq!(
            carmy::movement_route_prefixes(
                &memory,
                first,
                second,
                &mut staging(path.len() + alternate.len()),
                &mut path,
                &mut alternate
            ),
            Err("movement route unreadable"),
        );
        assert_eq!(path, expected);
        assert_eq!(alternate, [0xdead_beef]);
    }
    let mut path = [0; 3];
    carmy::movement_route_prefixes(
        &memory,
        bad,
        route,
        &mut staging(path.len()),
        &mut [],
        &mut path,
    )
    .unwrap();
    assert_eq!(path, values);
}

#[test]
fn worker_routes_grow_from_first_and_later_mod_sized_original_seed_headers() {
    let mut source = shared_geometry_fixture();
    source.bytes.resize(0x70000, 0);
    let mut packet = carmy::MovementPacket::new(0, 0);
    for length in [50_u32, 120] {
        array(
            &mut source.bytes,
            A1 + crate::cunit::ORDINARY_ROUTE_DATA,
            0x30000,
            length,
        );
        array(
            &mut source.bytes,
            A1 + crate::cunit::ALTERNATE_ROUTE_DATA,
            0x50000,
            length,
        );
        for index in 0..length {
            p32(&mut source.bytes, 0x30000 + index as usize * 4, 1);
            p32(&mut source.bytes, 0x50000 + index as usize * 4, 77 + index);
        }
        let mut seeds = [carmy::DivisionSeed {
            pointer: A1,
            division: presentation_source()[0].division,
            ..Default::default()
        }];
        carmy::capture_division_seeds(&source, &mut seeds);
        let mut geometry = cprovince::GeometryPacket::default();
        geometry.begin(&source, 0, GS).unwrap();
        let outcome = carmy::capture_movement_inputs(
            &source,
            0,
            GS,
            &seeds,
            Outcome::Rows { len: 1 },
            &mut geometry,
            &mut packet,
        );
        assert_eq!(outcome.outcome, Outcome::Rows { len: 1 });
        assert_eq!(packet.paths.len(), length as usize * 2);
        assert_eq!(packet.paths[..length as usize], vec![1; length as usize]);
        assert_eq!(
            packet.paths[length as usize..],
            (77..77 + length).collect::<Vec<_>>()
        );
        packet.clear();
    }
}

