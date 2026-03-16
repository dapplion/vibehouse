pub use eth2::Error;
use eth2::types::beacon_response::EmptyMetadata;
use eth2::types::builder_bid::SignedBuilderBid;
use eth2::types::{
    ContentType, ContextDeserialize, EthSpec, ExecutionBlockHash, ForkName, ForkVersionDecode,
    ForkVersionedResponse, PublicKeyBytes, SignedValidatorRegistrationData, Slot,
};
use eth2::types::{FullPayloadContents, SignedBlindedBeaconBlock};
use eth2::{
    CONSENSUS_VERSION_HEADER, CONTENT_TYPE_HEADER, JSON_CONTENT_TYPE_HEADER,
    SSZ_CONTENT_TYPE_HEADER, StatusCode, ok_or_error, success_or_error,
};
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue};
use reqwest::{IntoUrl, Response};
use sensitive_url::SensitiveUrl;
use serde::Serialize;
use serde::de::DeserializeOwned;
use ssz::Encode;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const DEFAULT_TIMEOUT_MILLIS: u64 = 15000;

/// This timeout is in accordance with v0.2.0 of the [builder specs](https://github.com/flashbots/mev-boost/pull/20).
pub const DEFAULT_GET_HEADER_TIMEOUT_MILLIS: u64 = 1000;

/// Default user agent for HTTP requests.
pub const DEFAULT_USER_AGENT: &str = vibehouse_version::VERSION;

/// The value we set on the `ACCEPT` http header to indicate a preference for ssz response.
pub const PREFERENCE_ACCEPT_VALUE: &str = "application/octet-stream;q=1.0,application/json;q=0.9";
/// Only accept json responses.
pub const JSON_ACCEPT_VALUE: &str = "application/json";

#[derive(Clone)]
pub struct Timeouts {
    get_header: Duration,
    post_validators: Duration,
    post_blinded_blocks: Duration,
    get_builder_status: Duration,
}

impl Timeouts {
    fn new(get_header_timeout: Option<Duration>) -> Self {
        let get_header =
            get_header_timeout.unwrap_or(Duration::from_millis(DEFAULT_GET_HEADER_TIMEOUT_MILLIS));

        Self {
            get_header,
            post_validators: Duration::from_millis(DEFAULT_TIMEOUT_MILLIS),
            post_blinded_blocks: Duration::from_millis(DEFAULT_TIMEOUT_MILLIS),
            get_builder_status: Duration::from_millis(DEFAULT_TIMEOUT_MILLIS),
        }
    }
}

#[derive(Clone)]
pub struct BuilderHttpClient {
    client: reqwest::Client,
    server: SensitiveUrl,
    timeouts: Timeouts,
    user_agent: String,
    /// Only use json for all requests/responses types.
    disable_ssz: bool,
    /// Indicates that the `get_header` response had content-type ssz
    /// so we can set content-type header to ssz to make the `submit_blinded_blocks`
    /// request.
    ssz_available: Arc<AtomicBool>,
}

impl BuilderHttpClient {
    pub fn new(
        server: SensitiveUrl,
        user_agent: Option<String>,
        builder_header_timeout: Option<Duration>,
        disable_ssz: bool,
    ) -> Result<Self, Error> {
        let user_agent = user_agent.unwrap_or(DEFAULT_USER_AGENT.to_string());
        let client = reqwest::Client::builder().user_agent(&user_agent).build()?;
        Ok(Self {
            client,
            server,
            timeouts: Timeouts::new(builder_header_timeout),
            user_agent,
            disable_ssz,
            ssz_available: Arc::new(false.into()),
        })
    }

    pub fn get_user_agent(&self) -> &str {
        &self.user_agent
    }

    fn fork_name_from_header(&self, headers: &HeaderMap) -> Result<Option<ForkName>, String> {
        headers
            .get(CONSENSUS_VERSION_HEADER)
            .map(|fork_name| {
                fork_name
                    .to_str()
                    .map_err(|e| e.to_string())
                    .and_then(ForkName::from_str)
            })
            .transpose()
    }

