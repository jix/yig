use std::{hash::Hash, mem::take, sync::atomic::AtomicUsize};

use arcane::{arc::UniqueArc, dedup::DedupArc};


const LEAF_BITS: u32 = 6;
const LEAF_SIZE: usize = 1 << LEAF_BITS;

const INNER_BITS: u32 = 6;
const INNER_SIZE: usize = 1 << INNER_BITS;

const INNER_MASK: usize = !0 << LEAF_BITS;
const LEAF_MASK: usize = !INNER_MASK;

const INNER_SLOT_MASK: usize = !(!0 << INNER_BITS);

const fn level_mask(level: u8) -> usize {
    !(INNER_MASK << level_shift(level))
}

const fn level_shift(level: u8) -> u32 {
    if level == 0 {
        0
    } else {
        (level - 1) as u32 * INNER_BITS + LEAF_BITS
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Leaf<T: Send + Sync + Hash + Eq + 'static> {
    len: usize,
    items: [Option<T>; LEAF_SIZE],
}

#[derive(PartialEq, Eq, Hash, Debug)]
struct SharedInner<T: Send + Sync + Hash + Eq + 'static> {
    len: usize,
    children: [Option<SharedNodeRef<T>>; INNER_SIZE],
}

#[derive(PartialEq, Eq, Hash, Debug)]
enum SharedNodeRef<T: Send + Sync + Hash + Eq + 'static> {
    Leaf(DedupArc<Leaf<T>>),
    Inner(DedupArc<SharedInner<T>>),
}

impl<T: Send + Sync + Hash + Eq + 'static> Clone for SharedNodeRef<T> {
    fn clone(&self) -> Self {
        match self {
            Self::Leaf(arg0) => Self::Leaf(arg0.clone()),
            Self::Inner(arg0) => Self::Inner(arg0.clone()),
        }
    }
}

impl<T: Send + Sync + Hash + Eq + 'static + Clone> SharedNodeRef<T> {
    pub fn len(&self) -> usize {
        match self {
            SharedNodeRef::Leaf(leaf) => leaf.len,
            SharedNodeRef::Inner(inner) => inner.len,
        }
    }

    pub fn get(&self, level: u8, index: usize) -> Option<&T> {
        match self {
            SharedNodeRef::Leaf(leaf) => {
                let slot = index & LEAF_MASK;
                leaf.items[slot].as_ref()
            }
            SharedNodeRef::Inner(inner) => {
                let slot = (index >> level_shift(level)) & INNER_SLOT_MASK;
                let child = inner.children[slot].as_ref()?;
                child.get(level - 1, index)
            }
        }
    }

    fn unshare(&self) -> OwnedNodeRef<T> {
        // TODO unique ref optimization?
        match self {
            SharedNodeRef::Leaf(leaf) => OwnedNodeRef::Leaf(UniqueArc::new((**leaf).clone())),
            SharedNodeRef::Inner(inner) => OwnedNodeRef::Inner(UniqueArc::new(OwnedInner {
                len: LazyLen(inner.len.into()),
                shared: DedupArcOnce::pending(),
                children: std::array::from_fn(|i| {
                    inner.children[i]
                        .as_ref()
                        .map(|child| OwnedNodeRef::Shared(child.clone()))
                }),
            })),
        }
    }
}

#[derive(Debug)]
pub struct LazyLen(AtomicUsize);

impl LazyLen {
    const fn unknown() -> Self {
        Self(AtomicUsize::new(usize::MAX))
    }

    pub fn set(&self, value: usize) {
        self.0.store(value, std::sync::atomic::Ordering::Relaxed)
    }

    pub fn get(&self) -> Option<usize> {
        let value = self.0.load(std::sync::atomic::Ordering::Relaxed);
        (value != usize::MAX).then_some(value)
    }
}

#[derive(Debug)]
struct OwnedInner<T: Send + Sync + Hash + Eq + 'static> {
    len: LazyLen,
    shared: DedupArcOnce<SharedInner<T>>,
    children: [Option<OwnedNodeRef<T>>; INNER_SIZE],
}

enum OwnedNodeRef<T: Send + Sync + Hash + Eq + 'static> {
    Shared(SharedNodeRef<T>),
    Leaf(UniqueArc<Leaf<T>>),
    Inner(UniqueArc<OwnedInner<T>>),
    Taken,
}

impl<T: Send + Sync + Hash + Eq + 'static + Clone> Default for OwnedNodeRef<T> {
    fn default() -> Self {
        Self::Taken
    }
}

#[derive(Debug)]
struct OwnedRoot<T: Send + Sync + Hash + Eq + 'static> {
    level: u8,
    prefix: usize,
    node: OwnedNodeRef<T>,
}

#[derive(Clone)]
pub struct OwnedTree<T: Send + Sync + Hash + Eq + 'static>(Option<OwnedRoot<T>>);

