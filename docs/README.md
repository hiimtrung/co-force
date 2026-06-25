# Co-Force System Design & Specification Directory

Welcome to the **Project Co-Force** design repository. Co-Force is a decentralized agent orchestration and execution platform designed for distributed client-server environments using the **Agent2Agent (A2A) Protocol** and the **Model Context Protocol (MCP)**.

---

## Documentation Index

Please refer to the following documents for comprehensive specifications:

1.  **[User Requirements Document (URD)](file:///Users/trungtran/ai-agents/co-force/docs/URD.md)**
    *   Defines target user roles and operator needs.
    *   Details user stories (including dynamic workstation & multi-agent sandbox registration).
    *   Specifies functional and non-functional platform requirements.
2.  **[Software Requirements Specification (SRS)](file:///Users/trungtran/ai-agents/co-force/docs/SRS.md)**
    *   Outlines data schemas, including the **AgentCard**, **A2A Message**, and **Workstation Registration** schemas.
    *   Defines external REST, WebSocket, and MCP communication protocols.
    *   Sets strict architectural and coding quality constraints.
3.  **[System Architecture & Diagrams](file:///Users/trungtran/ai-agents/co-force/docs/system_architecture.md)**
    *   Explains physical multi-node topology layout (Tauri Clients vs. Headless Server).
    *   Includes **Mermaid Diagrams** mapping out system component structures, message flows, bootup registrations, and execution sequence diagrams.
    *   Contains the file layout schema mapping standard Clean Architecture concepts.
    *   Describes how SOLID design principles are map-registered within the module definitions.
4.  **[Developer Implementation Instructions](file:///Users/trungtran/ai-agents/co-force/docs/implementation_instructions.md)**
    *   Instructions for the Test-Driven Development (TDD) cycle (Red-Green-Refactor).
    *   Network security setups including mTLS certificate creation and Tailscale mesh layout.
    *   Environment configurations (`.env`) and startup instructions for the Orchestrator, Redis broker, and Workstation Daemon.
5.  **[Antigravity CLI A2A & Sandbox Research](file:///Users/trungtran/ai-agents/co-force/docs/antigravity_cli_a2a_research.md)**
    *   Examines the feasibility of CLI-to-CLI A2A communication.
    *   Provides SDK implementation guides for wrapping CLI agents in A2A listener daemons.
    *   Explains how to run multiple isolated CLI sandboxes on a single workstation without folder/configuration collisions.
6.  **[Official A2A Production Deployment Blueprint](file:///Users/trungtran/ai-agents/co-force/docs/a2a_production_blueprint.md)**
    *   Defines official (chính thống) vs hacky implementation methods for Google Antigravity.
    *   Outlines the recommended microservices architecture for A2A and MCP production nodes.
    *   Provides standard Docker configurations and FastAPI templates wrapping the programmatic Antigravity SDK.

---

## Architectural Stack Summary

*   **Orchestration Layer:** LangGraph running on the Headless Server.
*   **Execution Layer:** Isolated Antigravity CLI and daemon agents running on Client Workstations.
*   **A2A Protocol Layer:** Encrypted HTTP/2 / REST schemas.
*   **Telemetry Stream:** Event-driven architecture with Redis Streams publishing messages back to the orchestrator.
*   **Local Tool Execution:** Sandboxed MCP servers (bash, file systems) bound to individual worker instances.
*   **Visual Interface:** SvelteFlow/ReactFlow dashboard built inside a Tauri cross-platform mobile client app.
