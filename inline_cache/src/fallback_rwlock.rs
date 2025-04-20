use std::{
    alloc::Layout, any::TypeId, collections::HashMap, hash::BuildHasherDefault, process::abort,
    ptr::NonNull, sync::RwLock,
};

use super::identity_hasher::IdentityHasher;

struct Ptr(NonNull<u8>);

unsafe impl Send for Ptr {}
unsafe impl Sync for Ptr {}

static CACHE: RwLock<HashMap<TypeId, Ptr, BuildHasherDefault<IdentityHasher>>> = RwLock::new(
    HashMap::with_hasher(<BuildHasherDefault<IdentityHasher>>::new()),
);

#[inline]
pub unsafe fn type_cache(key: fn() -> TypeId, layout: Layout) -> NonNull<u8> {
    let type_id = key();
    {
        let Ok(cache) = CACHE.read() else {
            abort();
        };
        if let Some(found) = cache.get(&type_id) {
            return found.0;
        }
    };

    unsafe { type_cache_fallback(key, layout) }
}

#[inline(never)]
#[cold]
pub unsafe fn type_cache_fallback(key: fn() -> TypeId, layout: Layout) -> NonNull<u8> {
    let Ok(mut cache) = CACHE.write() else {
        abort();
    };

    match cache.entry(key()) {
        std::collections::hash_map::Entry::Occupied(entry) => entry.get().0,
        std::collections::hash_map::Entry::Vacant(entry) => {
            let Some(ptr) = NonNull::new(unsafe { std::alloc::alloc_zeroed(layout) }) else {
                std::alloc::handle_alloc_error(layout);
            };
            entry.insert(Ptr(ptr));

            ptr
        }
    }
}
