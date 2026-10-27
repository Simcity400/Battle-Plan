use std::{
    alloc::{GlobalAlloc, Layout},
    cell::Cell,
    marker::PhantomData,
    rc::Rc,
};

thread_local! {
    static WATCH: Cell<(usize, bool)> = const { Cell::new((0, false)) };
}

pub(crate) struct Allocator;

unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { mimalloc::MiMalloc.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        unsafe { mimalloc::MiMalloc.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        unsafe { mimalloc::MiMalloc.realloc(pointer, layout, size) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { mimalloc::MiMalloc.dealloc(pointer, layout) };
        let _ = WATCH.try_with(|watch| {
            let (address, freed) = watch.get();
            let start = pointer as usize;
            if !freed && address >= start && address - start < layout.size() {
                watch.set((address, true));
            }
        });
    }
}

// Watch the containing allocation without retaining any strong or weak pointer.
pub(crate) struct Watch(PhantomData<Rc<()>>);

impl Watch {
    pub(crate) fn new<T>(pointer: *const T) -> Self {
        let address = pointer as usize;
        assert_ne!(address, 0);
        WATCH.with(|watch| assert_eq!(watch.replace((address, false)).0, 0));
        Self(PhantomData)
    }
    pub(crate) fn freed(&self) -> bool {
        WATCH.with(|watch| watch.get().1)
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        WATCH.with(|watch| watch.set((0, false)));
    }
}
