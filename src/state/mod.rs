use crate::ai::{provider, AiProvider};
use crate::auth::{JwtService, NonceStore};
use crate::cache::redis::{RateLimiter, RedisPool, ScanStatusCache, SessionStore, WebhookDedup};
use crate::database::pool::DbPool;
use crate::database::repositories::notification_repository::NotificationRepositoryImpl;
use crate::database::repositories::*;
use crate::jobs::queue::JobQueue;
use crate::services::{ContractService, GitHubConfig, GitHubService, ScanService};
use std::sync::Arc;

pub struct AppState {
    pub pool: DbPool,
    pub redis: RedisPool,
    pub jwt_service: JwtService,
    pub nonce_store: NonceStore,
    pub queue: JobQueue,
    pub scan_service: ScanService,
    pub session_store: SessionStore,
    pub rate_limiter: RateLimiter,
    pub webhook_dedup: WebhookDedup,
    pub scan_status_cache: ScanStatusCache,
    pub user_repository: UserRepositoryImpl,
    pub organization_repository: OrganizationRepositoryImpl,
    pub project_repository: ProjectRepositoryImpl,
    pub scan_repository: ScanRepositoryImpl,
    pub finding_repository: FindingRepositoryImpl,
    pub report_repository: ReportRepositoryImpl,
    pub api_key_repository: ApiKeyRepositoryImpl,
    pub github_service: Option<GitHubService>,
    pub metrics_handle: metrics_exporter_prometheus::PrometheusHandle,
    pub notification_repository: NotificationRepositoryImpl,
    pub ai_provider: Box<dyn AiProvider>,
    pub contract_service: ContractService,
}

impl AppState {
    pub async fn new(pool: DbPool, redis: RedisPool) -> Self {
        let queue = JobQueue::new(redis.clone());
        let jwt_service = JwtService::from_env();

        let metrics_handle = {
            let builder = metrics_exporter_prometheus::PrometheusBuilder::new();
            builder
                .install_recorder()
                .expect("failed to install Prometheus recorder")
        };

        let scan_status_cache = ScanStatusCache::new(redis.clone());
        let github_service = GitHubConfig::from_env().map(GitHubService::new);
        let github_service_arc = github_service.as_ref().map(|gh| Arc::new(GitHubService::new(gh.config().clone())));

        let scan_service = ScanService::with_cache_and_github(
            pool.clone(),
            queue.clone(),
            ProjectRepositoryImpl::new(pool.clone()),
            scan_status_cache.clone(),
            github_service_arc,
        );

        Self {
            scan_service,
            queue,
            session_store: SessionStore::new(redis.clone()),
            rate_limiter: RateLimiter::new(redis.clone()),
            webhook_dedup: WebhookDedup::new(redis.clone()),
            scan_status_cache,
            nonce_store: NonceStore::new(),
            jwt_service,
            metrics_handle,
            redis,
            user_repository: UserRepositoryImpl::new(pool.clone()),
            organization_repository: OrganizationRepositoryImpl::new(pool.clone()),
            project_repository: ProjectRepositoryImpl::new(pool.clone()),
            scan_repository: ScanRepositoryImpl::new(pool.clone()),
            finding_repository: FindingRepositoryImpl::new(pool.clone()),
            report_repository: ReportRepositoryImpl::new(pool.clone()),
            api_key_repository: ApiKeyRepositoryImpl::new(pool.clone()),
            ai_provider: provider::create_provider(),
            notification_repository: NotificationRepositoryImpl::new(pool.clone()),
            github_service,
            contract_service: ContractService::new(pool.clone()),
            pool,
        }
    }
}
