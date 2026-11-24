use super::*;

fn same<T: Spooled + PartialEq + std::fmt::Debug>(value: T) {
    let (read, _) = round_trip(&value).unwrap();
    assert_eq!(read, value);
}

#[test]
fn numbers_read_back_exactly_at_their_extremes() {
    for value in [0_u64, 1, 127, 128, u64::MAX] {
        same(value);
    }
    for value in [0_i64, -1, 1, i64::MIN, i64::MAX] {
        same(value);
    }
    for value in [i128::MIN, -1, 0, i128::MAX] {
        same(value);
    }
    same(u128::MAX);
    same(i8::MIN);
    same(i16::MIN);
    same(usize::MAX);
    same(f64::NEG_INFINITY.to_bits());
    let nan = f64::from_bits(0x7ff8_0000_dead_beef);
    assert_eq!(round_trip(&nan).unwrap().0.to_bits(), nan.to_bits());
    assert_eq!(round_trip(&-0.0_f32).unwrap().0.to_bits(), (-0.0_f32).to_bits());
    same('ß');
}

#[test]
fn containers_read_back_exactly() {
    same(vec![1_u8, 2, 3]);
    same(Some(vec![(1_u32, String::from("a")), (2, String::from("é"))]));
    same([[1_i16; 3]; 2]);
    same(BTreeMap::from([(3_u32, vec![1_i64]), (1, Vec::new())]));
    same(BTreeSet::from([5_u64, 9]));
    same(VecDeque::from([1_u32, 2, 3]));
    same(HashMap::<u32, bool>::from([(1, true), (7, false)]));
    same(Box::<str>::from("box"));
    same(Ok::<u8, String>(3));
    same(Err::<u8, String>("no".into()));
    same(3_u32..9);
    same((1_u8, 2_u16, 3_u32, 4_u64, 5_i8, 6_i16, 7_i32, 8_i64, 9_u128, 10_i128, true, ()));
    same(Duration::new(5, 999_999_999));
}

#[test]
fn plain_slices_go_as_their_bytes() {
    same(vec![u32::MAX, 0, 300]);
    same(vec![i64::MIN, -1, 7]);
    same(vec![1.5_f64, -2.25]);
    same(vec![u128::MAX, 1]);
    same(vec![usize::MAX, 0]);
    same([-3_i16; 4]);
    assert_eq!(round_trip(&vec![1_u32; 3]).unwrap().1.len(), 1 + 3 * 4);
}

#[test]
fn an_allocation_only_its_value_held_is_released_with_it() {
    let mut shared = Shared::default();
    let mut bytes = Vec::new();
    let held = Arc::new(5_u32);
    Arc::new(7_u32)
        .spool(&mut Writer::new(&mut bytes, &mut shared))
        .unwrap();
    held.spool(&mut Writer::new(&mut bytes, &mut shared)).unwrap();
    shared.sweep_recent().unwrap();
    assert_eq!(shared.released().len(), 1);
    assert_eq!(shared.len(), 1);
}

#[test]
fn a_shared_allocation_reads_back_as_one() {
    let shared: Arc<str> = Arc::from("tag");
    let other: Arc<str> = Arc::from("tag");
    let value = vec![Arc::clone(&shared), Arc::clone(&shared), other];
    let (read, bytes) = round_trip(&value).unwrap();
    assert!(Arc::ptr_eq(&read[0], &read[1]));
    assert!(!Arc::ptr_eq(&read[0], &read[2]));
    assert_eq!(&*read[2], "tag");
    assert!(bytes.len() < 3 * 5);
}

#[test]
fn a_stream_keeps_its_allocations_across_values_until_released() {
    let (mut shared, mut restored) = (Shared::default(), Restored::default());
    let held = Arc::new(vec![1_u32, 2, 3]);
    let mut stream = Vec::new();
    for _ in 0..2 {
        let mut bytes = Vec::new();
        Arc::clone(&held)
            .spool(&mut Writer::new(&mut bytes, &mut shared))
            .unwrap();
        shared.sweep().unwrap();
        assert!(shared.released().is_empty());
        stream.push(bytes);
    }
    drop(held);
    shared.sweep().unwrap();
    let released = std::mem::take(shared.released());
    assert_eq!(released.len(), 1);
    assert!(shared.is_empty());

    let first = Arc::<Vec<u32>>::unspool(&mut Reader::new(&stream[0], &mut restored)).unwrap();
    let second = Arc::<Vec<u32>>::unspool(&mut Reader::new(&stream[1], &mut restored)).unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    restored.release(&released).unwrap();
    assert!(restored.is_empty());
}

