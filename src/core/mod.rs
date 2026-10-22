//! The policy core that every client reaches agents through
//! (docs/design/20261022_01_ORCHESTRATION_DAEMON_PLAN.md, decisions 4–8):
//! who is asking, which agents exist, and what may be sent to them.

pub mod agents;
pub mod confirm;
pub mod policy;
pub mod principal;
