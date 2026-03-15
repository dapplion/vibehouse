//! Provides field-by-field comparisons for structs and vecs.
//!
//! Returns comparisons as data, without making assumptions about the desired equality (e.g.,
//! does not `panic!` on inequality).
//!
//! Note: `compare_fields_derive` requires `PartialEq` and `Debug` implementations.
//!
//! ## Example
//!
//! ```rust
//! use compare_fields::{CompareFields, Comparison, FieldComparison};
//! use compare_fields_derive::CompareFields;
//!
//! #[derive(PartialEq, Debug, CompareFields)]
//! pub struct Bar {
//!     a: u64,
//!     b: u16,
//!     #[compare_fields(as_slice)]
//!     c: Vec<Foo>
//! }
//!
//! #[derive(Clone, PartialEq, Debug, CompareFields)]
//! pub struct Foo {
//!     d: String
//! }
//!
//! let cat = Foo {d: "cat".to_string()};
//! let dog = Foo {d: "dog".to_string()};
//! let chicken = Foo {d: "chicken".to_string()};
//!
//! let mut bar_a = Bar {
//!     a: 42,
//!     b: 12,
//!     c: vec![ cat.clone(), dog.clone() ],
//! };
//!
//! let mut bar_b = Bar {
//!     a: 42,
//!     b: 99,
//!     c: vec![ chicken.clone(), dog.clone()]
//! };
//!
//! let cat_dog = Comparison::Child(FieldComparison {
//!     field_name: "d".to_string(),
//!     equal: false,
//!     a: "\"cat\"".to_string(),
//!     b: "\"dog\"".to_string(),
//! });
//! assert_eq!(cat.compare_fields(&dog), vec![cat_dog]);
//!
//! let bar_a_b = vec![
//!     Comparison::Child(FieldComparison {
//!         field_name: "a".to_string(),
//!         equal: true,
//!         a: "42".to_string(),
//!         b: "42".to_string(),
//!     }),
//!     Comparison::Child(FieldComparison {
//!         field_name: "b".to_string(),
//!         equal: false,
//!         a: "12".to_string(),
//!         b: "99".to_string(),
//!     }),
//!     Comparison::Parent{
//!         field_name: "c".to_string(),
//!         equal: false,
//!         children: vec![
//!             FieldComparison {
//!                 field_name: "0".to_string(),
//!                 equal: false,
//!                 a: "Some(Foo { d: \"cat\" })".to_string(),
//!                 b: "Some(Foo { d: \"chicken\" })".to_string(),
//!             },
//!             FieldComparison {
//!                 field_name: "1".to_string(),
//!                 equal: true,
//!                 a: "Some(Foo { d: \"dog\" })".to_string(),
//!                 b: "Some(Foo { d: \"dog\" })".to_string(),
//!             }
//!         ]
//!     }
//! ];
//! assert_eq!(bar_a.compare_fields(&bar_b), bar_a_b);
//! ```
use itertools::{EitherOrBoth, Itertools};
use std::fmt::Debug;

#[derive(Debug, PartialEq, Clone)]
pub enum Comparison {
    Child(FieldComparison),
    Parent {
        field_name: String,
        equal: bool,
        children: Vec<FieldComparison>,
    },
}

impl Comparison {
    pub fn child<T: Debug + PartialEq<T>>(field_name: String, a: &T, b: &T) -> Self {
        Comparison::Child(FieldComparison::new(field_name, a, b))
    }

    pub fn parent(field_name: String, equal: bool, children: Vec<FieldComparison>) -> Self {
        Comparison::Parent {
            field_name,
            equal,
            children,
        }
    }

    pub fn from_slice<T: Debug + PartialEq<T>>(field_name: String, a: &[T], b: &[T]) -> Self {
        Self::from_iter(field_name, a.iter(), b.iter())
    }

    pub fn from_into_iter<'a, T: Debug + PartialEq + 'a>(
        field_name: String,
        a: impl IntoIterator<Item = &'a T>,
        b: impl IntoIterator<Item = &'a T>,
    ) -> Self {
        Self::from_iter(field_name, a.into_iter(), b.into_iter())
    }

