use aws_config::Region;
use aws_credential_types::Credentials;
use aws_sdk_s3::{
    config::Builder as S3ConfigBuilder,
    presigning::PresigningConfig,
    Client,
};
use std::time::Duration;
use tracing::info;

use crate::error::{Result, StorageError};

#[derive(Clone)]
pub struct StorageClient {
    client: Client,
    bucket: String,
}

#[derive(Debug, Clone)]
pub struct StorageConfig {
    pub endpoint_url: String,
    pub access_key: String,
    pub secret_key: String,
    pub region: String,
    pub bucket: String,
    pub force_path_style: bool,
}

impl StorageClient {
    pub async fn new(config: StorageConfig) -> Result<Self> {
        info!(bucket = %config.bucket, endpoint = %config.endpoint_url, "connecting to object storage");

        let creds = Credentials::new(
            &config.access_key,
            &config.secret_key,
            None,
            None,
            "voicebot-storage",
        );

        let s3_config = S3ConfigBuilder::new()
            .endpoint_url(&config.endpoint_url)
            .region(Region::new(config.region.clone()))
            .credentials_provider(creds)
            .force_path_style(config.force_path_style)
            .build();

        let client = Client::from_conf(s3_config);
        Ok(Self {
            client,
            bucket: config.bucket,
        })
    }

    /// Upload bytes to a key, returning the full key.
    pub async fn upload(&self, key: &str, data: Vec<u8>, content_type: &str) -> Result<String> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(data.into())
            .content_type(content_type)
            .send()
            .await
            .map_err(|e| StorageError::S3(e.to_string()))?;
        Ok(key.to_owned())
    }

    /// Download object as bytes.
    pub async fn download(&self, key: &str) -> Result<Vec<u8>> {
        let resp = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| {
                if e.to_string().contains("NoSuchKey") {
                    StorageError::NotFound(key.to_owned())
                } else {
                    StorageError::S3(e.to_string())
                }
            })?;

        resp.body
            .collect()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| StorageError::S3(e.to_string()))
    }

    /// Delete an object.
    pub async fn delete(&self, key: &str) -> Result<()> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| StorageError::S3(e.to_string()))?;
        Ok(())
    }

    /// Generate a presigned GET URL valid for the given duration.
    pub async fn presign_get(&self, key: &str, expires_in: Duration) -> Result<String> {
        let presigning = PresigningConfig::expires_in(expires_in)
            .map_err(|e| StorageError::S3(e.to_string()))?;

        let resp = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .presigned(presigning)
            .await
            .map_err(|e| StorageError::S3(e.to_string()))?;

        Ok(resp.uri().to_string())
    }

    /// Generate a presigned PUT URL.
    pub async fn presign_put(
        &self,
        key: &str,
        content_type: &str,
        expires_in: Duration,
    ) -> Result<String> {
        let presigning = PresigningConfig::expires_in(expires_in)
            .map_err(|e| StorageError::S3(e.to_string()))?;

        let resp = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
            .presigned(presigning)
            .await
            .map_err(|e| StorageError::S3(e.to_string()))?;

        Ok(resp.uri().to_string())
    }

    pub fn recording_key(tenant_id: &str, session_id: &str) -> String {
        format!("recordings/{tenant_id}/{session_id}.wav")
    }
}
