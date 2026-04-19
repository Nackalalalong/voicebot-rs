use std::env;

use uuid::Uuid;

#[tokio::test]
#[ignore = "requires S3 integration env vars"]
async fn upload_download_and_delete_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
    let client = storage::StorageClient::new(storage::StorageConfig {
        endpoint_url: env::var("S3_ENDPOINT_URL")?,
        access_key: env::var("S3_ACCESS_KEY")?,
        secret_key: env::var("S3_SECRET_KEY")?,
        region: env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".into()),
        bucket: env::var("S3_BUCKET")?,
        force_path_style: true,
    })
    .await?;

    let key = format!("integration-tests/{}.txt", Uuid::new_v4());
    let bytes = b"voicebot storage integration".to_vec();

    client.upload(&key, bytes.clone(), "text/plain").await?;
    let downloaded = client.download(&key).await?;
    assert_eq!(downloaded, bytes);

    client.delete(&key).await?;
    let err = client
        .download(&key)
        .await
        .expect_err("deleted object should not be readable");
    assert!(matches!(err, storage::StorageError::NotFound(_)));

    Ok(())
}