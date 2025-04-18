use std::cell::UnsafeCell;

use crate::type_map::StaticTypeMap;

thread_local! {
    static LOCAL_SINGLETON_TABLE: UnsafeCell<StaticTypeMap> = const { UnsafeCell::new(StaticTypeMap::new()) };
}

#[inline]
pub fn singleton_local<T: Sync + 'static>(fallback: impl FnOnce() -> &'static T) -> &'static T {
    LOCAL_SINGLETON_TABLE.with(|local_table| {
        {
            let local_table = unsafe { &*local_table.get() };
            if let Some(found) = local_table.get() {
                return found;
            }
        }

        let result = fallback();

        {
            let local_table = unsafe { &mut *local_table.get() };
            local_table.get_or_insert_with(|| result);
        }

        result
    })
}
