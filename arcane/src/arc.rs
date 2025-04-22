use std::ops::Deref;

use crate::ptr::ArcPtr;

#[repr(transparent)]
pub struct Arc<T: ?Sized> {
    ptr: ArcPtr<T>,
}

unsafe impl<T: Sync + Send + ?Sized> Sync for Arc<T> {}
unsafe impl<T: Sync + Send + ?Sized> Send for Arc<T> {}

impl<T: ?Sized> Clone for Arc<T> {
    #[inline(always)]
    fn clone(&self) -> Self {
        unsafe { self.ptr.inc_count() };
        Self { ptr: self.ptr }
    }
}

impl<T: ?Sized> Drop for Arc<T> {
    #[inline]
    fn drop(&mut self) {
        unsafe { self.ptr.dec_count_drop_on_zero() };
    }
}

impl<T: ?Sized> Deref for Arc<T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        unsafe { self.ptr.data_ptr().as_ref() }
    }
}

impl<T> Arc<T> {
    #[inline]
    pub fn new(value: T) -> Self {
        let ptr = <ArcPtr<T>>::alloc();
        unsafe { ptr.data_ptr().write(value) };
        Self { ptr }
    }
}

impl<T: ?Sized> Arc<T> {
    // TODO relax the rhs even more so any ArcPtr wrapper can be used
    #[inline(always)]
    pub fn ptr_eq<U: ?Sized>(lhs: &Self, rhs: Arc<U>) -> bool {
        std::ptr::addr_eq(lhs.ptr.data_ptr().as_ptr(), rhs.ptr.data_ptr().as_ptr())
    }

    #[inline(always)]
    pub fn into_arc_ptr(this: Self) -> ArcPtr<T> {
        ManuallyDrop::new(this).ptr
    }

    #[inline(always)]
    pub fn as_arc_ptr(this: &Self) -> ArcPtr<T> {
        this.ptr
    }

    #[inline(always)]
    pub unsafe fn from_arc_ptr(ptr: ArcPtr<T>) -> Self {
        Self { ptr }
    }
}
}
