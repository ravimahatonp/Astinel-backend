use astinel_backend::services::github::{CheckRunAnnotation, CheckRunOutput};

#[tokio::test]
async fn test_check_run_annotation_formatting() {
    let annotation = CheckRunAnnotation {
        path: "src/contracts/vault.rs".to_string(),
        start_line: 42,
        end_line: 42,
        annotation_level: "failure".to_string(),
        message: "Authorization check missing before state mutation".to_string(),
        title: Some("AUTH_MISSING".to_string()),
    };

    assert_eq!(annotation.start_line, 42);
    assert_eq!(annotation.annotation_level, "failure");
    assert_eq!(annotation.title.as_deref(), Some("AUTH_MISSING"));
}

#[tokio::test]
async fn test_check_run_output_structure() {
    let output = CheckRunOutput {
        title: "Security Scan Completed: 2 Findings".to_string(),
        summary: "Astinel security analysis completed with 2 findings.".to_string(),
        text: Some("Detailed report on security scans".to_string()),
        annotations: vec![
            CheckRunAnnotation {
                path: "src/contracts/token.rs".to_string(),
                start_line: 10,
                end_line: 10,
                annotation_level: "warning".to_string(),
                message: "Integer overflow risk".to_string(),
                title: Some("INTEGER_OVERFLOW".to_string()),
            },
        ],
    };

    assert_eq!(output.annotations.len(), 1);
    assert!(output.title.contains("2 Findings"));
}

#[test]
fn test_milestone_progress_values() {
    let milestones = [
        ("parsing", 20u8),
        ("rule_execution", 50u8),
        ("report_generation", 80u8),
        ("completed", 100u8),
    ];

    for (phase, progress) in milestones {
        assert!(progress > 0 && progress <= 100);
        assert!(!phase.is_empty());
    }
}
