use crate::cache::redis::ScanStatusCache;
use crate::database::models::ScanJob;
use crate::database::pool::DbPool;
use crate::database::repositories::{ProjectRepositoryImpl, ScanRepository, ScanRepositoryImpl};
use crate::jobs::queue::{JobQueue, QueuedJob};
use crate::jobs::scan_job::ScanJobExecutor;
use crate::jobs::status::JobStatus;
use crate::services::github::GitHubService;
use std::sync::Arc;
use tracing::instrument;
use uuid::Uuid;

pub struct ScanService {
    pool: DbPool,
    queue: JobQueue,
    executor: Arc<ScanJobExecutor>,
}

impl ScanService {
    pub fn new(pool: DbPool, queue: JobQueue, project_repo: ProjectRepositoryImpl) -> Self {
        Self {
            executor: Arc::new(ScanJobExecutor::new(pool.clone(), project_repo)),
            pool,
            queue,
        }
    }

    pub fn with_cache_and_github(
        pool: DbPool,
        queue: JobQueue,
        project_repo: ProjectRepositoryImpl,
        cache: ScanStatusCache,
        github: Option<Arc<GitHubService>>,
    ) -> Self {
        let executor = ScanJobExecutor::new(pool.clone(), project_repo).with_cache_and_github(cache, github);
        Self {
            executor: Arc::new(executor),
            pool,
            queue,
        }
    }

    pub fn executor(&self) -> Arc<ScanJobExecutor> {
        self.executor.clone()
    }

    #[instrument(skip(self), fields(project_id = %project_id))]
    pub async fn enqueue_scan(
        &self,
        project_id: Uuid,
        branch: String,
    ) -> Result<ScanJob, crate::api::errors::ApiError> {
        self.enqueue_scan_with_config(project_id, branch, serde_json::json!({})).await
    }

    #[instrument(skip(self), fields(project_id = %project_id))]
    pub async fn enqueue_scan_with_config(
        &self,
        project_id: Uuid,
        branch: String,
        config: serde_json::Value,
    ) -> Result<ScanJob, crate::api::errors::ApiError> {
        let repo = ScanRepositoryImpl::new(self.pool.clone());

        let job = ScanJob::new(
            project_id,
            branch.clone(),
            String::new(),
            "queued".to_string(),
            "manual".to_string(),
            config.clone(),
            0,
        );

        let saved = repo.create_job(&job).await?;

        let queued = QueuedJob {
            id: saved.id,
            project_id: saved.project_id,
            branch: saved.branch.clone(),
            status: JobStatus::Queued,
            config,
        };

        self.queue.enqueue(queued).await.map_err(|_| {
            crate::api::errors::ApiError::Internal("Failed to enqueue scan".to_string())
        })?;

        Ok(saved)
    }

    pub async fn execute_direct(&self, project_id: Uuid, branch: String) -> Result<(), String> {
        let job = QueuedJob {
            id: Uuid::new_v4(),
            project_id,
            branch,
            status: JobStatus::Queued,
            config: serde_json::json!({}),
        };
        self.executor.execute(job).await
    }
}
