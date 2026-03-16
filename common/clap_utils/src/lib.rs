//! A helper library for parsing values from `clap::ArgMatches`.

use clap::ArgMatches;
use clap::builder::styling::*;
use eth2_network_config::{DEFAULT_HARDCODED_NETWORK, Eth2NetworkConfig};
use ssz::Decode;
use std::path::PathBuf;
use std::str::FromStr;
use types::{ChainSpec, Config, EthSpec};

pub mod flags;

pub const BAD_TESTNET_DIR_MESSAGE: &str = "The hard-coded testnet directory was invalid. \
                                        This happens when Vibehouse is migrating between spec versions \
                                        or when there is no default public network to connect to. \
                                        During these times you must specify a --testnet-dir.";

pub const FLAG_HEADER: &str = "Flags";

/// Try to parse the eth2 network config from the `network`, `testnet-dir` flags in that order.
/// Returns the default hardcoded testnet if neither flags are set.
pub fn get_eth2_network_config(cli_args: &ArgMatches) -> Result<Eth2NetworkConfig, String> {
    let optional_network_config = if cli_args.contains_id("network") {
        parse_hardcoded_network(cli_args, "network")?
    } else if cli_args.contains_id("testnet-dir") {
        parse_testnet_dir(cli_args, "testnet-dir")?
    } else {
        // if neither is present, assume the default network
        Eth2NetworkConfig::constant(DEFAULT_HARDCODED_NETWORK)?
    };

    let eth2_network_config =
        optional_network_config.ok_or_else(|| BAD_TESTNET_DIR_MESSAGE.to_string())?;

    Ok(eth2_network_config)
}

/// Attempts to load the testnet dir at the path if `name` is in `matches`, returning an error if
/// the path cannot be found or the testnet dir is invalid.
pub fn parse_testnet_dir(
    matches: &ArgMatches,
    name: &'static str,
) -> Result<Option<Eth2NetworkConfig>, String> {
    let path = parse_required::<PathBuf>(matches, name)?;
    Eth2NetworkConfig::load(path.clone())
        .map_err(|e| format!("Unable to open testnet dir at {:?}: {}", path, e))
        .map(Some)
}

/// Attempts to load a hardcoded network config if `name` is in `matches`, returning an error if
/// the name is not a valid network name.
pub fn parse_hardcoded_network(
    matches: &ArgMatches,
    name: &str,
) -> Result<Option<Eth2NetworkConfig>, String> {
    let network_name = parse_required::<String>(matches, name)?;
    Eth2NetworkConfig::constant(network_name.as_str())
}

/// If `name` is in `matches`, parses the value as a path. Otherwise, attempts to find the user's
/// home directory and appends `default` to it.
pub fn parse_path_with_default_in_home_dir(
    matches: &ArgMatches,
    name: &'static str,
    default: PathBuf,
) -> Result<PathBuf, String> {
    matches
        .get_one::<String>(name)
        .map(|dir| {
            dir.parse::<PathBuf>()
                .map_err(|e| format!("Unable to parse {}: {}", name, e))
        })
        .unwrap_or_else(|| {
            dirs::home_dir()
                .map(|home| home.join(default))
                .ok_or_else(|| format!("Unable to locate home directory. Try specifying {}", name))
        })
}

/// Returns the value of `name` or an error if it is not in `matches` or does not parse
/// successfully using `std::string::FromStr`.
pub fn parse_required<T>(matches: &ArgMatches, name: &str) -> Result<T, String>
where
    T: FromStr,
    <T as FromStr>::Err: std::fmt::Display,
{
    parse_optional(matches, name)?.ok_or_else(|| format!("{} not specified", name))
}

/// Returns the value of `name` (if present) or an error if it does not parse successfully using
/// `std::string::FromStr`.
pub fn parse_optional<T>(matches: &ArgMatches, name: &str) -> Result<Option<T>, String>
where
    T: FromStr,
    <T as FromStr>::Err: std::fmt::Display,
{
    matches
        .try_get_one::<String>(name)
        .map_err(|e| format!("Unable to parse {}: {}", name, e))?
        .map(|val| {
            val.parse()
                .map_err(|e| format!("Unable to parse {}: {}", name, e))
        })
        .transpose()
}

