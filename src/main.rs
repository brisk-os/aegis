use anyhow::Context;
use axum::Router;
use dotenvy::dotenv;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod config;
mod error;
mod middleware;
mod models;
mod routes;
mod services;

use config::Config;
use services::jwt::JwtService;

#[derive(Clone)]
pub struct AppState {
    pub db: sqlx::PgPool,
    pub config: Arc<Config>,
    pub jwt: Arc<JwtService>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "aegis=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = Arc::new(Config::from_env()?);

    let db = PgPoolOptions::new()
        .max_connections(20)
        .connect(&config.database_url)
        .await?;

    sqlx::migrate!("./migrations").run(&db).await?;

    let jwt = JwtService::new(
        &config.jwt_private_key,
        &config.jwt_public_key,
        config.access_token_expiry_secs,
        config.refresh_token_expiry_secs,
    )
    .context("failed to initialise JWT service — check JWT_PRIVATE_KEY and JWT_PUBLIC_KEY")?;

    let state = AppState {
        db,
        config: config.clone(),
        jwt: Arc::new(jwt),
    };

    let protected = Router::new()
        .merge(routes::auth::router())
        .merge(routes::users::router())
        .merge(routes::orgs::router())
        .merge(routes::members::router())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::app_auth::authenticate_app,
        ));

    let app = Router::new()
        .merge(protected)
        .merge(routes::admin::router())
        .merge(routes::jwks::router())
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("aegis listening on {}", addr);
    axum::serve(listener, app).await?;

    Ok(())
}
