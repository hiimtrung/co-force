# Software Requirements Specification (SRS)
## Project Co-Force: Centralized Agent Orchestration & Canvas Swarm Platform

### 1. Introduction
This specification defines the technical software requirements for Co-Force. Co-Force is a Clean Architecture, Event-Driven system designed to compile canvas-designed agent networks into executable **LangGraph** workflows. The system coordinates and tracks distributed agents/sub-agents using the **Agent2Agent (A2A) Protocol**, while grounding agent intelligence and toolsets in the **Model Context Protocol (MCP)**.

---

### 2. External Interface & Protocol Requirements

#### 2.1 Central Client (UI Canvas) <=> Central Orchestrator
*   **Workflow Deployment API:** REST endpoints (`POST /api/v1/workflows/deploy`) accepting visual canvas JSON definitions and compiling them into LangGraph configurations.
*   **Telemetry Stream:** WebSockets (`wss://`) routing real-time traces from the Event Broker to the UI client.
*   **Control Commands:** API calls to pause, step, override, or terminate running agent graph processes.

#### 2.2 Agent-to-Agent (A2A) Network Interfaces
*   **Protocol:** Transport over HTTP/2 (gRPC or REST over TLS) for control negotiations, and AMQP / Redis Streams / MQTT for asynchronous event-driven messages.
*   **Registry Discovery:** Every agent registers its routing address and capabilities (`AgentCard`) with the Central Agent Registry.

#### 2.3 Model Context Protocol (MCP) Integration
All agents and sub-agents interface with MCP servers to obtain:
*   **Knowledge (Resource Reads):** Querying static indices, vector search layers, file systems, and enterprise documentation.
*   **Grounding Truth (Context Providers):** Fetching real-time system logs, database states, and live environment parameters to prevent LLM hallucinations.
*   **Skills (Tool Invocations):** Executing code, compiling builds, modifying database records, or sending API requests.

```
+-------------------------------------------------------+
|                       LLM Core                        |
+-------------------------------------------------------+
                           |
       +-------------------+-------------------+
       | (A2A Protocol)                        | (MCP Protocol)
       v                                       v
+-----------------------------+         +-------------------------------+
|   Agent-to-Agent Swarm      |         |   Knowledge, Truth & Skills   |
|  - Task Delegation          |         |  - Vector Search & Documents  |
|  - Negotiation Agreements   |         |  - Live Database Grounding    |
|  - Status Tracing           |         |  - System & Development Tools |
+-----------------------------+         +-------------------------------+
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

---

### 4. Functional Specifications

#### SF-1: Visual Workflow Compiler
*   **Description:** The compiler takes the node-link graph from the UI client and maps nodes to A2A routing keys or endpoints, and edges to LangGraph transitions and conditional edges.
*   **Validation:** It verifies that a path exists from start to finish, and checks schema compatibility between linked nodes.

#### SF-2: Dynamic Registry & Router
*   **Description:** Central registry acts as the service locator. When an agent instance registers, the router updates its routing map. If an agent fails to respond to heartbeats, the router dynamically updates the active graph.
*   **Capability Matching:** If a compiled graph demands a capability (e.g. `python-compiler`), the orchestrator queries the registry to bind the node to an active agent exposing that capability.

#### SF-3: MCP Context & Tool Injector
*   **Description:** During execution, before prompting the LLM core of the agent, the runtime queries configured MCP servers to fetch necessary facts (resource read) and appends them to the system prompt. Tool declarations returned by MCP servers are injected as available tools.

---

### 5. Architectural Quality Attributes

*   **Clean Architecture Adherence:** Logic must remain split between Domain (Entities), Application (Use Cases), Presentation (Controllers/Presenters), and infrastructure. Framework dependencies (like LangGraph execution schemas) must be decoupled using adapters.
*   **Event-Driven telemetry:** Telemetry must be published asynchronously to avoid bottlenecking agent execution.
*   **Test-Driven Development:** Any change in schema validation or compilation logic must first be covered by failing unit tests.
