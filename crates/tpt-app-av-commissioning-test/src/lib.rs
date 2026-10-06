//! TPT AV Commissioning — test model and schema.
//!
//! The `CommissioningTest` trait, `TestStatus`, `TestResult`, requirement and
//! procedure types. See `docs/test-model.md` and §12–16 of `spec.txt`.
//!
//! Licensed under either of MIT OR Apache-2.0, at your option.

pub mod check;
pub mod defect_link;
pub mod definition;
pub mod kinds;
pub mod manual;
pub mod net;
pub mod procedure;
pub mod result;
pub mod status;
pub mod test_kind;

pub use check::{evaluate, Check, Evaluation, Expectation};
pub use defect_link::defect_from_result;
pub use definition::{CommissioningTest, ExecutionMode, TestError, TestId, TestRequirements};
pub use kinds::{
    share, CommandTest, ConnectivityProbe, ConnectivityTest, SharedDriver, StateCheckTest,
};
pub use manual::{
    ChecklistError, ChecklistItem, ChecklistVerdict, Confirmation, ConfirmationError,
    ManualChecklist, ManualTest, PendingConfirmation,
};
pub use net::{RequiredPortsTest, TcpReachableTest};
pub use procedure::{ChecklistEntry, ProcedureStep, TestProcedure};
pub use result::TestResult;
pub use status::TestStatus;
pub use test_kind::{classify_field, TestKind};
pub use tpt_app_av_commissioning_model::MutationKind;
