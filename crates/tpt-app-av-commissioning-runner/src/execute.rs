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
//! Execution modes (§15): automated tests run end to end; manual tests are
//! never executed by software — the runner produces a `Manual` result with
//! the test's checklist and no device is touched; semi-automated tests run
//! their software part and the result is marked pending the engineer's
//! confirmation (see `TestResult::confirm`).
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
    CommissioningTest, ExecutionMode, PendingConfirmation, TestId, TestResult, TestStatus,
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
#[derive(Debug, Clone, PartialEq)]
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

/// What a worker reports back to the coordinator.
enum WorkerResult {
    /// The worker started running the test (dispatch → live progress, §44).
    Started,
    Result(Box<TestResult>),
    Cancelled,
}

/// Result of one execution attempt inside a worker.
enum Attempt {
    Ok(Box<TestResult>),
    Err(String),
    TimedOut,
    Cancelled,
}

/// Executes a planned run with the given options.
pub struct TestExecutor<'a> {
    options: RunOptions,
    tests: HashMap<TestId, Arc<dyn CommissioningTest + Send>>,
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
            .map(|t| (t.id().clone(), Arc::from(t)))
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
        // How many workers were spawned (the cancellation tail waits for
        // exactly one terminal report from each).
        let mut dispatched = 0usize;

        let concurrency = self.options.concurrency.max(1);
        let owner_base = format!("run-{run_id}");
        let tick = self
            .options
            .min_start_interval
            .unwrap_or(Duration::from_millis(1));

        let cancel = self.cancel.clone();

        loop {
            // Drain completed workers first (cheap, maintains progress).
            loop {
                match rx.try_recv() {
                    Ok((id, worker_result)) => {
                        match worker_result {
                            WorkerResult::Started => {
                                self.observer.on_test_started(&id);
                            }
                            WorkerResult::Result(result) => {
                                running.remove(&id);
                                self.observer.on_test_completed(&result);
                                active_mutations = recompute_active_mutations(order, &running);
                                results.insert(id, *result);
                            }
                            WorkerResult::Cancelled => {
                                // Cancelled before running; synthesized later.
                                running.remove(&id);
                                active_mutations = recompute_active_mutations(order, &running);
                            }
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
                    if !dependencies_satisfied(planned, &statuses_of(&results)) {
                        continue;
                    }

                    // Execution policy (§36): a test the project does not
                    // permit is blocked before dispatch — it never runs and
                    // never misreports as a failure.
                    if !self.options.execution_policy.allows(planned.mutation) {
                        let mode = self.tests[id].requirements().mode;
                        let result = TestResult::new(id.clone(), TestStatus::Blocked, mode)
                            .message(format!(
                                "blocked by execution policy: {} changes are not allowed (§36)",
                                planned.mutation.as_str()
                            ));
                        self.observer.on_test_completed(&result);
                        results.insert(id.clone(), result);
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

                    let test_box = Arc::clone(self.tests.get(id).expect("test in map"));

                    let retries = effective_retries(self.options.default_retries, &*test_box);
                    let max_duration = effective_timeout(
                        self.options.timeout,
                        test_box.requirements().max_duration,
                    );

                    let worker_tx = tx.clone();
                    let locking = Arc::clone(&self.locking);
                    let cancel = self.cancel.clone();
                    let owner = format!("{owner_base}.{}", id.as_str());
                    let worker_id = id.clone();
                    thread::spawn(move || {
                        let _ = worker_tx.send((worker_id.clone(), WorkerResult::Started));
                        let result =
                            run_worker_job(test_box, locking, cancel, owner, retries, max_duration);
                        let _ = worker_tx.send((worker_id, result));
                    });

                    running.insert(id.clone());
                    active_mutations = recompute_active_mutations(order, &running);
                    dispatched += 1;

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

        // Cancellation tail: wait for the in-flight workers' terminal reports
        // so locks are released and every dispatched test has an outcome.
        if cancel.is_cancelled() {
            let mut settled = 0usize;
            while settled < dispatched {
                match rx.recv() {
                    Ok((id, WorkerResult::Result(result))) => {
                        settled += 1;
                        results.insert(id, *result);
                    }
                    Ok((_, WorkerResult::Cancelled)) => settled += 1,
                    Ok((_, WorkerResult::Started)) => {}
                    Err(_) => break,
                }
            }
        }

        // A snapshot of what finished, for dependency checks on placeholders.
        let final_map: HashMap<TestId, TestStatus> = order
            .iter()
            .filter_map(|t| results.get(&t.id).map(|r| (t.id.clone(), r.status)))
            .collect();

        // Produce an entry for every scheduled test, in plan order.
        // Synthesized results (never dispatched) are reported through the
        // observer too, so a persisting caller sees every scheduled test.
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
                self.observer.on_test_completed(&result);
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
    test: Arc<dyn CommissioningTest + Send>,
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

    // A manual test is performed by the engineer (§15): software runs
    // nothing, so no device is touched and no lock is taken. The result
    // stays `Manual` — carrying the checklist — until the engineer confirms
    // it (see `TestResult::confirm`).
    if mode == ExecutionMode::Manual {
        let mut result = TestResult::new(id, TestStatus::Manual, mode)
            .message("manual test: awaiting engineer verdict");
        if let Some(checklist) = test.checklist() {
            result.checklist = Some(checklist);
        }
        return WorkerResult::Result(Box::new(result));
    }

    let mut attempts = 0u32;
    loop {
        if cancel.is_cancelled() {
            return WorkerResult::Cancelled;
        }

        let locks = match acquire_locks(&locking, &mutate_devices, &owner, &cancel) {
            Some(locks) => locks,
            None => return WorkerResult::Cancelled,
        };
        let attempt = execute_once_with_timeout(Arc::clone(&test), max_duration, &cancel);
        release_locks(&locking, &locks, &owner);

        match attempt {
            Attempt::Ok(mut result) => {
                // §37 enforcement: a test that declared restoration support
                // always carries the explicit restoration-failure report,
                // even if the implementation forgot to apply it.
                if test.requirements().restores_state {
                    result.apply_restoration_failure();
                }
                let result = if mode == ExecutionMode::SemiAutomated {
                    // The software part is done; the engineer confirms the
                    // measured outcome (§15 semi-automated).
                    await_confirmation(*result, &*test)
                } else {
                    *result
                };
                return WorkerResult::Result(Box::new(result));
            }
            Attempt::Cancelled => return WorkerResult::Cancelled,
            Attempt::TimedOut => {
                if attempts < retries {
                    attempts += 1;
                    continue;
                }
                let message = match max_duration {
                    Some(d) => format!("timed out after {}", describe(d)),
                    None => "timed out".to_owned(),
                };
                return WorkerResult::Result(Box::new(
                    TestResult::new(id.clone(), TestStatus::Fail, mode).message(message),
                ));
            }
            Attempt::Err(error) => {
                if attempts < retries {
                    attempts += 1;
                    continue;
                }
                let mut result = TestResult::new(id.clone(), TestStatus::Fail, mode)
                    .message(error.clone())
                    .message("exhausted retries".to_owned());
                result.error = Some(error);
                return WorkerResult::Result(Box::new(result));
            }
        }
    }
}

/// Turn a finished semi-automated software run into a result awaiting the
/// engineer's confirmation: the measured status is recorded and restored on
/// approval; the reported status becomes `Manual` until then.
fn await_confirmation(result: TestResult, test: &dyn CommissioningTest) -> TestResult {
    let software_status = result.status;
    let prompt = format!("Confirm the result of “{}”", test.name());
    let mut result = result
        .awaiting_confirmation(PendingConfirmation {
            software_status: Some(software_status),
            prompt,
        })
        .message("software part complete — awaiting engineer confirmation");
    result.status = TestStatus::Manual;
    result
}

/// Execute once, honouring an optional deadline. The worker thread is polled
/// at 1 ms granularity; on timeout the thread is abandoned with its own
/// `Arc` clone of the test (its result, if it ever arrives, is discarded —
/// retries run on a fresh clone).
fn execute_once_with_timeout(
    test: Arc<dyn CommissioningTest + Send>,
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
        Ok(Ok(result)) => Attempt::Ok(Box::new(result)),
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
    while !devices
        .iter()
        .all(|d| !locking.lock().unwrap().is_locked(d))
    {
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
fn recompute_active_mutations(
    order: &[PlannedTest],
    running: &HashSet<TestId>,
) -> HashSet<DeviceId> {
    order
        .iter()
        .filter(|t| running.contains(&t.id))
        .flat_map(|t| t.mutate_devices.iter().cloned())
        .collect()
}

/// Whether every dependency has a result with status `Pass` or `Warning`.
fn dependencies_satisfied(test: &PlannedTest, results: &HashMap<TestId, TestStatus>) -> bool {
    test.depends_on.iter().all(|dep| {
        matches!(
            results.get(dep),
            Some(TestStatus::Pass | TestStatus::Warning)
        )
    })
}

/// The current status of every finished test, for dependency checks.
fn statuses_of(results: &HashMap<TestId, TestResult>) -> HashMap<TestId, TestStatus> {
    results
        .iter()
        .map(|(id, result)| (id.clone(), result.status))
        .collect()
}

fn effective_retries(default_retries: u32, test: &dyn CommissioningTest) -> u32 {
    let declared = test.requirements().retries;
    if declared > 0 {
        declared
    } else {
        default_retries
    }
}

fn effective_timeout(
    override_timeout: Option<Duration>,
    declared: Option<Duration>,
) -> Option<Duration> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use tpt_app_av_commissioning_test::{ManualChecklist, TestError, TestRequirements};

    /// What a scripted [`ScriptedTest`] does on one `execute` call.
    #[derive(Clone, Copy)]
    enum Step {
        /// Finish with a status.
        Status(TestStatus),
        /// Return an infra error (drives retry handling).
        Error,
    }

    /// Box a concrete test as the runner's job type.
    fn boxed(t: impl CommissioningTest + 'static) -> Box<dyn CommissioningTest + Send> {
        Box::new(t)
    }

    /// A test whose outcomes are scripted in advance.
    struct ScriptedTest {
        id: TestId,
        name: String,
        requirements: TestRequirements,
        /// Popped per attempt; the last entry repeats once exhausted.
        script: Mutex<Vec<Step>>,
        checklist: Option<ManualChecklist>,
        sleep: Duration,
        runs: AtomicUsize,
    }

    impl ScriptedTest {
        fn passing(id: &str) -> Box<Self> {
            Self::scripted(id, vec![Step::Status(TestStatus::Pass)])
        }

        fn scripted(id: &str, script: Vec<Step>) -> Box<Self> {
            Box::new(Self {
                id: TestId::new(id),
                name: format!("Scripted {id}"),
                requirements: TestRequirements::default(),
                script: Mutex::new(script),
                checklist: None,
                sleep: Duration::ZERO,
                runs: AtomicUsize::new(0),
            })
        }

        fn with_requirements(mut self, requirements: TestRequirements) -> Self {
            self.requirements = requirements;
            self
        }

        fn with_sleep(mut self, sleep: Duration) -> Self {
            self.sleep = sleep;
            self
        }
    }

    impl CommissioningTest for ScriptedTest {
        fn id(&self) -> &TestId {
            &self.id
        }
        fn name(&self) -> &str {
            &self.name
        }
        fn requirements(&self) -> &TestRequirements {
            &self.requirements
        }
        fn checklist(&self) -> Option<ManualChecklist> {
            self.checklist.clone()
        }

        fn execute(&self) -> Result<TestResult, TestError> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            if !self.sleep.is_zero() {
                thread::sleep(self.sleep);
            }
            let mut script = self.script.lock().unwrap();
            let step = if script.len() > 1 {
                script.remove(0)
            } else {
                script
                    .first()
                    .cloned()
                    .expect("scripted test has at least one step")
            };
            drop(script);
            match step {
                Step::Status(status) => Ok(TestResult::new(
                    self.id.clone(),
                    status,
                    self.requirements.mode,
                )),
                Step::Error => Err(TestError::Fixture("scripted failure".to_owned())),
            }
        }
    }

    /// Records every observer event for assertions.
    #[derive(Default)]
    struct Recorder {
        run_started: Mutex<Option<(String, Vec<TestId>)>>,
        started: Mutex<Vec<TestId>>,
        started_at: Mutex<Vec<std::time::Instant>>,
        completed: Mutex<Vec<TestResult>>,
        run_finished: Mutex<Vec<String>>,
    }

    impl RunObserver for Recorder {
        fn on_run_started(&mut self, run_id: &str, order: &[TestId]) {
            *self.run_started.lock().unwrap() = Some((run_id.to_owned(), order.to_vec()));
        }
        fn on_test_started(&mut self, test_id: &TestId) {
            self.started.lock().unwrap().push(test_id.clone());
            self.started_at
                .lock()
                .unwrap()
                .push(std::time::Instant::now());
        }
        fn on_test_completed(&mut self, result: &TestResult) {
            self.completed.lock().unwrap().push(result.clone());
        }
        fn on_run_finished(&mut self, outcome: &RunOutcome) {
            self.run_finished
                .lock()
                .unwrap()
                .push(outcome.run_id.clone());
        }
    }

    fn statuses(outcome: &RunOutcome) -> Vec<(&str, TestStatus)> {
        outcome
            .results
            .iter()
            .map(|r| (r.test_id.as_str(), r.status))
            .collect()
    }

    #[test]
    fn every_scheduled_test_runs_and_reports_in_plan_order() {
        let mut observer = Recorder::default();
        let executor = TestExecutor::new(
            RunOptions::default(),
            vec![
                boxed(*ScriptedTest::passing("a")),
                boxed(*ScriptedTest::passing("b")),
                boxed(*ScriptedTest::passing("c")),
            ],
            &mut observer,
        );
        let outcome = executor.execute().unwrap();

        assert!(!outcome.run_id.is_empty());
        assert_eq!(
            statuses(&outcome),
            vec![
                ("a", TestStatus::Pass),
                ("b", TestStatus::Pass),
                ("c", TestStatus::Pass),
            ]
        );
        let (run_id, order) = observer.run_started.lock().unwrap().clone().unwrap();
        assert_eq!(run_id, outcome.run_id);
        assert_eq!(
            order.iter().map(|t| t.as_str()).collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
        assert_eq!(observer.completed.lock().unwrap().len(), 3);
        assert_eq!(observer.run_finished.lock().unwrap().len(), 1);
        // Started fires once per test, before its completion.
        assert_eq!(observer.started.lock().unwrap().len(), 3);
    }

    #[test]
    fn failed_dependency_blocks_dependents_not_independent_tests() {
        let mut observer = Recorder::default();
        let mut b = *ScriptedTest::passing("b");
        b.requirements.depends_on.push(TestId::new("a"));
        let executor = TestExecutor::new(
            RunOptions::default(),
            vec![
                boxed(*ScriptedTest::scripted(
                    "a",
                    vec![Step::Status(TestStatus::Fail)],
                )),
                boxed(b),
                boxed(*ScriptedTest::passing("c")),
            ],
            &mut observer,
        );
        let outcome = executor.execute().unwrap();

        assert_eq!(
            statuses(&outcome),
            vec![
                ("a", TestStatus::Fail),
                // Plan order: the ready queue runs a and c before b unblocks.
                ("c", TestStatus::Pass),
                ("b", TestStatus::Blocked),
            ]
        );
        let blocked = outcome.result(&TestId::new("b")).unwrap();
        assert!(blocked.messages[0].contains("blocked"));
        // Blocked tests never executed.
        assert_eq!(observer.completed.lock().unwrap().len(), 3);
    }

    #[test]
    fn serial_execution_runs_one_test_at_a_time() {
        let active = Arc::new((AtomicUsize::new(0), AtomicUsize::new(0)));
        let make = |id: &str| {
            let t = ScriptedTest::passing(id).with_sleep(Duration::from_millis(30));
            let counter = Arc::clone(&active);
            boxed(ProbedTest { inner: t, counter })
        };
        let mut observer = Recorder::default();
        let executor = TestExecutor::new(
            RunOptions {
                concurrency: 1,
                ..RunOptions::default()
            },
            vec![make("a"), make("b"), make("c")],
            &mut observer,
        );
        let outcome = executor.execute().unwrap();
        assert!(outcome.results.iter().all(|r| r.status == TestStatus::Pass));
        let (_, max) = (&active.0, &active.1);
        assert_eq!(max.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn parallel_execution_overlaps_independent_tests() {
        let active = Arc::new((AtomicUsize::new(0), AtomicUsize::new(0)));
        let make = |id: &str| {
            let t = ScriptedTest::passing(id).with_sleep(Duration::from_millis(80));
            let counter = Arc::clone(&active);
            boxed(ProbedTest { inner: t, counter })
        };
        let mut observer = Recorder::default();
        let executor = TestExecutor::new(
            RunOptions {
                concurrency: 3,
                ..RunOptions::default()
            },
            vec![make("a"), make("b"), make("c")],
            &mut observer,
        );
        let outcome = executor.execute().unwrap();
        assert!(outcome.results.iter().all(|r| r.status == TestStatus::Pass));
        assert!(
            active.1.load(Ordering::SeqCst) >= 2,
            "tests did not overlap"
        );
    }

    #[test]
    fn mutating_tests_on_one_device_never_overlap() {
        let active = Arc::new((AtomicUsize::new(0), AtomicUsize::new(0)));
        let make = |id: &str| {
            let device = tpt_app_av_commissioning_model::DeviceId::new("matrix-01");
            let req = TestRequirements {
                devices: vec![device.clone()],
                mutate_devices: vec![device],
                ..TestRequirements::default()
            };
            let t = ScriptedTest::passing(id)
                .with_requirements(req)
                .with_sleep(Duration::from_millis(40));
            let counter = Arc::clone(&active);
            boxed(ProbedTest { inner: t, counter })
        };
        let mut observer = Recorder::default();
        let executor = TestExecutor::new(
            RunOptions::default(),
            vec![make("a"), make("b")],
            &mut observer,
        );
        let outcome = executor.execute().unwrap();
        assert!(outcome.results.iter().all(|r| r.status == TestStatus::Pass));
        assert_eq!(active.1.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn timeouts_fail_overrunning_tests() {
        let mut observer = Recorder::default();
        let slow = ScriptedTest::passing("slow")
            .with_sleep(Duration::from_millis(10_000))
            .with_requirements(TestRequirements {
                max_duration: Some(Duration::from_millis(50)),
                ..TestRequirements::default()
            });
        let executor = TestExecutor::new(RunOptions::default(), vec![boxed(slow)], &mut observer);
        let outcome = executor.execute().unwrap();
        let (id, status) = &statuses(&outcome)[0];
        assert_eq!(*id, "slow");
        assert_eq!(*status, TestStatus::Fail);
        let result = outcome.result(&TestId::new("slow")).unwrap();
        assert!(result.messages[0].contains("timed out"));
    }

    #[test]
    fn retries_recover_from_transient_errors() {
        let mut observer = Recorder::default();
        let flaky =
            ScriptedTest::scripted("flaky", vec![Step::Error, Step::Status(TestStatus::Pass)])
                .with_requirements(TestRequirements {
                    retries: 1,
                    ..TestRequirements::default()
                });
        let executor = TestExecutor::new(RunOptions::default(), vec![boxed(flaky)], &mut observer);
        let outcome = executor.execute().unwrap();
        assert_eq!(statuses(&outcome), vec![("flaky", TestStatus::Pass)]);
    }

    #[test]
    fn exhausted_retries_fail_with_error() {
        let mut observer = Recorder::default();
        let broken = ScriptedTest::scripted("broken", vec![Step::Error, Step::Error])
            .with_requirements(TestRequirements {
                retries: 1,
                ..TestRequirements::default()
            });
        let executor = TestExecutor::new(RunOptions::default(), vec![boxed(broken)], &mut observer);
        let outcome = executor.execute().unwrap();
        assert_eq!(statuses(&outcome), vec![("broken", TestStatus::Fail)]);
        let result = outcome.result(&TestId::new("broken")).unwrap();
        assert!(result.error.is_some());
        assert!(result.messages.iter().any(|m| m == "exhausted retries"));
    }

    #[test]
    fn cancellation_before_the_run_skips_everything() {
        let mut observer = Recorder::default();
        let executor = TestExecutor::new(
            RunOptions::default(),
            vec![
                boxed(*ScriptedTest::passing("a")),
                boxed(*ScriptedTest::passing("b")),
            ],
            &mut observer,
        );
        let token = executor.cancel_token();
        token.cancel();
        let outcome = executor.execute().unwrap();

        assert!(outcome.cancelled);
        assert_eq!(
            statuses(&outcome),
            vec![("a", TestStatus::Skipped), ("b", TestStatus::Skipped)]
        );
        for result in &outcome.results {
            assert_eq!(result.messages[0], "cancelled before start");
        }
        // Nothing executed, but every scheduled test is reported (including
        // the synthesized skips) so callers can persist complete runs.
        let completed = observer.completed.lock().unwrap();
        assert_eq!(completed.len(), 2);
        assert!(completed.iter().all(|r| r.status == TestStatus::Skipped));
    }

    #[test]
    fn cancellation_mid_run_lets_in_flight_tests_finish() {
        use std::sync::mpsc;

        let mut observer = Recorder::default();
        let (slow_tx, notify_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        struct SlowCancel {
            id: TestId,
            notify: mpsc::Sender<()>,
            wait: Arc<Mutex<mpsc::Receiver<()>>>,
        }
        impl CommissioningTest for SlowCancel {
            fn id(&self) -> &TestId {
                &self.id
            }
            fn name(&self) -> &str {
                "slow"
            }
            fn requirements(&self) -> &TestRequirements {
                static REQ: std::sync::OnceLock<TestRequirements> = std::sync::OnceLock::new();
                REQ.get_or_init(TestRequirements::default)
            }
            fn execute(&self) -> Result<TestResult, TestError> {
                let _ = self.notify.send(());
                // Blocks until the test lets it finish; bounded so a failure
                // cannot hang the suite.
                let deadline = Instant::now() + Duration::from_secs(5);
                while Instant::now() < deadline {
                    if self.wait.lock().unwrap().try_recv().is_ok() {
                        break;
                    }
                    thread::sleep(Duration::from_millis(1));
                }
                Ok(TestResult::new(
                    self.id.clone(),
                    TestStatus::Pass,
                    ExecutionMode::Automated,
                ))
            }
        }
        let mut queued = *ScriptedTest::passing("queued");
        queued.requirements.depends_on.push(TestId::new("slow"));

        let executor = TestExecutor::new(
            RunOptions {
                concurrency: 1,
                ..RunOptions::default()
            },
            vec![
                boxed(SlowCancel {
                    id: TestId::new("slow"),
                    notify: slow_tx,
                    wait: Arc::new(Mutex::new(release_rx)),
                }),
                boxed(queued),
            ],
            &mut observer,
        );
        // Cancel from a helper thread once `slow` has started, then release it.
        let canceller = {
            let token = executor.cancel_token();
            thread::spawn(move || {
                notify_rx.recv().unwrap();
                token.cancel();
                let _ = release_tx.send(());
            })
        };
        let outcome = executor.execute().unwrap();
        canceller.join().unwrap();

        assert!(outcome.cancelled);
        let slow = outcome.result(&TestId::new("slow")).unwrap();
        let queued = outcome.result(&TestId::new("queued")).unwrap();
        // In-flight work finished and was reported; the queued test never ran.
        assert_eq!(slow.status, TestStatus::Pass);
        assert_eq!(queued.status, TestStatus::Skipped);
    }

    #[test]
    fn rate_limiting_staggers_test_starts() {
        let mut observer = Recorder::default();
        let executor = TestExecutor::new(
            RunOptions {
                concurrency: 3,
                min_start_interval: Some(Duration::from_millis(60)),
                ..RunOptions::default()
            },
            vec![
                boxed(*ScriptedTest::passing("a")),
                boxed(*ScriptedTest::passing("b")),
                boxed(*ScriptedTest::passing("c")),
            ],
            &mut observer,
        );
        let started = Instant::now();
        executor.execute().unwrap();
        let elapsed = started.elapsed();

        // Three tests at a 60 ms minimum start interval force two waits.
        // (Unrate-limited, this run of instant tests finishes in ~ms.)
        assert!(
            elapsed >= Duration::from_millis(120),
            "run was not rate limited: {elapsed:?}"
        );
        // Every test still ran exactly once.
        assert_eq!(observer.completed.lock().unwrap().len(), 3);
    }

    #[test]
    fn execution_policy_blocks_disallowed_mutations() {
        let mut config_change = *ScriptedTest::passing("config-change");
        config_change.requirements.mutation =
            tpt_app_av_commissioning_model::MutationKind::Configuration;
        config_change
            .requirements
            .mutate_devices
            .push(tpt_app_av_commissioning_model::DeviceId::new("d1"));

        let mut observer = Recorder::default();
        let executor = TestExecutor::new(
            RunOptions {
                execution_policy: tpt_app_av_commissioning_model::ExecutionPolicy::read_only(),
                ..RunOptions::default()
            },
            vec![boxed(config_change)],
            &mut observer,
        );
        let outcome = executor.execute().unwrap();

        assert_eq!(
            statuses(&outcome),
            vec![("config-change", TestStatus::Blocked)]
        );
        let result = outcome.result(&TestId::new("config-change")).unwrap();
        assert!(result.messages[0].contains("execution policy"));
        assert!(result.messages[0].contains("configuration"));
    }

    #[test]
    fn permissive_policy_lets_mutating_tests_run() {
        let mut config_change = *ScriptedTest::passing("config-change");
        config_change.requirements.mutation =
            tpt_app_av_commissioning_model::MutationKind::Configuration;
        let mut observer = Recorder::default();
        let executor = TestExecutor::new(
            RunOptions::default(),
            vec![boxed(config_change)],
            &mut observer,
        );
        let outcome = executor.execute().unwrap();
        assert_eq!(
            statuses(&outcome),
            vec![("config-change", TestStatus::Pass)]
        );
    }

    #[test]
    fn restoration_failures_are_enforced_on_declared_tests() {
        /// A test that declares restoration support, passes, but reports a
        /// failed restoration without applying the reporting contract — the
        /// runner must apply it.
        struct SloppyRestore {
            id: TestId,
        }
        impl CommissioningTest for SloppyRestore {
            fn id(&self) -> &TestId {
                &self.id
            }
            fn name(&self) -> &str {
                "sloppy"
            }
            fn requirements(&self) -> &TestRequirements {
                static REQ: std::sync::OnceLock<TestRequirements> = std::sync::OnceLock::new();
                REQ.get_or_init(|| TestRequirements {
                    restores_state: true,
                    ..TestRequirements::default()
                })
            }
            fn execute(&self) -> Result<TestResult, TestError> {
                let mut r =
                    TestResult::new(self.id.clone(), TestStatus::Pass, ExecutionMode::Automated);
                r.restoration_failed = true;
                Ok(r)
            }
        }

        let mut observer = Recorder::default();
        let executor = TestExecutor::new(
            RunOptions::default(),
            vec![boxed(SloppyRestore {
                id: TestId::new("sloppy"),
            })],
            &mut observer,
        );
        let outcome = executor.execute().unwrap();
        let result = outcome.result(&TestId::new("sloppy")).unwrap();
        // §37: explicit report, pass downgraded to warning.
        assert_eq!(result.status, TestStatus::Warning);
        assert!(result.restoration_failed);
        assert!(result
            .messages
            .iter()
            .any(|m| m == "Test passed, but device state restoration failed."));
    }

    #[test]
    fn dry_run_reports_what_would_run_without_executing() {
        let a = ScriptedTest::passing("a");
        let mut observer = Recorder::default();
        let executor = TestExecutor::new(
            RunOptions {
                dry_run: true,
                ..RunOptions::default()
            },
            vec![a as Box<dyn CommissioningTest + Send>],
            &mut observer,
        );
        let outcome = executor.execute().unwrap();
        assert_eq!(statuses(&outcome), vec![("a", TestStatus::Skipped)]);
        let result = outcome.result(&TestId::new("a")).unwrap();
        assert!(result.messages[0].contains("dry run"));
    }

    #[test]
    fn manual_tests_are_never_executed_by_the_runner() {
        let mut manual = *ScriptedTest::passing("manual");
        manual.requirements.mode = ExecutionMode::Manual;
        manual
            .requirements
            .devices
            .push(tpt_app_av_commissioning_model::DeviceId::new(
                "projector-01",
            ));
        manual.checklist = Some(ManualChecklist::new(
            "Image quality",
            ["Focus uniform", "Geometry undistorted"],
        ));

        let mut observer = Recorder::default();
        let executor = TestExecutor::new(RunOptions::default(), vec![boxed(manual)], &mut observer);
        let outcome = executor.execute().unwrap();

        let result = outcome.result(&TestId::new("manual")).unwrap();
        assert_eq!(result.status, TestStatus::Manual);
        assert_eq!(result.mode, ExecutionMode::Manual);
        assert!(result.needs_confirmation());
        let checklist = result.checklist.as_ref().unwrap();
        assert_eq!(checklist.items.len(), 2);
        assert_eq!(checklist.items[0].label, "Focus uniform");
    }

    #[test]
    fn semi_automated_tests_run_software_then_await_confirmation() {
        let mut semi = *ScriptedTest::scripted("semi", vec![Step::Status(TestStatus::Warning)]);
        semi.requirements.mode = ExecutionMode::SemiAutomated;

        let mut observer = Recorder::default();
        let executor = TestExecutor::new(RunOptions::default(), vec![boxed(semi)], &mut observer);
        let outcome = executor.execute().unwrap();

        let result = outcome.result(&TestId::new("semi")).unwrap().clone();
        assert_eq!(result.status, TestStatus::Manual);
        assert_eq!(result.mode, ExecutionMode::SemiAutomated);
        assert!(result.needs_confirmation());
        let pending = result.pending.as_ref().unwrap();
        assert_eq!(pending.software_status, Some(TestStatus::Warning));

        // The engineer's approval restores the measured status.
        let approved = result
            .clone()
            .confirm(
                tpt_app_av_commissioning_test::Confirmation::Approve,
                "J. Doe",
                None,
            )
            .unwrap();
        assert_eq!(approved.status, TestStatus::Warning);
        let rejected = result
            .confirm(
                tpt_app_av_commissioning_test::Confirmation::Reject,
                "J. Doe",
                None,
            )
            .unwrap();
        assert_eq!(rejected.status, TestStatus::Fail);
    }

    /// Wraps a scripted test with a shared concurrency probe so tests can
    /// observe how many executions overlapped.
    struct ProbedTest {
        inner: ScriptedTest,
        counter: Arc<(AtomicUsize, AtomicUsize)>,
    }

    impl CommissioningTest for ProbedTest {
        fn id(&self) -> &TestId {
            self.inner.id()
        }
        fn name(&self) -> &str {
            self.inner.name()
        }
        fn requirements(&self) -> &TestRequirements {
            self.inner.requirements()
        }
        fn execute(&self) -> Result<TestResult, TestError> {
            let now = self.counter.0.fetch_add(1, Ordering::SeqCst) + 1;
            let max = self.counter.1.load(Ordering::SeqCst);
            if now > max {
                self.counter.1.store(now, Ordering::SeqCst);
            }
            let result = self.inner.execute();
            self.counter.0.fetch_sub(1, Ordering::SeqCst);
            result
        }
    }
}
