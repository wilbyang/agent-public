use crate::Agent;
use crate::engine_ext::EngineExtension;
use axum::extract::Path;
use axum::http::{HeaderMap, StatusCode};
use axum::{Extension, Json};
use serde::Serialize;
use std::sync::Arc;

#[utoipa::path(
    get,
    path = "/api/projects/{project}",
    params(
        ("project" = String, Path, description = "Project slug or id"),
        ("X-Tenant-ID" = Option<String>, Header, description = "Tenant identifier (required in SaaS / Postgres mode)")
    ),
    responses(
        (status = OK, body = ProjectInfo)
    )
)]
pub async fn project_info(
    Extension(agent): Extension<Agent>,
    headers: HeaderMap,
    Path(project): Path<String>,
) -> Result<Json<ProjectInfo>, (StatusCode, String)> {
    let p = if agent.is_saas_mode() {
        let tenant = extract_tenant(&headers)?;
        agent.project_for_tenant(tenant, &project)
    } else {
        agent.project(&project)
    };

    let Some(p) = p else {
        return Err((StatusCode::NOT_FOUND, "Project not found".to_string()));
    };

    let Some(release_data) = p.engine.release_data() else {
        return Err((
            StatusCode::BAD_REQUEST,
            "Project data not available".to_string(),
        ));
    };

    Ok(Json(ProjectInfo {
        project: ProjectInfoProject {
            id: release_data.project.id.clone(),
            key: release_data.project.key.clone(),
        },
        release: ProjectInfoRelease {
            id: release_data.release.id.clone(),
            version: release_data.release.version.clone(),
        },
        access_tokens: release_data.access_tokens.clone(),
    }))
}

fn extract_tenant(headers: &HeaderMap) -> Result<&str, (StatusCode, String)> {
    headers
        .get("X-Tenant-ID")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "Missing or invalid X-Tenant-ID header".to_string(),
            )
        })
}

// ── Response types ────────────────────────────────────────────────────────────

#[derive(Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    pub project: ProjectInfoProject,
    pub release: ProjectInfoRelease,
    pub access_tokens: Vec<Arc<str>>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct ProjectInfoProject {
    pub id: Arc<str>,
    pub key: Arc<str>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct ProjectInfoRelease {
    pub id: Arc<str>,
    pub version: Arc<str>,
}
