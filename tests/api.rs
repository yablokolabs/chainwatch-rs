use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusBuilder;
use tower::ServiceExt;

use chainwatch_rs::{
    api::{AppState, router},
    application::ports::Repository,
    config::ApiSettings,
    domain::{Chain, ChainId},
    infrastructure::memory::MemoryRepository,
};

#[tokio::test]
async fn health_endpoint_returns_ok() -> anyhow::Result<()> {
    let repo = Arc::new(MemoryRepository::new()) as Arc<dyn Repository>;
    repo.upsert_chain(&Chain {
        id: ChainId(1),
        name: "test".to_owned(),
        rpc_url_redacted: "http://localhost:8545".to_owned(),
    })
    .await?;
    let recorder = PrometheusBuilder::new().build_recorder();
    let state = AppState::new(repo, ChainId(1), recorder.handle(), 100);
    let app = router(state, &ApiSettings::default())?;

    let response = app
        .oneshot(Request::builder().uri("/health").body(Body::empty())?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await?.to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(json["status"], "ok");
    Ok(())
}

#[tokio::test]
async fn watchlist_lifecycle_validates_address() -> anyhow::Result<()> {
    let repo = Arc::new(MemoryRepository::new()) as Arc<dyn Repository>;
    repo.upsert_chain(&Chain {
        id: ChainId(1),
        name: "test".to_owned(),
        rpc_url_redacted: "http://localhost:8545".to_owned(),
    })
    .await?;
    let recorder = PrometheusBuilder::new().build_recorder();
    let state = AppState::new(repo, ChainId(1), recorder.handle(), 100);
    let app = router(state, &ApiSettings::default())?;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/watchlist")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"address":"0x0000000000000000000000000000000000000001","label":"case"}"#,
                ))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::CREATED);

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/watchlist/0x0000000000000000000000000000000000000001")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    Ok(())
}
