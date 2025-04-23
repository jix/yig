use std::{
    cell::{Cell, UnsafeCell},
    mem::{ManuallyDrop, MaybeUninit, take},
    ops::Deref,
    ptr::{NonNull, null_mut},
    sync::atomic::{
        AtomicPtr,
        Ordering::{AcqRel, Acquire, Relaxed, Release},
    },
};

use crate::ptr::TransparentArcVariant;

#[cfg(any(
    target_pointer_width = "16",
    target_pointer_width = "32",
    target_pointer_width = "64"
))]
const fn niche_offfset_in_nonnull<T: ?Sized>() -> usize {
    assert!(std::mem::size_of::<*const u8>() == std::mem::size_of::<usize>());
    assert!(std::mem::size_of::<*const u8>() == std::mem::size_of::<AtomicPtr<u8>>());

    if std::mem::size_of::<*const T>() == std::mem::size_of::<usize>() {
        return 0;
    }

    assert!(std::mem::size_of::<*const T>() == std::mem::size_of::<usize>() * 2);

    #[allow(dead_code)]
    enum FindNiche<T: ?Sized> {
        Wide(NonNull<T>),
        Narrow(usize),
    }

    assert!(std::mem::size_of::<FindNiche<T>>() == std::mem::size_of::<usize>() * 2);

    let value = <FindNiche<T>>::Narrow(0);

    assert!(matches!(
        unsafe { std::mem::transmute_copy::<_, [usize; 2]>(&value) },
        [0, 0]
    ));

    let non_niche_offset = match &value {
        FindNiche::Wide(_) => unreachable!(),
        FindNiche::Narrow(other) => unsafe {
            (other as *const usize).offset_from_unsigned(&value as *const _ as *const usize)
        },
    };

    non_niche_offset ^ 1
}

#[repr(transparent)]
pub struct ArcOnce<A: TransparentArcVariant> {
    inner: UnsafeCell<MaybeUninit<A>>,
}

impl<A: TransparentArcVariant> Default for ArcOnce<A> {
    fn default() -> Self {
        Self {
            inner: UnsafeCell::new(MaybeUninit::zeroed()),
        }
    }
}

impl<A: TransparentArcVariant + Deref<Target: std::fmt::Debug>> std::fmt::Debug for ArcOnce<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.get() {
            Some(value) => f.debug_tuple("Present").field(&&**value).finish(),
            None => f.debug_tuple("Pending").finish(),
        }
    }
}

impl<A: TransparentArcVariant> ArcOnce<A> {
    const fn has_metadata() -> bool {
        std::mem::size_of::<*const A::Target>() != std::mem::size_of::<usize>()
    }

    unsafe fn atomic(&self) -> &AtomicPtr<u8> {
        unsafe {
            &*(self as *const _ as *const AtomicPtr<u8>)
                .add(niche_offfset_in_nonnull::<A::Target>())
        }
    }

    unsafe fn nonatomic(&self) -> &Cell<*const u8> {
        assert!(Self::has_metadata());
        unsafe {
            &*(self as *const _ as *const Cell<*const u8>)
                .add(niche_offfset_in_nonnull::<A::Target>() ^ 1)
        }
    }

    pub fn get(&self) -> Option<&A> {
        if Self::has_metadata() {
            unsafe {
                loop {
                    match self.atomic().load(Acquire).addr() {
                        0 => return None,
                        usize::MAX => (),
                        _ => return Some((*self.inner.get()).assume_init_ref()),
                    }
                }
            }
        } else {
            unsafe {
                if self.atomic().load(Acquire).addr() == 0 {
                    None
                } else {
                    Some((*self.inner.get()).assume_init_ref())
                }
            }
        }
    }

    pub fn set(&self, value: A) -> Option<A> {
        let value = ManuallyDrop::new(value);
        if Self::has_metadata() {
            unsafe {
                let value_niche_ptr = &*(&value as *const _ as *const *mut u8)
                    .add(niche_offfset_in_nonnull::<A::Target>());
                let value_nonatomic_ptr = &*(&value as *const _ as *const *mut u8)
                    .add(niche_offfset_in_nonnull::<A::Target>() ^ 1);

                loop {
                    match self.atomic().compare_exchange_weak(
                        null_mut(),
                        null_mut::<u8>().with_addr(usize::MAX),
                        AcqRel,
                        Relaxed,
                    ) {
                        Ok(_) => {
                            self.nonatomic().set(*value_nonatomic_ptr);

                            self.atomic().store(*value_niche_ptr, Release);
                            return None;
                        }
                        Err(niche_value) => {
                            if niche_value.addr().wrapping_add(1) <= 1 {
                                std::hint::spin_loop();
                            } else {
                                return Some(ManuallyDrop::into_inner(value));
                            }
                        }
                    }
                }
            };
        } else {
            unsafe {
                if self
                    .atomic()
                    .compare_exchange(
                        null_mut(),
                        std::mem::transmute_copy(&value),
                        Release,
                        Relaxed,
                    )
                    .is_ok()
                {
                    None
                } else {
                    Some(ManuallyDrop::into_inner(value))
                }
            }
        }
    }

    pub fn take(&mut self) -> Option<A> {
        if Self::has_metadata() {
            unsafe {
                loop {
                    match self.atomic().load(Acquire).addr() {
                        0 => return None,
                        usize::MAX => (),
                        _ => return Some(std::mem::transmute_copy(&ManuallyDrop::new(take(self)))),
                    }
                }
            }
        } else {
            unsafe {
                if self.atomic().load(Acquire).addr() == 0 {
                    None
                } else {
                    Some(std::mem::transmute_copy(&ManuallyDrop::new(take(self))))
                }
            }
        }
    }
}

impl<A: TransparentArcVariant> Drop for ArcOnce<A> {
    fn drop(&mut self) {
        self.take();
    }
}
