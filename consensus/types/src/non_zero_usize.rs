use std::num::NonZeroUsize;

pub const fn new_non_zero_usize(x: usize) -> NonZeroUsize {
    match NonZeroUsize::new(x) {
        Some(n) => n,
        None => panic!("Expected a non zero usize."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_zero_usize_valid() {
        let n = new_non_zero_usize(1);
        assert_eq!(n.get(), 1);
    }

    #[test]
    fn non_zero_usize_large_value() {
        let n = new_non_zero_usize(usize::MAX);
        assert_eq!(n.get(), usize::MAX);
    }

    #[test]
    #[should_panic(expected = "Expected a non zero usize")]
    fn non_zero_usize_zero_panics() {
        let _ = new_non_zero_usize(0);
    }
}
