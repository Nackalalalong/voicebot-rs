use std::{env, sync::Arc};

use reqwest::StatusCode;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires DATABASE_URL and REDIS_URL"]
async fn tenant_b_cannot_fetch_tenant_a_campaign() -> Result<(), Box<dyn std::error::Error>> {
    let database_url = env::var("DATABASE_URL")?;
    let redis_url = env::var("REDIS_URL")?;
    let db = db::connect(&database_url).await?;
    db::run_migrations(&db).await?;
    let redis = cache::connect(&redis_url).await?;
    let storage = storage::StorageClient::new(storage::StorageConfig {
        endpoint_url: "http://localhost:9000".into(),
        access_key: "minioadmin".into(),
        secret_key: "minioadmin".into(),
        region: "us-east-1".into(),
        bucket: "voicebot".into(),
        force_path_style: true,
    })
    .await?;

    let jwt_secret = format!("test-secret-{}", Uuid::new_v4());
    let state = api::state::AppState::new(db.clone(), redis, storage, jwt_secret.clone());
    let router = api::create_router(Arc::clone(&state));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base_url = format!("http://{}", listener.local_addr()?);
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.expect("test server should run");
    });

    let suffix = Uuid::new_v4().simple().to_string();
    let (tenant_a, user_a) = db::queries::tenants::create_with_owner(
        &db,
        &format!("Tenant A {suffix}"),
        &format!("api-tenant-a-{suffix}"),
        &format!("a-{suffix}@example.com"),
        "hash-a",
        "Tenant A Admin",
    )
    .await?;
    let (tenant_b, user_b) = db::queries::tenants::create_with_owner(
        &db,
        &format!("Tenant B {suffix}"),
        &format!("api-tenant-b-{suffix}"),
        &format!("b-{suffix}@example.com"),
        "hash-b",
        "Tenant B Admin",
    )
    .await?;

    let tenant_a_token = auth::issue_access_token(
        &jwt_secret,
        user_a.id,
        tenant_a.id,
        &user_a.email,
        "admin",
    )?;
    let tenant_b_token = auth::issue_access_token(
        &jwt_secret,
        user_b.id,
        tenant_b.id,
        &user_b.email,
        "admin",
    )?;

    let client = reqwest::Client::new();
    let create_response = client
        .post(format!("{base_url}/api/v1/campaigns"))
        .bearer_auth(&tenant_a_token)
        .json(&serde_json::json!({
            "name": "Tenant A Campaign",
            "system_prompt": "prompt"
        }))
        .send()
        .await?;
    assert_eq!(create_response.status(), StatusCode::CREATED);
    let campaign: db::models::Campaign = create_response.json().await?;

    let fetch_response = client
        .get(format!("{base_url}/api/v1/campaigns/{}", campaign.id))
        .bearer_auth(&tenant_b_token)
        .send()
        .await?;
    assert_eq!(fetch_response.status(), StatusCode::NOT_FOUND);

    server.abort();
    let _ = server.await;

    Ok(())
}