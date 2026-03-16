use filesystem::create_with_600_perms;
use rand::distr::Alphanumeric;
use rand::{Rng, rng};
use std::fs;
use std::path::{Path, PathBuf};

/// The default name of the file which stores the API token.
pub const PK_FILENAME: &str = "api-token.txt";

pub const PK_LEN: usize = 33;

/// Contains a randomly generated string which is used for authorization of requests to the HTTP API.
///
/// Provides convenience functions to ultimately provide:
///
///  - Verification of proof-of-knowledge of the public key in `self` for incoming HTTP requests,
///    via the `Authorization` header.
///
///  This scheme has been simplified to remove VC response signing and secp256k1 key generation.
pub struct ApiSecret {
    pk: String,
    pk_path: PathBuf,
}

impl ApiSecret {
    /// If the public key is already on-disk, use it.
    ///
    /// The provided `pk_path` is a path containing API token.
    ///
    /// If the public key file is missing on disk, create a new key and
    /// write it to disk (over-writing any existing files).
    pub fn create_or_open<P: AsRef<Path>>(pk_path: P) -> Result<Self, String> {
        let pk_path = pk_path.as_ref();

        // Check if the path is a directory
        if pk_path.is_dir() {
            return Err(format!(
                "API token path {:?} is a directory, not a file",
                pk_path
            ));
        }

        if !pk_path.exists() {
            // Create parent directories if they don't exist
            if let Some(parent) = pk_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    format!(
                        "Unable to create parent directories for {:?}: {:?}",
                        pk_path, e
                    )
                })?;
            }

            let length = PK_LEN;
            let pk: String = rng()
                .sample_iter(&Alphanumeric)
                .take(length)
                .map(char::from)
                .collect();

            // Create and write the public key to file with appropriate permissions
            create_with_600_perms(pk_path, pk.to_string().as_bytes()).map_err(|e| {
                format!(
                    "Unable to create file with permissions for {:?}: {:?}",
                    pk_path, e
                )
            })?;
        }

        let pk = fs::read(pk_path)
            .map_err(|e| format!("cannot read {}: {}", pk_path.display(), e))?
            .iter()
            .map(|&c| char::from(c))
            .collect();

        Ok(Self {
            pk,
            pk_path: pk_path.to_path_buf(),
        })
    }

    /// Returns the API token.
    pub fn api_token(&self) -> String {
        self.pk.clone()
    }

    /// Returns the path for the API token file
    pub fn api_token_path(&self) -> PathBuf {
        self.pk_path.clone()
    }

    /// Returns the values of the `Authorization` header which indicate a valid incoming HTTP
    /// request.
    ///
    /// For backwards-compatibility we accept the token in a basic authentication style, but this is
    /// technically invalid according to RFC 7617 because the token is not a base64-encoded username
    /// and password. As such, bearer authentication should be preferred.
    pub fn auth_header_values(&self) -> Vec<String> {
        vec![
            format!("Basic {}", self.api_token()),
            format!("Bearer {}", self.api_token()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn constants() {
        assert_eq!(PK_FILENAME, "api-token.txt");
        assert_eq!(PK_LEN, 33);
    }

    #[test]
    fn create_new_token() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(PK_FILENAME);
        let secret = ApiSecret::create_or_open(&path).unwrap();
        assert_eq!(secret.api_token().len(), PK_LEN);
        assert_eq!(secret.api_token_path(), path);
        assert!(path.exists());
    }

    #[test]
    fn open_existing_token() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(PK_FILENAME);
        let secret1 = ApiSecret::create_or_open(&path).unwrap();
        let secret2 = ApiSecret::create_or_open(&path).unwrap();
        assert_eq!(secret1.api_token(), secret2.api_token());
    }

    #[test]
    fn auth_header_values_basic_and_bearer() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(PK_FILENAME);
        let secret = ApiSecret::create_or_open(&path).unwrap();
        let headers = secret.auth_header_values();
        assert_eq!(headers.len(), 2);
        assert!(headers[0].starts_with("Basic "));
        assert!(headers[1].starts_with("Bearer "));
        let token = secret.api_token();
        assert_eq!(headers[0], format!("Basic {}", token));
        assert_eq!(headers[1], format!("Bearer {}", token));
    }

    #[test]
    fn rejects_directory_path() {
        let dir = tempdir().unwrap();
        let result = ApiSecret::create_or_open(dir.path());
        match result {
            Err(e) => assert!(
                e.contains("directory"),
                "Expected 'directory' in error: {}",
                e
            ),
            Ok(_) => panic!("Expected error for directory path"),
        }
    }

    #[test]
    fn creates_parent_directories() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested").join("dir").join(PK_FILENAME);
        let secret = ApiSecret::create_or_open(&path).unwrap();
        assert_eq!(secret.api_token().len(), PK_LEN);
        assert!(path.exists());
    }

    #[test]
    fn token_is_alphanumeric() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(PK_FILENAME);
        let secret = ApiSecret::create_or_open(&path).unwrap();
        assert!(
            secret
                .api_token()
                .chars()
                .all(|c| c.is_ascii_alphanumeric())
        );
    }

    #[test]
    fn two_creates_different_tokens() {
        let dir = tempdir().unwrap();
        let path1 = dir.path().join("token1.txt");
        let path2 = dir.path().join("token2.txt");
        let s1 = ApiSecret::create_or_open(&path1).unwrap();
        let s2 = ApiSecret::create_or_open(&path2).unwrap();
        // Extremely unlikely to be equal with 33 random alphanumeric chars
        assert_ne!(s1.api_token(), s2.api_token());
    }
}
