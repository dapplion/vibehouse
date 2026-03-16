use std::fmt;

pub const PURPOSE: u32 = 12381;
pub const COIN_TYPE: u32 = 3600;

pub enum KeyType {
    Voting,
    Withdrawal,
}

pub struct ValidatorPath(Vec<u32>);

impl ValidatorPath {
    pub fn new(index: u32, key_type: KeyType) -> Self {
        let mut vec = vec![PURPOSE, COIN_TYPE, index, 0];

        match key_type {
            KeyType::Voting => vec.push(0),
            KeyType::Withdrawal => {}
        }

        Self(vec)
    }

    pub fn iter_nodes(&self) -> impl Iterator<Item = &u32> {
        self.0.iter()
    }
}

impl fmt::Display for ValidatorPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m")?;

        for node in self.iter_nodes() {
            write!(f, "/{}", node)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voting_path_has_five_nodes() {
        let path = ValidatorPath::new(0, KeyType::Voting);
        let nodes: Vec<_> = path.iter_nodes().copied().collect();
        assert_eq!(nodes.len(), 5);
        assert_eq!(nodes, vec![PURPOSE, COIN_TYPE, 0, 0, 0]);
    }

    #[test]
    fn withdrawal_path_has_four_nodes() {
        let path = ValidatorPath::new(0, KeyType::Withdrawal);
        let nodes: Vec<_> = path.iter_nodes().copied().collect();
        assert_eq!(nodes.len(), 4);
        assert_eq!(nodes, vec![PURPOSE, COIN_TYPE, 0, 0]);
    }

    #[test]
    fn voting_display_index_zero() {
        let path = ValidatorPath::new(0, KeyType::Voting);
        assert_eq!(format!("{}", path), "m/12381/3600/0/0/0");
    }

    #[test]
    fn withdrawal_display_index_zero() {
        let path = ValidatorPath::new(0, KeyType::Withdrawal);
        assert_eq!(format!("{}", path), "m/12381/3600/0/0");
    }

    #[test]
    fn voting_display_nonzero_index() {
        let path = ValidatorPath::new(42, KeyType::Voting);
        assert_eq!(format!("{}", path), "m/12381/3600/42/0/0");
    }

    #[test]
    fn withdrawal_display_nonzero_index() {
        let path = ValidatorPath::new(42, KeyType::Withdrawal);
        assert_eq!(format!("{}", path), "m/12381/3600/42/0");
    }

    #[test]
    fn constants_match_eip2334() {
        assert_eq!(PURPOSE, 12381);
        assert_eq!(COIN_TYPE, 3600);
    }

    #[test]
    fn large_index() {
        let path = ValidatorPath::new(u32::MAX, KeyType::Voting);
        let nodes: Vec<_> = path.iter_nodes().copied().collect();
        assert_eq!(nodes[2], u32::MAX);
        assert_eq!(
            format!("{}", path),
            format!("m/12381/3600/{}/0/0", u32::MAX)
        );
    }
}
