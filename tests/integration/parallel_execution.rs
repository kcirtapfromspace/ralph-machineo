//! Integration tests for parallel story execution
//!
//! These tests verify that the parallel execution mode works correctly
//! by running the ralph binary with --parallel and --max-concurrency flags.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Test PRD with 3 independent stories (no dependencies, different target files).
/// All stories have passes: true to verify parallel execution completes successfully.
const TEST_PRD_PARALLEL: &str = r#"{
    "project": "ParallelTestProject",
    "branchName": "test/parallel-execution",
    "description": "Test PRD for parallel execution integration test",
    "userStories": [
        {
            "id": "PAR-001",
            "title": "First Independent Story",
            "description": "First story for parallel execution test",
            "acceptanceCriteria": ["Criterion 1"],
            "priority": 1,
            "passes": true,
            "dependsOn": [],
            "targetFiles": ["src/feature_a.rs"]
        },
        {
            "id": "PAR-002",
            "title": "Second Independent Story",
            "description": "Second story for parallel execution test",
            "acceptanceCriteria": ["Criterion 2"],
            "priority": 2,
            "passes": true,
            "dependsOn": [],
            "targetFiles": ["src/feature_b.rs"]
        },
        {
            "id": "PAR-003",
            "title": "Third Independent Story",
            "description": "Third story for parallel execution test",
            "acceptanceCriteria": ["Criterion 3"],
            "priority": 3,
            "passes": true,
            "dependsOn": [],
            "targetFiles": ["src/feature_c.rs"]
        }
    ],
    "parallel": {
        "enabled": true,
        "maxConcurrency": 3,
        "conflictStrategy": "file_based",
        "inferenceMode": "auto"
    }
}"#;

/// Get a Command instance for the ralph binary
#[allow(deprecated)]
fn ralph_cmd() -> Command {
    Command::cargo_bin("ralph").expect("Failed to find ralph binary")
}

/// Test that parallel execution with 3 independent stories completes successfully.
///
/// This test verifies:
/// 1. The --parallel flag is recognized
/// 2. The --max-concurrency flag is recognized
/// 3. All 3 stories complete (passes: true)
/// 4. The run exits successfully (exit code 0)
#[test]
fn test_parallel_execution_with_three_independent_stories() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let prd_path = temp_dir.path().join("prd.json");

    fs::write(&prd_path, TEST_PRD_PARALLEL).expect("Failed to write test PRD");

    // When all stories already pass, the parallel runner completes successfully
    // with exit code 0. This verifies the parallel execution path works.
    ralph_cmd()
        .current_dir(temp_dir.path())
        .arg("--parallel")
        .arg("--max-concurrency")
        .arg("3")
        .timeout(std::time::Duration::from_secs(10))
        .assert()
        .success();
}

/// Test parallel execution using explicit run command.
#[test]
fn test_parallel_execution_with_run_command() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let prd_path = temp_dir.path().join("prd.json");

    fs::write(&prd_path, TEST_PRD_PARALLEL).expect("Failed to write test PRD");

    ralph_cmd()
        .current_dir(temp_dir.path())
        .arg("run")
        .arg("--parallel")
        .arg("--max-concurrency")
        .arg("3")
        .timeout(std::time::Duration::from_secs(10))
        .assert()
        .success();
}

/// Test parallel execution with explicit PRD path.
#[test]
fn test_parallel_execution_with_explicit_prd_path() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let prd_path = temp_dir.path().join("custom_prd.json");

    fs::write(&prd_path, TEST_PRD_PARALLEL).expect("Failed to write test PRD");

    ralph_cmd()
        .current_dir(temp_dir.path())
        .arg("--prd")
        .arg(&prd_path)
        .arg("--parallel")
        .arg("--max-concurrency")
        .arg("3")
        .timeout(std::time::Duration::from_secs(10))
        .assert()
        .success();
}

/// Test parallel execution with max_concurrency set to 1 (effectively sequential).
#[test]
fn test_parallel_execution_with_concurrency_one() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let prd_path = temp_dir.path().join("prd.json");

    fs::write(&prd_path, TEST_PRD_PARALLEL).expect("Failed to write test PRD");

    ralph_cmd()
        .current_dir(temp_dir.path())
        .arg("--parallel")
        .arg("--max-concurrency")
        .arg("1")
        .timeout(std::time::Duration::from_secs(10))
        .assert()
        .success();
}

/// Test parallel execution with default max_concurrency (3).
#[test]
fn test_parallel_execution_with_default_concurrency() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let prd_path = temp_dir.path().join("prd.json");

    fs::write(&prd_path, TEST_PRD_PARALLEL).expect("Failed to write test PRD");

    // Only pass --parallel, use default max_concurrency
    ralph_cmd()
        .current_dir(temp_dir.path())
        .arg("--parallel")
        .timeout(std::time::Duration::from_secs(10))
        .assert()
        .success();
}

