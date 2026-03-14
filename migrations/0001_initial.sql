CREATE TABLE tenants (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    slug        TEXT        NOT NULL UNIQUE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE projects (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id   UUID        NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    project_id  TEXT        NOT NULL,
    project_key TEXT        NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (tenant_id, project_key),
    UNIQUE (tenant_id, project_id)
);

CREATE TABLE releases (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    project_pk  UUID        NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    release_id  TEXT        NOT NULL,
    version     TEXT        NOT NULL,
    is_latest   BOOLEAN     NOT NULL DEFAULT FALSE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (project_pk, release_id)
);

CREATE TABLE decision_models (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    release_pk  UUID        NOT NULL REFERENCES releases(id) ON DELETE CASCADE,
    model_key   TEXT        NOT NULL,
    content     JSONB       NOT NULL,
    version_id  TEXT,
    UNIQUE (release_pk, model_key)
);

CREATE TABLE project_tokens (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    project_pk  UUID        NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    token       TEXT        NOT NULL,
    UNIQUE (project_pk, token)
);

CREATE INDEX idx_projects_tenant      ON projects(tenant_id);
CREATE INDEX idx_releases_latest      ON releases(project_pk) WHERE is_latest = TRUE;
CREATE INDEX idx_decision_release     ON decision_models(release_pk);
CREATE INDEX idx_project_tokens_proj  ON project_tokens(project_pk);
