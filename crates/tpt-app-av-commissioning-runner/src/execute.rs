//! Test executor (§17): dependency-aware concurrent execution with device
//! locking, timeouts, retries, cancellation, and rate limiting.
//!
//! Tests are dispatched on worker threads while a coordinator honours the
//! dependency order from [`RunPlanner`]. A test starts only when:
//!
//! * all of its dependencies have completed with `Pass`/`Warning`,
//! * it does not read a device another running test is mutating, and
//! * it can lock every device it mutates (see [`crate::lock`]).
//!
//! Results are reported through the [`RunObserver`] so callers can persist
//! them (§17 result persistence) and drive progress reporting (§44).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tpt_app_av_commissioning_model::DeviceId;
use tpt_app_av_commissioning_test::{
    CommissioningTest, ExecutionMode, TestError, TestId, TestResult, TestStatus,
};
use uuid::Uuid;

use crate::lock::{DeviceLock, LockRegistry};
use crate::plan::{PlanError, PlannedTest, RunOptions, RunPlanner};

/// A cooperative cancellation handle for a run in progress.
#[derive(Debug, Clone, Default)]
pub struct CancelToken {
    cancelled: Arc<AtomicBool>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation. Tests already running see this between retries,
    /// lock waits, and (optionally) time-limited polls.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// Progress notifications. The default implementations are no-ops.
pub trait RunObserver {
    fn on_run_started(&mut self, _run_id: &str, _order: &[TestId]) {}
    fn on_test_started(&mut self, _test_id: &TestId) {}
    fn on_test_completed(&mut self, _result: &TestResult) {}
    fn on_run_finished(&mut self, _outcome: &RunOutcome) {}
}

/// Errors produced by [`TestExecutor`].
#[derive(Debug, thiserror::Error)]
pub enum ExecutionError {
    #[error("run was cancelled")]
    Cancelled,
    #[error("scheduling failed: {0}")]
    Plan(#[from] PlanError),
}

/// A finished run: every scheduled test has a result (including synthesized
/// `Blocked`/`Skipped` entries for tests that never executed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome {
    pub run_id: String,
    /// One result per scheduled test, in plan order.
    pub results: Vec<TestResult>,
    /// Whether cancellation was requested during the run.
    pub cancelled: bool,
}

impl RunOutcome {
    /// Look up the result for a test id.
    pub fn result(&self, test_id: &TestId) -> Option<&TestResult> {
        self.results.iter().find(|r| &r.test_id == test_id)
    }
}

/// What a worker reports back after finishing a job.
enum WorkerResult {
    Result(TestResult),
    Cancelled,
}

/// Result of one execution attempt inside a worker.
enum Attempt {
    Ok(TestResult),
    Err(String),
    TimedOut,
    Cancelled,
}

/// Executes a planned run with the given options.
pub struct TestExecutor<'a> {
    options: RunOptions,
    tests: HashMap<TestId, Box<dyn CommissioningTest + Send>>,
    observer: &'a mut dyn RunObserver,
    locking: Arc<Mutex<LockRegistry>>,
    cancel: CancelToken,
}

impl<'a> TestExecutor<'a> {
    pub fn new(
        options: RunOptions,
        tests: impl IntoIterator<Item = Box<dyn CommissioningTest + Send>>,
        observer: &'a mut dyn RunObserver,
    ) -> Self {
        let tests = tests
            .into_iter()
            .map(|t| (t.id().clone(), t))
            .collect();
        Self {
            options,
            tests,
            observer,
            locking: Arc::new(Mutex::new(LockRegistry::new())),
            cancel: CancelToken::new(),
        }
    }

    pub fn cancel_token(&self) -> CancelToken {
        self.cancel.clone()
    }

    /// Non-blocking acquire for coordination; may fail.
    fn can_acquire(&self, device: &DeviceId) -> bool {
        !self.locking.lock().unwrap().is_locked(device)
    }