impl<T: Send + Sync + Hash + Eq + 'static + std::fmt::Debug> std::fmt::Debug for OwnedNodeRef<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Shared(shared) => f.debug_tuple("Shared").field(shared).finish(),
            Self::Leaf(leaf) => f.debug_tuple("Leaf").field(&**leaf).finish(),
            Self::Inner(inner) => f.debug_tuple("Inner").field(&**inner).finish(),
            Self::Taken => f.debug_tuple("Taken").finish(),
        }
    }
}

// impl<T: Dedup + Clone> Clone for OwnedTree<T> {

// }

impl<T: Send + Sync + Hash + Eq + 'static + Clone> OwnedTree<T> {
    pub const fn new() -> Self {
        Self(None)
    }

    pub fn share(&mut self) {
        if let Some(root) = &mut self.0 {
            root.node.share();
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    pub fn len(&self) -> usize {
        if let Some(root) = &self.0 {
            root.len()
        } else {
            0
        }
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        self.0.as_ref()?.get(index)
    }

    pub fn insert(&mut self, index: usize, value: T) -> Option<T> {
        if let Some(root) = &mut self.0 {
            root.insert(index, value)
        } else {
            self.0 = Some(OwnedRoot::new(index, value));
            None
        }
    }

    pub fn remove(&mut self, index: usize) -> Option<T> {
        if let Some(root) = &mut self.0 {
            let (result, cleared) = root.remove(index)?;
            if cleared {
                self.0 = None;
            }

            Some(result)
        } else {
            None
        }
    }
}

impl<T: Send + Sync + Hash + Eq + 'static + Clone> Default for OwnedTree<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Send + Sync + Hash + Eq + 'static + Clone> Clone for OwnedRoot<T> {
    fn clone(&self) -> Self {
        let weak_clone = self.node.share_weak();
        Self {
            level: self.level,
            prefix: self.prefix,
            node: OwnedNodeRef::Shared(weak_clone),
        }
    }
}

impl<T: Send + Sync + Hash + Eq + 'static + Clone> OwnedRoot<T> {
    pub fn new(index: usize, value: T) -> Self {
        let mut items = std::array::from_fn(|_| None);
        items[index & LEAF_MASK] = Some(value);

        Self {
            level: 0,
            prefix: index & INNER_MASK,
            node: OwnedNodeRef::Leaf(UniqueArc::new(Leaf { len: 1, items })),
        }
    }

    pub fn len(&self) -> usize {
        self.node.len()
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        let delta = (self.prefix ^ index) & !level_mask(self.level);
        if delta != 0 {
            None
        } else {
            self.node.get(self.level, index)
        }
    }

    pub fn insert(&mut self, index: usize, value: T) -> Option<T> {
        let delta = (self.prefix ^ index) & !level_mask(self.level);
        if delta != 0 {
            self.grow_for_level(delta);
        }
        self.node.insert(self.level, index, value)
    }

    pub fn remove(&mut self, index: usize) -> Option<(T, bool)> {
        let delta = (self.prefix ^ index) & !level_mask(self.level);
        if delta != 0 {
            None
        } else {
            let (result, isolated) = self.node.remove(self.level, index, true)?;

            if isolated {
                let cleared = self.shrink_levels();

                Some((result, cleared))
            } else {
                Some((result, false))
            }
        }
    }

    #[cold]
    fn grow_for_level(&mut self, mut delta: usize) {
        loop {
            self.level += 1;
            delta &= !level_mask(self.level);

            let slot = (self.prefix >> level_shift(self.level)) & INNER_SLOT_MASK;

            let mut inner = UniqueArc::new(OwnedInner {
                len: LazyLen::unknown(),
                shared: DedupArcOnce::pending(),
                children: std::array::from_fn(|_| None),
            });

            inner.children[slot] = Some(take(&mut self.node));

            self.node = OwnedNodeRef::Inner(inner);

            self.prefix &= level_mask(self.level);

            if delta == 0 {
                break;
            }
        }
    }

    #[cold]
    fn shrink_levels(&mut self) -> bool {
        'outer: loop {
            match &mut self.node {
                OwnedNodeRef::Shared(_) => todo!(),
                OwnedNodeRef::Leaf(leaf) => {
                    return !leaf.items.iter().any(|i| i.is_some());
                }
                OwnedNodeRef::Inner(inner) => {
                    for i in 0..inner.children.len() {
                        if inner.children[i].is_some() {
                            for j in i + 1..inner.children.len() {
                                if inner.children[j].is_some() {
                                    return false;
                                }
                            }
                            self.prefix |= i << level_shift(self.level);
                            self.level -= 1;
                            let child = inner.children[i].take().unwrap();
                            self.node = child;
                            continue 'outer;
                        }
                    }
                }
                OwnedNodeRef::Taken => unreachable!(),
            }
        }
    }
}

