# Software Requirements Specification (SRS)
## Project Co-Force: Centralized Agent Orchestration & Canvas Swarm Platform

### 1. Introduction
This specification defines the technical software requirements for Co-Force. Co-Force is a Clean Architecture, Event-Driven system designed to compile canvas-designed agent networks into executable **LangGraph** workflows. The system coordinates and tracks distributed agents/sub-agents using the **Agent2Agent (A2A) Protocol**, while grounding agent intelligence and toolsets in the **Model Context Protocol (MCP)**.

---

### 2. External Interface & Protocol Requirements

#### 2.1 Tauri Control Client (UI Canvas) <=> Headless Server
*   **Workflow Deployment API:** REST endpoint (`POST /api/v1/workflows/deploy`) accepting JSON canvas structures from SvelteFlow/ReactFlow and compiling them into executable LangGraph definitions.
*   **Live Telemetry Stream:** WebSockets (`wss://`) routing node executions, trace events, and tokens from the Server's event broker to the Control Client GUI.
*   **State Control API:** REST endpoints to trigger (`POST /api/v1/workflows/{id}/run`), pause, or terminate running agent pipelines.

#### 2.2 Tauri Client Agent (Local AI Agent) <=> Headless Server
*   **A2A Listener/Polling:** The Client Agent (Antigravity running locally on macOS/Linux) runs a lightweight HTTP server or maintains a persistent WebSocket connection to the Headless Server.
*   **Task Ingestion:** Receives `TASK_DELEGATION` messages via the A2A protocol from the server, runs local compilers/actions, and reports progress events back via `TASK_PROGRESS` messages.
*   **AgentCard Registration:** On startup, the Client Agent POSTs its `AgentCard` metadata (detailing its local terminal execution and file modification capabilities) to the Server's Central Registry.

#### 2.3 Headless Server Core Services
The server lacks a frontend and exposes the following backend boundaries:
*   **Gateway API:** Fast API endpoints handling client authentication, session tracking, and routing.
*   **State DB / Storage:** PostgreSQL or SQLite database for storing execution logs, registered AgentCards, and active canvas schemas.
*   **Local LLM Integration (Ollama):** The server communicates with local model instances via Ollama's HTTP API (`http://localhost:11434/api/generate` and `/api/chat`), hosting models such as Llama3 or Qwen2.5-Coder.

#### 2.4 Model Context Protocol (MCP) Integration
Both server-side sub-agents and the local Client Agent connect to MCP servers to fetch:
*   **Knowledge (Resource Reads):** Querying documentation indices, PDF text nodes, and codebase folders.
*   **Grounding Truth (Context Providers):** Reading environment variables, database schemas, and configuration files to ground the LLM prompt.
*   **Skills (Tool Invocations):** Invoking local file writes, terminal shell processes, or testing suites.

```
+-------------------------------------------------------------+
|               Tauri Clients (Desktop / Mobile)              |
|   +--------------------------+  +-----------------------+   |
|   | Control Client (Canvas)  |  | Client Agent (AI)     |   |
|   +--------------------------+  +-----------------------+   |
+-------------------------------------------------------------+
               | (WebSocket/REST)             | (A2A Protocol)
               v                              v
+-------------------------------------------------------------+
|                     Headless Server Core                    |
|  - API Gateway   - DB Storage   - LangGraph   - Ollama (LLM) |
+-------------------------------------------------------------+
                                              | (MCP Protocol)
                                              v
                                  +-----------------------+
                                  | MCP Knowledge/Skills  |
                                  +-----------------------+
```

---

### 3. Data Schemas