/// Returns the value of `name` or an error if it is not in `matches` or does not parse
/// successfully using `ssz::Decode`.
///
/// Expects the value of `name` to be 0x-prefixed ASCII-hex.
pub fn parse_ssz_required<T: Decode>(
    matches: &ArgMatches,
    name: &'static str,
) -> Result<T, String> {
    parse_ssz_optional(matches, name)?.ok_or_else(|| format!("{} not specified", name))
}

/// Returns the value of `name` (if present) or an error if it does not parse successfully using
/// `ssz::Decode`.
///
/// Expects the value of `name` (if any) to be 0x-prefixed ASCII-hex.
pub fn parse_ssz_optional<T: Decode>(
    matches: &ArgMatches,
    name: &'static str,
) -> Result<Option<T>, String> {
    matches
        .get_one::<String>(name)
        .map(|val| {
            if let Some(stripped) = val.strip_prefix("0x") {
                let vec = hex::decode(stripped)
                    .map_err(|e| format!("Unable to parse {} as hex: {:?}", name, e))?;

                T::from_ssz_bytes(&vec)
                    .map_err(|e| format!("Unable to parse {} as SSZ: {:?}", name, e))
            } else {
                Err(format!("Unable to parse {}, must have 0x prefix", name))
            }
        })
        .transpose()
}

/// Writes configs to file if `dump-config` or `dump-chain-config` flags are set
pub fn check_dump_configs<S, E>(
    matches: &ArgMatches,
    config: S,
    spec: &ChainSpec,
) -> Result<(), String>
where
    S: serde::Serialize,
    E: EthSpec,
{
    if let Some(dump_path) = parse_optional::<PathBuf>(matches, "dump-config")? {
        let mut file = std::fs::File::create(dump_path)
            .map_err(|e| format!("Failed to open file for writing config: {:?}", e))?;
        serde_json::to_writer(&mut file, &config)
            .map_err(|e| format!("Error serializing config: {:?}", e))?;
    }
    if let Some(dump_path) = parse_optional::<PathBuf>(matches, "dump-chain-config")? {
        let chain_config = Config::from_chain_spec::<E>(spec);
        let mut file = std::fs::File::create(dump_path)
            .map_err(|e| format!("Failed to open file for writing chain config: {:?}", e))?;
        serde_yaml::to_writer(&mut file, &chain_config)
            .map_err(|e| format!("Error serializing config: {:?}", e))?;
    }
    Ok(())
}

pub fn get_color_style() -> Styles {
    Styles::styled()
        .header(AnsiColor::Yellow.on_default())
        .usage(AnsiColor::Green.on_default())
        .literal(AnsiColor::Green.on_default())
        .placeholder(AnsiColor::Green.on_default())
}