    /// Run the whole run to completion (or cancellation).
    pub fn execute(mut self) -> Result<RunOutcome, ExecutionError> {
        let planned: Vec<PlannedTest> = self
            .tests
            .iter()
            .map(|(id, t)| PlannedTest::from_requirements(id.clone(), t.requirements()))
            .collect();
        let order = RunPlanner::new(planned).plan()?.order;

        let run_id = Uuid::new_v4().to_string();
        let order_ids: Vec<TestId> = order.iter().map(|t| t.id.clone()).collect();
        self.observer.on_run_started(&run_id, &order_ids);

        let outcome = if self.options.dry_run {
            self.dry_run(&run_id, &order)
        } else {
            self.execute_plan(&run_id, &order)
        };

        self.observer.on_run_finished(&outcome);
        Ok(outcome)
    }

    fn dry_run(&mut self, run_id: &str, order: &[PlannedTest]) -> RunOutcome {
        let mut results = Vec::with_capacity(order.len());
        for planned in order {
            let requirements = self.tests[&planned.id].requirements();
            self.observer.on_test_started(&planned.id);
            let result =
                TestResult::new(planned.id.clone(), TestStatus::Skipped, requirements.mode)
                    .message("dry run: scheduled but not executed");
            self.observer.on_test_completed(&result);
            results.push(result);
        }
        RunOutcome {
            run_id: run_id.to_owned(),
            results,
            cancelled: false,
        }
    }

