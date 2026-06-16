use serde::{Deserialize, Serialize};

/// Which Mercurial/Git repository a push lives in.
pub type Repo = String;

/// A specific push (revision + repo).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Push {
    pub revision: String,
    pub repo: Repo,
}

impl Push {
    pub fn new(revision: impl Into<String>, repo: impl Into<String>) -> Self {
        Self {
            revision: revision.into(),
            repo: repo.into(),
        }
    }

    pub fn autoland(revision: impl Into<String>) -> Self {
        Self::new(revision, "autoland")
    }
}

/// Parsed form of any supported input string.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputSpec {
    /// A raw Perfherder alert summary ID.
    Alert { alert_id: u64 },
    /// A Bugzilla bug number.
    Bug { bug_id: u64 },
    /// A pair of revisions from PerfCompare.
    PerfCompare { base: Push, new: Push },
    /// A pair of Lando landing IDs from PerfCompare.
    Lando {
        base_lando_id: String,
        new_lando_id: String,
        base_repo: Repo,
        new_repo: Repo,
    },
    /// A single push (revision + repo).
    Push { push: Push },
}

/// A resolved Perfherder alert summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertSummary {
    pub id: u64,
    pub status: String,
    pub framework: String,
    pub repository: String,
    pub push_id: u64,
    pub prev_push_id: u64,
    pub regressions: Vec<AlertResult>,
    pub improvements: Vec<AlertResult>,
}

/// One alert within a summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertResult {
    pub test: String,
    pub platform: String,
    pub suite: String,
    pub prev_value: f64,
    pub new_value: f64,
    pub delta_percent: f64,
    pub is_regression: bool,
    pub has_profile: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_url: Option<String>,
    pub base_push: Push,
    pub new_push: Push,
}

/// A Treeherder job row (normalised from the column-indexed response).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: u64,
    pub job_type_name: String,
    pub platform: String,
    pub result: String,
    pub task_id: String,
}

/// Taskcluster artifact reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub name: String,
    pub url: String,
    pub task_id: String,
}
