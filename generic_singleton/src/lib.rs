use std::sync::{
    OnceLock, RwLock,
    atomic::{AtomicPtr, Ordering::Relaxed},
};

use inline_cache::inline_cache;
use type_map::StaticTypeMap;

mod type_map;

/// Returns the unique singleton value of type `T`.
///
/// Uses `T::default()` to construct the singleton value if it hasn't been constructed before.
#[inline(always)]
pub fn singleton<T: Default + Sync + 'static>() -> &'static T {
    singleton_with(Default::default)
}

/// Returns the unique singleton value of type `T`.
///
/// Uses the `construct` argument to construct the singleton value if it hasn't been constructed
/// before.
#[inline(always)]
pub fn singleton_with<T: Sync + 'static>(construct: impl FnOnce() -> T) -> &'static T {
    let cache = inline_cache!(AtomicPtr<T>);

    if let Some(cached_ptr) = unsafe { cache.load(Relaxed).as_ref() } {
        return cached_ptr;
    };

    fill_cache(cache, construct)
}

#[inline(never)]
#[cold]
fn fill_cache<T: Sync + 'static>(
    cache: &'static AtomicPtr<T>,
    construct: impl FnOnce() -> T,
) -> &'static T {
    let singleton_ref = singleton_global(construct);
    cache.store(singleton_ref as *const T as *mut T, Relaxed);
    singleton_ref
}

static GLOBAL_SINGLETON_TABLE: RwLock<StaticTypeMap> = RwLock::new(StaticTypeMap::new());

#[inline(never)]
fn singleton_global<T: Sync + 'static>(construct: impl FnOnce() -> T) -> &'static T {
    loop {
        let found = {
            let Ok(read) = GLOBAL_SINGLETON_TABLE.read() else {
                std::process::abort();
            };

            read.get::<OnceLock<&'static T>>()
        };

        // We already dropped the read guard to make sure we're not holding any global lock while
        // running the (potentially expensive) constructor
        if let Some(found) = found {
            let found = *found.get_or_init(
                #[cold]
                move || Box::leak(Box::new(construct())),
            );
            return found;
        }

        {
            let Ok(mut write) = GLOBAL_SINGLETON_TABLE.write() else {
                std::process::abort();
            };

            write.get_or_insert_with::<OnceLock<&'static T>>(|| {
                Box::leak(Box::new(OnceLock::new()))
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_singleton_with() {
        struct A(usize);
        struct B(usize);

        assert_eq!(singleton_with::<A>(|| A(1)).0, 1);
        assert_eq!(singleton_with::<A>(|| A(2)).0, 1);
        assert_eq!(singleton_with::<B>(|| B(3)).0, 3);
        assert_eq!(singleton_with::<B>(|| B(4)).0, 3);
        assert_eq!(singleton_with::<A>(|| A(5)).0, 1);
        assert_eq!(singleton_with::<B>(|| B(6)).0, 3);
    }
}