    pub fn from_iter<'a, T: Debug + PartialEq + 'a>(
        field_name: String,
        a: impl Iterator<Item = &'a T>,
        b: impl Iterator<Item = &'a T>,
    ) -> Self {
        let mut children = vec![];
        let mut all_equal = true;

        for (i, entry) in a.zip_longest(b).enumerate() {
            let comparison = match entry {
                EitherOrBoth::Both(x, y) => {
                    FieldComparison::new(format!("{i}"), &Some(x), &Some(y))
                }
                EitherOrBoth::Left(x) => FieldComparison::new(format!("{i}"), &Some(x), &None),
                EitherOrBoth::Right(y) => FieldComparison::new(format!("{i}"), &None, &Some(y)),
            };
            all_equal = all_equal && comparison.equal();
            children.push(comparison);
        }

        Self::parent(field_name, all_equal, children)
    }

    pub fn retain_children<F>(&mut self, f: F)
    where
        F: FnMut(&FieldComparison) -> bool,
    {
        match self {
            Comparison::Child(_) => (),
            Comparison::Parent { children, .. } => children.retain(f),
        }
    }

    pub fn equal(&self) -> bool {
        match self {
            Comparison::Child(fc) => fc.equal,
            Comparison::Parent { equal, .. } => *equal,
        }
    }