impl<T: Send + Sync + Hash + Eq + 'static + Clone> OwnedInner<T> {
    fn modify(&mut self) {
        self.len = LazyLen::unknown();
        self.shared = DedupArcOnce::pending();
    }
}

impl<T: Send + Sync + Hash + Eq + 'static + Clone> OwnedNodeRef<T> {
    pub fn new(level: u8) -> Self {
        if level == 0 {
            OwnedNodeRef::Leaf(UniqueArc::new(Leaf {
                len: 0,
                items: std::array::from_fn(|_| None),
            }))
        } else {
            OwnedNodeRef::Inner(UniqueArc::new(OwnedInner {
                len: LazyLen::unknown(),
                shared: DedupArcOnce::pending(),
                children: std::array::from_fn(|_| None),
            }))
        }
    }

    pub fn into_shared(self) -> SharedNodeRef<T> {
        match self {
            OwnedNodeRef::Shared(shared) => shared,
            OwnedNodeRef::Leaf(leaf) => SharedNodeRef::Leaf(DedupArc::from(leaf)),
            OwnedNodeRef::Inner(mut inner) => {
                if let Some(shared) = inner.shared.get() {
                    SharedNodeRef::Inner(shared.clone_arc())
                } else {
                    let mut len = 0;
                    SharedNodeRef::Inner(DedupArc::new(SharedInner {
                        children: std::array::from_fn(|i| {
                            inner.children[i].take().map(|child| {
                                let child = child.into_shared();
                                len += child.len();
                                child
                            })
                        }),
                        len,
                    }))
                }
            }
            OwnedNodeRef::Taken => unreachable!(),
        }
    }

    pub fn share(&mut self) -> &SharedNodeRef<T> {
        if let OwnedNodeRef::Shared(shared) = self {
            return shared;
        }

        let shared = take(self).into_shared();

        *self = OwnedNodeRef::Shared(shared);
        if let OwnedNodeRef::Shared(shared) = self {
            return shared;
        }
        unreachable!();
    }

    pub fn share_weak(&self) -> SharedNodeRef<T> {
        match self {
            OwnedNodeRef::Shared(shared) => shared.clone(),
            OwnedNodeRef::Leaf(leaf) => SharedNodeRef::Leaf(DedupArc::new((*leaf).clone())),
            OwnedNodeRef::Inner(inner) => {
                if let Some(gotten) = inner.shared.get() {
                    SharedNodeRef::Inner(gotten.clone_arc())
                } else {
                    let mut len = 0;
                    let shared = DedupArc::new(SharedInner {
                        children: std::array::from_fn(|i| {
                            inner.children[i].as_ref().map(|child| {
                                let child = child.share_weak();
                                len += child.len();
                                child
                            })
                        }),
                        len,
                    });
                    inner.shared.provide(shared.clone());
                    SharedNodeRef::Inner(shared)
                }
            }
            OwnedNodeRef::Taken => todo!(),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            OwnedNodeRef::Shared(shared) => shared.len(),
            OwnedNodeRef::Leaf(leaf) => leaf.len,
            OwnedNodeRef::Inner(inner) => {
                if let Some(len) = inner.len.get() {
                    return len;
                }

                let len = inner
                    .children
                    .iter()
                    .flatten()
                    .map(|child| child.len())
                    .sum();

                inner.len.set(len);

                len
            }
            OwnedNodeRef::Taken => todo!(),
        }
    }

    pub fn get(&self, level: u8, index: usize) -> Option<&T> {
        match self {
            OwnedNodeRef::Shared(shared) => shared.get(level, index),
            OwnedNodeRef::Leaf(leaf) => {
                let slot = index & LEAF_MASK;
                leaf.items[slot].as_ref()
            }
            OwnedNodeRef::Inner(inner) => {
                let slot = (index >> level_shift(level)) & INNER_SLOT_MASK;
                let child = inner.children[slot].as_ref()?;
                child.get(level - 1, index)
            }
            OwnedNodeRef::Taken => unreachable!(),
        }
    }

    pub fn insert(&mut self, level: u8, index: usize, value: T) -> Option<T> {
        match self {
            OwnedNodeRef::Shared(_) => {
                let OwnedNodeRef::Shared(shared) = take(self) else {
                    unreachable!()
                };
                *self = shared.unshare();
                self.insert(level, index, value)
            }
            OwnedNodeRef::Leaf(leaf) => {
                let slot = index & LEAF_MASK;
                let result = leaf.items[slot].replace(value);
                leaf.len += result.is_none() as usize;
                result
            }
            OwnedNodeRef::Inner(inner) => {
                inner.modify();
                let slot = (index >> level_shift(level)) & INNER_SLOT_MASK;
                let child = inner.children[slot].get_or_insert_with(|| Self::new(level - 1));
                child.insert(level - 1, index, value)
            }
            OwnedNodeRef::Taken => unreachable!(),
        }
    }

    pub fn remove(&mut self, level: u8, index: usize, top_level: bool) -> Option<(T, bool)> {
        match self {
            OwnedNodeRef::Shared(_) => {
                // TODO optimize misses and/or clearing singletons?
                let OwnedNodeRef::Shared(shared) = take(self) else {
                    unreachable!()
                };
                *self = shared.unshare();
                self.remove(level, index, top_level)
            }
            OwnedNodeRef::Leaf(leaf) => {
                let slot = index & LEAF_MASK;
                let result = leaf.items[slot].take()?;
                leaf.len -= 1;

                Some((result, leaf.len == 0))
            }
            OwnedNodeRef::Inner(inner) => {
                inner.modify();
                let slot = (index >> level_shift(level)) & INNER_SLOT_MASK;
                let child = inner.children[slot].as_mut()?;

                let (result, cleared) = child.remove(level - 1, index, false)?;

                if cleared {
                    inner.children[slot] = None;

                    let mut count_to_two = (!top_level) as usize;

                    for child in inner.children.iter() {
                        count_to_two += child.is_some() as usize;
                        if count_to_two == 2 {
                            return Some((result, false));
                        }
                    }

                    Some((result, true))
                } else {
                    Some((result, false))
                }
            }
            OwnedNodeRef::Taken => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use zwohash::HashMap;

    use super::*;

    fn memstats() {
        // unsafe { libc::malloc_trim(0) };
        // println!("{:?}", memory_stats::memory_stats());
    }

    const SCALE: usize = 1_000;
    const SCANS: usize = 1000;
    const MASK: usize = 0x3;

    #[test]
    fn testthis() {
        let mut foo: Vec<OwnedTree<usize>> = vec![OwnedTree::new()];

        for i in 0..SCALE {
            foo[0].insert(i, (i * i) ^ (7 * i));
        }

        let mut total = 0;
        let mut replacements = 0;

        for j in 0..SCANS {
            foo.push(foo[j].clone());
            let mut current = 0;

            for i in 0..SCALE {
                let next =
                    (foo[j + 1].get(current).copied().unwrap_or_default() + i * j) % SCALE;
                if (next ^ i) & MASK == 0 {
                    foo[j + 1].insert(current, (!next) % SCALE);
                    replacements += 1;
                }
                current = next;
            }

            total += current;
        }

        memstats();

        std::hint::black_box(&foo);
        println!("{total} {replacements}");
    }

    #[test]
    fn testvec() {
        let mut foo = vec![vec![]];

        for i in 0..SCALE {
            foo[0].insert(i, (i * i) ^ (7 * i));
        }

        let mut total = 0;
        let mut replacements = 0;

        for j in 0..SCANS {
            foo.push(foo[j].clone());
            let mut current = 0;

            for i in 0..SCALE {
                let next =
                    (foo[j + 1].get(current).copied().unwrap_or_default() + i * j) % SCALE;
                if next & MASK == 0 {
                    foo[j + 1][current] = (!next) % SCALE;
                    replacements += 1;
                }
                current = next;
            }

            total += current;
        }

        memstats();

        std::hint::black_box(&foo);
        println!("{total} {replacements}");
    }

    #[test]
    fn testbtreemap() {
        let mut foo = vec![BTreeMap::default()];

        for i in 0..SCALE {
            foo[0].insert(i, (i * i) ^ (7 * i));
        }

        let mut total = 0;
        let mut replacements = 0;

        for j in 0..SCANS {
            foo.push(foo[j].clone());
            let mut current = 0;

            for i in 0..SCALE {
                let next =
                    (foo[j + 1].get(&current).copied().unwrap_or_default() + i * j) % SCALE;
                if next & MASK == 0 {
                    foo[j + 1].insert(current, (!next) % SCALE);
                    replacements += 1;
                }
                current = next;
            }

            total += current;
        }

        memstats();

        std::hint::black_box(&foo);
        println!("{total} {replacements}");
    }

    #[test]
    fn testhashmap() {
        let mut foo = vec![HashMap::default()];

        for i in 0..SCALE {
            foo[0].insert(i, (i * i) ^ (7 * i));
        }

        let mut total = 0;
        let mut replacements = 0;

        for j in 0..SCANS {
            foo.push(foo[j].clone());
            let mut current = 0;

            for i in 0..SCALE {
                let next =
                    (foo[j + 1].get(&current).copied().unwrap_or_default() + i * j) % SCALE;
                if next & MASK == 0 {
                    foo[j + 1].insert(current, (!next) % SCALE);
                    replacements += 1;
                }
                current = next;
            }

            total += current;
        }

        memstats();

        std::hint::black_box(&foo);
        println!("{total} {replacements}");
    }
}