    fn content_type_from_header(&self, headers: &HeaderMap) -> ContentType {
        let Some(content_type) = headers.get(CONTENT_TYPE_HEADER).map(|content_type| {
            let content_type = content_type.to_str();
            match content_type {
                Ok(SSZ_CONTENT_TYPE_HEADER) => ContentType::Ssz,
                _ => ContentType::Json,
            }
        }) else {
            return ContentType::Json;
        };
        content_type
    }

    async fn get_with_header<
        T: DeserializeOwned + ForkVersionDecode + for<'de> ContextDeserialize<'de, ForkName>,
        U: IntoUrl,
    >(
        &self,
        url: U,
        timeout: Duration,
        headers: HeaderMap,
    ) -> Result<ForkVersionedResponse<T>, Error> {
        let response = self
            .get_response_with_header(url, Some(timeout), headers)
            .await?;

        let headers = response.headers().clone();
        let response_bytes = response.bytes().await?;

        let Ok(Some(fork_name)) = self.fork_name_from_header(&headers) else {
            // if no fork version specified, attempt to fallback to JSON
            self.ssz_available.store(false, Ordering::SeqCst);
            return serde_json::from_slice(&response_bytes).map_err(Error::InvalidJson);
        };

        let content_type = self.content_type_from_header(&headers);

        match content_type {
            ContentType::Ssz => {
                self.ssz_available.store(true, Ordering::SeqCst);
                T::from_ssz_bytes_by_fork(&response_bytes, fork_name)
                    .map(|data| ForkVersionedResponse {
                        version: fork_name,
                        metadata: EmptyMetadata {},
                        data,
                    })
                    .map_err(Error::InvalidSsz)
            }
            ContentType::Json => {
                self.ssz_available.store(false, Ordering::SeqCst);
                serde_json::from_slice(&response_bytes).map_err(Error::InvalidJson)
            }
        }
    }

    /// Return `true` if the most recently received response from the builder had SSZ Content-Type.
    /// Return `false` otherwise.
    /// Also returns `false` if we have explicitly disabled ssz.
    pub fn is_ssz_available(&self) -> bool {
        !self.disable_ssz && self.ssz_available.load(Ordering::SeqCst)
    }

    async fn get_with_timeout<T: DeserializeOwned, U: IntoUrl>(
        &self,
        url: U,
        timeout: Duration,
    ) -> Result<T, Error> {
        self.get_response_with_timeout(url, Some(timeout))
            .await?
            .json()
            .await
            .map_err(Into::into)
    }

    /// Perform a HTTP GET request, returning the `Response` for further processing.
    async fn get_response_with_header<U: IntoUrl>(
        &self,
        url: U,
        timeout: Option<Duration>,
        headers: HeaderMap,
    ) -> Result<Response, Error> {
        let mut builder = self.client.get(url);
        if let Some(timeout) = timeout {
            builder = builder.timeout(timeout);
        }
        let response = builder.headers(headers).send().await.map_err(Error::from)?;
        ok_or_error(response).await
    }

    /// Perform a HTTP GET request, returning the `Response` for further processing.
    async fn get_response_with_timeout<U: IntoUrl>(
        &self,
        url: U,
        timeout: Option<Duration>,
    ) -> Result<Response, Error> {
        let mut builder = self.client.get(url);
        if let Some(timeout) = timeout {
            builder = builder.timeout(timeout);
        }
        let response = builder.send().await.map_err(Error::from)?;
        ok_or_error(response).await
    }

    /// Generic POST function supporting arbitrary responses and timeouts.
    async fn post_generic<T: Serialize, U: IntoUrl>(
        &self,
        url: U,
        body: &T,
        timeout: Option<Duration>,
    ) -> Result<Response, Error> {
        let mut builder = self.client.post(url);
        if let Some(timeout) = timeout {
            builder = builder.timeout(timeout);
        }
        let response = builder.json(body).send().await?;
        ok_or_error(response).await
    }

