# Co-Force System Design & Specification Directory

Welcome to the **Project Co-Force** design repository. Co-Force is a decentralized agent orchestration and execution platform designed for multi-node setups (e.g. Mac Mini 1 Orchestrator + Mac Mini 2 Worker Swarm) using the **Agent2Agent (A2A) Protocol** and the **Model Context Protocol (MCP)**.

---

## Documentation Index

Please refer to the following documents for comprehensive specifications:

1.  **[User Requirements Document (URD)](file:///Users/trungtran/ai-agents/co-force/docs/URD.md)**
    *   Defines target user roles and operator needs.
    *   Details user stories (e.g. task delegation, workflow builder, telemetry tracking).
    *   Specifies functional and non-functional platform requirements.
2.  **[Software Requirements Specification (SRS)](file:///Users/trungtran/ai-agents/co-force/docs/SRS.md)**
    *   Outlines data schemas, including the **AgentCard JSON Schema** and **A2A Message JSON Schema**.
    *   Defines external REST, WebSocket, and MCP communication protocols.
    *   Sets strict architectural and coding quality constraints.
3.  **[System Architecture & Diagrams](file:///Users/trungtran/ai-agents/co-force/docs/system_architecture.md)**
    *   Explains physical multi-node distribution (Mac Mini 1 vs Mac Mini 2).
    *   Includes **Mermaid Diagrams** mapping out system component structures, message flows, and sequence diagrams.
    *   Contains the file layout schema mapping standard Clean Architecture concepts.
    *   Describes how SOLID design principles are map-registered within the module definitions.
4.  **[Developer Implementation Instructions](file:///Users/trungtran/ai-agents/co-force/docs/implementation_instructions.md)**
    *   Instructions for the Test-Driven Development (TDD) cycle (Red-Green-Refactor).
    *   Network security setups including mTLS certificate creation and Tailscale mesh layout.
    *   Environment configurations (`.env`) and startup instructions for the Orchestrator, Worker Nodes, and Redis event stream broker.

---

## Architectural Stack Summary

*   **Orchestration Layer:** LangGraph running on Mac Mini 1.
*   **Execution Layer:** Isolated Antigravity CLI and daemon agents running on Mac Mini 2.
*   **A2A Protocol Layer:** Encrypted HTTP/2 JSON-RPC / REST schemas.
*   **Telemetry Stream:** Event-driven architecture with Redis Streams on Mac Mini 2 publishing messages back to the orchestrator.
*   **Local Tool Execution:** Sandboxed MCP servers (bash, file systems) bound to individual worker instances.
*   **Visual Interface:** SvelteFlow/ReactFlow dashboard built inside a Tauri cross-platform mobile client app.