pub fn parse_flag(matches: &ArgMatches, name: &str) -> bool {
    *matches.get_one::<bool>(name).unwrap_or(&false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{Arg, Command};

    /// Helper to build ArgMatches from a list of args for a command with given arg definitions.
    fn make_matches(args: &[Arg], values: &[&str]) -> ArgMatches {
        let mut cmd = Command::new("test");
        for arg in args {
            cmd = cmd.arg(arg.clone());
        }
        let mut all_args = vec!["test"];
        all_args.extend_from_slice(values);
        cmd.try_get_matches_from(all_args).unwrap()
    }

    // --- parse_optional ---

    #[test]
    fn parse_optional_present_valid() {
        let matches = make_matches(&[Arg::new("port").long("port")], &["--port", "8080"]);
        let result: Result<Option<u16>, _> = parse_optional(&matches, "port");
        assert_eq!(result.unwrap(), Some(8080));
    }

    #[test]
    fn parse_optional_absent() {
        let matches = make_matches(&[Arg::new("port").long("port")], &[]);
        let result: Result<Option<u16>, _> = parse_optional(&matches, "port");
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn parse_optional_invalid_value() {
        let matches = make_matches(
            &[Arg::new("port").long("port")],
            &["--port", "not_a_number"],
        );
        let result: Result<Option<u16>, _> = parse_optional(&matches, "port");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unable to parse"));
    }

    #[test]
    fn parse_optional_string_type() {
        let matches = make_matches(&[Arg::new("name").long("name")], &["--name", "hello"]);
        let result: Result<Option<String>, _> = parse_optional(&matches, "name");
        assert_eq!(result.unwrap(), Some("hello".to_string()));
    }

    #[test]
    fn parse_optional_pathbuf() {
        let matches = make_matches(&[Arg::new("dir").long("dir")], &["--dir", "/tmp/foo"]);
        let result: Result<Option<PathBuf>, _> = parse_optional(&matches, "dir");
        assert_eq!(result.unwrap(), Some(PathBuf::from("/tmp/foo")));
    }

    // --- parse_required ---

    #[test]
    fn parse_required_present_valid() {
        let matches = make_matches(&[Arg::new("count").long("count")], &["--count", "42"]);
        let result: Result<u64, _> = parse_required(&matches, "count");
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn parse_required_absent() {
        let matches = make_matches(&[Arg::new("count").long("count")], &[]);
        let result: Result<u64, _> = parse_required(&matches, "count");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not specified"));
    }

    #[test]
    fn parse_required_invalid_value() {
        let matches = make_matches(&[Arg::new("count").long("count")], &["--count", "abc"]);
        let result: Result<u64, _> = parse_required(&matches, "count");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unable to parse"));
    }

    // --- parse_ssz_optional ---

    #[test]
    fn parse_ssz_optional_absent() {
        let matches = make_matches(&[Arg::new("data").long("data")], &[]);
        let result: Result<Option<u64>, _> = parse_ssz_optional(&matches, "data");
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn parse_ssz_optional_valid_u64() {
        // SSZ encoding of u64 42 is 8 bytes little-endian: 2a00000000000000
        let matches = make_matches(
            &[Arg::new("data").long("data")],
            &["--data", "0x2a00000000000000"],
        );
        let result: Result<Option<u64>, _> = parse_ssz_optional(&matches, "data");
        assert_eq!(result.unwrap(), Some(42));
    }

    #[test]
    fn parse_ssz_optional_no_0x_prefix() {
        let matches = make_matches(
            &[Arg::new("data").long("data")],
            &["--data", "2a00000000000000"],
        );
        let result: Result<Option<u64>, _> = parse_ssz_optional(&matches, "data");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must have 0x prefix"));
    }

    #[test]
    fn parse_ssz_optional_invalid_hex() {
        let matches = make_matches(&[Arg::new("data").long("data")], &["--data", "0xZZZZ"]);
        let result: Result<Option<u64>, _> = parse_ssz_optional(&matches, "data");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("hex"));
    }

    #[test]
    fn parse_ssz_optional_invalid_ssz_length() {
        // Too short for u64 (only 2 bytes)
        let matches = make_matches(&[Arg::new("data").long("data")], &["--data", "0x2a00"]);
        let result: Result<Option<u64>, _> = parse_ssz_optional(&matches, "data");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("SSZ"));
    }

    // --- parse_ssz_required ---

    #[test]
    fn parse_ssz_required_present_valid() {
        let matches = make_matches(
            &[Arg::new("data").long("data")],
            &["--data", "0x0100000000000000"],
        );
        let result: Result<u64, _> = parse_ssz_required(&matches, "data");
        assert_eq!(result.unwrap(), 1);
    }

    #[test]
    fn parse_ssz_required_absent() {
        let matches = make_matches(&[Arg::new("data").long("data")], &[]);
        let result: Result<u64, _> = parse_ssz_required(&matches, "data");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not specified"));
    }

    // --- parse_flag ---

    #[test]
    fn parse_flag_present() {
        let matches = make_matches(
            &[Arg::new("verbose")
                .long("verbose")
                .action(clap::ArgAction::SetTrue)],
            &["--verbose"],
        );
        assert!(parse_flag(&matches, "verbose"));
    }

    #[test]
    fn parse_flag_absent() {
        let matches = make_matches(
            &[Arg::new("verbose")
                .long("verbose")
                .action(clap::ArgAction::SetTrue)],
            &[],
        );
        assert!(!parse_flag(&matches, "verbose"));
    }

    // --- parse_path_with_default_in_home_dir ---

    #[test]
    fn parse_path_with_default_explicit_path() {
        let matches = make_matches(
            &[Arg::new("datadir").long("datadir")],
            &["--datadir", "/custom/path"],
        );
        let result =
            parse_path_with_default_in_home_dir(&matches, "datadir", PathBuf::from(".vibehouse"));
        assert_eq!(result.unwrap(), PathBuf::from("/custom/path"));
    }

    #[test]
    fn parse_path_with_default_uses_home_dir() {
        let matches = make_matches(&[Arg::new("datadir").long("datadir")], &[]);
        let result =
            parse_path_with_default_in_home_dir(&matches, "datadir", PathBuf::from(".vibehouse"));
        // Should succeed and end with .vibehouse (home dir joined with default)
        let path = result.unwrap();
        assert!(path.ends_with(".vibehouse"));
        // Should be an absolute path (starts with home dir)
        assert!(path.is_absolute());
    }

    // --- get_color_style ---

    #[test]
    fn get_color_style_does_not_panic() {
        let _style = get_color_style();
    }

    // --- constants ---

    #[test]
    fn bad_testnet_dir_message_is_not_empty() {
        assert!(!BAD_TESTNET_DIR_MESSAGE.is_empty());
    }

    #[test]
    fn flag_header_constant() {
        assert_eq!(FLAG_HEADER, "Flags");
    }

    // --- flags module ---

    #[test]
    fn disable_malloc_tuning_flag_constant() {
        assert_eq!(flags::DISABLE_MALLOC_TUNING_FLAG, "disable-malloc-tuning");
    }

    // --- parse_optional edge cases ---

    #[test]
    fn parse_optional_bool_true() {
        let matches = make_matches(&[Arg::new("flag").long("flag")], &["--flag", "true"]);
        let result: Result<Option<bool>, _> = parse_optional(&matches, "flag");
        assert_eq!(result.unwrap(), Some(true));
    }

    #[test]
    fn parse_optional_bool_false() {
        let matches = make_matches(&[Arg::new("flag").long("flag")], &["--flag", "false"]);
        let result: Result<Option<bool>, _> = parse_optional(&matches, "flag");
        assert_eq!(result.unwrap(), Some(false));
    }

    #[test]
    fn parse_optional_negative_number() {
        let matches = make_matches(
            &[Arg::new("val").long("val").allow_hyphen_values(true)],
            &["--val", "-42"],
        );
        let result: Result<Option<i64>, _> = parse_optional(&matches, "val");
        assert_eq!(result.unwrap(), Some(-42));
    }

    #[test]
    fn parse_optional_zero() {
        let matches = make_matches(&[Arg::new("val").long("val")], &["--val", "0"]);
        let result: Result<Option<u64>, _> = parse_optional(&matches, "val");
        assert_eq!(result.unwrap(), Some(0));
    }

    #[test]
    fn parse_optional_float() {
        let matches = make_matches(&[Arg::new("val").long("val")], &["--val", "2.5"]);
        let result: Result<Option<f64>, _> = parse_optional(&matches, "val");
        let val = result.unwrap().unwrap();
        assert!((val - 2.5).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_optional_empty_string() {
        let matches = make_matches(&[Arg::new("val").long("val")], &["--val", ""]);
        let result: Result<Option<String>, _> = parse_optional(&matches, "val");
        assert_eq!(result.unwrap(), Some(String::new()));
    }

    #[test]
    fn parse_required_overflow_u8() {
        let matches = make_matches(&[Arg::new("val").long("val")], &["--val", "256"]);
        let result: Result<u8, _> = parse_required(&matches, "val");
        assert!(result.is_err());
    }

    // --- parse_ssz edge cases ---

    #[test]
    fn parse_ssz_optional_empty_0x() {
        // "0x" with no hex data — for u64 this should fail SSZ decode (0 bytes)
        let matches = make_matches(&[Arg::new("data").long("data")], &["--data", "0x"]);
        let result: Result<Option<u64>, _> = parse_ssz_optional(&matches, "data");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("SSZ"));
    }

    #[test]
    fn parse_ssz_optional_max_u64() {
        // SSZ encoding of u64::MAX = ffffffffffffffff
        let matches = make_matches(
            &[Arg::new("data").long("data")],
            &["--data", "0xffffffffffffffff"],
        );
        let result: Result<Option<u64>, _> = parse_ssz_optional(&matches, "data");
        assert_eq!(result.unwrap(), Some(u64::MAX));
    }

    #[test]
    fn parse_ssz_optional_zero_u64() {
        let matches = make_matches(
            &[Arg::new("data").long("data")],
            &["--data", "0x0000000000000000"],
        );
        let result: Result<Option<u64>, _> = parse_ssz_optional(&matches, "data");
        assert_eq!(result.unwrap(), Some(0));
    }
}