    async fn post_ssz_with_raw_response<U: IntoUrl>(
        &self,
        url: U,
        ssz_body: Vec<u8>,
        headers: HeaderMap,
        timeout: Option<Duration>,
    ) -> Result<Response, Error> {
        let mut builder = self.client.post(url);
        if let Some(timeout) = timeout {
            builder = builder.timeout(timeout);
        }

        let response = builder
            .headers(headers)
            .body(ssz_body)
            .send()
            .await
            .map_err(Error::from)?;
        success_or_error(response).await
    }

    async fn post_with_raw_response<T: Serialize, U: IntoUrl>(
        &self,
        url: U,
        body: &T,
        headers: HeaderMap,
        timeout: Option<Duration>,
    ) -> Result<Response, Error> {
        let mut builder = self.client.post(url);
        if let Some(timeout) = timeout {
            builder = builder.timeout(timeout);
        }

        let response = builder
            .headers(headers)
            .json(body)
            .send()
            .await
            .map_err(Error::from)?;
        success_or_error(response).await
    }

    /// `POST /eth/v1/builder/validators`
    pub async fn post_builder_validators(
        &self,
        validator: &[SignedValidatorRegistrationData],
    ) -> Result<(), Error> {
        let mut path = self.server.full.clone();

        path.path_segments_mut()
            .map_err(|()| Error::InvalidUrl(self.server.clone()))?
            .push("eth")
            .push("v1")
            .push("builder")
            .push("validators");

        self.post_generic(path, &validator, Some(self.timeouts.post_validators))
            .await?;
        Ok(())
    }

    /// `POST /eth/v1/builder/blinded_blocks` with SSZ serialized request body
    pub async fn post_builder_blinded_blocks_v1_ssz<E: EthSpec>(
        &self,
        blinded_block: &SignedBlindedBeaconBlock<E>,
    ) -> Result<FullPayloadContents<E>, Error> {
        let mut path = self.server.full.clone();

        let body = blinded_block.as_ssz_bytes();

        path.path_segments_mut()
            .map_err(|()| Error::InvalidUrl(self.server.clone()))?
            .push("eth")
            .push("v1")
            .push("builder")
            .push("blinded_blocks");

        let mut headers = HeaderMap::new();
        headers.insert(
            CONSENSUS_VERSION_HEADER,
            HeaderValue::from_str(&blinded_block.fork_name_unchecked().to_string())
                .map_err(|e| Error::InvalidHeaders(format!("{}", e)))?,
        );
        headers.insert(
            CONTENT_TYPE_HEADER,
            HeaderValue::from_str(SSZ_CONTENT_TYPE_HEADER)
                .map_err(|e| Error::InvalidHeaders(format!("{}", e)))?,
        );
        headers.insert(
            ACCEPT,
            HeaderValue::from_str(PREFERENCE_ACCEPT_VALUE)
                .map_err(|e| Error::InvalidHeaders(format!("{}", e)))?,
        );

        let result = self
            .post_ssz_with_raw_response(
                path,
                body,
                headers,
                Some(self.timeouts.post_blinded_blocks),
            )
            .await?
            .bytes()
            .await?;

        FullPayloadContents::from_ssz_bytes_by_fork(&result, blinded_block.fork_name_unchecked())
            .map_err(Error::InvalidSsz)
    }

