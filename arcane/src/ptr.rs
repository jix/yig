use std::ptr::NonNull;

use crate::untyped::{ArcLayout, UntypedArcPtr};

#[repr(transparent)]
pub struct ArcPtr<T: ?Sized> {
    ptr: NonNull<T>,
}

impl<T: ?Sized> Copy for ArcPtr<T> {}
impl<T: ?Sized> Clone for ArcPtr<T> {
    #[inline(always)]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: ?Sized> ArcPtr<T> {
    /// # Safety
    /// To safely call this, the target must still be allocated.
    #[inline(always)]
    pub fn data_ptr(self) -> NonNull<T> {
        self.ptr
    }

    /// # Safety
    /// To safely call this, `ptr` must point at a data_ptr obtained via `Self::data_ptr`.
    #[inline(always)]
    pub unsafe fn from_data_ptr(ptr: NonNull<T>) -> Self {
        Self { ptr }
    }

    #[inline(always)]
    pub fn as_untyped_ptr(self) -> UntypedArcPtr {
        unsafe { UntypedArcPtr::from_data_ptr(self.ptr.cast()) }
    }

    /// # Safety
    /// To safely call this, the target must still be allocated.
    #[inline(always)]
    pub unsafe fn inc_count(self) {
        unsafe { self.as_untyped_ptr().inc_count() }
    }

    /// # Safety
    /// To safely call this, the target must still be allocated.
    #[inline(always)]
    pub unsafe fn dec_count(self) -> usize {
        unsafe { self.as_untyped_ptr().dec_count() }
    }

    /// # Safety
    /// To safely call this, the target must still be allocated.
    #[inline(always)]
    pub unsafe fn layout(self) -> ArcLayout {
        unsafe { ArcLayout::from_dereferencable_ptr(self.data_ptr()) }
    }

    /// # Safety
    /// To safely call this, the target must still be allocated. Note that this is an unconditional
    /// dealloc that does not check the reference count. It also does not drop the stored data.
    #[inline]
    pub unsafe fn dealloc(self) {
        unsafe { self.as_untyped_ptr().dealloc(self.layout()) }
    }

    /// # Safety
    /// To safely call this, the target must still be allocated. Note that this is an unconditional
    /// dealloc that does not check the reference count. It also does not drop the stored data.
    #[inline]
    pub unsafe fn acquire_unique_drop_and_dealloc(self) {
        unsafe {
            self.as_untyped_ptr().acquire();
            self.data_ptr().drop_in_place();
            self.dealloc();
        }
    }

    /// # Safety
    /// To safely call this, the target must still be allocated.
    #[inline(always)]
    pub unsafe fn dec_count_drop_on_zero(self) {
        unsafe {
            if self.dec_count() == 0 {
                self.acquire_unique_drop_and_dealloc()
            }
        }
    }
}

impl<T> ArcPtr<T> {
    #[inline(always)]
    pub fn alloc() -> Self {
        unsafe { Self::from_untyped_ptr(UntypedArcPtr::alloc(ArcLayout::new::<T>())) }
    }

    #[inline(always)]
    pub unsafe fn from_untyped_ptr(ptr: UntypedArcPtr) -> Self {
        Self {
            ptr: unsafe { ptr.data_ptr().cast() },
        }
    }
}

impl<T> ArcPtr<[T]> {
    #[inline(always)]
    pub fn alloc_array(len: usize) -> Self {
        unsafe {
            Self::from_untyped_ptr_and_len(
                UntypedArcPtr::alloc(ArcLayout::array::<T>(len).unwrap()),
                len,
            )
        }
    }

    #[inline(always)]
    pub unsafe fn from_untyped_ptr_and_len(ptr: UntypedArcPtr, len: usize) -> Self {
        Self {
            ptr: unsafe { NonNull::slice_from_raw_parts(ptr.data_ptr().cast(), len) },
        }
    }
}
