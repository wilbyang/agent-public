use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use crate::config::PostgresProviderConfig;
use crate::data::extended_decision::{DecisionContentMeta, FileDecisionGraph};
use crate::data::release_data::{ReleaseData, ReleaseDataProject, ReleaseDataRelease};
use crate::immutable_loader::ImmutableLoader;
use crate::provider::{Agent, AgentData, AgentDataProvider, Project, ProjectData, ProjectDiff};
use dashmap::DashMap;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct PostgresProvider {
    pool: PgPool,
}

impl PostgresProvider {
    pub async fn new(config: &PostgresProviderConfig) -> anyhow::Result<Self> {
        let pool = PgPool::connect(&config.database_url).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        tracing::info!("PostgreSQL provider initialised and migrations applied");
        Ok(Self { pool })
    }
}

impl AgentDataProvider for PostgresProvider {
    fn load_data(
        &self,
        data: Arc<AgentData>,
    ) -> impl Future<Output = anyhow::Result<Vec<ProjectDiff>>> + Send + 'static {
        let pool = self.pool.clone();

        async move {
            let project_data = list_project_data(&pool).await?;
            let diff = data.calculate_diff(project_data);
            let to_refresh = Agent::get_refresh_list(&diff);
            let refreshed = generate_projects(&pool, to_refresh).await;
            let diff = Agent::get_diff_result(data, diff, refreshed);
            Ok(diff)
        }
    }
}

// ── Row types ────────────────────────────────────────────────────────────────

#[derive(sqlx::FromRow)]
struct ProjectSummaryRow {
    composite_key: String,
    release_pk: Uuid,
}

#[derive(sqlx::FromRow)]
struct ProjectDetailRow {
    project_pk: Uuid,
    project_id: String,
    project_key: String,
    release_pk: Uuid,
    release_id: String,
    version: String,
}

#[derive(sqlx::FromRow)]
struct DecisionModelRow {
    model_key: String,
    content: serde_json::Value,
    version_id: Option<String>,
}

#[derive(sqlx::FromRow)]
struct TokenRow {
    token: String,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Returns lightweight project data (composite key + release UUID as content hash)
/// so that `calculate_diff` can detect which projects need reloading.
async fn list_project_data(pool: &PgPool) -> anyhow::Result<Vec<ProjectData>> {
    let rows = sqlx::query_as::<_, ProjectSummaryRow>(
        r#"
        SELECT t.slug || ':' || p.project_key AS composite_key,
               r.id                           AS release_pk
        FROM   projects  p
        JOIN   tenants   t ON t.id = p.tenant_id
        JOIN   releases  r ON r.project_pk = p.id AND r.is_latest = TRUE
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| ProjectData {
            key: r.composite_key,
            content_hash: Some(r.release_pk.as_bytes().to_vec()),
        })
        .collect())
}

/// Loads full project data for each `"tenant_slug:project_key"` in `keys`.
async fn generate_projects(pool: &PgPool, keys: Vec<String>) -> DashMap<String, Arc<Project>> {
    let map = DashMap::new();
    for key in keys {
        match load_project(pool, &key).await {
            Ok(project) => {
                map.insert(key, Arc::new(project));
            }
            Err(err) => {
                tracing::error!("[PG - Skip] Failed to load project '{}': {}", key, err);
            }
        }
    }
    map
}

/// Loads a single project identified by `"tenant_slug:project_key"`.
async fn load_project(pool: &PgPool, composite_key: &str) -> anyhow::Result<Project> {
    let (tenant_slug, project_key) = split_composite_key(composite_key)?;

    // ── Project + latest release ─────────────────────────────────────────────
    let row = sqlx::query_as::<_, ProjectDetailRow>(
        r#"
        SELECT p.id     AS project_pk,
               p.project_id,
               p.project_key,
               r.id     AS release_pk,
               r.release_id,
               r.version
        FROM   projects p
        JOIN   tenants  t ON t.id = p.tenant_id
        JOIN   releases r ON r.project_pk = p.id AND r.is_latest = TRUE
        WHERE  t.slug        = $1
          AND  p.project_key = $2
        "#,
    )
    .bind(tenant_slug)
    .bind(project_key)
    .fetch_one(pool)
    .await?;

    // ── Decision models ──────────────────────────────────────────────────────
    let model_rows = sqlx::query_as::<_, DecisionModelRow>(
        "SELECT model_key, content, version_id FROM decision_models WHERE release_pk = $1",
    )
    .bind(row.release_pk)
    .fetch_all(pool)
    .await?;

    // ── Access tokens ────────────────────────────────────────────────────────
    let token_rows = sqlx::query_as::<_, TokenRow>(
        "SELECT token FROM project_tokens WHERE project_pk = $1",
    )
    .bind(row.project_pk)
    .fetch_all(pool)
    .await?;

    // ── Build ImmutableLoader ────────────────────────────────────────────────
    let content: HashMap<String, FileDecisionGraph> = model_rows
        .into_iter()
        .filter_map(|m| {
            match serde_json::from_value::<FileDecisionGraph>(m.content) {
                Ok(mut graph) => {
                    // Preserve the version_id stored alongside the model content.
                    if m.version_id.is_some() {
                        graph.meta = DecisionContentMeta {
                            version_id: m.version_id.map(Arc::from),
                        };
                    }
                    Some((m.model_key.to_lowercase(), graph))
                }
                Err(err) => {
                    tracing::warn!(
                        "[PG] Failed to deserialise decision model '{}': {}",
                        m.model_key,
                        err
                    );
                    None
                }
            }
        })
        .collect();

    let release_data = ReleaseData {
        version: Some(Arc::from(row.version.as_str())),
        project: ReleaseDataProject {
            id: Arc::from(row.project_id.as_str()),
            key: Arc::from(row.project_key.as_str()),
        },
        access_tokens: token_rows
            .into_iter()
            .map(|t| Arc::from(t.token.as_str()))
            .collect(),
        release: ReleaseDataRelease {
            id: Arc::from(row.release_id.as_str()),
            version: Arc::from(row.version.as_str()),
        },
    };

    let engine = ImmutableLoader::new(content, Some(release_data)).into_engine();

    Ok(Project {
        engine,
        content_hash: Some(row.release_pk.as_bytes().to_vec()),
    })
}

fn split_composite_key(key: &str) -> anyhow::Result<(&str, &str)> {
    let mut parts = key.splitn(2, ':');
    let tenant = parts
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("composite key missing tenant part: '{}'", key))?;
    let project = parts
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("composite key missing project part: '{}'", key))?;
    Ok((tenant, project))
}
