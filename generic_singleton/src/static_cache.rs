use std::sync::atomic::{AtomicPtr, Ordering::Relaxed};

pub struct CacheSlot<T> {
    ptr: AtomicPtr<T>,
}

impl<T: 'static> CacheSlot<T> {
    #[inline(always)]
    pub fn slot() -> &'static CacheSlot<T> {
        macro_rules! asm_template {
            ($($inst:literal),* $(,)?) => {
                unsafe {
                    let slot_ptr: *const CacheSlot<T>;
                    core::arch::asm!(
                        ".comm {symbol}_SLOT, {size}",
                        $($inst,)*
                        size = const std::mem::size_of::<CacheSlot<T>>(),
                        symbol = sym Self::slot,
                        slot_ptr = out(reg) slot_ptr,
                        options(pure, nomem, preserves_flags, nostack),
                    );
                    &*slot_ptr
                }
            };
        }

        #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
        asm_template! {
            "mov {slot_ptr}, [rip + {symbol}_SLOT@GOTPCREL]",
        }

        #[cfg(all(target_arch = "x86_64", target_os = "windows"))]
        asm_template! {
            "lea {slot_ptr}, [rip + {symbol}_SLOT]",
        }

        #[cfg(target_arch = "aarch64")]
        asm_template! {
            "adrp {slot_ptr}, :got:{symbol}_SLOT",
            "ldr {slot_ptr}, [{slot_ptr}, :got_lo12:{symbol}_SLOT]",
        }
    }

    #[inline(always)]
    pub fn load(&self) -> Option<&'static T> {
        unsafe { self.ptr.load(Relaxed).as_ref() }
    }

    #[inline(always)]
    pub fn store(&self, item: &'static T) {
        self.ptr.store(item as *const _ as *mut _, Relaxed);
    }

    #[inline(always)]
    pub fn get(&self, fallback: impl FnOnce() -> &'static T) -> &'static T {
        if let Some(cached) = self.load() {
            return cached;
        }

        let obtained = fallback();

        self.store(obtained);

        obtained
    }
}
