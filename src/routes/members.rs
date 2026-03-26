use axum::{
    extract::{Path, State},
    routing::{delete, get},
    Extension, Json, Router,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    error::{AppError, Result},
    middleware::app_auth::AppIdentity,
    models::member::Member,
    AppState,
};

#[derive(Debug, Deserialize)]
pub struct AddMemberRequest {
    pub user_id: Uuid,
    pub role: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/orgs/{org_id}/members", get(list_members).post(add_member))
        .route("/orgs/{org_id}/members/{user_id}", delete(remove_member))
}

async fn list_members(
    State(state): State<AppState>,
    Extension(app): Extension<AppIdentity>,
    Path(org_id): Path<Uuid>,
) -> Result<Json<Vec<Member>>> {
    ensure_org_belongs_to_app(&state, org_id, app.app_id).await?;

    let members: Vec<Member> = sqlx::query_as(
        "SELECT id, organization_id, user_id, role, created_at, updated_at FROM member WHERE organization_id = $1",
    )
    .bind(org_id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(members))
}

async fn add_member(
    State(state): State<AppState>,
    Extension(app): Extension<AppIdentity>,
    Path(org_id): Path<Uuid>,
    Json(body): Json<AddMemberRequest>,
) -> Result<Json<Member>> {
    ensure_org_belongs_to_app(&state, org_id, app.app_id).await?;

    let user_exists: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT id FROM "user" WHERE id = $1 AND app_id = $2"#,
    )
    .bind(body.user_id)
    .bind(app.app_id)
    .fetch_optional(&state.db)
    .await?;

    if user_exists.is_none() {
        return Err(AppError::NotFound("user not found".to_string()));
    }

    let role = body.role.unwrap_or_else(|| "member".to_string());

    let member: Member = sqlx::query_as(
        "INSERT INTO member (id, organization_id, user_id, role) VALUES ($1, $2, $3, $4) RETURNING id, organization_id, user_id, role, created_at, updated_at",
    )
    .bind(Uuid::new_v4())
    .bind(org_id)
    .bind(body.user_id)
    .bind(role)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(member))
}

async fn remove_member(
    State(state): State<AppState>,
    Extension(app): Extension<AppIdentity>,
    Path((org_id, user_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>> {
    ensure_org_belongs_to_app(&state, org_id, app.app_id).await?;

    sqlx::query("DELETE FROM member WHERE organization_id = $1 AND user_id = $2")
        .bind(org_id)
        .bind(user_id)
        .execute(&state.db)
        .await?;

    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn ensure_org_belongs_to_app(state: &AppState, org_id: Uuid, app_id: Uuid) -> Result<()> {
    let exists: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM organization WHERE id = $1 AND app_id = $2",
    )
    .bind(org_id)
    .bind(app_id)
    .fetch_optional(&state.db)
    .await?;

    exists
        .map(|_| ())
        .ok_or_else(|| AppError::NotFound("organization not found".to_string()))
}
