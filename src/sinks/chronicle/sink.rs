use bytes::Bytes;
use http::{Request, Uri};
use serde_json::{json, Value};
use snafu::ResultExt;

use super::{
    config::ChronicleSinkConfig,
    encoder::{ChronicleSinkEventEncoder, Encoding},
};
use crate::{
    gcp::{GcpCredentials, Scope},
    sinks::{
        util::{encoding::EncodingConfigWithDefault, http::HttpSink, BoxedRawValue},
        UriParseSnafu,
    },
};

pub(super) struct ChronicleSink {
    api_key: Option<String>,
    pub(super) creds: Option<GcpCredentials>,
    uri_base: String,
    encoding: EncodingConfigWithDefault<Encoding>,
}

// https://cloud.google.com/chronicle/docs/reference/ingestion-api#ingestion_api_reference
// We can send UDM (unified data model - https://cloud.google.com/chronicle/docs/reference/udm-field-list)
// events or unstructured log entries.
const CHRONICLE_URL: &str = "https://malachiteingestion-pa.googleapis.com";

impl ChronicleSink {
    pub(super) async fn from_config(config: &ChronicleSinkConfig) -> crate::Result<Self> {
        let creds = if config.skip_authentication {
            None
        } else {
            // We need the scope `https://www.googleapis.com/auth/malachite-ingestion`
            // https://cloud.google.com/chronicle/docs/reference/ingestion-api#getting_api_authentication_credentials
            // This doesn't exist in the list of scopes.
            config.auth.make_credentials(Scope::Activity).await?
        };

        let uri_base = match config.endpoint.as_ref() {
            Some(host) => host.to_string(),
            None => CHRONICLE_URL.into(),
        };

        // This url is for the unstructured log entries.
        let uri_base = format!("{}/v2/unstructuredlogentries", uri_base,);

        Ok(Self {
            api_key: config.auth.api_key.clone(),
            encoding: config.encoding.clone(),
            creds,
            uri_base,
        })
    }

    pub(super) fn uri(&self, suffix: &str) -> crate::Result<Uri> {
        let mut uri = format!("{}{}", self.uri_base, suffix);
        if let Some(key) = &self.api_key {
            uri = format!("{}?key={}", uri, key);
        }
        uri.parse::<Uri>()
            .context(UriParseSnafu)
            .map_err(Into::into)
    }
}

#[async_trait::async_trait]
impl HttpSink for ChronicleSink {
    type Input = Value;
    type Output = Vec<BoxedRawValue>;
    type Encoder = ChronicleSinkEventEncoder;

    fn build_encoder(&self) -> Self::Encoder {
        ChronicleSinkEventEncoder {
            encoding: self.encoding.clone(),
        }
    }

    /// https://cloud.google.com/chronicle/docs/reference/ingestion-api#unstructuredlogentries
    async fn build_request(&self, events: Self::Output) -> crate::Result<Request<Bytes>> {
        // TODO We have to somehow batch by log_type...
        let body = json!({ "customer_id": "zork",
                            "log_type": "NOOG",
                            "entries": events });
        let body = crate::serde::json::to_bytes(&body)?.freeze();
        let uri = self.uri(":batchCreate")?;

        let builder = Request::post(uri).header("Content-Type", "application/json");

        let mut request = builder.body(body).unwrap();
        if let Some(creds) = &self.creds {
            creds.apply(&mut request);
        }

        Ok(request)
    }
}
