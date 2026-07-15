//! Background daemon for detecting agent disconnects and reclaiming tasks/locks.
//!
//! Implements Plan 03 §5.5 and §6.

use anyhow::Result;
use chrono::Utc;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tokio_rusqlite::Connection;
use tracing::{error, info};

use crate::engine::ports::{
    ActivityRepository, AgentRepository, HandoverRepository, LockRepository, TaskRepository,
};
use crate::orchestration::bus::{WorkspaceEvent, WorkspaceEventBus};
use crate::types::{Agent, AgentState, Handover, TaskStatus, WorkspaceId};

/// Runs the reclaim background daemon loop.
pub async fn run_reclaim_daemon(
    bus: WorkspaceEventBus,
    agent_repo: Arc<dyn AgentRepository>,
    task_repo: Arc<dyn TaskRepository>,
    lock_repo: Arc<dyn LockRepository>,
    activity_repo: Arc<dyn ActivityRepository>,
    handover_repo: Arc<dyn HandoverRepository>,
    conn: Connection,
    workspace_id: WorkspaceId,
    disconnect_timeout: Duration,
    reclaim_timeout: Duration,
    poll_interval: Duration,
) {
    info!(
        "Starting reclaim daemon for workspace: {}",
        workspace_id.as_ref()
    );

    loop {
        sleep(poll_interval).await;

        let now = Utc::now();
        let agents = match agent_repo.list_all(&workspace_id).await {
            Ok(a) => a,
            Err(e) => {
                error!("ReclaimDaemon: failed to list agents: {e}");
                continue;
            }
        };

        for mut agent in agents {
            if agent.state == AgentState::Disconnected {
                // If it is disconnected, check if the grace period has expired
                if let Some(last_seen) = agent.last_seen {
                    if now.signed_duration_since(last_seen)
                        > chrono::Duration::from_std(reclaim_timeout).unwrap_or(chrono::Duration::seconds(120))
                    {
                        info!(
                            "ReclaimDaemon: agent {} disconnected grace period expired. Reclaiming...",
                            agent.agent_id.as_ref()
                        );
                        if let Err(e) = reclaim_agent_resources(
                            &bus,
                            agent_repo.as_ref(),
                            task_repo.as_ref(),
                            lock_repo.as_ref(),
                            activity_repo.as_ref(),
                            handover_repo.as_ref(),
                            &conn,
                            &workspace_id,
                            &agent,
                        )
                        .await
                        {
                            error!(
                                "ReclaimDaemon: failed to reclaim agent {}: {e}",
                                agent.agent_id.as_ref()
                            );
                        }
                    }
                }
            } else if agent.state == AgentState::Idle
                || agent.state == AgentState::Working
                || agent.state == AgentState::Paused
            {
                // If it is active but last_seen is too old, mark as Disconnected
                if let Some(last_seen) = agent.last_seen {
                    if now.signed_duration_since(last_seen)
                        > chrono::Duration::from_std(disconnect_timeout).unwrap_or(chrono::Duration::seconds(30))
                    {
                        info!(
                            "ReclaimDaemon: agent {} inactive. Marking as Disconnected.",
                            agent.agent_id.as_ref()
                        );
                        agent.state = AgentState::Disconnected;
                        if let Err(e) = agent_repo.upsert(&agent).await {
                            error!(
                                "ReclaimDaemon: failed to mark agent {} as disconnected: {e}",
                                agent.agent_id.as_ref()
                            );
                        } else {
                            bus.send(WorkspaceEvent::TaskUpdated {
                                task_id: "".to_string(),
                                new_status: "disconnected".to_string(),
                            });
                        }
                    }
                }
            }
        }
    }
}

