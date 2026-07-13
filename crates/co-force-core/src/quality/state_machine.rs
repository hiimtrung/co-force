//! Quality Gate State Machine (Plan 07 §3).
//!
//! Pure function — no side effects, no DB calls.
//! Deterministic gate validation for task transitions.
//!
//! This is the central rule engine that prevents:
//! - Skipping verification gates
//! - Self-reviews (same agent reviewing own work)
//! - Infinite rework loops
//! - Completing tasks without passing evidence

use serde::{Deserialize, Serialize};

use crate::types::{AgentId, TaskStatus};

// ---------------------------------------------------------------------------
// Quality Policy (workspace-level configuration)
// ---------------------------------------------------------------------------

/// Workspace quality policy — defines thresholds and requirements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityPolicy {
    pub reviews_required: u8,
    /// "agent" (distinct identity) | "provider" (distinct provider)
    pub reviewer_must_differ: String,
    pub require_recheck: bool,
    pub require_verification_evidence: bool,
    pub required_evidence_kinds: Vec<String>,
    pub critique_fanout: u8,
    pub max_rework_cycles: u8,
    pub definition_of_done: Vec<String>,
}

impl Default for QualityPolicy {
    fn default() -> Self {
        Self {
            reviews_required: 1,
            reviewer_must_differ: "agent".to_string(),
            require_recheck: true,
            require_verification_evidence: true,
            required_evidence_kinds: vec!["test".to_string()],
            critique_fanout: 2,
            max_rework_cycles: 3,
            definition_of_done: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Gate Violation — describes why a transition was rejected
// ---------------------------------------------------------------------------

/// Describes why a state machine transition was rejected by a quality gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateViolation {
    /// Agent attempting to self-review their own work.
    SelfReview { agent_id: String },
    /// Task has no verification evidence before code_review gate.
    MissingVerificationEvidence { required_kinds: Vec<String> },
    /// Rework cycle limit exceeded — task is escalated to blocked.
    MaxReworkExceeded { cycles: u8, limit: u8 },
    /// Invalid status transition per the state machine.
    InvalidTransition { from: String, to: String },
    /// Reviewer does not meet the differ policy.
    ReviewerPolicyViolation { reason: String },
    /// Required review count not met.
    InsufficientReviews { required: u8, provided: u8 },
    /// Task marked completed without passing all gates.
    IncompleteGates { gates: Vec<String> },
}

impl std::fmt::Display for GateViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SelfReview { agent_id } => {
                write!(f, "GATE_VIOLATION: Agent {agent_id} cannot review their own work. Separation of duties required.")
            }
            Self::MissingVerificationEvidence { required_kinds } => {
                write!(
                    f,
                    "GATE_VIOLATION: Missing verification evidence. Required kinds: {}. \
                     Call co_force_submit_verification with real test output.",
                    required_kinds.join(", ")
                )
            }
            Self::MaxReworkExceeded { cycles, limit } => {
                write!(
                    f,
                    "GATE_VIOLATION: Rework cycle limit exceeded ({cycles}/{limit}). \
                     Task will be escalated to blocked. User intervention required."
                )
            }
            Self::InvalidTransition { from, to } => {
                write!(
                    f,
                    "GATE_VIOLATION: Transition from {from} to {to} is not allowed by the state machine."
                )
            }
            Self::ReviewerPolicyViolation { reason } => {
                write!(f, "GATE_VIOLATION: Reviewer policy violation — {reason}")
            }
            Self::InsufficientReviews { required, provided } => {
                write!(
                    f,
                    "GATE_VIOLATION: Insufficient reviews. Required: {required}, provided: {provided}."
                )
            }
            Self::IncompleteGates { gates } => {
                write!(
                    f,
                    "GATE_VIOLATION: Cannot complete task — gates not passed: {}",
                    gates.join(", ")
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Evidence summary (passed from verification_records)
// ---------------------------------------------------------------------------

/// Summary of verification evidence for gate checking.
#[derive(Debug, Clone)]
pub struct EvidenceSummary {
    pub has_passing_test: bool,
    pub kinds_present: Vec<String>,
    pub commit_sha: Option<String>,
}

impl EvidenceSummary {
    pub fn none() -> Self {
        Self {
            has_passing_test: false,
            kinds_present: Vec::new(),
            commit_sha: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Review summary (passed from reviews table)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ReviewSummary {
    pub reviewer_agent_id: AgentId,
    pub reviewer_provider: Option<String>,
    pub verdict: String, // "approved" | "changes_requested"
}

// ---------------------------------------------------------------------------
// State Machine — transition function
// ---------------------------------------------------------------------------

/// Context needed by the state machine to evaluate gates.
#[derive(Debug, Clone)]
pub struct TransitionContext<'a> {
    /// The agent requesting the transition.
    pub agent_id: &'a AgentId,
    /// The agent that authored the work (for self-review gate).
    pub author_agent_id: Option<&'a AgentId>,
    /// Provider of the requesting agent.
    pub agent_provider: Option<&'a str>,
    /// Provider of the author agent (for provider-level differ gate).
    pub author_provider: Option<&'a str>,
    /// Current rework cycle count.
    pub rework_cycle: u8,
    /// Verification evidence summary.
    pub evidence: Option<&'a EvidenceSummary>,
    /// Reviews submitted for this task.
    pub reviews: &'a [ReviewSummary],
    /// Workspace quality policy.
    pub policy: &'a QualityPolicy,
}

/// Validates a task status transition through the quality gate state machine.
///
/// This is a pure function — all state is passed via context.
/// Returns `Err(GateViolation)` if a gate blocks the transition.
/// Returns `Ok(target_status)` if the transition is allowed.
pub fn validate_transition(
    from: &TaskStatus,
    to: &TaskStatus,
    ctx: &TransitionContext<'_>,
) -> Result<TaskStatus, GateViolation> {
    use TaskStatus::*;

    // 1. Basic state machine transition validity
    let basic_ok = matches!(
        (from, to),
        (Draft, SpecReview)
            | (Draft, Cancelled)
            | (SpecReview, Draft)
            | (SpecReview, AwaitingApproval)
            | (AwaitingApproval, Approved)
            | (AwaitingApproval, Draft)
            | (AwaitingApproval, Cancelled)
            | (Approved, InProgress)
            | (Approved, Cancelled)
            | (InProgress, Verification)
            | (InProgress, Blocked)
            | (InProgress, PendingHandover)
            | (InProgress, Cancelled)
            | (Verification, InProgress)
            | (Verification, CodeReview)
            | (CodeReview, Rework)
            | (CodeReview, Completed)
            | (Rework, InProgress)
            | (Blocked, InProgress)
            | (Blocked, Cancelled)
            | (PendingHandover, InProgress)
            | (PendingHandover, Approved)
    ) || (from == to); // self-loop for progress notes

    if !basic_ok {
        return Err(GateViolation::InvalidTransition {
            from: format!("{from:?}"),
            to: format!("{to:?}"),
        });
    }

    // 2. Gate: InProgress → Verification requires evidence
    if matches!((from, to), (InProgress, Verification)) {
        if ctx.policy.require_verification_evidence {
            let default_evidence = EvidenceSummary::none();
            let ev = ctx.evidence.unwrap_or(&default_evidence);
            if !ev.has_passing_test {
                return Err(GateViolation::MissingVerificationEvidence {
                    required_kinds: ctx.policy.required_evidence_kinds.clone(),
                });
            }
        }
    }

    // 3. Gate: Verification → CodeReview requires evidence
    if matches!((from, to), (Verification, CodeReview)) {
        if ctx.policy.require_verification_evidence {
            let default_evidence = EvidenceSummary::none();
            let ev = ctx.evidence.unwrap_or(&default_evidence);
            let missing_kinds: Vec<String> = ctx
                .policy
                .required_evidence_kinds
                .iter()
                .filter(|k| !ev.kinds_present.contains(k))
                .cloned()
                .collect();

            if !missing_kinds.is_empty() {
                return Err(GateViolation::MissingVerificationEvidence {
                    required_kinds: missing_kinds,
                });
            }
        }
    }

    // 4. Gate: CodeReview → Completed requires N reviews with approved verdict
    if matches!((from, to), (CodeReview, Completed)) {
        let approved_reviews: Vec<&ReviewSummary> = ctx
            .reviews
            .iter()
            .filter(|r| r.verdict == "approved")
            .collect();

        if (approved_reviews.len() as u8) < ctx.policy.reviews_required {
            return Err(GateViolation::InsufficientReviews {
                required: ctx.policy.reviews_required,
                provided: approved_reviews.len() as u8,
            });
        }

        // Check each reviewer vs policy
        for review in &approved_reviews {
            check_reviewer_policy(review, ctx)?;
        }
    }

    // 5. Gate: CodeReview → Rework increments rework cycle; check limit
    if matches!((from, to), (CodeReview, Rework)) {
        let next_cycle = ctx.rework_cycle + 1;
        if next_cycle > ctx.policy.max_rework_cycles {
            return Err(GateViolation::MaxReworkExceeded {
                cycles: next_cycle,
                limit: ctx.policy.max_rework_cycles,
            });
        }
    }

    Ok(to.clone())
}

/// Checks that the reviewer satisfies the differ policy.
fn check_reviewer_policy(
    review: &ReviewSummary,
    ctx: &TransitionContext<'_>,
) -> Result<(), GateViolation> {
    let reviewer_id = &review.reviewer_agent_id;

    // Self-review check (always — regardless of policy)
    if let Some(author_id) = ctx.author_agent_id {
        if reviewer_id == author_id {
            return Err(GateViolation::SelfReview {
                agent_id: reviewer_id.to_string(),
            });
        }
    }

    match ctx.policy.reviewer_must_differ.as_str() {
        "provider" => {
            // Reviewer must be from a different provider than the author
            if let (Some(reviewer_prov), Some(author_prov)) =
                (&review.reviewer_provider, ctx.author_provider)
            {
                if reviewer_prov == author_prov {
                    return Err(GateViolation::ReviewerPolicyViolation {
                        reason: format!(
                            "reviewer_must_differ=provider: reviewer and author both use '{reviewer_prov}'"
                        ),
                    });
                }
            }
        }
        _ => {} // "agent" — already checked above
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Unit Tests (TDD)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgentId, TaskStatus};

    fn default_policy() -> QualityPolicy {
        QualityPolicy::default()
    }

    fn approved_review(reviewer: &str) -> ReviewSummary {
        ReviewSummary {
            reviewer_agent_id: AgentId::from(reviewer),
            reviewer_provider: Some("claude".to_string()),
            verdict: "approved".to_string(),
        }
    }

    #[test]
    fn test_valid_transition_draft_to_spec_review() {
        let author = AgentId::from("author-1");
        let agent = AgentId::from("pm-1");
        let policy = default_policy();
        let ctx = TransitionContext {
            agent_id: &agent,
            author_agent_id: Some(&author),
            agent_provider: None,
            author_provider: None,
            rework_cycle: 0,
            evidence: None,
            reviews: &[],
            policy: &policy,
        };
        let result = validate_transition(&TaskStatus::Draft, &TaskStatus::SpecReview, &ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_invalid_transition_draft_to_in_progress() {
        let agent = AgentId::from("agent-1");
        let policy = default_policy();
        let ctx = TransitionContext {
            agent_id: &agent,
            author_agent_id: None,
            agent_provider: None,
            author_provider: None,
            rework_cycle: 0,
            evidence: None,
            reviews: &[],
            policy: &policy,
        };
        let result = validate_transition(&TaskStatus::Draft, &TaskStatus::InProgress, &ctx);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            GateViolation::InvalidTransition { .. }
        ));
    }

    #[test]
    fn test_gate_verification_requires_test_evidence() {
        let agent = AgentId::from("agent-1");
        let policy = default_policy(); // require_verification_evidence = true
        let ctx = TransitionContext {
            agent_id: &agent,
            author_agent_id: None,
            agent_provider: None,
            author_provider: None,
            rework_cycle: 0,
            evidence: None, // No evidence!
            reviews: &[],
            policy: &policy,
        };
        let result = validate_transition(&TaskStatus::InProgress, &TaskStatus::Verification, &ctx);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            GateViolation::MissingVerificationEvidence { .. }
        ));
    }

    #[test]
    fn test_gate_verification_passes_with_test_evidence() {
        let agent = AgentId::from("agent-1");
        let policy = default_policy();
        let evidence = EvidenceSummary {
            has_passing_test: true,
            kinds_present: vec!["test".to_string()],
            commit_sha: Some("abc123".to_string()),
        };
        let ctx = TransitionContext {
            agent_id: &agent,
            author_agent_id: None,
            agent_provider: None,
            author_provider: None,
            rework_cycle: 0,
            evidence: Some(&evidence),
            reviews: &[],
            policy: &policy,
        };
        let result = validate_transition(&TaskStatus::InProgress, &TaskStatus::Verification, &ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_gate_code_review_to_completed_requires_review() {
        let agent = AgentId::from("agent-1");
        let policy = default_policy(); // reviews_required = 1
        let evidence = EvidenceSummary {
            has_passing_test: true,
            kinds_present: vec!["test".to_string()],
            commit_sha: None,
        };
        let ctx = TransitionContext {
            agent_id: &agent,
            author_agent_id: None,
            agent_provider: None,
            author_provider: None,
            rework_cycle: 0,
            evidence: Some(&evidence),
            reviews: &[], // No reviews!
            policy: &policy,
        };
        let result = validate_transition(&TaskStatus::CodeReview, &TaskStatus::Completed, &ctx);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            GateViolation::InsufficientReviews {
                required: 1,
                provided: 0
            }
        ));
    }

    #[test]
    fn test_gate_self_review_blocked() {
        let author = AgentId::from("author-1");
        let policy = default_policy();
        let evidence = EvidenceSummary {
            has_passing_test: true,
            kinds_present: vec!["test".to_string()],
            commit_sha: None,
        };
        // Reviewer is the same as author
        let self_review = ReviewSummary {
            reviewer_agent_id: author.clone(),
            reviewer_provider: Some("claude".to_string()),
            verdict: "approved".to_string(),
        };
        let ctx = TransitionContext {
            agent_id: &author,
            author_agent_id: Some(&author),
            agent_provider: Some("claude"),
            author_provider: Some("claude"),
            rework_cycle: 0,
            evidence: Some(&evidence),
            reviews: &[self_review],
            policy: &policy,
        };
        let result = validate_transition(&TaskStatus::CodeReview, &TaskStatus::Completed, &ctx);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            GateViolation::SelfReview { .. }
        ));
    }

    #[test]
    fn test_gate_code_review_to_completed_passes_with_valid_review() {
        let author = AgentId::from("author-1");
        let reviewer = AgentId::from("reviewer-1");
        let policy = default_policy();
        let evidence = EvidenceSummary {
            has_passing_test: true,
            kinds_present: vec!["test".to_string()],
            commit_sha: None,
        };
        let review = approved_review("reviewer-1");
        let ctx = TransitionContext {
            agent_id: &reviewer,
            author_agent_id: Some(&author),
            agent_provider: Some("claude"),
            author_provider: Some("agy"),
            rework_cycle: 0,
            evidence: Some(&evidence),
            reviews: &[review],
            policy: &policy,
        };
        let result = validate_transition(&TaskStatus::CodeReview, &TaskStatus::Completed, &ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_gate_max_rework_exceeded() {
        let agent = AgentId::from("agent-1");
        let policy = default_policy(); // max_rework_cycles = 3
        let ctx = TransitionContext {
            agent_id: &agent,
            author_agent_id: None,
            agent_provider: None,
            author_provider: None,
            rework_cycle: 3, // At limit
            evidence: None,
            reviews: &[],
            policy: &policy,
        };
        let result = validate_transition(&TaskStatus::CodeReview, &TaskStatus::Rework, &ctx);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            GateViolation::MaxReworkExceeded {
                cycles: 4,
                limit: 3
            }
        ));
    }

    #[test]
    fn test_gate_rework_allowed_within_limit() {
        let agent = AgentId::from("agent-1");
        let policy = default_policy(); // max_rework_cycles = 3
        let ctx = TransitionContext {
            agent_id: &agent,
            author_agent_id: None,
            agent_provider: None,
            author_provider: None,
            rework_cycle: 2, // Below limit
            evidence: None,
            reviews: &[],
            policy: &policy,
        };
        let result = validate_transition(&TaskStatus::CodeReview, &TaskStatus::Rework, &ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_gate_provider_differ_policy() {
        let author = AgentId::from("author-1");
        let reviewer = AgentId::from("reviewer-2");
        let mut policy = default_policy();
        policy.reviewer_must_differ = "provider".to_string(); // Must differ by provider
        let evidence = EvidenceSummary {
            has_passing_test: true,
            kinds_present: vec!["test".to_string()],
            commit_sha: None,
        };

        // Reviewer using same provider as author → violation
        let review = ReviewSummary {
            reviewer_agent_id: reviewer.clone(),
            reviewer_provider: Some("claude".to_string()),
            verdict: "approved".to_string(),
        };
        let ctx = TransitionContext {
            agent_id: &reviewer,
            author_agent_id: Some(&author),
            agent_provider: Some("claude"),
            author_provider: Some("claude"), // Same provider!
            rework_cycle: 0,
            evidence: Some(&evidence),
            reviews: &[review],
            policy: &policy,
        };
        let result = validate_transition(&TaskStatus::CodeReview, &TaskStatus::Completed, &ctx);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            GateViolation::ReviewerPolicyViolation { .. }
        ));
    }
}