/// Test that parallel flag appears in run command help output.
///
/// The `run` subcommand help shows all available options including --parallel
/// and --max-concurrency.
#[test]
fn test_run_command_help_shows_parallel_options() {
    ralph_cmd()
        .args(["run", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--parallel"))
        .stdout(predicate::str::contains("--max-concurrency"));
}

// ============================================================================
// Circuit Breaker Integration Tests
// ============================================================================

/// Test PRD with failing stories to trigger circuit breaker.
/// These stories have passes: false and will require an agent to run,
/// which will fail since no real agent is available in tests.
const TEST_PRD_FAILING_STORIES: &str = r#"{
    "project": "CircuitBreakerTestProject",
    "branchName": "test/circuit-breaker",
    "description": "Test PRD for circuit breaker integration test",
    "userStories": [
        {
            "id": "CB-001",
            "title": "First Failing Story",
            "description": "First story that will fail",
            "acceptanceCriteria": ["Will fail because no agent"],
            "priority": 1,
            "passes": false,
            "dependsOn": [],
            "targetFiles": ["src/feature_a.rs"]
        },
        {
            "id": "CB-002",
            "title": "Second Failing Story",
            "description": "Second story that will fail",
            "acceptanceCriteria": ["Will fail because no agent"],
            "priority": 2,
            "passes": false,
            "dependsOn": [],
            "targetFiles": ["src/feature_b.rs"]
        },
        {
            "id": "CB-003",
            "title": "Third Failing Story",
            "description": "Third story that will fail",
            "acceptanceCriteria": ["Will fail because no agent"],
            "priority": 3,
            "passes": false,
            "dependsOn": [],
            "targetFiles": ["src/feature_c.rs"]
        }
    ],
    "parallel": {
        "enabled": true,
        "maxConcurrency": 3
    }
}"#;

/// Test that circuit breaker threshold option is recognized by the CLI.
///
/// This verifies the --circuit-breaker-threshold flag is properly parsed
/// even when combined with --parallel mode.
#[test]
fn test_parallel_circuit_breaker_threshold_option_recognized() {
    ralph_cmd()
        .args(["run", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--circuit-breaker-threshold"));
}

/// Test that parallel execution with failing stories exits with error
/// when no agent is available.
///
/// This test verifies that when stories fail (because no agent can execute them),
/// the parallel runner properly handles the failure and exits with an error.
#[test]
fn test_parallel_execution_fails_without_agent() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let prd_path = temp_dir.path().join("prd.json");

    fs::write(&prd_path, TEST_PRD_FAILING_STORIES).expect("Failed to write test PRD");

    // With failing stories and no agent, should fail
    ralph_cmd()
        .current_dir(temp_dir.path())
        .arg("--parallel")
        .arg("--max-concurrency")
        .arg("3")
        .arg("--no-checkpoint") // Disable checkpointing for cleaner test
        .timeout(std::time::Duration::from_secs(15))
        .assert()
        .failure(); // Should fail because no agent is available
}

/// Test that parallel execution with custom circuit breaker threshold is accepted.
///
/// This verifies the CLI accepts a custom threshold value with parallel mode.
#[test]
fn test_parallel_with_custom_circuit_breaker_threshold_option() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let prd_path = temp_dir.path().join("prd.json");

    // Use passing stories to verify option is accepted
    fs::write(&prd_path, TEST_PRD_PARALLEL).expect("Failed to write test PRD");

    ralph_cmd()
        .current_dir(temp_dir.path())
        .arg("--parallel")
        .arg("--max-concurrency")
        .arg("3")
        .arg("--circuit-breaker-threshold")
        .arg("3")
        .timeout(std::time::Duration::from_secs(10))
        .assert()
        .success();
}

/// Test that parallel execution with circuit breaker threshold of 1 is accepted.
///
/// A threshold of 1 means the circuit breaker triggers on the first failure.
#[test]
fn test_parallel_with_circuit_breaker_threshold_one() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let prd_path = temp_dir.path().join("prd.json");

    // Use passing stories - we're just testing the option is accepted
    fs::write(&prd_path, TEST_PRD_PARALLEL).expect("Failed to write test PRD");

    ralph_cmd()
        .current_dir(temp_dir.path())
        .arg("--parallel")
        .arg("--circuit-breaker-threshold")
        .arg("1")
        .timeout(std::time::Duration::from_secs(10))
        .assert()
        .success();
}
