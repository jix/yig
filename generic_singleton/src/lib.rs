use std::sync::{OnceLock, RwLock};

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
/// Uses the `construc` argument to construct the singleton value if it hasn't been constructed
/// before.
#[inline(always)]
pub fn singleton_with<T: Sync + 'static>(construct: impl FnOnce() -> T) -> &'static T {
    with_construct_ref(
        #[cold]
        move || Box::leak(Box::new(construct())),
    )
}

#[inline(never)]
fn singleton_global<T: Sync + 'static>(construct: impl FnOnce() -> &'static T) -> &'static T {
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
            let found = *found.get_or_init(construct);
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

macro_rules! cfg_else {
    (if cfg $cfg:tt { $($then:item)* } else { $($else:item)* } ) => {
        $(#[cfg $cfg] $then)* $(#[cfg(not $cfg)] $else)*
    };
}
macro_rules! enable {
    ($name:ident) => {enable!(($) $name);};
    (($D:tt) $name:ident) => { macro_rules! $name { ($D($D x:tt)*) => { $D($D x)* } } }
}
macro_rules! disable {
    ($name:ident) => {disable!(($) $name);};
    (($D:tt) $name:ident) => { macro_rules! $name { ($D($D x:tt)*) => {} } }
}

cfg_else! {
    if cfg(
        any(
            all(
                target_arch = "x86_64",
                any(target_os = "linux", target_os = "windows"),
            ),
            all(target_arch = "aarch64", target_os = "linux"),
        )
    ) {
        disable!(if_thread_local);
        enable!(if_static_cache);
    } else {
        enable!(if_thread_local);
        disable!(if_static_cache);
    }
}

static GLOBAL_SINGLETON_TABLE: RwLock<StaticTypeMap> = RwLock::new(StaticTypeMap::new());

if_thread_local! {
    mod thread_local;
}

if_static_cache! {
    mod static_cache;
}

#[inline(always)]
fn with_construct_ref<T: Sync + 'static>(construct_ref: impl FnOnce() -> &'static T) -> &'static T {
    let fallback = move || singleton_global(construct_ref);

    if_thread_local! {
        let fallback = move || thread_local::singleton_local(fallback);
    }

    if_static_cache! {
        let fallback = move || static_cache::CacheSlot::slot().get(fallback);
    }

    fallback()
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
