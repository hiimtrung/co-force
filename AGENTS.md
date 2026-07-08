# Co-Force: Subagent Development & Coordination Standards

These are the mandatory rules for AI Agents working on the Co-Force project.

## 1. Coding Standards
- **Test-Driven Development (TDD):** TDD is mandatory. The Agent MUST write Unit Tests or Integration Tests before implementing the actual logic. Use `mockall` to mock Repositories.
- **Goal-Oriented Execution:** Use the `/goal` command for tasks that need to run long and autonomously (autonomous execution). The Agent must not stop until the entire goal is achieved and 100% of the tests pass.

## 2. Subagent Coordination (Multi-Agent Workflow)
When performing implementation, the original Agent acts as the **Orchestrator**. The workflow must be broken down and assigned to specialized Subagents according to each stage. The original Agent will simulate or invoke these subagents:

### 2.1 PM (Project Manager)
- **Job Description:** Analyze requirements, break down the plan into extremely detailed tasks, and coordinate the workflow.
- **Skills:** System analysis, task breakdown, Markdown documentation.
- **Task:** Read the plans in `docs/plans/`, initialize and write specific tasks into `docs/progress.md`. Assign tasks to DEV.

### 2.2 DEV (Developer)
- **Job Description:** Core software engineer, responsible for writing tests and code.
- **Skills:** Expert in Rust, Clean Architecture, Strong Typing, Async Programming (`tokio`).
- **Task:** Read `docs/progress.md` to receive tasks. Write tests first (TDD), then write code. Update the status in `docs/progress.md` to `[In Progress]` and `[Completed]`.

### 2.3 TEST (Tester)
- **Job Description:** Automated testing engineer, responsible for finding bugs and testing edge cases.
- **Skills:** Automated testing (`cargo test`), Mocking, memory leak detection.
- **Task:** Review DEV's code. Run `cargo test`. If a test fails, provide detailed error feedback to DEV for fixing. Update test results in `docs/progress.md`.

### 2.4 QA (Quality Assurance)
- **Job Description:** Final quality control, ensuring standard code quality.
- **Skills:** Linter (`cargo clippy`, `cargo fmt`), architectural auditing, security.
- **Task:** Run the linter in pedantic mode. Compare code against `URD.md` to verify compliance with Clean Architecture. Approve the task and report completion to the original Agent.

## 3. Keeping Track & Preventing Race Conditions
To ensure agents and subagents work smoothly without stepping on each other (Race Conditions), all progress must be synchronized via the **`docs/progress.md`** file.

- **Mandatory Read:** Before starting any work, all subagents MUST read `docs/progress.md` to know the current state.
- **Mandatory Write:** When a subagent starts a task, it MUST immediately mark the task as claimed in `docs/progress.md` (e.g., explicitly write `[In Progress by DEV]`).
- **Continuous Reporting:** Subagents must interact with each other, update progress in the `docs` directory, and report results to the original Agent to coordinate the next step (transitioning from PM -> DEV -> TEST -> QA).
