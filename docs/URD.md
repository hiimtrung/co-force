# User Requirements Document (URD)
## Project Co-Force: Centralized Agent Orchestration & Canvas Swarm Platform

### 1. Vision & Objectives
Project Co-Force is an enterprise-grade centralized agent orchestration and execution platform. It utilizes the **Agent2Agent (A2A) Protocol** for cross-agent coordination and the **Model Context Protocol (MCP)** as the primary layer for knowledge retrieval, grounding, and system capabilities (skills).

The platform decouples workflow design and monitoring from execution nodes, allowing operators to:
*   **Visually Configure Workflows:** Design complex multi-agent execution graphs using a drag-and-drop Canvas UI (e.g., SvelteFlow/ReactFlow) that compiles directly into executable **LangGraph** workflows.
*   **Orchestrate and Monitor via A2A:** Use a centralized client to coordinate, trace, and audit agents and sub-agents communicating through A2A network protocols.
*   **Empower Agents via MCP:** Supply agents and sub-agents with verified data context, business truths, documents, database access, and executable skills through dedicated MCP servers.

---

### 2. Target Audience & Personas
*   **The Workflow Designer (Developer/Solutions Architect):** Wants to visually connect specialized agents, configure routing conditions, test outputs, and deploy graphs without writing orchestrator boilerplate.
*   **The Operations Monitor (SRE/Swarm Auditor):** Wants real-time transparency of agent-to-agent negotiations, sub-task performance, token costs, and live execution traces.
*   **The Agent Developer:** Writes specialized agent runtimes, packs their tools/context as MCP servers, publishes their capability as an `AgentCard`, and registers them to the central manager.

---

### 3. Core User Stories & Use Cases

#### US-1: Canvas-Based Workflow Design & Compiler
*   **As a** Workflow Designer,
*   **I want to** use a visual canvas interface to drop agent nodes, connect inputs/outputs, and define logic branches,
*   **So that** I can automatically compile the layout into a runnable LangGraph workflow.
*   **Acceptance Criteria:**
    *   The canvas displays registered agents from the registry as draggable nodes.
    *   Connection points validate that data schemas match between output and input sockets.
    *   The compiled JSON output can be executed by the LangGraph runner.

#### US-2: Centralized Orchestration, Coordination & Tracing
*   **As a** Operations Monitor,
*   **I want to** watch the live execution of a workflow from a single control client, tracking tasks delegated from the master agent to sub-agents,
*   **So that** I can inspect payload details, negotiation outcomes, and errors at each hop.
*   **Acceptance Criteria:**
    *   The client streams execution events (negotiations, progress updates, task completion) from all involved agents.
    *   The user can pause, step through, or terminate active runs.
    *   Full execution logs are stored and available for audit playback.

#### US-3: A2A-Driven Task Delegation
*   **As a** Parent Agent,
*   **I want to** discover specialized sub-agents via their published `AgentCard` and delegate sub-tasks using A2A payloads,
*   **So that** I do not need tight compile-time coupling with worker implementations.
*   **Acceptance Criteria:**
    *   Agents check capability mappings in the central registry.
    *   Task requests and progress updates follow a standardized, secure network protocol (A2A).

#### US-4: MCP as the Knowledge & Skill Engine
*   **As an** Execution Agent,
   *   **I want to** connect to standardized MCP servers to read vector search data (knowledge), fetch system statuses (grounding truths), and invoke tools (skills),
*   **So that** my prompt context remains grounded, verified, and capable of operating on target environments.
*   **Acceptance Criteria:**
    *   Agents dynamically load available skills, database queries, and tools from configured MCP endpoints.
    *   All external side effects (writing files, run scripts, database edits) are mediated through MCP.

---

### 4. Functional Requirements

| Req ID | Title | Description | Priority |
| :--- | :--- | :--- | :--- |
| **FR-1** | Centralized Agent Registry | The system must host an active directory of registered agents, containing their identity, location, and published `AgentCard` schemas. | P0 |
| **FR-2** | Visual Canvas UI | A node-based UI editor allowing users to assemble, edit, and export agent execution paths. | P0 |
| **FR-3** | LangGraph Workflow Compiler | The engine must parse canvas configurations and execute them as LangGraph state machines. | P0 |
| **FR-4** | A2A Protocol Implementation | Core engine must support standardized task negotiation, delegation message passing, and status streams between parent/sub-agents. | P0 |
| **FR-5** | MCP Knowledge & Skill Layer | Sub-agents must utilize MCP connections to ground their logic in real-time information systems and trigger external actions securely. | P0 |
| **FR-6** | Centralized Monitoring Stream | The client app must receive execution logs, agent message trails, and state changes via a real-time event pipeline (WebSocket/SSE). | P1 |

---

### 5. Non-Functional Requirements

#### NFR-1: Scalability & Network Abstraction
*   The architecture must run seamlessly across any network topology (single host containerized setup, multi-node VM clusters, or hybrid cloud environments).
*   Agent discovery and registration must rely on dynamic service resolution.

#### NFR-2: Grounding & Security
*   MCP servers must enforce access control lists (ACL) ensuring agents only access authorized databases and file directories.
*   A2A communications must support mutual authentication and end-to-end payload signing.
