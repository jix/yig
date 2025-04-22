use std::{
    alloc::{Layout, LayoutError},
    hint::unreachable_unchecked,
    process::abort,
    ptr::NonNull,
    sync::atomic::{
        AtomicUsize,
        Ordering::{Acquire, Relaxed, Release},
    },
};

const MAX_COUNT: usize = isize::MAX as usize;

#[derive(Clone, Copy, Debug)]
pub struct ArcLayout {
    // SAFETY: Both size and alignment may not be below those of `AtomicUsize`, must be padded to
    // the alignment
    full_layout: Layout,
    header_size: usize,
}

impl ArcLayout {
    pub const fn new<T>() -> Self {
        match Self::from_data_layout(Layout::new::<T>()) {
            Ok(layout) => layout,
            Err(_) => panic!("excessive type size"),
        }
    }

    pub const fn array<T>(len: usize) -> Result<Self, LayoutError> {
        match Layout::array::<T>(len) {
            Ok(ok) => Self::from_data_layout(ok),
            Err(err) => Err(err),
        }
    }

    pub const fn from_data_layout(data_layout: Layout) -> Result<Self, LayoutError> {
        let header_layout = Layout::new::<AtomicUsize>();
        let (unpadded_layout, header_size) = match header_layout.extend(data_layout) {
            Ok(ok) => ok,
            Err(err) => return Err(err),
        };

        let full_layout = unpadded_layout.pad_to_align();
        Ok(Self {
            full_layout,
            header_size,
        })
    }

    pub const fn data_layout(&self) -> Layout {
        // SAFETY: safe due to our type level invariants
        unsafe {
            Layout::from_size_align_unchecked(
                self.full_layout.size() - self.header_size,
                self.full_layout.align(),
            )
        }
    }

    pub const fn full_layout(&self) -> Layout {
        self.full_layout
    }

    pub const fn header_size(&self) -> usize {
        self.header_size
    }

    /// # Safety
    /// May only be called for pointers that point into an allocation suitable for holding the full
    /// layout.
    pub unsafe fn from_dereferencable_ptr<T: ?Sized>(ptr: NonNull<T>) -> Self {
        // SAFETY: This should be using `for_value_raw` but that hasn't been stabilized yet. This
        // dereferences the pointer, creating a transient reference to potentially invalid data, but
        // it never reads anything from the target address since the alignment and size are
        // statically known, stored in the pointer or the pointer's vtable. When running this with
        // miri, it only checks that `ptr` points into a valid and sufficiently large allocation, so
        // this is fine for now but should be changed when the `_raw` variants become stable
        let data_layout = Layout::for_value(unsafe { ptr.as_ref() });

        let Ok(ok) = Self::from_data_layout(data_layout) else {
            // SAFETY: We require that ptr points to an allocation suitable holding the full layout
            // and for such an allocation to exist, the full layout must be valid
            unsafe {
                unreachable_unchecked();
            }
        };
        ok
    }
}

#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct UntypedArcPtr {
    ptr: NonNull<u8>,
}

impl UntypedArcPtr {
    #[inline(always)]
    unsafe fn from_alloc_ptr(layout: ArcLayout, ptr: NonNull<u8>) -> Self {
        unsafe {
            Self {
                ptr: ptr.byte_add(layout.header_size()),
            }
        }
    }

    /// # Safety
    /// To safely call this, the target must still be allocated.
    #[inline(always)]
    unsafe fn as_alloc_ptr(self, layout: ArcLayout) -> NonNull<u8> {
        unsafe { self.ptr.sub(layout.header_size()) }
    }

    /// # Safety
    /// To safely call this, the target must still be allocated.
    #[inline(always)]
    pub unsafe fn count_ptr(self) -> NonNull<AtomicUsize> {
        unsafe { self.ptr.cast::<AtomicUsize>().sub(1) }
    }

    /// # Safety
    /// To safely call this, the target must still be allocated.
    #[inline(always)]
    pub unsafe fn count<'a>(self) -> &'a AtomicUsize {
        unsafe { self.count_ptr().as_ref() }
    }

    /// # Safety
    /// To safely call this, the target must still be allocated.
    #[inline(always)]
    pub unsafe fn data_ptr(self) -> NonNull<u8> {
        self.ptr
    }

    /// # Safety
    /// To safely call this, `ptr` must point at a data_ptr obtained via `Self::data_ptr`.
    #[inline(always)]
    pub unsafe fn from_data_ptr(ptr: NonNull<u8>) -> Self {
        Self { ptr }
    }

    #[inline]
    pub fn alloc(layout: ArcLayout) -> Self {
        // SAFETY: full_layout includes `count` and thus is guaranteed to have a non-zero size
        unsafe {
            let Some(ptr) = NonNull::new(std::alloc::alloc(layout.full_layout())) else {
                std::alloc::handle_alloc_error(layout.full_layout())
            };
            let arc_ptr = Self::from_alloc_ptr(layout, ptr);
            arc_ptr.count_ptr().write(AtomicUsize::new(1));
            arc_ptr
        }
    }

    #[inline]
    pub fn alloc_zeroed(layout: ArcLayout) -> Self {
        // SAFETY: full_layout includes `count` and thus is guaranteed to have a non-zero size
        unsafe {
            let Some(alloc_ptr) = NonNull::new(std::alloc::alloc_zeroed(layout.full_layout()))
            else {
                std::alloc::handle_alloc_error(layout.full_layout())
            };
            let arc_ptr = Self::from_alloc_ptr(layout, alloc_ptr);
            arc_ptr.count_ptr().write(AtomicUsize::new(1));
            arc_ptr
        }
    }

    /// # Safety
    /// To safely call this, the target must still be allocated. Note that this is an unconditional
    /// dealloc that does not check the reference count.
    #[inline]
    pub unsafe fn dealloc(self, layout: ArcLayout) {
        unsafe {
            let alloc_ptr = self.as_alloc_ptr(layout);
            std::alloc::dealloc(alloc_ptr.as_ptr(), layout.full_layout());
        }
    }

    /// # Safety
    /// To safely call this, the target must still be allocated.
    #[inline(always)]
    pub unsafe fn inc_count(self) {
        let prev_count = unsafe { self.count().fetch_add(1, Relaxed) };
        if prev_count >= MAX_COUNT {
            abort();
        }
    }

    /// # Safety
    /// To safely call this, the target must still be allocated.
    #[inline(always)]
    pub unsafe fn dec_count(self) -> usize {
        unsafe { self.count().fetch_sub(1, Release) - 1 }
    }

    /// # Safety
    /// To safely call this, the target must still be allocated.
    #[inline(always)]
    pub unsafe fn load_count(self) -> usize {
        unsafe { self.count().load(Relaxed) }
    }

    /// # Safety
    /// To safely call this, the target must still be allocated.
    #[inline(always)]
    pub unsafe fn acquire(self) {
        unsafe {
            self.count().load(Acquire);
        }
    }
}
