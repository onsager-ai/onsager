use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// An Onsager-native workspace identity.  Owns membership, GitHub
/// installations, and projects.  Identity is owned by Onsager (not borrowed
/// from any external provider), so future source providers can hang off the
/// same workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
}
