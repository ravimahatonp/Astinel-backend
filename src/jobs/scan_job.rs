use crate::cache::redis::ScanStatusCache;
use crate::database::models::ScanResult;
use crate::database::pool::DbPool;
use crate::database::repositories::{
    FindingRepository, FindingRepositoryImpl, ProjectRepository, ProjectRepositoryImpl,
    ScanRepository, ScanRepositoryImpl,
};
use crate::jobs::queue::QueuedJob;
use crate::scanner::{DefaultScanner, ScanRequest, Scanner};
use crate::services::github::{CheckRunAnnotation, CheckRunOutput, GitHubService};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, instrument};

pub struct ScanJobExecutor {
    pool: DbPool,
    project_repo: ProjectRepositoryImpl,
    scanner: DefaultScanner,
    scan_status_cache: Option<ScanStatusCache>,
    github_service: Option<Arc<GitHubService>>,
}

impl ScanJobExecutor {
    pub fn new(pool: DbPool, project_repo: ProjectRepositoryImpl) -> Self {
        Self {
            pool,
            project_repo,
            scanner: DefaultScanner,
            scan_status_cache: None,
            github_service: None,
        }
    }

    pub fn with_cache_and_github(
        mut self,
        cache: ScanStatusCache,
        github: Option<Arc<GitHubService>>,
    ) -> Self {
        self.scan_status_cache = Some(cache);
        self.github_service = github;
        self
    }

    #[instrument(skip(self, job), fields(job_id = %job.id, project_id = %job.project_id))]
    pub async fn execute(&self, job: QueuedJob) -> Result<(), String> {
        info!("Executing scan job {}", job.id);

        let scan_repo = ScanRepositoryImpl::new(self.pool.clone());
        let finding_repo = FindingRepositoryImpl::new(self.pool.clone());

        let mut db_job = scan_repo
            .find_job_by_id(job.id)
            .await
            .map_err(|e| format!("Failed to find job: {}", e))?
            .ok_or_else(|| "Job not found".to_string())?;
        db_job.status = "running".to_string();
        scan_repo
            .update_job(&db_job)
            .await
            .map_err(|e| format!("Update failed: {}", e))?;

        // Extract GitHub check-run metadata if triggered by commit/PR
        let check_run_id = job.config.get("check_run_id").and_then(|v| v.as_i64());
        let installation_id = job.config.get("installation_id").and_then(|v| v.as_i64());
        let repo_owner = job.config.get("owner").and_then(|v| v.as_str());
        let repo_name = job.config.get("repo").and_then(|v| v.as_str());

        // Helper closure to report progress to both Redis cache and GitHub API
        let report_milestone = |phase: &str, progress: u8| {
            let cache = self.scan_status_cache.clone();
            let github = self.github_service.clone();
            let scan_id = job.id.to_string();
            let phase_str = phase.to_string();

            async move {
                if let Some(c) = &cache {
                    let _ = c.set_progress(&scan_id, progress, &phase_str).await;
                }

                if let (Some(gh), Some(c_id), Some(i_id), Some(o), Some(r)) =
                    (github, check_run_id, installation_id, repo_owner, repo_name)
                {
                    let _ = gh
                        .update_check_run_progress(
                            i_id,
                            o,
                            r,
                            c_id,
                            "in_progress",
                            &phase_str,
                            progress,
                            None,
                        )
                        .await;
                }
            }
        };

        // Milestone 1: Parsing
        report_milestone("parsing", 20).await;

        // Resolve scan target from the project's local_path
        let project = self
            .project_repo
            .find_by_id(job.project_id)
            .await
            .map_err(|e| format!("Failed to find project: {}", e))?
            .ok_or_else(|| format!("Project {} not found", job.project_id))?;

        let scan_target = project
            .local_path
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!("/tmp/astinel-workspace/{}", project.slug)));

        // Milestone 2: Rule execution
        report_milestone("rule_execution", 50).await;

        let request = ScanRequest::builder().target(&scan_target).build();

        let result = match self.scanner.scan(request) {
            Ok(r) => r,
            Err(e) => {
                db_job.status = "failed".to_string();
                db_job.error_message = Some(e.to_string());
                scan_repo
                    .update_job(&db_job)
                    .await
                    .map_err(|e| format!("Update failed: {}", e))?;

                if let (Some(gh), Some(c_id), Some(i_id), Some(o), Some(r)) = (
                    self.github_service.clone(),
                    check_run_id,
                    installation_id,
                    repo_owner,
                    repo_name,
                ) {
                    let out = CheckRunOutput {
                        title: "Scan Failed".to_string(),
                        summary: format!("Scan execution encountered an error: {}", e),
                        text: None,
                        annotations: vec![],
                    };
                    let _ = gh
                        .update_check_run_progress(
                            i_id,
                            o,
                            r,
                            c_id,
                            "completed",
                            "failed",
                            100,
                            Some(out),
                        )
                        .await;
                }
                return Err(format!("Scan failed: {}", e));
            }
        };

        // Milestone 3: Report generation
        report_milestone("report_generation", 80).await;

        let scan_result = ScanResult::new(job.id, "completed".to_string());
        let saved_result = scan_repo
            .create_result(&scan_result)
            .await
            .map_err(|e| format!("Failed to save result: {}", e))?;

        let mut annotations = Vec::new();

        for finding in &result.core.findings {
            let db_finding = crate::database::models::Finding::new(
                saved_result.id,
                finding.rule_id.as_str().to_string(),
                format!("{:?}", finding.severity),
                format!("{:?}", finding.category),
                finding.span.file.to_string_lossy().to_string(),
                finding.span.line as i32,
                finding.span.column as i32,
                finding.message.clone(),
                finding.recommendation.clone(),
            );
            let _ = finding_repo.create(&db_finding).await;

            let level = match format!("{:?}", finding.severity).to_lowercase().as_str() {
                "critical" | "high" => "failure",
                "medium" => "warning",
                _ => "notice",
            };

            annotations.push(CheckRunAnnotation {
                path: finding.span.file.to_string_lossy().to_string(),
                start_line: finding.span.line as u32,
                end_line: finding.span.line as u32,
                annotation_level: level.to_string(),
                message: finding.message.clone(),
                title: Some(finding.rule_id.as_str().to_string()),
            });
        }

        db_job.status = "completed".to_string();
        scan_repo
            .update_job(&db_job)
            .await
            .map_err(|e| format!("Update failed: {}", e))?;

        // Final Milestone: Completed
        if let Some(c) = &self.scan_status_cache {
            let _ = c.set_progress(&job.id.to_string(), 100, "completed").await;
        }

        if let (Some(gh), Some(c_id), Some(i_id), Some(o), Some(r)) = (
            self.github_service.clone(),
            check_run_id,
            installation_id,
            repo_owner,
            repo_name,
        ) {
            let findings_count = result.core.findings.len();
            let summary = format!(
                "Astinel security analysis completed. Found {} potential findings.",
                findings_count
            );
            let output = CheckRunOutput {
                title: format!("Security Scan Completed: {} Findings", findings_count),
                summary,
                text: None,
                annotations,
            };
            let _ = gh
                .update_check_run_progress(
                    i_id,
                    o,
                    r,
                    c_id,
                    "completed",
                    "completed",
                    100,
                    Some(output),
                )
                .await;
        }

        info!(
            "Scan job {} completed with {} findings",
            job.id,
            result.core.findings.len()
        );
        Ok(())
    }
}