/// Reclaims locks and active tasks for a disconnected agent, auto-redispatching if possible.
async fn reclaim_agent_resources(
    bus: &WorkspaceEventBus,
    agent_repo: &dyn AgentRepository,
    task_repo: &dyn TaskRepository,
    lock_repo: &dyn LockRepository,
    activity_repo: &dyn ActivityRepository,
    handover_repo: &dyn HandoverRepository,
    conn: &Connection,
    workspace_id: &WorkspaceId,
    agent: &Agent,
) -> Result<()> {
    // 1. Release locks
    lock_repo
        .release_all_for_agent(workspace_id, &agent.agent_id)
        .await?;

    // 2. Find tasks assigned to this agent
    let tasks = task_repo.list_by_agent(&agent.agent_id).await?;

    // 3. Process each task
    for mut task in tasks {
        if task.status.is_terminal() {
            continue;
        }

        // Find if another online/active agent of a different provider is online
        let active_agents = agent_repo.list_active(workspace_id).await?;
        let target_agent = active_agents.iter().find(|a| {
            a.agent_id != agent.agent_id
                && a.state != AgentState::Disconnected
                && a.provider.is_some()
                && a.provider != agent.provider
        });

        if let Some(target) = target_agent {
            info!(
                "ReclaimDaemon: auto-redispatching task {} to agent {} (provider: {:?})",
                task.task_id.as_ref(),
                target.agent_id.as_ref(),
                target.provider
            );

            // Synthesize handover package from agent activity journal
            let activities = activity_repo.get_agent_activities(&agent.agent_id, 20).await?;
            let mut done_steps = Vec::new();
            for act in activities {
                if act.related_task_id.as_ref() == Some(&task.task_id) {
                    if let Some(ref content) = act.content {
                        if let Some(summary) = content.get("summary").and_then(|s| s.as_str()) {
                            done_steps.push(summary.to_string());
                        }
                    }
                }
            }
            if done_steps.is_empty() {
                done_steps.push("Work started by previous agent".to_string());
            }

            let package = serde_json::json!({
                "reason": "session_end",
                "progress": {
                    "done": done_steps,
                    "remaining": ["Resume task objectives"]
                },
                "decisions": [],
                "gotchas": [],
                "code_state": {
                    "kind": "unknown",
                    "branch": "",
                    "commit_sha": ""
                },
                "next_steps": ["Inspect modified files, run cargo test, resume work"]
            });

            let handover_id = uuid::Uuid::new_v4().to_string();
            let handover = Handover {
                handover_id: handover_id.clone(),
                task_id: task.task_id.clone(),
                from_agent_id: agent.agent_id.clone(),
                to_agent_id: Some(target.agent_id.clone()),
                reason: "session_end".to_string(),
                package,
                provider_cooldown_until: None,
                created_at: Some(Utc::now()),
                accepted_at: None,
            };

            handover_repo.insert_handover(&handover).await?;

            // Update task status to PendingHandover
            task.status = TaskStatus::PendingHandover;
            task_repo.update(&task).await?;

            // Send notification to the target agent inbox via agent_messages
            let conn_clone = conn.clone();
            let msg_id = uuid::Uuid::new_v4().to_string();
            let ws_id = workspace_id.to_string();
            let from_id = agent.agent_id.to_string();
            let to_id = target.agent_id.to_string();
            let payload_str = serde_json::to_string(&handover.package).unwrap_or_default();
            let now_str = Utc::now().to_rfc3339();

            let _ = conn_clone
                .call(move |c| {
                    let res = c.execute(
                        "INSERT INTO agent_messages \
                         (message_id, workspace_id, from_agent_id, to_agent_id, role_filter, \
                          kind, payload, correlation_id, requires_response, created_at) \
                         VALUES (?1, ?2, ?3, ?4, NULL, 'info', ?5, ?6, 1, ?7)",
                        rusqlite::params![
                            msg_id,
                            ws_id,
                            from_id,
                            to_id,
                            payload_str,
                            handover_id,
                            now_str,
                        ],
                    );
                    match res {
                        Ok(_) => Ok(()),
                        Err(e) => Err(tokio_rusqlite::Error::Rusqlite(e)),
                    }
                })
                .await;

            // Broadcast Handover event
            bus.send(WorkspaceEvent::HandoverRequested {
                old_agent_id: agent.agent_id.to_string(),
                task_id: task.task_id.to_string(),
                next_provider: target.provider.clone().unwrap_or_default(),
            });
        } else {
            info!(
                "ReclaimDaemon: no alternative provider agent online. Returning task {} to backlog.",
                task.task_id.as_ref()
            );
            task.status = TaskStatus::Approved;
            task.assigned_agent_id = None;
            task_repo.update(&task).await?;
        }

        // Broadcast task status update
        bus.send(WorkspaceEvent::TaskUpdated {
            task_id: task.task_id.to_string(),
            new_status: task.status.to_string(),
        });
    }

    // Clear current task and keep agent disconnected
    let mut updated_agent = agent.clone();
    updated_agent.current_task_id = None;
    updated_agent.state = AgentState::Disconnected;
    agent_repo.upsert(&updated_agent).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::activity_repo::SqliteActivityRepo;
    use crate::db::agent_repo::SqliteAgentRepo;
    use crate::db::handover_repo::SqliteHandoverRepo;
    use crate::db::lock_repo::SqliteLockRepo;
    use crate::db::task_repo::SqliteTaskRepo;
    use crate::db::Database;
    use crate::types::{ActivityId, ActivityType, AgentActivity, AgentId, FileLock, Task, TaskId};

    async fn setup() -> (
        Database,
        Arc<SqliteAgentRepo>,
        Arc<SqliteTaskRepo>,
        Arc<SqliteLockRepo>,
        Arc<SqliteActivityRepo>,
        Arc<SqliteHandoverRepo>,
        WorkspaceEventBus,
    ) {
        let db = Database::open_in_memory().await.unwrap();
        let agent_repo = Arc::new(SqliteAgentRepo::new(db.conn().clone()));
        let task_repo = Arc::new(SqliteTaskRepo::new(db.conn().clone()));
        let lock_repo = Arc::new(SqliteLockRepo::new(db.conn().clone()));
        let activity_repo = Arc::new(SqliteActivityRepo::new(db.conn().clone()));
        let handover_repo = Arc::new(SqliteHandoverRepo::new(db.conn().clone()));
        let bus = WorkspaceEventBus::new(10);
        (
            db,
            agent_repo,
            task_repo,
            lock_repo,
            activity_repo,
            handover_repo,
            bus,
        )
    }

    #[tokio::test]
    async fn test_reclaim_agent_returns_tasks_to_backlog_if_no_other_agent_online() {
        let (db, agent_repo, task_repo, lock_repo, activity_repo, handover_repo, bus) =
            setup().await;
        let ws_id = WorkspaceId::from("ws-1");
        let agent_id = AgentId::from("agent-claude");

        // 1. Create agent A
        let agent = Agent {
            agent_id: agent_id.clone(),
            workspace_id: ws_id.clone(),
            name: "Claude".to_string(),
            role: "developer".to_string(),
            provider: Some("claude".to_string()),
            machine_id: "m-1".to_string(),
            state: AgentState::Working,
            current_task_id: Some(TaskId::from("task-1")),
            last_seen: Some(Utc::now()),
            created_at: Some(Utc::now()),
        };
        agent_repo.upsert(&agent).await.unwrap();

        // 2. Create task 1
        let task = Task {
            task_id: TaskId::from("task-1"),
            workspace_id: ws_id.clone(),
            title: "Task 1".to_string(),
            objective: None,
            status: TaskStatus::InProgress,
            revision: 1,
            rework_cycle: 0,
            assigned_agent_id: Some(agent_id.clone()),
            delegated_from_agent_id: None,
            parent_task_id: None,
            use_cases: None,
            prerequisites: None,
            verification_plan: None,
            required_skills: None,
            locked_files: None,
            impact_analysis: None,
            priority: 0,
            created_at: None,
            updated_at: None,
            completed_at: None,
        };
        task_repo.insert(&task).await.unwrap();

        // 3. Lock a file
        let lock = FileLock {
            id: None,
            workspace_id: ws_id.clone(),
            file_path: "src/main.rs".to_string(),
            agent_id: agent_id.clone(),
            machine_id: "m-1".to_string(),
            task_id: Some(TaskId::from("task-1")),
            reason: Some("work".to_string()),
            locked_at: None,
            expires_at: None,
        };
        lock_repo.acquire_locks(&[lock]).await.unwrap();

        // 4. Run reclaim
        reclaim_agent_resources(
            &bus,
            agent_repo.as_ref(),
            task_repo.as_ref(),
            lock_repo.as_ref(),
            activity_repo.as_ref(),
            handover_repo.as_ref(),
            db.conn(),
            &ws_id,
            &agent,
        )
        .await
        .unwrap();

        // 5. Verify task goes back to backlog (Approved)
        let t = task_repo.find_by_id(&TaskId::from("task-1")).await.unwrap().unwrap();
        assert_eq!(t.status, TaskStatus::Approved);
        assert!(t.assigned_agent_id.is_none());

        // 6. Verify locks released
        let locks = lock_repo.list_locks(&ws_id).await.unwrap();
        assert!(locks.is_empty());
    }

    #[tokio::test]
    async fn test_reclaim_agent_redispatches_to_other_provider_if_online() {
        let (db, agent_repo, task_repo, lock_repo, activity_repo, handover_repo, bus) =
            setup().await;
        let ws_id = WorkspaceId::from("ws-1");
        let agent_id_a = AgentId::from("agent-claude");
        let agent_id_b = AgentId::from("agent-agy");

        // 1. Create agent A (claude)
        let agent_a = Agent {
            agent_id: agent_id_a.clone(),
            workspace_id: ws_id.clone(),
            name: "Claude".to_string(),
            role: "developer".to_string(),
            provider: Some("claude".to_string()),
            machine_id: "m-1".to_string(),
            state: AgentState::Working,
            current_task_id: Some(TaskId::from("task-1")),
            last_seen: Some(Utc::now()),
            created_at: Some(Utc::now()),
        };
        agent_repo.upsert(&agent_a).await.unwrap();

        // 2. Create agent B (agy)
        let agent_b = Agent {
            agent_id: agent_id_b.clone(),
            workspace_id: ws_id.clone(),
            name: "Agy".to_string(),
            role: "developer".to_string(),
            provider: Some("agy".to_string()),
            machine_id: "m-2".to_string(),
            state: AgentState::Idle,
            current_task_id: None,
            last_seen: Some(Utc::now()),
            created_at: Some(Utc::now()),
        };
        agent_repo.upsert(&agent_b).await.unwrap();

        // 3. Create task 1
        let task = Task {
            task_id: TaskId::from("task-1"),
            workspace_id: ws_id.clone(),
            title: "Task 1".to_string(),
            objective: None,
            status: TaskStatus::InProgress,
            revision: 1,
            rework_cycle: 0,
            assigned_agent_id: Some(agent_id_a.clone()),
            delegated_from_agent_id: None,
            parent_task_id: None,
            use_cases: None,
            prerequisites: None,
            verification_plan: None,
            required_skills: None,
            locked_files: None,
            impact_analysis: None,
            priority: 0,
            created_at: None,
            updated_at: None,
            completed_at: None,
        };
        task_repo.insert(&task).await.unwrap();

        // 4. Log some activity for agent A
        let activity = AgentActivity {
            activity_id: ActivityId::new(),
            workspace_id: ws_id.clone(),
            agent_id: agent_id_a.clone(),
            activity_type: ActivityType::FileEdited,
            content: Some(serde_json::json!({
                "summary": "Edited src/main.rs"
            })),
            related_task_id: Some(TaskId::from("task-1")),
            related_files: None,
            version: 1,
            occurred_at: Utc::now(),
        };
        activity_repo.log_activity(&activity).await.unwrap();

        // 5. Run reclaim
        reclaim_agent_resources(
            &bus,
            agent_repo.as_ref(),
            task_repo.as_ref(),
            lock_repo.as_ref(),
            activity_repo.as_ref(),
            handover_repo.as_ref(),
            db.conn(),
            &ws_id,
            &agent_a,
        )
        .await
        .unwrap();

        // 6. Verify task status is PendingHandover
        let t = task_repo.find_by_id(&TaskId::from("task-1")).await.unwrap().unwrap();
        assert_eq!(t.status, TaskStatus::PendingHandover);

        // 7. Verify handover record exists
        let handover = handover_repo.find_pending_for_task(&TaskId::from("task-1")).await.unwrap().unwrap();
        assert_eq!(handover.from_agent_id, agent_id_a);
        assert_eq!(handover.to_agent_id, Some(agent_id_b.clone()));
        assert_eq!(handover.reason, "session_end");

        let done_steps = handover.package.get("progress").unwrap().get("done").unwrap().as_array().unwrap();
        assert_eq!(done_steps[0].as_str().unwrap(), "Edited src/main.rs");

        // 8. Verify agent B received an inbox message in agent_messages
        let has_msg: bool = db.conn().call(move |c| {
            let count: i64 = c.query_row(
                "SELECT count(*) FROM agent_messages WHERE to_agent_id = 'agent-agy' AND kind = 'info'",
                [],
                |row| row.get(0)
            )?;
            Ok(count > 0)
        }).await.unwrap();
        assert!(has_msg);
    }
}