#### 3.1 AgentCard JSON Schema
Exposed by each agent to describe its inputs, outputs, capabilities, and A2A endpoints.

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "AgentCard",
  "type": "object",
  "properties": {
    "agent_id": { "type": "string", "format": "uuid" },
    "name": { "type": "string" },
    "version": { "type": "string" },
    "description": { "type": "string" },
    "capabilities": {
      "type": "array",
      "items": { "type": "string" }
    },
    "endpoints": {
      "type": "object",
      "properties": {
        "a2a_endpoint": { "type": "string", "format": "uri" },
        "mcp_servers": {
          "type": "array",
          "items": { "type": "string", "format": "uri" },
          "description": "List of MCP endpoints this agent consumes for knowledge/skills"
        }
      },
      "required": ["a2a_endpoint"]
    },
    "input_schema": {
      "type": "object",
      "description": "JSON Schema defining expected inputs"
    },
    "output_schema": {
      "type": "object",
      "description": "JSON Schema defining expected outputs"
    }
  },
  "required": ["agent_id", "name", "version", "description", "capabilities", "endpoints", "input_schema", "output_schema"]
}
```

#### 3.2 A2A Message Schema
Used for all inter-agent routing, coordination, and status updates.

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "A2AMessage",
  "type": "object",
  "properties": {
    "message_id": { "type": "string", "format": "uuid" },
    "parent_message_id": { "type": "string", "format": "uuid", "nullable": true },
    "timestamp": { "type": "string", "format": "date-time" },
    "sender": {
      "type": "object",
      "properties": {
        "agent_id": { "type": "string" },
        "role": { "type": "string" }
      },
      "required": ["agent_id", "role"]
    },
    "receiver": {
      "type": "object",
      "properties": {
        "agent_id": { "type": "string" }
      },
      "required": ["agent_id"]
    },
    "message_type": {
      "type": "string",
      "enum": ["TASK_DELEGATION", "TASK_PROGRESS", "TASK_COMPLETE", "TASK_ERROR", "NEGOTIATION_REQUEST", "NEGOTIATION_RESPONSE"]
    },
    "payload": {
      "type": "object",
      "description": "Parameters or results matching input_schema / output_schema"
    }
  },
  "required": ["message_id", "timestamp", "sender", "receiver", "message_type", "payload"]
}
```

#### 3.3 Workstation Registration Schema
Sent by the workstation daemon on boot to declare its network coordinates and hosted agents.

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "WorkstationRegistration",
  "type": "object",
  "properties": {
    "workstation_id": { "type": "string", "format": "uuid" },
    "name": { "type": "string" },
    "network_address": { "type": "string", "format": "uri" },
    "timestamp": { "type": "string", "format": "date-time" },
    "agents": {
      "type": "array",
      "items": { "$ref": "#/definitions/AgentCard" }
    }
  },
  "required": ["workstation_id", "name", "network_address", "timestamp", "agents"]
}
```

---

### 4. Functional Specifications

#### SF-1: Visual Workflow Compiler
*   **Description:** The compiler takes the node-link graph from the UI client and maps nodes to A2A routing keys or endpoints, and edges to LangGraph transitions and conditional edges.
*   **Validation:** It verifies that a path exists from start to finish, and checks schema compatibility between linked nodes.

#### SF-2: Dynamic Workstation Discovery & Batch Registry
*   **Description:** Central registry acts as the service locator. When a client workstation boots, its local daemon POSTs the `WorkstationRegistration` package to the backend. The registry inserts/updates all contained `AgentCards`.
*   **Live UI Canvas Push:** Upon successful registry insertion, the server broadcasts an `AGENT_ADDED` WebSocket event containing the new `AgentCard` schemas to all active Tauri Control Clients. The visual canvas UI automatically injects these agents into the node catalog.
*   **Heartbeat & Pruning:** Workstations emit heartbeats every 10 seconds. If a workstation heartbeats fail, all associated agents are marked inactive, and `AGENT_REMOVED` events are streamed, disabling those nodes in the Canvas UI.

#### SF-3: Sandbox Execution & Routing
*   **Description:** When the LangGraph engine targets an agent node, the router looks up the associated `workstation_id` and `agent_id`. The A2A `TASK_DELEGATION` message is routed to the workstation's listener.
*   **Sandbox Isolation:** The workstation daemon spins up a dedicated runner process for the designated agent. The process executes inside an isolated runtime shell and directory path: `workspaces/{workstation_id}/sandboxes/{agent_id}/`. File writing and command execution tools exposed via local MCP are strictly scoped to this directory.

#### SF-4: MCP Context & Tool Injector
*   **Description:** During execution, before prompting the LLM core of the agent, the runtime queries configured MCP servers to fetch necessary facts (resource read) and appends them to the system prompt. Tool declarations returned by MCP servers are injected as available tools.

---

### 5. Architectural Quality Attributes

*   **Clean Architecture Adherence:** Logic must remain split between Domain (Entities), Application (Use Cases), Presentation (Controllers/Presenters), and infrastructure. Framework dependencies (like LangGraph execution schemas) must be decoupled using adapters.
*   **Event-Driven telemetry:** Telemetry must be published asynchronously to avoid bottlenecking agent execution.
*   **Test-Driven Development:** Any change in schema validation or compilation logic must first be covered by failing unit tests.

