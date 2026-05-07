#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use super::super::*;
    use crate::core::{
        GitHubAccountType, GitHubAppInstallation, Project, Session, SessionState, Workspace,
        WorkspaceMember,
    };
    use chrono::Utc;
    use sqlx::pool::PoolOptions;
    use sqlx::AnyPool;
    use uuid::Uuid;

    async fn test_pool() -> AnyPool {
        sqlx::any::install_default_drivers();
        let pool = PoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("failed to connect to sqlite in-memory");
        run_migrations(&pool)
            .await
            .expect("migrations should succeed");
        pool
    }

    async fn seed_user(pool: &AnyPool, user_id: &str) {
        // Derive a stable non-colliding github_id from the user_id bytes.
        let github_id: i64 = user_id.bytes().fold(0i64, |acc, b| acc * 131 + b as i64);
        sqlx::query(
            "INSERT INTO users (id, github_id, github_login, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $4)",
        )
        .bind(user_id)
        .bind(github_id)
        .bind(user_id)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await
        .unwrap();
    }

    fn new_workspace(created_by: &str) -> Workspace {
        Workspace {
            id: Uuid::new_v4().to_string(),
            slug: format!("workspace-{}", Uuid::new_v4().simple()),
            name: "Test Workspace".to_string(),
            created_by: created_by.to_string(),
            created_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn workspace_crud_roundtrip() {
        let pool = test_pool().await;
        seed_user(&pool, "u1").await;

        let workspace = new_workspace("u1");
        insert_workspace(&pool, &workspace).await.unwrap();

        let fetched = get_workspace(&pool, &workspace.id).await.unwrap().unwrap();
        assert_eq!(fetched.id, workspace.id);
        assert_eq!(fetched.slug, workspace.slug);
    }

    #[tokio::test]
    async fn membership_query_and_list_workspaces_for_user() {
        let pool = test_pool().await;
        seed_user(&pool, "u1").await;
        seed_user(&pool, "u2").await;

        let w = new_workspace("u1");
        insert_workspace(&pool, &w).await.unwrap();
        insert_workspace_member(
            &pool,
            &WorkspaceMember {
                workspace_id: w.id.clone(),
                user_id: "u1".to_string(),
                joined_at: Utc::now(),
            },
        )
        .await
        .unwrap();

        assert!(is_workspace_member(&pool, &w.id, "u1").await.unwrap());
        assert!(!is_workspace_member(&pool, &w.id, "u2").await.unwrap());

        let u1_workspaces = list_workspaces_for_user(&pool, "u1").await.unwrap();
        assert_eq!(u1_workspaces.len(), 1);
        assert_eq!(u1_workspaces[0].id, w.id);

        let u2_workspaces = list_workspaces_for_user(&pool, "u2").await.unwrap();
        assert!(u2_workspaces.is_empty());
    }

    #[tokio::test]
    async fn list_workspace_members_with_users_joins_github_profile() {
        let pool = test_pool().await;
        seed_user(&pool, "u1").await;
        let w = new_workspace("u1");
        insert_workspace(&pool, &w).await.unwrap();
        insert_workspace_member(
            &pool,
            &WorkspaceMember {
                workspace_id: w.id.clone(),
                user_id: "u1".to_string(),
                joined_at: Utc::now(),
            },
        )
        .await
        .unwrap();

        let members = list_workspace_members_with_users(&pool, &w.id)
            .await
            .unwrap();
        assert_eq!(members.len(), 1);
        // `seed_user` writes `github_login = user_id`, so this exercises the
        // JOIN without needing to fixture a realistic avatar URL.
        assert_eq!(members[0].user_id, "u1");
        assert_eq!(members[0].github_login.as_deref(), Some("u1"));
    }

    #[tokio::test]
    async fn installation_and_project_crud() {
        let pool = test_pool().await;
        seed_user(&pool, "u1").await;
        let w = new_workspace("u1");
        insert_workspace(&pool, &w).await.unwrap();

        let install = GitHubAppInstallation {
            id: Uuid::new_v4().to_string(),
            workspace_id: w.id.clone(),
            install_id: 42,
            account_login: "acme".to_string(),
            account_type: GitHubAccountType::Organization,
            created_at: Utc::now(),
        };
        insert_github_app_installation(&pool, &install, Some("ciphertext"))
            .await
            .unwrap();

        let installs = list_github_app_installations_for_workspace(&pool, &w.id)
            .await
            .unwrap();
        assert_eq!(installs.len(), 1);
        assert_eq!(installs[0].install_id, 42);

        let project = Project {
            id: Uuid::new_v4().to_string(),
            workspace_id: w.id.clone(),
            github_app_installation_id: install.id.clone(),
            repo_owner: "acme".to_string(),
            repo_name: "widgets".to_string(),
            default_branch: "main".to_string(),
            created_at: Utc::now(),
        };
        insert_project(&pool, &project).await.unwrap();

        let projects = list_projects_for_workspace(&pool, &w.id).await.unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].repo_name, "widgets");
    }

    #[tokio::test]
    async fn delete_project_blocks_on_live_sessions() {
        let pool = test_pool().await;
        seed_user(&pool, "u1").await;
        let w = new_workspace("u1");
        insert_workspace(&pool, &w).await.unwrap();

        let install = GitHubAppInstallation {
            id: Uuid::new_v4().to_string(),
            workspace_id: w.id.clone(),
            install_id: 7,
            account_login: "acme".to_string(),
            account_type: GitHubAccountType::Organization,
            created_at: Utc::now(),
        };
        insert_github_app_installation(&pool, &install, None)
            .await
            .unwrap();

        let project = Project {
            id: Uuid::new_v4().to_string(),
            workspace_id: w.id.clone(),
            github_app_installation_id: install.id.clone(),
            repo_owner: "acme".to_string(),
            repo_name: "widgets".to_string(),
            default_branch: "main".to_string(),
            created_at: Utc::now(),
        };
        insert_project(&pool, &project).await.unwrap();

        let session = Session {
            id: Uuid::new_v4().to_string(),
            task_id: Uuid::new_v4().to_string(),
            node_id: "node-1".to_string(),
            state: SessionState::Running,
            prompt: "hello".to_string(),
            output: None,
            working_dir: None,
            artifact_id: None,
            artifact_version: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        insert_session_with_user_and_project(&pool, &session, Some("u1"), Some(&project.id))
            .await
            .unwrap();

        let live = count_live_sessions_for_project(&pool, &project.id)
            .await
            .unwrap();
        assert_eq!(live, 1);

        // Transition to a terminal state — live count should drop to zero.
        update_session_state(&pool, &session.id, SessionState::Done)
            .await
            .unwrap();
        let live = count_live_sessions_for_project(&pool, &project.id)
            .await
            .unwrap();
        assert_eq!(live, 0);

        delete_project(&pool, &project.id).await.unwrap();
        assert!(get_project(&pool, &project.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn insert_workspace_with_creator_is_atomic() {
        let pool = test_pool().await;
        seed_user(&pool, "u1").await;
        let w = new_workspace("u1");
        let m = WorkspaceMember {
            workspace_id: w.id.clone(),
            user_id: "u1".to_string(),
            joined_at: Utc::now(),
        };
        insert_workspace_with_creator(&pool, &w, &m).await.unwrap();
        assert!(get_workspace(&pool, &w.id).await.unwrap().is_some());
        assert!(is_workspace_member(&pool, &w.id, "u1").await.unwrap());

        // Reusing the same slug must fail and — because the helper uses a
        // transaction — must not create a new member row either.
        let w2 = Workspace {
            id: Uuid::new_v4().to_string(),
            slug: w.slug.clone(),
            ..new_workspace("u1")
        };
        let m2 = WorkspaceMember {
            workspace_id: w2.id.clone(),
            user_id: "u1".to_string(),
            joined_at: Utc::now(),
        };
        assert!(insert_workspace_with_creator(&pool, &w2, &m2)
            .await
            .is_err());
        assert!(get_workspace(&pool, &w2.id).await.unwrap().is_none());
        assert!(!is_workspace_member(&pool, &w2.id, "u1").await.unwrap());
    }

    #[tokio::test]
    async fn count_projects_for_installation_blocks_delete() {
        let pool = test_pool().await;
        seed_user(&pool, "u1").await;
        let w = new_workspace("u1");
        insert_workspace(&pool, &w).await.unwrap();

        let install = GitHubAppInstallation {
            id: Uuid::new_v4().to_string(),
            workspace_id: w.id.clone(),
            install_id: 99,
            account_login: "acme".to_string(),
            account_type: GitHubAccountType::Organization,
            created_at: Utc::now(),
        };
        insert_github_app_installation(&pool, &install, None)
            .await
            .unwrap();

        assert_eq!(
            count_projects_for_installation(&pool, &install.id)
                .await
                .unwrap(),
            0
        );

        let project = Project {
            id: Uuid::new_v4().to_string(),
            workspace_id: w.id.clone(),
            github_app_installation_id: install.id.clone(),
            repo_owner: "acme".to_string(),
            repo_name: "widgets".to_string(),
            default_branch: "main".to_string(),
            created_at: Utc::now(),
        };
        insert_project(&pool, &project).await.unwrap();

        assert_eq!(
            count_projects_for_installation(&pool, &install.id)
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn list_projects_for_user_follows_membership() {
        let pool = test_pool().await;
        seed_user(&pool, "u1").await;
        seed_user(&pool, "u2").await;

        let w1 = new_workspace("u1");
        insert_workspace(&pool, &w1).await.unwrap();
        insert_workspace_member(
            &pool,
            &WorkspaceMember {
                workspace_id: w1.id.clone(),
                user_id: "u1".to_string(),
                joined_at: Utc::now(),
            },
        )
        .await
        .unwrap();

        let install = GitHubAppInstallation {
            id: Uuid::new_v4().to_string(),
            workspace_id: w1.id.clone(),
            install_id: 1,
            account_login: "acme".to_string(),
            account_type: GitHubAccountType::User,
            created_at: Utc::now(),
        };
        insert_github_app_installation(&pool, &install, None)
            .await
            .unwrap();
        let project = Project {
            id: Uuid::new_v4().to_string(),
            workspace_id: w1.id.clone(),
            github_app_installation_id: install.id.clone(),
            repo_owner: "acme".to_string(),
            repo_name: "widgets".to_string(),
            default_branch: "main".to_string(),
            created_at: Utc::now(),
        };
        insert_project(&pool, &project).await.unwrap();

        let u1_projects = list_projects_for_user(&pool, "u1").await.unwrap();
        assert_eq!(u1_projects.len(), 1);

        let u2_projects = list_projects_for_user(&pool, "u2").await.unwrap();
        assert!(u2_projects.is_empty());
    }
}