    fn execute_plan(&mut self, run_id: &str, order: &[PlannedTest]) -> RunOutcome {
        let (tx, rx) = mpsc::channel();

        let mut results: HashMap<TestId, TestResult> = HashMap::new();
        let mut running: HashSet<TestId> = HashSet::new();
        let mut active_mutations: HashSet<DeviceId> = HashSet::new();

        let concurrency = self.options.concurrency.max(1);
        let owner_base = format!("run-{run_id}");
        let tick = self.options.min_start_interval.unwrap_or(Duration::from_millis(1));

        let cancel = self.cancel.clone();

        loop {
            // Drain completed workers first (cheap, maintains progress).
            loop {
                match rx.try_recv() {
                    Ok(worker_result) => {
                        let (id, result) = worker_result;
                        running.remove(&id);
                        if let WorkerResult::Result(result) = result {
                            self.observer.on_test_started(&id);
                            self.observer.on_test_completed(&result);
                            active_mutations = recompute_active_mutations(order, &running);
                            results.insert(id, result);
                        } else {
                            // Cancelled before running; synthesized later.
                            active_mutations = recompute_active_mutations(order, &running);
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => break,
                }
            }

            if cancel.is_cancelled() {
                break;
            }

            let slots = concurrency.saturating_sub(running.len());
            if slots > 0 {
                for planned in order {
                    if running.len() >= concurrency {
                        break;
                    }
                    let id = &planned.id;
                    if running.contains(id) || results.contains_key(id) {
                        continue;
                    }
                    if !dependencies_satisfied(planned, &results) {
                        continue;
                    }

                    let requirements = self.tests[id].requirements();

                    // Never read a device that a running test is mutating.
                    if requirements
                        .devices
                        .iter()
                        .any(|d| active_mutations.contains(d))
                    {
                        continue;
                    }
                    // Locks for every device we mutate must be obtainable now.
                    if requirements
                        .mutate_devices
                        .iter()
                        .any(|d| !self.can_acquire(d))
                    {
                        continue;
                    }

                    let mut test_box = self.tests.remove(id).expect("test in map");
                    drop(requirements);

                    let retries = effective_retries(self.options.default_retries, &test_box);
                    let max_duration = effective_timeout(
                        self.options.timeout,
                        test_box.requirements().max_duration,
                    );

                    let worker_tx = tx.clone();
                    let locking = Arc::clone(&self.locking);
                    let cancel = self.cancel.clone();
                    let owner = format!("{owner_base}.{}", id.as_str());
                    thread::spawn(move || {
                        let result = run_worker_job(
                            test_box,
                            locking,
                            cancel,
                            owner,
                            retries,
                            max_duration,
                        );
                        let _ = worker_tx.send((id.clone(), result));
                    });

                    running.insert(id.clone());
                    active_mutations = recompute_active_mutations(order, &running);

                    if let Some(interval) = self.options.min_start_interval {
                        thread::sleep(interval);
                    }
                }
            }

            if running.is_empty() {
                // Either everything is done, or nothing left is dispatchable
                // (blocked behind a failed dependency). Exit the loop.
                break;
            }

            thread::sleep(tick);
        }

        // Cancellation tail: wait for in-flight workers so locks are released.
        if cancel.is_cancelled() {
            while let Ok(result) = rx.recv() {
                let (id, worker_result) = result;
                running.remove(&id);
                if let WorkerResult::Result(result) = worker_result {
                    results.insert(id, result);
                } else {
                    // Cancelled before running; synthesized below.
                }
            }
        }

        // A snapshot of what finished, for dependency checks on placeholders.
        let final_map: HashMap<TestId, TestStatus> = order
            .iter()
            .filter_map(|t| results.get(&t.id).map(|r| (t.id.clone(), r.status)))
            .collect();

        // Produce an entry for every scheduled test, in plan order.
        let mut outcome_results = Vec::with_capacity(order.len());
        for planned in order {
            let id = &planned.id;
            if let Some(result) = results.remove(id) {
                outcome_results.push(result);
            } else {
                let mode = self.tests[id].requirements().mode;
                let result = if dependencies_satisfied(planned, &final_map) {
                    cancelled_result(id, mode, &cancel)
                } else {
                    blocked_result(id, mode)
                };
                outcome_results.push(result);
            }
        }

        RunOutcome {
            run_id: run_id.to_owned(),
            results: outcome_results,
            cancelled: cancel.is_cancelled(),
        }
    }
}

/// Tests a worker runs after acquiring its device locks.
fn run_worker_job(
    test: Box<dyn CommissioningTest + Send>,
    locking: Arc<Mutex<LockRegistry>>,
    cancel: CancelToken,
    owner: String,
    retries: u32,
    max_duration: Option<Duration>,
) -> WorkerResult {
    let id = test.id().clone();
    let mode = test.requirements().mode;
    let mutate_devices = test.requirements().mutate_devices.clone();

    if cancel.is_cancelled() {
        return WorkerResult::Cancelled;
    }

    let mut attempt = 0u32;
    loop {
        if cancel.is_cancelled() {
            return WorkerResult::Cancelled;
        }

        let locks = match acquire_locks(&locking, &mutate_devices, &owner, &cancel) {
            Some(locks) => locks,
            None => return WorkerResult::Cancelled,
        };
        let attempt = execute_once_with_timeout(&*test, max_duration, &cancel);
        release_locks(&locking, &locks, &owner);

        match attempt {
            Attempt::Ok(result) => return WorkerResult::Result(result),
            Attempt::Cancelled => return WorkerResult::Cancelled,
            Attempt::TimedOut => {
                if attempt < retries {
                    attempt += 1;
                    continue;
                }
                let message = match max_duration {
                    Some(d) => format!("timed out after {}", describe(d)),
                    None => "timed out".to_owned(),
                };
                return WorkerResult::Result(
                    TestResult::new(id.clone(), TestStatus::Fail, mode).message(message),
                );
            }
            Attempt::Err(error) => {
                if attempt < retries {
                    attempt += 1;
                    continue;
                }
                let mut result = TestResult::new(id.clone(), TestStatus::Fail, mode)
                    .message(error.clone())
                    .message("exhausted retries".to_owned());
                result.error = Some(error);
                return WorkerResult::Result(result);
            }
        }
    }
}

/// Execute once, honouring an optional deadline. The worker thread is polled
/// at 1 ms granularity; on timeout the thread is abandoned (its result, if it
/// ever arrives, is discarded).
fn execute_once_with_timeout(
    test: &dyn CommissioningTest,
    max_duration: Option<Duration>,
    cancel: &CancelToken,
) -> Attempt {
    let handle = thread::spawn(move || test.execute());

    if let Some(deadline) = max_duration {
        let deadline = Instant::now() + deadline;
        while !handle.is_finished() {
            if Instant::now() >= deadline {
                return Attempt::TimedOut;
            }
            thread::sleep(Duration::from_millis(1));
        }
    } else {
        while !handle.is_finished() && !cancel.is_cancelled() {
            thread::sleep(Duration::from_millis(1));
        }
        if !handle.is_finished() {
            return Attempt::Cancelled;
        }
    }

    match handle.join() {
        Ok(Ok(result)) => Attempt::Ok(result),
        Ok(Err(error)) => Attempt::Err(error.to_string()),
        Err(_) => Attempt::Err("test panicked".to_owned()),
    }
}

/// Acquire locks for every device in `devices`, waiting for contention
/// (devices are never locked for long: runs are bounded by timeouts).
fn acquire_locks(
    locking: &Arc<Mutex<LockRegistry>>,
    devices: &[DeviceId],
    owner: &str,
    cancel: &CancelToken,
) -> Option<Vec<DeviceLock>> {
    let mut locks = Vec::with_capacity(devices.len());
    while !devices.iter().all(|d| !locking.lock().unwrap().is_locked(d)) {
        if cancel.is_cancelled() {
            return None;
        }
        thread::sleep(Duration::from_millis(1));
    }
    for device in devices {
        match locking.lock().unwrap().acquire(device.clone(), owner) {
            Ok(lock) => locks.push(lock),
            Err(_) => {
                // Raced with another worker; release what we hold and retry.
                release_locks(locking, &locks, owner);
                return acquire_locks(locking, devices, owner, cancel);
            }
        }
    }
    Some(locks)
}

fn release_locks(locking: &Arc<Mutex<LockRegistry>>, locks: &[DeviceLock], owner: &str) {
    let mut registry = locking.lock().unwrap();
    for lock in locks {
        let _ = registry.release(lock, owner);
    }
}

/// Which devices currently being mutated by `running` tests.
fn recompute_active_mutations(order: &[PlannedTest], running: &HashSet<TestId>) -> HashSet<DeviceId> {
    order
        .iter()
        .filter(|t| running.contains(&t.id))
        .flat_map(|t| t.mutate_devices.iter().cloned())
        .collect()
}

/// Whether every dependency has a result with status `Pass` or `Warning`.
fn dependencies_satisfied(
    test: &PlannedTest,
    results: &HashMap<TestId, TestStatus>,
) -> bool {
    test.depends_on
        .iter()
        .all(|dep| matches!(results.get(dep), Some(TestStatus::Pass | TestStatus::Warning)))
}

fn effective_retries(default_retries: u32, test: &dyn CommissioningTest) -> u32 {
    let declared = test.requirements().retries;
    if declared > 0 {
        declared
    } else {
        default_retries
    }
}

fn effective_timeout(override_timeout: Option<Duration>, declared: Option<Duration>) -> Option<Duration> {
    override_timeout.or(declared)
}

fn blocked_result(id: &TestId, mode: ExecutionMode) -> TestResult {
    TestResult::new(id.clone(), TestStatus::Blocked, mode)
        .message("blocked: a dependency did not pass (blocked-not-failed, §14)")
}

fn cancelled_result(id: &TestId, mode: ExecutionMode, cancel: &CancelToken) -> TestResult {
    let message = if cancel.is_cancelled() {
        "cancelled before start"
    } else {
        "not run"
    };
    TestResult::new(id.clone(), TestStatus::Skipped, mode).message(message)
}

fn describe(duration: Duration) -> String {
    format!("{:.0} ms", duration.as_millis())
}