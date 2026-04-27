use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Many-to-many join between users and workspaces.  v1 has no `role`
/// column — every member has equal access.  Role enum lands in a
/// follow-up spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMember {
    pub workspace_id: String,
    pub user_id: String,
    pub joined_at: DateTime<Utc>,
}