    pub fn not_equal(&self) -> bool {
        !self.equal()
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct FieldComparison {
    pub field_name: String,
    pub equal: bool,
    pub a: String,
    pub b: String,
}

pub trait CompareFields {
    fn compare_fields(&self, b: &Self) -> Vec<Comparison>;
}

impl FieldComparison {
    pub fn new<T: Debug + PartialEq<T>>(field_name: String, a: &T, b: &T) -> Self {
        Self {
            field_name,
            equal: a == b,
            a: format!("{a:?}"),
            b: format!("{b:?}"),
        }
    }

    pub fn equal(&self) -> bool {
        self.equal
    }

    pub fn not_equal(&self) -> bool {
        !self.equal()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── FieldComparison ──────────────────────────────────────────

    #[test]
    fn field_comparison_equal_values() {
        let fc = FieldComparison::new("x".to_string(), &42u64, &42u64);
        assert!(fc.equal());
        assert!(!fc.not_equal());
        assert_eq!(fc.field_name, "x");
        assert_eq!(fc.a, "42");
        assert_eq!(fc.b, "42");
    }

    #[test]
    fn field_comparison_unequal_values() {
        let fc = FieldComparison::new("y".to_string(), &1u32, &2u32);
        assert!(!fc.equal());
        assert!(fc.not_equal());
        assert_eq!(fc.a, "1");
        assert_eq!(fc.b, "2");
    }

    #[test]
    fn field_comparison_string_debug_format() {
        let fc = FieldComparison::new("name".to_string(), &"hello", &"world");
        assert!(!fc.equal());
        assert_eq!(fc.a, "\"hello\"");
        assert_eq!(fc.b, "\"world\"");
    }

    // ── Comparison::child ────────────────────────────────────────

    #[test]
    fn comparison_child_equal() {
        let c = Comparison::child("field".to_string(), &10u64, &10u64);
        assert!(c.equal());
        assert!(!c.not_equal());
        match &c {
            Comparison::Child(fc) => assert_eq!(fc.field_name, "field"),
            _ => panic!("expected Child variant"),
        }
    }

    #[test]
    fn comparison_child_unequal() {
        let c = Comparison::child("f".to_string(), &1u64, &2u64);
        assert!(!c.equal());
        assert!(c.not_equal());
    }

    // ── Comparison::parent ───────────────────────────────────────

    #[test]
    fn comparison_parent_equal_empty_children() {
        let c = Comparison::parent("p".to_string(), true, vec![]);
        assert!(c.equal());
    }

    #[test]
    fn comparison_parent_not_equal() {
        let children = vec![FieldComparison::new("0".to_string(), &1, &2)];
        let c = Comparison::parent("p".to_string(), false, children);
        assert!(!c.equal());
        assert!(c.not_equal());
    }

    // ── Comparison::from_slice ───────────────────────────────────

    #[test]
    fn from_slice_equal() {
        let a = vec![1, 2, 3];
        let b = vec![1, 2, 3];
        let c = Comparison::from_slice("nums".to_string(), &a, &b);
        assert!(c.equal());
        match &c {
            Comparison::Parent { children, .. } => assert_eq!(children.len(), 3),
            _ => panic!("expected Parent"),
        }
    }

    #[test]
    fn from_slice_unequal_middle_element() {
        let a = vec![1, 2, 3];
        let b = vec![1, 9, 3];
        let c = Comparison::from_slice("nums".to_string(), &a, &b);
        assert!(!c.equal());
        match &c {
            Comparison::Parent { children, .. } => {
                assert!(children[0].equal());
                assert!(!children[1].equal());
                assert!(children[2].equal());
            }
            _ => panic!("expected Parent"),
        }
    }

    #[test]
    fn from_slice_right_longer() {
        let a = vec![1, 2];
        let b = vec![1, 2, 3];
        let c = Comparison::from_slice("nums".to_string(), &a, &b);
        assert!(!c.equal());
        match &c {
            Comparison::Parent { children, .. } => {
                assert_eq!(children.len(), 3);
                assert!(children[0].equal());
                assert!(children[1].equal());
                assert!(!children[2].equal()); // None vs Some(3)
            }
            _ => panic!("expected Parent"),
        }
    }

    #[test]
    fn from_slice_left_longer() {
        let a = vec![10, 20, 30];
        let b = vec![10];
        let c = Comparison::from_slice("v".to_string(), &a, &b);
        assert!(!c.equal());
        match &c {
            Comparison::Parent { children, .. } => {
                assert_eq!(children.len(), 3);
                assert!(children[0].equal());
                assert!(!children[1].equal());
                assert!(!children[2].equal());
            }
            _ => panic!("expected Parent"),
        }
    }

    #[test]
    fn from_slice_both_empty() {
        let a: Vec<u32> = vec![];
        let b: Vec<u32> = vec![];
        let c = Comparison::from_slice("empty".to_string(), &a, &b);
        assert!(c.equal());
        match &c {
            Comparison::Parent { children, .. } => assert!(children.is_empty()),
            _ => panic!("expected Parent"),
        }
    }

    // ── retain_children ──────────────────────────────────────────

    #[test]
    fn retain_children_filters_equal() {
        let a = vec![1, 2, 3];
        let b = vec![1, 9, 3];
        let mut c = Comparison::from_slice("v".to_string(), &a, &b);
        c.retain_children(|fc| fc.not_equal());
        match &c {
            Comparison::Parent { children, .. } => {
                assert_eq!(children.len(), 1);
                assert_eq!(children[0].field_name, "1");
            }
            _ => panic!("expected Parent"),
        }
    }

    #[test]
    fn retain_children_on_child_is_noop() {
        let mut c = Comparison::child("f".to_string(), &1, &2);
        c.retain_children(|_| false); // should not panic
        assert!(!c.equal()); // unchanged
    }

    // ── from_into_iter ───────────────────────────────────────────

    #[test]
    fn from_into_iter_equal() {
        let a = vec![100, 200];
        let b = vec![100, 200];
        let c = Comparison::from_into_iter("it".to_string(), &a, &b);
        assert!(c.equal());
    }

    #[test]
    fn from_into_iter_unequal() {
        let a = vec![100, 200];
        let b = vec![100, 999];
        let c = Comparison::from_into_iter("it".to_string(), &a, &b);
        assert!(!c.equal());
    }

    // ── Clone / PartialEq ────────────────────────────────────────

    #[test]
    fn comparison_clone_and_eq() {
        let c = Comparison::child("f".to_string(), &42, &42);
        let c2 = c.clone();
        assert_eq!(c, c2);
    }

    #[test]
    fn field_comparison_clone_and_eq() {
        let fc = FieldComparison::new("a".to_string(), &1, &1);
        let fc2 = fc.clone();
        assert_eq!(fc, fc2);
    }
}