    /// `POST /eth/v2/builder/blinded_blocks` with SSZ serialized request body
    pub async fn post_builder_blinded_blocks_v2_ssz<E: EthSpec>(
        &self,
        blinded_block: &SignedBlindedBeaconBlock<E>,
    ) -> Result<(), Error> {
        let mut path = self.server.full.clone();

        let body = blinded_block.as_ssz_bytes();

        path.path_segments_mut()
            .map_err(|()| Error::InvalidUrl(self.server.clone()))?
            .push("eth")
            .push("v2")
            .push("builder")
            .push("blinded_blocks");

        let mut headers = HeaderMap::new();
        headers.insert(
            CONSENSUS_VERSION_HEADER,
            HeaderValue::from_str(&blinded_block.fork_name_unchecked().to_string())
                .map_err(|e| Error::InvalidHeaders(format!("{}", e)))?,
        );
        headers.insert(
            CONTENT_TYPE_HEADER,
            HeaderValue::from_str(SSZ_CONTENT_TYPE_HEADER)
                .map_err(|e| Error::InvalidHeaders(format!("{}", e)))?,
        );
        headers.insert(
            ACCEPT,
            HeaderValue::from_str(PREFERENCE_ACCEPT_VALUE)
                .map_err(|e| Error::InvalidHeaders(format!("{}", e)))?,
        );

        let result = self
            .post_ssz_with_raw_response(
                path,
                body,
                headers,
                Some(self.timeouts.post_blinded_blocks),
            )
            .await?;

        if result.status() == StatusCode::ACCEPTED {
            Ok(())
        } else {
            // ACCEPTED is the only valid status code response
            Err(Error::StatusCode(result.status()))
        }
    }

    /// `POST /eth/v1/builder/blinded_blocks`
    pub async fn post_builder_blinded_blocks_v1<E: EthSpec>(
        &self,
        blinded_block: &SignedBlindedBeaconBlock<E>,
    ) -> Result<ForkVersionedResponse<FullPayloadContents<E>>, Error> {
        let mut path = self.server.full.clone();

        path.path_segments_mut()
            .map_err(|()| Error::InvalidUrl(self.server.clone()))?
            .push("eth")
            .push("v1")
            .push("builder")
            .push("blinded_blocks");

        let mut headers = HeaderMap::new();
        headers.insert(
            CONSENSUS_VERSION_HEADER,
            HeaderValue::from_str(&blinded_block.fork_name_unchecked().to_string())
                .map_err(|e| Error::InvalidHeaders(format!("{}", e)))?,
        );
        headers.insert(
            CONTENT_TYPE_HEADER,
            HeaderValue::from_str(JSON_CONTENT_TYPE_HEADER)
                .map_err(|e| Error::InvalidHeaders(format!("{}", e)))?,
        );
        headers.insert(
            ACCEPT,
            HeaderValue::from_str(JSON_ACCEPT_VALUE)
                .map_err(|e| Error::InvalidHeaders(format!("{}", e)))?,
        );

        Ok(self
            .post_with_raw_response(
                path,
                &blinded_block,
                headers,
                Some(self.timeouts.post_blinded_blocks),
            )
            .await?
            .json()
            .await?)
    }

    /// `POST /eth/v2/builder/blinded_blocks`
    pub async fn post_builder_blinded_blocks_v2<E: EthSpec>(
        &self,
        blinded_block: &SignedBlindedBeaconBlock<E>,
    ) -> Result<(), Error> {
        let mut path = self.server.full.clone();

        path.path_segments_mut()
            .map_err(|()| Error::InvalidUrl(self.server.clone()))?
            .push("eth")
            .push("v2")
            .push("builder")
            .push("blinded_blocks");

        let mut headers = HeaderMap::new();
        headers.insert(
            CONSENSUS_VERSION_HEADER,
            HeaderValue::from_str(&blinded_block.fork_name_unchecked().to_string())
                .map_err(|e| Error::InvalidHeaders(format!("{}", e)))?,
        );
        headers.insert(
            CONTENT_TYPE_HEADER,
            HeaderValue::from_str(JSON_CONTENT_TYPE_HEADER)
                .map_err(|e| Error::InvalidHeaders(format!("{}", e)))?,
        );
        headers.insert(
            ACCEPT,
            HeaderValue::from_str(JSON_ACCEPT_VALUE)
                .map_err(|e| Error::InvalidHeaders(format!("{}", e)))?,
        );

        let result = self
            .post_with_raw_response(
                path,
                &blinded_block,
                headers,
                Some(self.timeouts.post_blinded_blocks),
            )
            .await?;

        if result.status() == StatusCode::ACCEPTED {
            Ok(())
        } else {
            // ACCEPTED is the only valid status code response
            Err(Error::StatusCode(result.status()))
        }
    }