#[derive(Debug, PartialEq)]
struct Viewed(u32);

impl Spooled for Viewed {
    const WEAKLY: bool = true;
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        self.0.spool(out)
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        Ok(Self(u32::unspool(input)?))
    }
}

impl Placeholder for Viewed {
    fn placeholder() -> Self {
        Self(0)
    }
}

#[test]
fn a_weak_names_what_its_allocation_was_read_back_as() {
    let (mut shared, mut restored) = (Shared::default(), Restored::default());
    let view = Arc::new(Viewed(7));
    let weak = Arc::downgrade(&view);
    let mut first = Vec::new();
    (Arc::clone(&view), weak.clone())
        .spool(&mut Writer::new(&mut first, &mut shared))
        .unwrap();
    drop(view);
    shared.sweep().unwrap();
    let released = std::mem::take(shared.released());
    assert_eq!(shared.len(), 1);
    let mut second = Vec::new();
    (weak.clone(), Weak::<Viewed>::new())
        .spool(&mut Writer::new(&mut second, &mut shared))
        .unwrap();

    let (arc, named) =
        <(Arc<Viewed>, Weak<Viewed>)>::unspool(&mut Reader::new(&first, &mut restored)).unwrap();
    assert!(named.ptr_eq(&Arc::downgrade(&arc)));
    restored.release(&released).unwrap();
    drop(arc);
    let (later, none) =
        <(Weak<Viewed>, Weak<Viewed>)>::unspool(&mut Reader::new(&second, &mut restored)).unwrap();
    assert!(later.ptr_eq(&named));
    assert!(later.upgrade().is_none());
    assert!(none.ptr_eq(&Weak::new()));
}

#[test]
fn a_weak_to_a_dead_allocation_it_never_met_is_one_of_its_own() {
    let dead = Arc::downgrade(&Arc::new(Viewed(1)));
    let other = Arc::downgrade(&Arc::new(Viewed(2)));
    let (read, _) = round_trip(&(dead.clone(), dead, other)).unwrap();
    assert!(read.0.ptr_eq(&read.1));
    assert!(!read.0.ptr_eq(&read.2));
    assert!(!read.0.ptr_eq(&Weak::new()));
}

#[derive(Spooled, Debug, PartialEq)]
struct Named<T> {
    first: T,
    #[spool(skip)]
    cached: Option<u32>,
    pub(crate) last: Vec<T>,
}

#[derive(Spooled, Debug, PartialEq)]
struct Tuple(u8, pub i64);

#[derive(Spooled, Debug, PartialEq)]
struct Unit;

#[derive(Spooled, Debug, PartialEq)]
enum Shape {
    Empty,
    Pair(u32, String),
    Fields { r#type: u8, inner: Option<Box<Shape>> },
}

#[test]
fn derived_codecs_read_back_their_fields_in_order() {
    same(Tuple(3, -9));
    same(Unit);
    same(Shape::Fields {
        r#type: 2,
        inner: Some(Box::new(Shape::Pair(1, "x".into()))),
    });
    same(Shape::Empty);
    let (read, _) = round_trip(&Named {
        first: 1_u16,
        cached: Some(5),
        last: vec![2, 3],
    })
    .unwrap();
    assert_eq!(
        read,
        Named {
            first: 1,
            cached: None,
            last: vec![2, 3],
        }
    );
}

#[test]
fn a_value_cut_short_is_refused() {
    let (_, bytes) = round_trip(&vec![300_u32, 400]).unwrap();
    let mut restored = Restored::default();
    let mut reader = Reader::new(&bytes[..bytes.len() - 1], &mut restored);
    assert!(Vec::<u32>::unspool(&mut reader).is_err());
}
