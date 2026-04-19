use std::env;

use uuid::Uuid;

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn tenant_rls_requires_context_and_blocks_cross_tenant_reads(
) -> Result<(), Box<dyn std::error::Error>> {
    let database_url = env::var("DATABASE_URL")?;
    let pool = db::connect(&database_url).await?;
    db::run_migrations(&pool).await?;

    let suffix = Uuid::new_v4().simple().to_string();
    let tenant_a = db::queries::tenants::create(
        &pool,
        &format!("Tenant A {suffix}"),
        &format!("tenant-a-{suffix}"),
        "starter",
    )
    .await?;
    let tenant_b = db::queries::tenants::create(
        &pool,
        &format!("Tenant B {suffix}"),
        &format!("tenant-b-{suffix}"),
        "starter",
    )
    .await?;

    let campaign = db::queries::campaigns::create(
        &pool,
        db::queries::campaigns::CreateCampaign {
            tenant_id: tenant_a.id,
            name: "rls test campaign",
            system_prompt: "prompt",
            language: "en",
            voice_id: None,
            asr_provider: "whisper",
            tts_provider: "kokoro",
            llm_provider: "openai",
            llm_model: "gpt-4o-mini",
            max_call_duration_secs: 300,
            recording_enabled: false,
            tools_config: serde_json::json!([]),
            custom_metrics: serde_json::json!({}),
            schedule_config: serde_json::json!({}),
        },
    )
    .await?;

    let mut tx = pool.begin().await?;
    let missing_context_err = sqlx::query(
        "INSERT INTO campaigns (id, tenant_id, name) VALUES (gen_random_uuid(), $1, $2)",
    )
    .bind(tenant_a.id)
    .bind("missing context campaign")
    .execute(&mut *tx)
    .await
    .expect_err("insert without app.tenant_id should be rejected by RLS");
    assert!(matches!(missing_context_err, sqlx::Error::Database(_)));
    tx.rollback().await?;

    let mut tenant_b_tx = db::begin_tenant_tx(&pool, tenant_b.id).await?;
    let visible_campaign: Option<Uuid> = sqlx::query_scalar("SELECT id FROM campaigns WHERE id = $1")
        .bind(campaign.id)
        .fetch_optional(&mut *tenant_b_tx)
        .await?;
    assert!(visible_campaign.is_none(), "tenant B should not see tenant A rows via raw SQL");
    tenant_b_tx.commit().await?;

    Ok(())
}