    /// `GET /eth/v1/builder/header`
    pub async fn get_builder_header<E: EthSpec>(
        &self,
        slot: Slot,
        parent_hash: ExecutionBlockHash,
        pubkey: &PublicKeyBytes,
    ) -> Result<Option<ForkVersionedResponse<SignedBuilderBid<E>>>, Error> {
        let mut path = self.server.full.clone();

        path.path_segments_mut()
            .map_err(|()| Error::InvalidUrl(self.server.clone()))?
            .push("eth")
            .push("v1")
            .push("builder")
            .push("header")
            .push(slot.to_string().as_str())
            .push(format!("{parent_hash:?}").as_str())
            .push(pubkey.as_hex_string().as_str());

        let mut headers = HeaderMap::new();
        if self.disable_ssz {
            headers.insert(
                ACCEPT,
                HeaderValue::from_str(JSON_CONTENT_TYPE_HEADER)
                    .map_err(|e| Error::InvalidHeaders(format!("{}", e)))?,
            );
        } else {
            // Indicate preference for ssz response in the accept header
            headers.insert(
                ACCEPT,
                HeaderValue::from_str(PREFERENCE_ACCEPT_VALUE)
                    .map_err(|e| Error::InvalidHeaders(format!("{}", e)))?,
            );
        }

        let resp = self
            .get_with_header(path, self.timeouts.get_header, headers)
            .await;

        if matches!(resp, Err(Error::StatusCode(StatusCode::NO_CONTENT))) {
            Ok(None)
        } else {
            resp.map(Some)
        }
    }

