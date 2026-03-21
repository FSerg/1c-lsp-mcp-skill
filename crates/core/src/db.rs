use std::path::Path;
use std::str::FromStr;

use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Pool, Sqlite};

use crate::models::StoredProject;

#[derive(Debug, Clone)]
pub struct Database {
    pool: Pool<Sqlite>,
}

impl Database {
    pub async fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("failed to create db dir")?;
        }

        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
            .context("failed to build sqlite connection string")?
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .context("failed to open sqlite database")?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS projects (
                id          TEXT PRIMARY KEY,
                name        TEXT NOT NULL,
                root_path   TEXT NOT NULL UNIQUE,
                jvm_args    TEXT NOT NULL DEFAULT '',
                debug       INTEGER NOT NULL DEFAULT 0,
                created_at  TEXT NOT NULL,
                updated_at  TEXT NOT NULL
            )
            "#,
        )
        .execute(&pool)
        .await
        .context("failed to initialize projects table")?;

        // Migration: add debug column to existing databases
        let _ = sqlx::query("ALTER TABLE projects ADD COLUMN debug INTEGER NOT NULL DEFAULT 0")
            .execute(&pool)
            .await;

        // Migration: add project_root_path column
        let _ = sqlx::query(
            "ALTER TABLE projects ADD COLUMN project_root_path TEXT NOT NULL DEFAULT ''",
        )
        .execute(&pool)
        .await;

        // Migration: add bsl_config column
        let _ = sqlx::query(
            "ALTER TABLE projects ADD COLUMN bsl_config TEXT NOT NULL DEFAULT ''",
        )
        .execute(&pool)
        .await;

        Ok(Self { pool })
    }

    pub async fn list_projects(&self) -> Result<Vec<StoredProject>> {
        sqlx::query_as::<_, StoredProject>(
            r#"
            SELECT id, name, root_path, project_root_path, jvm_args, bsl_config, debug, created_at, updated_at
            FROM projects
            ORDER BY created_at ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .context("failed to list projects")
    }

    pub async fn get_project(&self, id: &str) -> Result<Option<StoredProject>> {
        sqlx::query_as::<_, StoredProject>(
            r#"
            SELECT id, name, root_path, project_root_path, jvm_args, bsl_config, debug, created_at, updated_at
            FROM projects
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .context("failed to get project")
    }

    pub async fn insert_project(&self, project: &StoredProject) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO projects (id, name, root_path, project_root_path, jvm_args, bsl_config, debug, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&project.id)
        .bind(&project.name)
        .bind(&project.root_path)
        .bind(&project.project_root_path)
        .bind(&project.jvm_args)
        .bind(&project.bsl_config)
        .bind(project.debug)
        .bind(&project.created_at)
        .bind(&project.updated_at)
        .execute(&self.pool)
        .await
        .context("failed to insert project")?;
        Ok(())
    }

    pub async fn update_project(&self, project: &StoredProject) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE projects
            SET name = ?, root_path = ?, project_root_path = ?, jvm_args = ?, bsl_config = ?, debug = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(&project.name)
        .bind(&project.root_path)
        .bind(&project.project_root_path)
        .bind(&project.jvm_args)
        .bind(&project.bsl_config)
        .bind(project.debug)
        .bind(&project.updated_at)
        .bind(&project.id)
        .execute(&self.pool)
        .await
        .context("failed to update project")?;
        Ok(())
    }

    pub async fn delete_project(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM projects WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .context("failed to delete project")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::Database;
    use crate::models::StoredProject;

    #[tokio::test]
    async fn crud_roundtrip_works() {
        let dir = tempdir().unwrap();
        let db = Database::open(&dir.path().join("data.db")).await.unwrap();

        let project = StoredProject {
            id: "p1".into(),
            name: "Test".into(),
            root_path: "/tmp/project".into(),
            project_root_path: String::new(),
            jvm_args: "-Xmx1g".into(),
            bsl_config: String::new(),
            debug: false,
            created_at: "2026-03-18T00:00:00Z".into(),
            updated_at: "2026-03-18T00:00:00Z".into(),
        };

        db.insert_project(&project).await.unwrap();
        let fetched = db.get_project("p1").await.unwrap().unwrap();
        assert_eq!(fetched.name, "Test");

        db.delete_project("p1").await.unwrap();
        assert!(db.get_project("p1").await.unwrap().is_none());
    }
}
