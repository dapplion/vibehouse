/// This value is an application index of 0 with the bitmask applied (so it's equivalent to the bit mask).
/// Little endian hex: 0x00000001, Binary: 1000000000000000000000000
pub const APPLICATION_DOMAIN_BUILDER: u32 = 16777216;

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum ApplicationDomain {
    Builder,
}

impl ApplicationDomain {
    pub fn get_domain_constant(&self) -> u32 {
        match self {
            ApplicationDomain::Builder => APPLICATION_DOMAIN_BUILDER,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_domain_constant_value() {
        // APPLICATION_DOMAIN_BUILDER = 0x01000000 in little-endian = 16777216
        assert_eq!(APPLICATION_DOMAIN_BUILDER, 16777216);
        assert_eq!(APPLICATION_DOMAIN_BUILDER, 1u32 << 24);
    }

    #[test]
    fn get_domain_constant_builder() {
        let domain = ApplicationDomain::Builder;
        assert_eq!(domain.get_domain_constant(), APPLICATION_DOMAIN_BUILDER);
    }

    #[test]
    fn copy_and_eq() {
        let domain = ApplicationDomain::Builder;
        let copied: ApplicationDomain = domain;
        assert_eq!(domain, copied);
    }

    #[test]
    fn copy_semantics() {
        let domain = ApplicationDomain::Builder;
        let copied = domain;
        // Both still usable (Copy trait)
        assert_eq!(domain.get_domain_constant(), copied.get_domain_constant());
    }

    #[test]
    fn debug_format() {
        let domain = ApplicationDomain::Builder;
        let debug = format!("{:?}", domain);
        assert!(debug.contains("Builder"));
    }
}