    /// `GET /eth/v1/builder/status`
    pub async fn get_builder_status<E: EthSpec>(&self) -> Result<(), Error> {
        let mut path = self.server.full.clone();

        path.path_segments_mut()
            .map_err(|()| Error::InvalidUrl(self.server.clone()))?
            .push("eth")
            .push("v1")
            .push("builder")
            .push("status");

        self.get_with_timeout(path, self.timeouts.get_builder_status)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eth2::types::builder_bid::{BuilderBid, BuilderBidFulu};
    use eth2::types::test_utils::{SeedableRng, TestRandom, XorShiftRng};
    use eth2::types::{MainnetEthSpec, Signature};
    use mockito::{Matcher, Server, ServerGuard};

    type E = MainnetEthSpec;

    #[test]
    fn test_headers_no_panic() {
        for fork in ForkName::list_all() {
            assert!(HeaderValue::from_str(&fork.to_string()).is_ok());
        }
        assert!(HeaderValue::from_str(PREFERENCE_ACCEPT_VALUE).is_ok());
        assert!(HeaderValue::from_str(JSON_ACCEPT_VALUE).is_ok());
        assert!(HeaderValue::from_str(JSON_CONTENT_TYPE_HEADER).is_ok());
    }

    #[tokio::test]
    async fn test_get_builder_header_ssz_response() {
        // Set up mock server
        let mut server = Server::new_async().await;
        let mock_response_body = fulu_signed_builder_bid();
        mock_get_header_response(
            &mut server,
            Some("fulu"),
            ContentType::Ssz,
            mock_response_body.clone(),
        );

        let builder_client = BuilderHttpClient::new(
            SensitiveUrl::from_str(&server.url()).unwrap(),
            None,
            None,
            false,
        )
        .unwrap();

        let response = builder_client
            .get_builder_header(
                Slot::new(1),
                ExecutionBlockHash::repeat_byte(1),
                &PublicKeyBytes::empty(),
            )
            .await
            .expect("should succeed in get_builder_header")
            .expect("should have response body");

        assert_eq!(response, mock_response_body);
    }

    #[tokio::test]
    async fn test_get_builder_header_json_response() {
        // Set up mock server
        let mut server = Server::new_async().await;
        let mock_response_body = fulu_signed_builder_bid();
        mock_get_header_response(
            &mut server,
            None,
            ContentType::Json,
            mock_response_body.clone(),
        );

        let builder_client = BuilderHttpClient::new(
            SensitiveUrl::from_str(&server.url()).unwrap(),
            None,
            None,
            false,
        )
        .unwrap();

        let response = builder_client
            .get_builder_header(
                Slot::new(1),
                ExecutionBlockHash::repeat_byte(1),
                &PublicKeyBytes::empty(),
            )
            .await
            .expect("should succeed in get_builder_header")
            .expect("should have response body");

        assert_eq!(response, mock_response_body);
    }

    #[tokio::test]
    async fn test_get_builder_header_no_version_header_fallback_json() {
        // Set up mock server
        let mut server = Server::new_async().await;
        let mock_response_body = fulu_signed_builder_bid();
        mock_get_header_response(
            &mut server,
            Some("fulu"),
            ContentType::Json,
            mock_response_body.clone(),
        );

        let builder_client = BuilderHttpClient::new(
            SensitiveUrl::from_str(&server.url()).unwrap(),
            None,
            None,
            false,
        )
        .unwrap();

        let response = builder_client
            .get_builder_header(
                Slot::new(1),
                ExecutionBlockHash::repeat_byte(1),
                &PublicKeyBytes::empty(),
            )
            .await
            .expect("should succeed in get_builder_header")
            .expect("should have response body");

        assert_eq!(response, mock_response_body);
    }

    fn mock_get_header_response(
        server: &mut ServerGuard,
        header_version_opt: Option<&str>,
        content_type: ContentType,
        response_body: ForkVersionedResponse<SignedBuilderBid<E>>,
    ) {
        let mut mock = server.mock(
            "GET",
            Matcher::Regex(r"^/eth/v1/builder/header/\d+/.+/.+$".to_string()),
        );

        if let Some(version) = header_version_opt {
            mock = mock.with_header(CONSENSUS_VERSION_HEADER, version);
        }

        match content_type {
            ContentType::Json => {
                mock = mock
                    .with_header(CONTENT_TYPE_HEADER, JSON_CONTENT_TYPE_HEADER)
                    .with_body(serde_json::to_string(&response_body).unwrap());
            }
            ContentType::Ssz => {
                mock = mock
                    .with_header(CONTENT_TYPE_HEADER, SSZ_CONTENT_TYPE_HEADER)
                    .with_body(response_body.data.as_ssz_bytes());
            }
        }

        mock.with_status(200).create();
    }

    // --- Timeouts ---

    #[test]
    fn timeouts_default_get_header() {
        let t = Timeouts::new(None);
        assert_eq!(
            t.get_header,
            Duration::from_millis(DEFAULT_GET_HEADER_TIMEOUT_MILLIS)
        );
        assert_eq!(
            t.post_validators,
            Duration::from_millis(DEFAULT_TIMEOUT_MILLIS)
        );
        assert_eq!(
            t.post_blinded_blocks,
            Duration::from_millis(DEFAULT_TIMEOUT_MILLIS)
        );
        assert_eq!(
            t.get_builder_status,
            Duration::from_millis(DEFAULT_TIMEOUT_MILLIS)
        );
    }

    #[test]
    fn timeouts_custom_get_header() {
        let custom = Duration::from_millis(500);
        let t = Timeouts::new(Some(custom));
        assert_eq!(t.get_header, custom);
        // Other timeouts remain default
        assert_eq!(
            t.post_validators,
            Duration::from_millis(DEFAULT_TIMEOUT_MILLIS)
        );
    }

    // --- BuilderHttpClient::new ---

    #[test]
    fn builder_client_default_user_agent() {
        let client = BuilderHttpClient::new(
            SensitiveUrl::from_str("http://localhost:18550").unwrap(),
            None,
            None,
            false,
        )
        .unwrap();
        assert_eq!(client.get_user_agent(), DEFAULT_USER_AGENT);
    }

    #[test]
    fn builder_client_custom_user_agent() {
        let client = BuilderHttpClient::new(
            SensitiveUrl::from_str("http://localhost:18550").unwrap(),
            Some("my-custom-agent/1.0".to_string()),
            None,
            false,
        )
        .unwrap();
        assert_eq!(client.get_user_agent(), "my-custom-agent/1.0");
    }

    #[test]
    fn builder_client_custom_header_timeout() {
        let client = BuilderHttpClient::new(
            SensitiveUrl::from_str("http://localhost:18550").unwrap(),
            None,
            Some(Duration::from_millis(2000)),
            false,
        )
        .unwrap();
        assert_eq!(client.timeouts.get_header, Duration::from_millis(2000));
    }

    // --- is_ssz_available ---

    #[test]
    fn is_ssz_available_default_false() {
        let client = BuilderHttpClient::new(
            SensitiveUrl::from_str("http://localhost:18550").unwrap(),
            None,
            None,
            false,
        )
        .unwrap();
        assert!(!client.is_ssz_available());
    }

    #[test]
    fn is_ssz_available_disabled_even_when_set() {
        let client = BuilderHttpClient::new(
            SensitiveUrl::from_str("http://localhost:18550").unwrap(),
            None,
            None,
            true, // disable_ssz
        )
        .unwrap();
        // Even if ssz_available is set to true, disable_ssz overrides
        client.ssz_available.store(true, Ordering::SeqCst);
        assert!(!client.is_ssz_available());
    }

    #[test]
    fn is_ssz_available_true_when_enabled() {
        let client = BuilderHttpClient::new(
            SensitiveUrl::from_str("http://localhost:18550").unwrap(),
            None,
            None,
            false,
        )
        .unwrap();
        client.ssz_available.store(true, Ordering::SeqCst);
        assert!(client.is_ssz_available());
    }

    // --- fork_name_from_header ---

    #[test]
    fn fork_name_from_header_present() {
        let client = BuilderHttpClient::new(
            SensitiveUrl::from_str("http://localhost:18550").unwrap(),
            None,
            None,
            false,
        )
        .unwrap();

        let mut headers = HeaderMap::new();
        headers.insert(CONSENSUS_VERSION_HEADER, HeaderValue::from_static("fulu"));
        let result = client.fork_name_from_header(&headers).unwrap();
        assert_eq!(result, Some(ForkName::Fulu));
    }

    #[test]
    fn fork_name_from_header_absent() {
        let client = BuilderHttpClient::new(
            SensitiveUrl::from_str("http://localhost:18550").unwrap(),
            None,
            None,
            false,
        )
        .unwrap();

        let headers = HeaderMap::new();
        let result = client.fork_name_from_header(&headers).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn fork_name_from_header_invalid() {
        let client = BuilderHttpClient::new(
            SensitiveUrl::from_str("http://localhost:18550").unwrap(),
            None,
            None,
            false,
        )
        .unwrap();

        let mut headers = HeaderMap::new();
        headers.insert(
            CONSENSUS_VERSION_HEADER,
            HeaderValue::from_static("nonexistent"),
        );
        let result = client.fork_name_from_header(&headers);
        assert!(result.is_err());
    }

    #[test]
    fn fork_name_from_header_all_forks() {
        let client = BuilderHttpClient::new(
            SensitiveUrl::from_str("http://localhost:18550").unwrap(),
            None,
            None,
            false,
        )
        .unwrap();

        for fork in ForkName::list_all() {
            let mut headers = HeaderMap::new();
            headers.insert(
                CONSENSUS_VERSION_HEADER,
                HeaderValue::from_str(&fork.to_string()).unwrap(),
            );
            let result = client.fork_name_from_header(&headers).unwrap();
            assert_eq!(result, Some(fork));
        }
    }

    // --- content_type_from_header ---

    #[test]
    fn content_type_from_header_ssz() {
        let client = BuilderHttpClient::new(
            SensitiveUrl::from_str("http://localhost:18550").unwrap(),
            None,
            None,
            false,
        )
        .unwrap();

        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE_HEADER,
            HeaderValue::from_static(SSZ_CONTENT_TYPE_HEADER),
        );
        assert!(matches!(
            client.content_type_from_header(&headers),
            ContentType::Ssz
        ));
    }

    #[test]
    fn content_type_from_header_json() {
        let client = BuilderHttpClient::new(
            SensitiveUrl::from_str("http://localhost:18550").unwrap(),
            None,
            None,
            false,
        )
        .unwrap();

        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE_HEADER,
            HeaderValue::from_static(JSON_CONTENT_TYPE_HEADER),
        );
        assert!(matches!(
            client.content_type_from_header(&headers),
            ContentType::Json
        ));
    }

