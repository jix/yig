use std::{
    alloc::Layout,
    any::TypeId,
    collections::HashMap,
    hash::BuildHasherDefault,
    process::abort,
    ptr::{NonNull, null_mut},
    sync::{
        Mutex,
        atomic::{
            AtomicPtr,
            Ordering::{Acquire, Relaxed, Release},
        },
    },
};

use super::identity_hasher::IdentityHasher;

struct Ptr(NonNull<u8>);

unsafe impl Send for Ptr {}
unsafe impl Sync for Ptr {}

static CACHE_BUF_INIT: cache_buf::CacheBufInit<AtomicPtr<u8>> =
    cache_buf::CacheBufInit::new(AtomicPtr::new(null_mut()));

static CACHE_BUF: cache_buf::CacheBuf<AtomicPtr<u8>> = cache_buf::CacheBuf::new(&CACHE_BUF_INIT);

static CACHE: Mutex<HashMap<TypeId, Ptr, BuildHasherDefault<IdentityHasher>>> = Mutex::new(
    HashMap::with_hasher(<BuildHasherDefault<IdentityHasher>>::new()),
);

#[inline]
pub unsafe fn type_cache(key: fn() -> TypeId, layout: Layout) -> NonNull<u8> {
    let ptr = CACHE_BUF.get(key as usize);

    unsafe {
        let target = ptr.load(Relaxed);
        if let Some(found) = NonNull::new(target) {
            found
        } else {
            type_cache_fallback(key, layout)
        }
    }
}

#[inline(never)]
#[cold]
pub unsafe fn type_cache_fallback(key: fn() -> TypeId, layout: Layout) -> NonNull<u8> {
    let Ok(mut cache) = CACHE.lock() else {
        abort();
    };
    let found = match cache.entry(key()) {
        std::collections::hash_map::Entry::Occupied(entry) => entry.get().0,
        std::collections::hash_map::Entry::Vacant(entry) => {
            let Some(ptr) = NonNull::new(unsafe { std::alloc::alloc_zeroed(layout) }) else {
                std::alloc::handle_alloc_error(layout);
            };
            entry.insert(Ptr(ptr));
            ptr
        }
    };

    if CACHE_BUF.len() <= key as usize {
        CACHE_BUF.grow(key as usize, |i, old| {
            if i == key as usize {
                AtomicPtr::new(found.as_ptr())
            } else if let Some(old_ptr) = old.get(i) {
                AtomicPtr::new(old_ptr.load(Acquire))
            } else {
                AtomicPtr::new(null_mut())
            }
        });
    } else {
        CACHE_BUF.get(key as usize).store(found.as_ptr(), Release);
    }

    found
}

mod cache_buf {
    use std::{
        alloc::Layout,
        mem::offset_of,
        process::abort,
        ptr::NonNull,
        sync::{
            Mutex,
            atomic::{
                AtomicPtr,
                Ordering::{Acquire, Release},
            },
        },
    };

    pub struct CacheBuf<T> {
        ptr: AtomicPtr<T>,
        grow_lock: Mutex<()>,
    }

    #[repr(C)]
    pub struct CacheBufInit<T> {
        last: usize,
        data: T,
    }

    impl<T> CacheBufInit<T> {
        pub const fn new(initial: T) -> Self {
            Self {
                last: 0,
                data: initial,
            }
        }
    }

    const fn data_offset<T>() -> usize {
        let Ok(data_layout) = Layout::array::<T>(0) else {
            unreachable!()
        };
        let Ok((_prefix_layout, data_offset)) = Layout::new::<usize>().extend(data_layout) else {
            unreachable!()
        };
        assert!(data_offset == offset_of!(CacheBufInit<T>, data));
        data_offset
    }

    impl<T> CacheBuf<T> {
        pub const fn new(init: &'static CacheBufInit<T>) -> Self {
            unsafe {
                let data_ptr = (init as *const _ as *const u8).add(data_offset::<T>());

                Self {
                    ptr: AtomicPtr::new(data_ptr.cast_mut().cast()),
                    grow_lock: Mutex::new(()),
                }
            }
        }

        #[inline]
        pub fn get(&self, index: usize) -> &'static T {
            // Acquire to make sure the pointed at data is visible
            let data_ptr = self.ptr.load(Acquire).cast_const();

            unsafe {
                let last = data_ptr
                    .cast::<u8>()
                    .sub(data_offset::<T>())
                    .cast::<usize>()
                    .read();

                &*data_ptr.add(index.min(last))
            }
        }

        pub fn len(&self) -> usize {
            let data_ptr = self.ptr.load(Acquire).cast_const();
            unsafe {
                data_ptr
                    .cast::<u8>()
                    .sub(data_offset::<T>())
                    .cast::<usize>()
                    .read()
            }
        }

        pub fn grow(&self, target: usize, mut init: impl FnMut(usize, &[T]) -> T) {
            let Ok(_locked) = self.grow_lock.lock() else {
                abort();
            };
            let old_data_ptr = self.ptr.load(Acquire).cast_const();

            let old_last = unsafe {
                old_data_ptr
                    .cast::<u8>()
                    .sub(offset_of!(CacheBufInit<T>, data))
                    .cast::<usize>()
                    .read()
            };

            if target >= isize::MAX as usize {
                abort();
            }

            let new_last = (old_last * 2 + 1).max(target + 1);

            if new_last > isize::MAX as usize {
                abort();
            }

            let new_len = new_last + 1;

            let Ok(data_layout) = Layout::array::<T>(new_len) else {
                abort();
            };
            let last_layout = Layout::new::<usize>();
            let Ok((alloc_layout, offset)) = last_layout.extend(data_layout) else {
                abort();
            };
            assert_eq!(offset, data_offset::<T>());
            unsafe {
                let Some(allocation) = NonNull::new(std::alloc::alloc(alloc_layout)) else {
                    std::alloc::handle_alloc_error(alloc_layout);
                };

                let data_ptr = allocation.add(data_offset::<T>()).cast::<T>();

                allocation.cast::<usize>().write(new_last);

                let old_data = std::slice::from_raw_parts(old_data_ptr, old_last + 1);

                for i in 0..new_len {
                    data_ptr.add(i).write(init(i, old_data));
                }

                self.ptr.store(data_ptr.as_ptr(), Release);
            }
        }
    }
}
