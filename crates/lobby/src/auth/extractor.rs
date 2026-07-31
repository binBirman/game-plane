use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use crate::http::error::ApiError;

#[derive(Clone, Debug)]
pub struct CurrentUser {
    pub uid: i64,
    pub username: String,
    pub nickname: String,
}

#[axum::async_trait]
impl FromRequestParts<Arc<crate::state::AppState>> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<crate::state::AppState>,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or(ApiError::InvalidCredentials)?;

        let token = header
            .strip_prefix("Bearer ")
            .ok_or(ApiError::InvalidCredentials)?
            .trim();

        let row: Option<(i64, String, String, String)> = sqlx::query_as(
            "SELECT u.id, u.username, u.nickname, s.expires_at
             FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token = ?",
        )
        .bind(token)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| ApiError::Internal(anyhow::Error::from(e)))?;

        let (uid, username, nickname, expires_at) = row.ok_or(ApiError::InvalidCredentials)?;

        // Lazy expiry check
        if let Ok(exp) = chrono::NaiveDateTime::parse_from_str(&expires_at, "%Y-%m-%dT%H:%M:%S") {
            let now = chrono::Utc::now().naive_utc();
            if exp < now {
                return Err(ApiError::InvalidCredentials);
            }
        }

        Ok(CurrentUser { uid, username, nickname })
    }
}