    #[test]
    fn content_type_from_header_missing() {
        let client = BuilderHttpClient::new(
            SensitiveUrl::from_str("http://localhost:18550").unwrap(),
            None,
            None,
            false,
        )
        .unwrap();

        let headers = HeaderMap::new();
        assert!(matches!(
            client.content_type_from_header(&headers),
            ContentType::Json
        ));
    }

    #[test]
    fn content_type_from_header_unknown_defaults_json() {
        let client = BuilderHttpClient::new(
            SensitiveUrl::from_str("http://localhost:18550").unwrap(),
            None,
            None,
            false,
        )
        .unwrap();

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE_HEADER, HeaderValue::from_static("text/plain"));
        assert!(matches!(
            client.content_type_from_header(&headers),
            ContentType::Json
        ));
    }

    // --- get_builder_header NO_CONTENT ---

    #[tokio::test]
    async fn test_get_builder_header_no_content_returns_none() {
        let mut server = Server::new_async().await;
        server
            .mock(
                "GET",
                Matcher::Regex(r"^/eth/v1/builder/header/\d+/.+/.+$".to_string()),
            )
            .with_status(204)
            .create();

        let client = BuilderHttpClient::new(
            SensitiveUrl::from_str(&server.url()).unwrap(),
            None,
            None,
            false,
        )
        .unwrap();

        let response = client
            .get_builder_header::<E>(
                Slot::new(1),
                ExecutionBlockHash::repeat_byte(1),
                &PublicKeyBytes::empty(),
            )
            .await
            .expect("should succeed");

        assert!(response.is_none());
    }

    // --- ssz_available tracking ---

    #[tokio::test]
    async fn test_ssz_available_set_after_ssz_response() {
        let mut server = Server::new_async().await;
        let mock_response_body = fulu_signed_builder_bid();
        mock_get_header_response(
            &mut server,
            Some("fulu"),
            ContentType::Ssz,
            mock_response_body.clone(),
        );

        let client = BuilderHttpClient::new(
            SensitiveUrl::from_str(&server.url()).unwrap(),
            None,
            None,
            false,
        )
        .unwrap();

        assert!(!client.is_ssz_available());

        let _response = client
            .get_builder_header::<E>(
                Slot::new(1),
                ExecutionBlockHash::repeat_byte(1),
                &PublicKeyBytes::empty(),
            )
            .await
            .unwrap();

        assert!(client.is_ssz_available());
    }

    // --- constants ---

    #[test]
    fn default_timeout_values() {
        assert_eq!(DEFAULT_TIMEOUT_MILLIS, 15000);
        assert_eq!(DEFAULT_GET_HEADER_TIMEOUT_MILLIS, 1000);
    }

    #[test]
    fn preference_accept_value_prefers_ssz() {
        assert!(PREFERENCE_ACCEPT_VALUE.contains("application/octet-stream"));
        assert!(PREFERENCE_ACCEPT_VALUE.contains("application/json"));
        // SSZ should have higher quality factor
        assert!(PREFERENCE_ACCEPT_VALUE.contains("q=1.0"));
        assert!(PREFERENCE_ACCEPT_VALUE.contains("q=0.9"));
    }

    fn fulu_signed_builder_bid() -> ForkVersionedResponse<SignedBuilderBid<E>> {
        let rng = &mut XorShiftRng::from_seed([42; 16]);
        ForkVersionedResponse {
            version: ForkName::Fulu,
            metadata: EmptyMetadata {},
            data: SignedBuilderBid {
                message: BuilderBid::Fulu(BuilderBidFulu::random_for_test(rng)),
                signature: Signature::empty(),
            },
        }
    }
}
