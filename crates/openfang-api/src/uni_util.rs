/*
 * @Author             : Felix
 * @Email              : 307253927@qq.com
 * @Date               : 2026-03-19 14:08:38
 * @LastEditors        : Felix
 * @LastEditTime       : 2026-03-20 14:34:00
 */

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use openfang_types::config::openfang_home_dir;
use std::path::Path;
use thiserror::Error;

pub enum UniResult {
    Ok(serde_json::Value),
    Err(UniError),
}

impl IntoResponse for UniResult {
    fn into_response(self) -> axum::response::Response {
        match self {
            UniResult::Ok(value) => (StatusCode::OK, Json(value)).into_response(),
            UniResult::Err(error) => error.into_response(),
        }
    }
}

#[derive(Debug, Error)]
pub enum UniError {
    #[error("invalid parameter: {0}")]
    InvalidParameter(String),
    #[error("internal error: {0}")]
    InternalError(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("bad request: {0}")]
    BadRequest(String),
}

impl IntoResponse for UniError {
    fn into_response(self) -> axum::response::Response {
        let (status, error_msg) = match self {
            UniError::InvalidParameter(msg) => (StatusCode::BAD_REQUEST, msg),
            UniError::InternalError(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
            UniError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            UniError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg),
            UniError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg),
            UniError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
        };
        (status, Json(serde_json::json!({"error": error_msg}))).into_response()
    }
}

impl From<UniError> for UniResult {
    fn from(error: UniError) -> Self {
        UniResult::Err(error)
    }
}

/// 解析axum响应为结果类型
/// 如果响应状态码为200，返回Ok(())
/// 如果响应状态码错误，返回Err(UniError)
pub async fn check_axum_response_to_result(res: axum::response::Response) -> UniResult {
    if res.status().is_success() {
        UniResult::Ok(serde_json::Value::Null)
    } else {
        let bytes = match axum::body::to_bytes(res.into_body(), usize::MAX).await {
            Ok(bytes) => bytes,
            Err(e) => return UniError::InternalError(e.to_string()).into(),
        };
        let value = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(e) => return UniError::InternalError(e.to_string()).into(),
        };

        UniResult::Ok(value)
    }
}

/// Check if the path is in the home directory.
/// if the path is in the home directory, return true.
/// if the path is not exist, return false.
pub fn is_in_home_dir<P: AsRef<Path>>(path: P) -> bool {
    let canonical_home = match openfang_home_dir().canonicalize() {
        Ok(path) => path,
        Err(_) => return false,
    };

    let canonical_path = match path.as_ref().canonicalize() {
        Ok(path) => path,
        Err(_) => return false,
    };

    canonical_path.starts_with(canonical_home)
}

#[test]
fn test_is_in_home_dir() {
    let home_dir = openfang_home_dir();
    println!("home_dir: {:?}", home_dir);
    assert!(is_in_home_dir(&home_dir), "Path1 is in home directory");
    assert!(!is_in_home_dir("~"), "Path2 is not in home directory");
}
