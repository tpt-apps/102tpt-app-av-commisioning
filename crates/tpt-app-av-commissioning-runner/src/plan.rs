//! Run planning (§14, §17): dependency-aware ordering and blocked–not-failed
//! status propagation.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tpt_app_av_commissioning_model::{DeviceId, ExecutionPolicy, MutationKind};
use tpt_app_av_commissioning_test::{TestId, TestRequirements, TestStatus};

/// Execution options for a test run (§17).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunOptions {
    /// Maximum number of tests executing at once. `1` = serial execution.
    pub concurrency: usize,
    /// Overrides per-test timeouts when set.
    pub timeout: Option<Duration>,
    /// Default retry count when a test does not declare one.
    pub default_retries: u32,
    /// Minimum wall-clock delay between starting tests (rate limiting, §44).
    pub min_start_interval: Option<Duration>,
    /// What the run may do to the installation (§36). Tests whose mutations
    /// are not permitted are blocked before dispatch — never failed.
    /// Defaults to permissive so the engine only ever narrows what a project
    /// opted into.
    #[serde(default = "permissive_policy")]
    pub execution_policy: ExecutionPolicy,
    /// Apply the execution policy up front and report what would run without
    /// touching devices.
    pub dry_run: bool,
}

fn permissive_policy() -> ExecutionPolicy {
    ExecutionPolicy::permissive()
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            concurrency: 4,
            timeout: None,
            default_retries: 0,
            min_start_interval: None,
            execution_policy: ExecutionPolicy::permissive(),
            dry_run: false,
        }
    }
}

/// A test the planner knows how to schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedTest {
    pub id: TestId,
    pub depends_on: Vec<TestId>,
    pub devices: Vec<DeviceId>,
    pub mutate_devices: Vec<DeviceId>,
    /// What the test changes (execution-policy gating, §36).
    pub mutation: MutationKind,
}

impl PlannedTest {
    /// Build a planned test from a test id and its requirements.
    pub fn from_requirements(id: TestId, requirements: &TestRequirements) -> Self {
        Self {
            id,
            depends_on: requirements.depends_on.clone(),
            devices: requirements.devices.clone(),
            mutate_devices: requirements.mutate_devices.clone(),
            mutation: requirements.mutation,
        }
    }

    /// Whether this test mutates any of the same devices as `other`.
    pub fn shares_mutated_devices(&self, other: &PlannedTest) -> bool {
        self.mutate_devices
            .iter()
            .any(|d| other.mutate_devices.contains(d))
    }
}

/// A fully scheduled run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunPlan {
    /// Tests in execution order (dependency-first). Later tests may still run
    /// in parallel with earlier ones.
    pub order: Vec<PlannedTest>,
}

/// Errors produced while planning a run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    #[error("test dependency `{0}` is not part of the run")]
    UnknownDependency(TestId),
    #[error("the dependency graph contains a cycle")]
    Cycle,
    #[error("duplicate test id `{0}`")]
    DuplicateTest(TestId),
}

/// Builds a dependency-ordered run plan.
#[derive(Debug)]
pub struct RunPlanner {
    tests: Vec<PlannedTest>,
}

impl RunPlanner {
    pub fn new(tests: Vec<PlannedTest>) -> Self {
        Self { tests }
    }

    /// Validate ids and dependencies, then produce a topological order using
    /// Kahn's algorithm. Tests without dependencies come first.
    pub fn plan(&self) -> Result<RunPlan, PlanError> {
        let ids: HashSet<&TestId> = self.tests.iter().map(|t| &t.id).collect();
        if ids.len() != self.tests.len() {
            return Err(PlanError::DuplicateTest(self.tests[0].id.clone()));
        }

        let by_id: HashMap<&TestId, &PlannedTest> = self.tests.iter().map(|t| (&t.id, t)).collect();
        let mut indegree: HashMap<TestId, usize> = HashMap::new();
        let mut dependents: HashMap<TestId, Vec<TestId>> = HashMap::new();

        for t in &self.tests {
            indegree.insert(t.id.clone(), 0);
            for dep in &t.depends_on {
                if !ids.contains(dep) {
                    return Err(PlanError::UnknownDependency(dep.clone()));
                }
                *indegree.entry(t.id.clone()).or_default() += 1;
                dependents
                    .entry(dep.clone())
                    .or_default()
                    .push(t.id.clone());
            }
        }

        let mut ready: Vec<TestId> = indegree
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(id, _)| id.clone())
            .collect();
        ready.sort();

