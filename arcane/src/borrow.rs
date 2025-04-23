use std::{marker::PhantomData, ops::Deref, ptr::NonNull};

use crate::{arc::Arc, ptr::ArcPtr};

#[repr(transparent)]
pub struct ArcBorrow<'a, T: ?Sized> {
    ptr: ArcPtr<T>,
    _phantom: PhantomData<&'a T>,
}

impl<'a, T: ?Sized> Copy for ArcBorrow<'a, T> {}
impl<'a, T: ?Sized> Clone for ArcBorrow<'a, T> {
    fn clone(&self) -> Self {
        *self
    }
}

// We require `T: Send` even for `ArcBorrow<T>: Sync` since an `ArcBorrow<T>` can be turned into an
// `Arc<T>`
unsafe impl<T: Sync + Send + ?Sized> Sync for ArcBorrow<'_, T> {}
unsafe impl<T: Sync + Send + ?Sized> Send for ArcBorrow<'_, T> {}

impl<T: ?Sized> Deref for ArcBorrow<'_, T> {
    type Target = Arc<T>;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        unsafe { NonNull::from(self).cast().as_ref() }
    }
}

impl<'a, T: ?Sized> From<&'a Arc<T>> for ArcBorrow<'a, T> {
    #[inline(always)]
    fn from(value: &'a Arc<T>) -> Self {
        Self {
            ptr: Arc::as_arc_ptr(value),
            _phantom: PhantomData,
        }
    }
}

impl<'a, T: ?Sized> ArcBorrow<'a, T> {
    #[inline(always)]
    pub fn clone_arc(this: Self) -> Arc<T> {
        let arc: &Arc<T> = &this;
        arc.clone()
    }

    #[inline(always)]
    pub unsafe fn from_arc_ptr(ptr: ArcPtr<T>) -> Self {
        Self { ptr, _phantom: PhantomData }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[test]
    fn test_arc_borrow() {
        #[derive(PartialEq, Eq, Debug)]
        enum Action {
            Created(usize),
            Dropped(usize),
        }
        use Action::*;

        struct Logging<'a>(&'a Mutex<Vec<Action>>, usize);

        impl<'a> Logging<'a> {
            pub fn new(log: &'a Mutex<Vec<Action>>, id: usize) -> Self {
                log.lock().unwrap().push(Action::Created(id));
                Self(log, id)
            }
        }

        impl<'a> Drop for Logging<'a> {
            fn drop(&mut self) {
                self.0.lock().unwrap().push(Action::Dropped(self.1))
            }
        }

        let log: Mutex<Vec<Action>> = Mutex::new(vec![]);

        let a = Arc::new(Logging::new(&log, 0));
        let a_borrow = ArcBorrow::from(&a);
        let b = Arc::new(Logging::new(&log, 1));
        let c: Arc<_> = ArcBorrow::clone_arc(a_borrow);
        drop(a);
        drop(b);
        drop(c);

        assert_eq!(
            log.into_inner().unwrap(),
            vec![Created(0), Created(1), Dropped(1), Dropped(0)]
        );
    }
}
