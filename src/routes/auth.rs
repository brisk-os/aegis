use axum::{
    extract::State,
    routing::post,
    Extension, Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use validator::Validate;

use crate::{
    error::{AppError, Result},
    middleware::app_auth::AppIdentity,
    services::auth as auth_service,
    AppState,
};

#[derive(Debug, Deserialize, Validate)]
pub struct SignupRequest {
    #[validate(length(min = 1))]
    pub name: String,
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 8))]
    pub password: String,
}

#[derive(Debug, Deserialize, Validate)]
pub struct LoginRequest {
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 1))]
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/signup", post(signup))
        .route("/auth/login", post(login))
        .route("/auth/refresh", post(refresh))
        .route("/auth/logout", post(logout))
}

async fn signup(
    State(state): State<AppState>,
    Extension(app): Extension<AppIdentity>,
    Json(body): Json<SignupRequest>,
) -> Result<Json<serde_json::Value>> {
    body.validate().map_err(|e| AppError::Validation(e.to_string()))?;

    let user = auth_service::signup(&state.db, app.app_id, &body.name, &body.email, &body.password).await?;

    Ok(Json(serde_json::json!({ "id": user.id, "email": user.email, "name": user.name })))
}

async fn login(
    State(state): State<AppState>,
    Extension(app): Extension<AppIdentity>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<TokenResponse>> {
    body.validate().map_err(|e| AppError::Validation(e.to_string()))?;

    let result = auth_service::login(&state.db, app.app_id, &body.email, &body.password).await?;

    let jwt = &state.jwt;

    let org_type_str = format!("{:?}", result.org_type).to_lowercase();
    let access_token = jwt
        .issue_access_token(result.user.id, app.app_id, result.org_id, &org_type_str, &result.role, &result.user.email)
        .map_err(AppError::Internal)?;

    let (refresh_token, jti) = jwt.issue_refresh_token(result.user.id, app.app_id).map_err(|e| AppError::Internal(e))?;

    sqlx::query(
        "INSERT INTO refresh_session (id, user_id, jti, expires_at) VALUES ($1, $2, $3, NOW() + ($4 * interval '1 second'))",
    )
    .bind(Uuid::new_v4())
    .bind(result.user.id)
    .bind(&jti)
    .bind(state.config.refresh_token_expiry_secs as f64)
    .execute(&state.db)
    .await?;

    Ok(Json(TokenResponse { access_token, refresh_token, token_type: "Bearer".to_string() }))
}

async fn refresh(
    State(state): State<AppState>,
    Extension(app): Extension<AppIdentity>,
    Json(body): Json<RefreshRequest>,
) -> Result<Json<TokenResponse>> {
    let jwt = &state.jwt;

    let token_data = jwt
        .verify_refresh_token(&body.refresh_token)
        .map_err(|_| AppError::Unauthorized("invalid or expired refresh token".to_string()))?;

    let claims = token_data.claims;

    if claims.app_id != app.app_id.to_string() {
        return Err(AppError::Unauthorized("token app mismatch".to_string()));
    }

    let session_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM refresh_session WHERE jti = $1 AND revoked_at IS NULL AND expires_at > NOW()",
    )
    .bind(&claims.jti)
    .fetch_optional(&state.db)
    .await?;

    let session_id = session_id
        .ok_or_else(|| AppError::Unauthorized("refresh token revoked or expired".to_string()))?;

    sqlx::query("UPDATE refresh_session SET revoked_at = NOW() WHERE id = $1")
        .bind(session_id)
        .execute(&state.db)
        .await?;

    let user_id: Uuid = claims.sub.parse().map_err(|_| AppError::Unauthorized("invalid token".to_string()))?;

    let row: (Uuid, String, crate::models::organization::OrgType, String) = sqlx::query_as(
        r#"SELECT u.id, u.email, o.org_type, m.role
           FROM "user" u
           JOIN member m ON m.user_id = u.id
           JOIN organization o ON o.id = m.organization_id
           WHERE u.id = $1 AND o.org_type = 'personal'"#,
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .map_err(|_| AppError::Unauthorized("user not found".to_string()))?;

    let (uid, email, org_type, role) = row;

    let org_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM organization WHERE app_id = $1 AND org_type = 'personal' AND id IN (SELECT organization_id FROM member WHERE user_id = $2)",
    )
    .bind(app.app_id)
    .bind(uid)
    .fetch_one(&state.db)
    .await?;

    let org_type_str = format!("{:?}", org_type).to_lowercase();
    let access_token = jwt
        .issue_access_token(uid, app.app_id, org_id, &org_type_str, &role, &email)
        .map_err(AppError::Internal)?;

    let (new_refresh_token, new_jti) = jwt.issue_refresh_token(uid, app.app_id).map_err(AppError::Internal)?;

    sqlx::query(
        "INSERT INTO refresh_session (id, user_id, jti, expires_at) VALUES ($1, $2, $3, NOW() + ($4 * interval '1 second'))",
    )
    .bind(Uuid::new_v4())
    .bind(uid)
    .bind(&new_jti)
    .bind(state.config.refresh_token_expiry_secs as f64)
    .execute(&state.db)
    .await?;

    Ok(Json(TokenResponse { access_token, refresh_token: new_refresh_token, token_type: "Bearer".to_string() }))
}

async fn logout(
    State(state): State<AppState>,
    Json(body): Json<RefreshRequest>,
) -> Result<Json<serde_json::Value>> {
    if let Some(jti) = extract_jti_unverified(&body.refresh_token) {
        sqlx::query("UPDATE refresh_session SET revoked_at = NOW() WHERE jti = $1")
            .bind(jti)
            .execute(&state.db)
            .await?;
    }

    Ok(Json(serde_json::json!({ "ok": true })))
}

fn extract_jti_unverified(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    json.get("jti")?.as_str().map(|s| s.to_string())
}
