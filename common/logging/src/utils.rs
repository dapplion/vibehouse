use std::collections::HashSet;
use tracing_subscriber::filter::FilterFn;
use workspace_members::workspace_crates;

const WORKSPACE_CRATES: &[&str] = workspace_crates!();

/// Constructs a filter which only permits logging from crates which are members of the workspace.
pub fn build_workspace_filter()
-> Result<FilterFn<impl Fn(&tracing::Metadata) -> bool + Clone>, String> {
    let workspace_crates: HashSet<&str> = WORKSPACE_CRATES.iter().copied().collect();

    Ok(tracing_subscriber::filter::FilterFn::new(move |metadata| {
        let target_crate = metadata.target().split("::").next().unwrap_or("");
        workspace_crates.contains(target_crate)
    }))
}

/// Function to filter out ascii control codes.
///
/// This helps to keep log formatting consistent.
/// Whitespace and padding control codes are excluded.
pub fn is_ascii_control(character: &u8) -> bool {
    matches!(
        character,
        b'\x00'..=b'\x08' |
        b'\x0b'..=b'\x0c' |
        b'\x0e'..=b'\x1f' |
        b'\x7f' |
        b'\x81'..=b'\x9f'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_byte_is_control() {
        assert!(is_ascii_control(&0x00));
    }

    #[test]
    fn bell_is_control() {
        assert!(is_ascii_control(&0x07));
    }

    #[test]
    fn backspace_is_control() {
        assert!(is_ascii_control(&0x08));
    }

    #[test]
    fn tab_is_not_control() {
        assert!(!is_ascii_control(&b'\t'));
    }

    #[test]
    fn newline_is_not_control() {
        assert!(!is_ascii_control(&b'\n'));
    }

    #[test]
    fn vertical_tab_is_control() {
        assert!(is_ascii_control(&0x0b));
    }

    #[test]
    fn form_feed_is_control() {
        assert!(is_ascii_control(&0x0c));
    }

    #[test]
    fn carriage_return_is_not_control() {
        assert!(!is_ascii_control(&b'\r'));
    }

    #[test]
    fn escape_is_control() {
        assert!(is_ascii_control(&0x1b));
    }

    #[test]
    fn del_is_control() {
        assert!(is_ascii_control(&0x7f));
    }

    #[test]
    fn space_is_not_control() {
        assert!(!is_ascii_control(&b' '));
    }

    #[test]
    fn printable_ascii_not_control() {
        for c in b'A'..=b'z' {
            assert!(!is_ascii_control(&c), "0x{c:02x} should not be control");
        }
    }

    #[test]
    fn digits_not_control() {
        for c in b'0'..=b'9' {
            assert!(!is_ascii_control(&c));
        }
    }

    #[test]
    fn high_control_chars_0x81_to_0x9f() {
        for c in 0x81..=0x9f {
            assert!(is_ascii_control(&c), "0x{c:02x} should be control");
        }
    }

    #[test]
    fn byte_0x80_is_not_control() {
        assert!(!is_ascii_control(&0x80));
    }

    #[test]
    fn bytes_0xa0_and_above_not_control() {
        for c in 0xa0..=0xff_u8 {
            assert!(!is_ascii_control(&c), "0x{c:02x} should not be control");
        }
    }
}
