//! Emulates a fixed size array but with the length set at runtime.
//!
//! The length of the list cannot be changed once it is set.

use std::fmt;
use std::fmt::Debug;

#[derive(Clone)]
pub struct RuntimeFixedVector<T> {
    vec: Vec<T>,
    len: usize,
}

impl<T: Debug> Debug for RuntimeFixedVector<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} (len={})", self.vec, self.len)
    }
}

impl<T: Clone + Default> RuntimeFixedVector<T> {
    pub fn new(vec: Vec<T>) -> Self {
        let len = vec.len();
        Self { vec, len }
    }

    pub fn to_vec(&self) -> Vec<T> {
        self.vec.clone()
    }

    pub fn as_slice(&self) -> &[T] {
        self.vec.as_slice()
    }

    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn into_vec(self) -> Vec<T> {
        self.vec
    }

    pub fn default(max_len: usize) -> Self {
        Self {
            vec: vec![T::default(); max_len],
            len: max_len,
        }
    }

    pub fn take(&mut self) -> Self {
        let new = std::mem::take(&mut self.vec);
        *self = Self::new(vec![T::default(); self.len]);
        Self {
            vec: new,
            len: self.len,
        }
    }
}

impl<T> std::ops::Deref for RuntimeFixedVector<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &self.vec[..]
    }
}

impl<T> std::ops::DerefMut for RuntimeFixedVector<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.vec[..]
    }
}

impl<T> IntoIterator for RuntimeFixedVector<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.vec.into_iter()
    }
}

impl<'a, T> IntoIterator for &'a RuntimeFixedVector<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.vec.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_from_vec() {
        let v = RuntimeFixedVector::new(vec![1, 2, 3]);
        assert_eq!(v.len(), 3);
        assert_eq!(v.as_slice(), &[1, 2, 3]);
    }

    #[test]
    fn new_empty() {
        let v = RuntimeFixedVector::<u32>::new(vec![]);
        assert_eq!(v.len(), 0);
        assert!(v.as_slice().is_empty());
    }

    #[test]
    fn default_fills_with_defaults() {
        let v = RuntimeFixedVector::<u32>::default(4);
        assert_eq!(v.len(), 4);
        assert_eq!(v.as_slice(), &[0, 0, 0, 0]);
    }

    #[test]
    fn to_vec_clones() {
        let v = RuntimeFixedVector::new(vec![10, 20, 30]);
        let cloned = v.to_vec();
        assert_eq!(cloned, vec![10, 20, 30]);
        // Original still usable
        assert_eq!(v.len(), 3);
    }

    #[test]
    fn into_vec_moves() {
        let v = RuntimeFixedVector::new(vec![5, 6, 7]);
        let moved = v.into_vec();
        assert_eq!(moved, vec![5, 6, 7]);
    }

    #[test]
    fn deref_slice_access() {
        let v = RuntimeFixedVector::new(vec![1, 2, 3, 4]);
        assert_eq!(v[0], 1);
        assert_eq!(v[3], 4);
        assert_eq!(&v[1..3], &[2, 3]);
    }

    #[test]
    fn deref_mut_modification() {
        let mut v = RuntimeFixedVector::new(vec![0, 0, 0]);
        v[1] = 42;
        assert_eq!(v.as_slice(), &[0, 42, 0]);
    }

    #[test]
    fn take_replaces_with_defaults() {
        let mut v = RuntimeFixedVector::new(vec![1, 2, 3]);
        let taken = v.take();
        // taken has the original data
        assert_eq!(taken.as_slice(), &[1, 2, 3]);
        assert_eq!(taken.len(), 3);
        // v is now reset to defaults but same length
        assert_eq!(v.as_slice(), &[0, 0, 0]);
        assert_eq!(v.len(), 3);
    }

    #[test]
    fn into_iter_owned() {
        let v = RuntimeFixedVector::new(vec![10, 20, 30]);
        let collected: Vec<_> = v.into_iter().collect();
        assert_eq!(collected, vec![10, 20, 30]);
    }

    #[test]
    fn into_iter_ref() {
        let v = RuntimeFixedVector::new(vec![1, 2, 3]);
        let collected: Vec<_> = (&v).into_iter().copied().collect();
        assert_eq!(collected, vec![1, 2, 3]);
    }

    #[test]
    fn clone() {
        let v = RuntimeFixedVector::new(vec![7, 8, 9]);
        let cloned = v.clone();
        assert_eq!(cloned.as_slice(), v.as_slice());
        assert_eq!(cloned.len(), v.len());
    }

    #[test]
    fn debug_format() {
        let v = RuntimeFixedVector::new(vec![1, 2]);
        let s = format!("{:?}", v);
        assert!(s.contains("(len=2)"));
    }
}