        let mut order: Vec<PlannedTest> = Vec::with_capacity(self.tests.len());
        let mut remaining = self.tests.len();

        while !ready.is_empty() {
            let id = ready.remove(0);
            let test = by_id[&id];
            order.push(test.clone());
            remaining -= 1;
            if let Some(children) = dependents.get(&id) {
                for child in children {
                    let d = indegree.get_mut(child).expect("child in indegree");
                    *d -= 1;
                    if *d == 0 {
                        ready.push(child.clone());
                    }
                }
            }
        }

        if remaining > 0 {
            return Err(PlanError::Cycle);
        }

        Ok(RunPlan { order })
    }

    /// Given the results of executed tests, mark tests whose dependencies did
    /// not pass as `Blocked` (they were never run — never misreport as Fail).
    ///
    /// A dependency "passes" if its status is `Pass` or `Warning`. A skipped
    /// or blocked dependency blocks dependents too.
    pub fn propagate_blocked(
        &self,
        results: &HashMap<TestId, TestStatus>,
    ) -> HashMap<TestId, TestStatus> {
        let by_id: HashMap<&TestId, &PlannedTest> = self.tests.iter().map(|t| (&t.id, t)).collect();
        let passes = |s: &TestStatus| matches!(s, TestStatus::Pass | TestStatus::Warning);

        results
            .iter()
            .map(|(id, status)| {
                let blocked = by_id
                    .get(id)
                    .map(|t| {
                        t.depends_on
                            .iter()
                            .any(|dep| !results.get(dep).map(passes).unwrap_or(false))
                    })
                    .unwrap_or(false);
                let final_status = if blocked {
                    TestStatus::Blocked
                } else {
                    *status
                };
                (id.clone(), final_status)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test(id: &str, deps: &[&str]) -> PlannedTest {
        PlannedTest {
            id: TestId::new(id),
            depends_on: deps.iter().map(|d| TestId::new(*d)).collect(),
            devices: Vec::new(),
            mutate_devices: Vec::new(),
            mutation: tpt_app_av_commissioning_model::MutationKind::None,
        }
    }

    #[test]
    fn serial_order_respects_dependencies() {
        let planner = RunPlanner::new(vec![test("c", &["b"]), test("a", &[]), test("b", &["a"])]);
        let plan = planner.plan().unwrap();
        let ids: Vec<&str> = plan.order.iter().map(|t| t.id.as_str()).collect();
        // a must precede b which must precede c.
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn unknown_dependency_is_rejected() {
        let planner = RunPlanner::new(vec![test("a", &["missing"])]);
        assert_eq!(
            planner.plan().unwrap_err(),
            PlanError::UnknownDependency(TestId::new("missing"))
        );
    }

    #[test]
    fn cycle_is_rejected() {
        let planner = RunPlanner::new(vec![test("a", &["b"]), test("b", &["a"])]);
        assert_eq!(planner.plan().unwrap_err(), PlanError::Cycle);
    }

    #[test]
    fn blocked_propagation_marks_dependents() {
        let planner = RunPlanner::new(vec![test("a", &[]), test("b", &["a"])]);
        let mut results = HashMap::new();
        results.insert(TestId::new("a"), TestStatus::Fail);
        results.insert(TestId::new("b"), TestStatus::Pass);
        let propagated = planner.propagate_blocked(&results);
        assert_eq!(propagated[&TestId::new("a")], TestStatus::Fail);
        assert_eq!(propagated[&TestId::new("b")], TestStatus::Blocked);
    }

    #[test]
    fn passing_dependency_does_not_block() {
        let planner = RunPlanner::new(vec![test("a", &[]), test("b", &["a"])]);
        let mut results = HashMap::new();
        results.insert(TestId::new("a"), TestStatus::Warning);
        results.insert(TestId::new("b"), TestStatus::Pass);
        let propagated = planner.propagate_blocked(&results);
        assert_eq!(propagated[&TestId::new("b")], TestStatus::Pass);
    }

    #[test]
    fn mutating_tests_share_locks() {
        let d = DeviceId::new("matrix-01");
        let mut a = test("a", &[]);
        a.mutate_devices.push(d.clone());
        let mut b = test("b", &[]);
        b.mutate_devices.push(d);
        assert!(a.shares_mutated_devices(&b));
    }
}
