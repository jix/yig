use std::{
    hash::{BuildHasher, BuildHasherDefault, Hash},
    marker::PhantomData,
    ops::Deref,
    pin::Pin,
    process::abort,
    ptr::NonNull,
    sync::RwLock,
};

use hashbrown::HashTable;

use crate::{
    arc::{Arc, UniqueArc},
    borrow::ArcBorrow,
    ptr::{ArcPtr, ArcVariant, TransparentArcVariant},
};

pub trait Dedup<T: ?Sized>: Default + Sync + Send + 'static {
    fn dedup_hash(&self, value: &T) -> u64;
    fn dedup_eq(&self, lhs: &T, rhs: &T) -> bool;
}

#[derive(Default)]
pub struct BuildHasherDedup<H>(H);

impl<T: Hash + Eq + ?Sized, H: BuildHasher + Default + Sync + Send + 'static> Dedup<T>
    for BuildHasherDedup<H>
{
    #[inline(always)]
    fn dedup_hash(&self, value: &T) -> u64 {
        self.0.hash_one(value)
    }

    #[inline(always)]
    fn dedup_eq(&self, lhs: &T, rhs: &T) -> bool {
        lhs == rhs
    }
}

pub struct DedupEntry<T: Send + Sync + ?Sized + 'static, D: Dedup<T>> {
    _phantom: PhantomData<D>,
    // SAFETY: May not implement Deref as that would lead to unchecked unpinning as we expose
    // DedupEntry publicly
    inner: T,
}

impl<T: Send + Sync + ?Sized + 'static, D: Dedup<T>> Drop for DedupEntry<T, D> {
    #[inline]
    fn drop(&mut self) {
        DedupTable::get().forget(self);
    }
}

pub type DefaultDedup = BuildHasherDedup<BuildHasherDefault<zwohash::ZwoHasher>>;

#[repr(transparent)]
pub struct DedupArc<T: Send + Sync + ?Sized + 'static, D: Dedup<T> = DefaultDedup> {
    inner: Pin<Arc<DedupEntry<T, D>>>,
}

impl<T: Send + Sync + ?Sized + 'static, D: Dedup<T>> Clone for DedupArc<T, D> {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

unsafe impl<T: Sync + Send + ?Sized, D: Dedup<T>> Sync for DedupArc<T, D> {}
unsafe impl<T: Sync + Send + ?Sized, D: Dedup<T>> Send for DedupArc<T, D> {}

unsafe impl<T: Send + Sync + ?Sized + 'static, D: Dedup<T>> TransparentArcVariant
    for DedupArc<T, D>
{
}
impl<T: Send + Sync + ?Sized + 'static, D: Dedup<T>> ArcVariant for DedupArc<T, D> {
    type Target = T;

    #[inline(always)]
    fn as_arc_ptr(this: &Self) -> ArcPtr<T> {
        unsafe {
            ArcPtr::from_data_ptr(NonNull::new_unchecked(
                ArcVariant::as_arc_ptr(&this.inner).data_ptr().as_ptr() as *mut T,
            ))
        }
    }

    #[inline(always)]
    fn into_arc_ptr(this: Self) -> ArcPtr<T> {
        unsafe {
            ArcPtr::from_data_ptr(NonNull::new_unchecked(
                ArcVariant::into_arc_ptr(DedupArc::into_entry(this))
                    .data_ptr()
                    .as_ptr() as *mut T,
            ))
        }
    }

    #[inline(always)]
    unsafe fn from_arc_ptr(ptr: ArcPtr<T>) -> Self {
        unsafe {
            Self {
                inner: ArcVariant::from_arc_ptr(ArcPtr::from_data_ptr(NonNull::new_unchecked(
                    ptr.data_ptr().as_ptr() as *mut DedupEntry<T, D>,
                ))),
            }
        }
    }
}

impl<T: Send + Sync + ?Sized + 'static, D: Dedup<T>> Deref for DedupArc<T, D> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        &self.inner.inner
    }
}

impl<T: Send + Sync + ?Sized + 'static, D: Dedup<T>> PartialEq for DedupArc<T, D> {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        ArcVariant::addr_eq(self, other)
    }
}

impl<T: Send + Sync + ?Sized + 'static, D: Dedup<T>> Eq for DedupArc<T, D> {}

impl<T: Send + Sync + ?Sized + 'static, D: Dedup<T>> Hash for DedupArc<T, D> {
    #[inline(always)]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // TODO customizable
        ArcVariant::as_arc_ptr(self).data_ptr().addr().hash(state);
    }
}

impl<T: std::fmt::Debug + Send + Sync + ?Sized + 'static, D: Dedup<T>> std::fmt::Debug
    for DedupArc<T, D>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&**self, f)
    }
}

struct DedupTable<T: Send + Sync + ?Sized + 'static, D: Dedup<T>> {
    dedup: D,
    // TODO use concurrent hash tables
    table: RwLock<HashTable<ArcPtr<DedupEntry<T, D>>>>,
}

unsafe impl<T: Sync + Send + ?Sized, D: Dedup<T>> Sync for DedupTable<T, D> {}
unsafe impl<T: Sync + Send + ?Sized, D: Dedup<T>> Send for DedupTable<T, D> {}

impl<T: Send + Sync + ?Sized + 'static, D: Dedup<T>> Default for DedupTable<T, D> {
    fn default() -> Self {
        Self {
            dedup: Default::default(),
            table: Default::default(),
        }
    }
}

