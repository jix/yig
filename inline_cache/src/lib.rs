use std::{any::TypeId, marker::PhantomData, ptr::NonNull};

use bytemuck::Zeroable;
use cfg_if::cfg_if;

#[macro_export]
macro_rules! type_cache {
    ($T:ty, $K:ty) => {
        $crate::private::type_cache::<$T, $K>()
    };
    ($T:ty) => {
        $crate::private::type_cache::<$T, ()>()
    };
}

#[macro_export]
macro_rules! inline_cache {
    ($T:ty, $K:ty) => {{
        struct InlineCache<K: ?Sized>(PhantomData<K>);
        $crate::private::type_cache::<$T, InlineCache<$K>>()
    }};
    ($T:ty) => {{
        struct InlineCache;
        $crate::private::type_cache::<$T, InlineCache>()
    }};
}

trait PhantomAny {
    fn inner_type_id(&self) -> std::any::TypeId
    where
        Self: 'static;
}

impl<T: ?Sized> PhantomAny for PhantomData<T> {
    #[inline(always)]
    fn inner_type_id(&self) -> std::any::TypeId
    where
        Self: 'static,
    {
        std::any::TypeId::of::<Self>()
    }
}

#[inline(always)]
fn erased_type_id<T>() -> TypeId {
    let phantom: PhantomData<T> = PhantomData;
    let dyn_phantom: &dyn PhantomAny = &phantom;
    let dyn_static_phantom: &(dyn PhantomAny + 'static) =
        unsafe { &*(dyn_phantom as *const _ as *const _) };
    PhantomAny::inner_type_id(dyn_static_phantom)
}

struct InlineCache<T: Sync + Zeroable, K: ?Sized>(PhantomData<T>, PhantomData<K>);

fn inline_cache_id<T: Sync + Zeroable, K: ?Sized>() -> TypeId {
    erased_type_id::<InlineCache<T, K>>()
}

#[path = "."]
#[doc(hidden)]
pub mod private {
    use super::*;

    macro_rules! type_cache_impl {
        (align = $align:ident $(, $ops:literal)* $(,)? ) => {
            #[inline(always)]
            pub fn type_cache<T: Sync + Zeroable, K: ?Sized>() -> &'static T {
                unsafe {
                    let slot_ptr: *mut T;
                    core::arch::asm!(
                        ".comm {symbol}_SLOT, {size}, {align}",
                        $($ops,)*
                        slot = out(reg) slot_ptr,
                        size = const std::mem::size_of::<T>(),
                        align = const type_cache_impl!(@align, $align, T),
                        symbol = sym inline_cache_id::<T, K>,
                        options(pure, nomem, preserves_flags, nostack),
                    );
                    NonNull::new_unchecked(slot_ptr).as_ref()
                }
            }
        };
        (@align, bytes, $T:ty) => {
            std::mem::align_of::<$T>()
        };
        (@align, shift, $T:ty) => {
            std::mem::align_of::<$T>().trailing_zeros()
        };
        (mod $fallback:ident $(; mod $mod:ident)* $(;)?) => {
            mod $fallback;
            $(
                mod $mod;
            )*

            pub fn type_cache<T: Sync + Zeroable, K: ?Sized>() -> &'static T {
                unsafe {
                    $fallback::type_cache(
                        inline_cache_id::<T, K>,
                        std::alloc::Layout::new::<T>(),
                    )
                    .cast()
                    .as_ref()
                }
            }
        };
    }

    cfg_if! {
        if #[cfg(feature = "force_fallback_impl")] {
            type_cache_impl! {
                mod fallback_rwlock;
                mod identity_hasher;
            }
        } else if #[cfg(all(target_arch = "x86_64", target_os = "linux"))] {
            type_cache_impl! {
                align = bytes,
                "mov {slot}, [rip + {symbol}_SLOT@GOTPCREL]",
            }
        } else if #[cfg(all(target_arch = "x86_64", target_os = "windows"))] {
            type_cache_impl! {
                align = shift,
                "lea {slot}, [rip + {symbol}_SLOT]",
            }
        } else if #[cfg(all(target_arch = "aarch64", target_os = "linux"))] {
            type_cache_impl! {
                align = bytes,
                "adrp {slot}, :got:{symbol}_SLOT",
                "ldr {slot}, [{slot}, :got_lo12:{symbol}_SLOT]",
            }
        } else if #[cfg(any(target_arch = "wasm32", target_arch = "wasm64"))] {
            type_cache_impl! {
                mod flat_wasm;
                mod identity_hasher;
            }
        } else {
            type_cache_impl! {
                mod fallback_rwlock;
                mod identity_hasher;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

    use super::*;

    #[test]
    fn inline_cache() {
        println!();

        struct A;
        struct B;

        assert_eq!(type_cache!(AtomicUsize, A).fetch_add(1, Relaxed), 0);
        assert_eq!(type_cache!(AtomicUsize, A).fetch_add(1, Relaxed), 1);
        assert_eq!(type_cache!(AtomicUsize, B).fetch_add(1, Relaxed), 0);
        assert_eq!(type_cache!(AtomicUsize, A).fetch_add(1, Relaxed), 2);
        assert_eq!(type_cache!(AtomicUsize, B).fetch_add(1, Relaxed), 1);
    }

    #[test]
    fn type_cache() {
        #[inline(always)]
        fn a() -> (&'static AtomicUsize, &'static AtomicUsize) {
            (inline_cache!(_), inline_cache!(_))
        }
        #[inline(always)]
        fn b() -> (&'static AtomicUsize, &'static AtomicUsize) {
            (inline_cache!(_), inline_cache!(_))
        }
        #[inline(always)]
        fn c() -> (&'static AtomicUsize, &'static AtomicUsize) {
            (a().0, b().1)
        }

        macro_rules! step {
            ($r:expr, $x:expr, $y:expr) => {{
                let (x, y) = $r;
                assert_eq!(x.fetch_add(1, Relaxed), $x);
                assert_eq!(y.fetch_add(1, Relaxed), $y);
            }};
        }

        step!(a(), 0, 0);
        step!(a(), 1, 1);
        step!(b(), 0, 0);
        step!(b(), 1, 1);
        step!(a(), 2, 2);
        step!(c(), 3, 2);
        step!(a(), 4, 3);
        step!(b(), 2, 3);
    }

    #[test]
    fn huge() {
        fn huge0<T>(i: usize) -> bool {
            inline_cache!(AtomicUsize, T).fetch_add(1, Relaxed) == i
        }
        macro_rules! huge_fn {
            ($a:ident, $b:ident) => {
                fn $b<T>(i: usize) -> bool {
                    struct A;
                    struct B;
                    struct C;
                    struct D;
                    $a::<(A, T)>(i) && $a::<(B, T)>(i) && $a::<(C, T)>(i) && $a::<(D, T)>(i)
                }
            };
        }
        huge_fn!(huge0, huge1);
        huge_fn!(huge1, huge2);
        huge_fn!(huge2, huge3);
        huge_fn!(huge3, huge4);
        huge_fn!(huge4, huge5);

        for i in 0..1usize << 10 {
            if i.is_power_of_two() {
                println!("iteration {i}");
            }
            assert!(huge5::<()>(i));
        }
    }
}
