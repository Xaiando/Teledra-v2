//! Localhost-only blackboard API for the role-separated swarm.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Router, serve};
use serde::{Deserialize, Serialize};

use crate::blackboard::{Blackboard, BlackboardError, Project, Task};

#[derive(Clone)]
struct AppState {
    board: Blackboard,
}

#[derive(Deserialize)]
struct ProjectCreate {
    name: String,
    description: Option<String>,
}

#[derive(Deserialize)]
struct TaskCreate {
    name: String,
    description: Option<String>,
    #[serde(default)]
    dependencies: Vec<String>,
}

#[derive(Deserialize)]
struct ClaimQuery {
    role: String,
    #[serde(default = "lease_minutes")]
    minutes: u64,
}

fn lease_minutes() -> u64 {
    15
}

#[derive(Deserialize)]
struct TransitionRequest {
    status: String,
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
}

pub async fn run(
    board_path: PathBuf,
    address: SocketAddr,
) -> Result<(), Box<dyn std::error::Error>> {
    if !address.ip().is_loopback() {
        return Err("Frankenswarm runtime may bind only to loopback".into());
    }
    let state = AppState {
        board: Blackboard::open(board_path)?,
    };
    let app = Router::new()
        .route("/health", get(health))
        .route("/projects", post(create_project))
        .route("/projects/{project_id}", get(get_project))
        .route("/projects/{project_id}/tasks", post(create_task))
        .route("/tasks/{task_id}", get(get_task))
        .route("/tasks/{task_id}/claim", post(claim_task))
        .route("/tasks/{task_id}/transition", post(transition_task))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(address).await?;
    serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

pub fn parse_loopback_address(raw: &str) -> Result<SocketAddr, String> {
    let address: SocketAddr = raw
        .parse::<SocketAddr>()
        .map_err(|error| error.to_string())?;
    if !address.ip().is_loopback() {
        return Err("address must be loopback".into());
    }
    Ok(address)
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn create_project(
    State(state): State<AppState>,
    Json(input): Json<ProjectCreate>,
) -> Result<(StatusCode, Json<Project>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(
            state
                .board
                .create_project(&input.name, input.description.as_deref())?,
        ),
    ))
}

async fn get_project(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Project>, ApiError> {
    Ok(Json(state.board.get_project(&id)?))
}

async fn create_task(
    State(state): State<AppState>,
    Path(project): Path<String>,
    Json(input): Json<TaskCreate>,
) -> Result<(StatusCode, Json<Task>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(state.board.create_task(
            &project,
            &input.name,
            input.description.as_deref(),
            &input.dependencies,
        )?),
    ))
}

async fn get_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Task>, ApiError> {
    Ok(Json(state.board.get_task(&id)?))
}

async fn claim_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ClaimQuery>,
) -> Result<Json<crate::blackboard::Lease>, ApiError> {
    Ok(Json(state.board.claim_task(
        &id,
        &query.role,
        Duration::from_secs(query.minutes.clamp(1, 1_440) * 60),
    )?))
}

async fn transition_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<TransitionRequest>,
) -> Result<Json<Task>, ApiError> {
    Ok(Json(state.board.transition_task(&id, &input.status)?))
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}

struct ApiError(BlackboardError);

impl From<BlackboardError> for ApiError {
    fn from(error: BlackboardError) -> Self {
        Self(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.0 {
            BlackboardError::MissingProject(_) | BlackboardError::MissingTask(_) => {
                StatusCode::NOT_FOUND
            }
            BlackboardError::NotClaimable(_)
            | BlackboardError::Dependencies(_)
            | BlackboardError::Transition(_, _) => StatusCode::CONFLICT,
            BlackboardError::InvalidText => StatusCode::BAD_REQUEST,
            BlackboardError::Sql(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (
            status,
            Json(serde_json::json!({"error": self.0.to_string()})),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_loopback_bind() {
        assert!(parse_loopback_address("0.0.0.0:8000").is_err());
        assert!(parse_loopback_address("127.0.0.1:8000").is_ok());
        assert!("::1".parse::<std::net::IpAddr>().unwrap().is_loopback());
    }
}