impl<T: Sync + Send + ?Sized, D: Dedup<T>> DedupTable<T, D> {
    pub fn get() -> &'static Self {
        generic_singleton::singleton()
    }

    pub fn find_or_remember(&self, unique: UniqueArc<T>) -> (DedupArc<T, D>, Option<UniqueArc<T>>) {
        let hash = self.dedup.dedup_hash(&unique);
        {
            let read = self.table.read().unwrap_or_else(|_| abort());
            if let Some(&found) = read.find(hash, |entry| {
                self.dedup
                    .dedup_eq(&unique, unsafe { &entry.data_ptr().as_ref().inner })
            }) {
                let arc_borrow = unsafe { Pin::new_unchecked(ArcBorrow::from_arc_ptr(found)) };
                return (
                    DedupArc {
                        inner: ArcBorrow::clone_pinned_arc(arc_borrow),
                    },
                    Some(unique),
                );
            }
        }

        {
            use hashbrown::hash_table::Entry::{Occupied, Vacant};
            let mut write = self.table.write().unwrap_or_else(|_| abort());
            match write.entry(
                hash,
                |entry| {
                    self.dedup
                        .dedup_eq(&unique, unsafe { &entry.data_ptr().as_ref().inner })
                },
                |entry| {
                    self.dedup
                        .dedup_hash(unsafe { &entry.data_ptr().as_ref().inner })
                },
            ) {
                Occupied(occupied_entry) => {
                    let found = *occupied_entry.get();
                    let arc_borrow = unsafe { Pin::new_unchecked(ArcBorrow::from_arc_ptr(found)) };
                    (
                        DedupArc {
                            inner: ArcBorrow::clone_pinned_arc(arc_borrow),
                        },
                        Some(unique),
                    )
                }
                Vacant(vacant_entry) => {
                    let data_ptr: *mut T = UniqueArc::into_arc_ptr(unique).data_ptr().as_ptr();
                    let entry_ptr = unsafe {
                        ArcPtr::from_data_ptr(NonNull::new_unchecked(
                            data_ptr as *mut DedupEntry<T, D>,
                        ))
                    };
                    let entry_unique = unsafe { UniqueArc::from_arc_ptr(entry_ptr) };
                    let entry_pinned = UniqueArc::into_pin(entry_unique);
                    let entry_arc = <Pin<Arc<DedupEntry<T, D>>>>::from(entry_pinned);
                    vacant_entry.insert(entry_ptr);
                    (DedupArc { inner: entry_arc }, None)
                }
            }
        }
    }

    pub fn forget(&self, entry: &mut DedupEntry<T, D>) {
        let hash = self.dedup.dedup_hash(&entry.inner);

        let mut write = self.table.write().unwrap_or_else(|_| abort());
        match write.find_entry(hash, |candidate| {
            std::ptr::addr_eq(candidate.data_ptr().as_ptr(), entry)
        }) {
            Err(_) => abort(),
            Ok(entry) => {
                entry.remove();
            }
        }
    }
}

impl<T: Sync + Send + ?Sized + 'static, D: Dedup<T>> DedupArc<T, D> {
    #[inline(always)]
    pub fn find_or_remember(unique: UniqueArc<T>) -> (Self, Option<UniqueArc<T>>) {
        DedupTable::get().find_or_remember(unique)
    }
}

impl<T: Send + Sync + ?Sized + 'static, D: Dedup<T>> DedupArc<T, D> {
    #[inline(always)]
    pub fn into_entry(this: Self) -> Pin<Arc<DedupEntry<T, D>>> {
        this.inner
    }

    #[inline(always)]
    pub fn as_entry(this: &Self) -> &Pin<Arc<DedupEntry<T, D>>> {
        &this.inner
    }

    #[inline(always)]
    pub fn from_entry(entry: Pin<Arc<DedupEntry<T, D>>>) -> Self {
        Self { inner: entry }
    }
}

impl<T: Sync + Send + 'static, D: Dedup<T>> DedupArc<T, D> {
    #[inline(always)]
    pub fn new(value: T) -> Self {
        Self::find_or_remember(UniqueArc::new(value)).0
    }
}

impl<T: Send + Sync + ?Sized + 'static, D: Dedup<T>> From<UniqueArc<T>> for DedupArc<T, D> {
    fn from(value: UniqueArc<T>) -> Self {
        Self::find_or_remember(value).0
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[test]
    fn dedup() {
        #[derive(PartialEq, Eq, Debug)]
        enum Action {
            Created(usize, usize),
            Dropped(usize, usize),
        }
        use Action::*;

        struct Logging(Arc<Mutex<Vec<Action>>>, usize, usize);

        impl PartialEq for Logging {
            fn eq(&self, other: &Self) -> bool {
                ArcVariant::as_arc_ptr(&self.0).data_ptr()
                    == ArcVariant::as_arc_ptr(&other.0).data_ptr()
                    && self.2 == other.2
            }
        }

        impl Eq for Logging {}

        impl Hash for Logging {
            fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
                ArcVariant::as_arc_ptr(&self.0).data_ptr().hash(state);
                self.2.hash(state);
            }
        }

        impl Logging {
            pub fn new(log: &Arc<Mutex<Vec<Action>>>, id: usize, value: usize) -> Self {
                log.lock().unwrap().push(Action::Created(id, value));
                Self(log.clone(), id, value)
            }
        }
        impl Drop for Logging {
            fn drop(&mut self) {
                self.0.lock().unwrap().push(Action::Dropped(self.1, self.2))
            }
        }
        let log: Arc<Mutex<Vec<Action>>> = Arc::new(Mutex::new(vec![]));

        let a = <DedupArc<_>>::new(Logging::new(&log, 0, 0));
        let b = <DedupArc<_>>::new(Logging::new(&log, 1, 0));
        let c = b.clone();
        drop(a);
        drop(b);
        drop(c);

        println!("{:#?}", log.lock());
        // XXX do more, check this
    }
}
