# Developer Implementation Instructions
## Project Co-Force: Centralized Agent Orchestration & Canvas Swarm Platform

This guide outlines setup, testing, and deployment instructions for Co-Force across containerized environments, cloud nodes, or local networks.

---

### 1. Test-Driven Development (TDD) Workflow

#### 1.1 Testing the Canvas-to-LangGraph Compiler
To ensure workflow configurations compile accurately, implement tests for the compiler logic before writing parsing code.

Example test (`tests/unit/use_cases/test_compile_workflow.py`):
```python
import pytest
from use_cases.compile_workflow import CompileWorkflow
from domain.entities.workflow import WorkflowGraph

def test_compiler_creates_valid_langgraph_from_canvas_json():
    # Arrange: Simple Canvas UI JSON containing two nodes and an edge
    canvas_payload = {
        "nodes": [
            {"id": "node-1", "type": "agentNode", "data": {"agent_id": "coder-uuid"}},
            {"id": "node-2", "type": "agentNode", "data": {"agent_id": "tester-uuid"}}
        ],
        "edges": [
            {"id": "edge-1", "source": "node-1", "target": "node-2", "type": "default"}
        ]
    }
    compiler = CompileWorkflow()

    # Act
    compiled_graph = compiler.execute(canvas_payload)

    # Assert
    assert isinstance(compiled_graph, WorkflowGraph)
    assert len(compiled_graph.nodes) == 2
    assert "node-1" in compiled_graph.transitions
    assert compiled_graph.transitions["node-1"] == "node-2"
```

#### 1.2 Testing MCP Tool Ingestion
Ensure sub-agents dynamically discover and register skills from an MCP server configuration:
```python
from unittest.mock import Mock
from use_cases.interfaces.mcp_connector import MCPConnector
from use_cases.load_agent_skills import LoadAgentSkills

def test_load_agent_skills_binds_mcp_tools():
    # Arrange
    mock_mcp = Mock(spec=MCPConnector)
    mock_mcp.list_tools.return_value = [
        {"name": "execute_bash", "description": "Run shell scripts"}
    ]
    
    loader = LoadAgentSkills(mcp_connector=mock_mcp)
    
    # Act
    skills = loader.execute(agent_id="test-agent")
    
    # Assert
    assert len(skills) == 1
    assert skills[0]["name"] == "execute_bash"
```

---

### 2. Environment Configuration

Define variables inside the appropriate `.env` files for the Headless Server and the Local Workstations.

**Headless Server Core Configuration (`server.env`):**
```ini
NODE_NAME=headless-orchestrator-server
PORT=8000
DATABASE_URL=sqlite:///./state_registry.db
REDIS_URL=redis://localhost:6379/0
# Ollama integration endpoint
OLLAMA_API_URL=http://localhost:11434
# Local LLM models to serve via Ollama
OLLAMA_MODEL=qwen2.5-coder:7b
```

**Tauri Workstation & Sandbox Configuration (`config/workstation.json`):**
Instead of simple environment variables, the workstation CLI / Daemon uses a JSON file to declare its agents and directories.

```json
{
  "workstation_id": "c1a938c0-82a1-432d-944d-d7be8d123456",
  "name": "dev-workstation-macos",
  "server_url": "http://headless-server:8000",
  "local_a2a_port": 8080,
  "sandboxes_root": "./workspaces/sandboxes",
  "hosted_agents": [
    {
      "agent_id": "aa1a2a3b-4c5d-6e7f-8a9b-0c1d2e3f4g5h",
      "name": "local-coder-agent",
      "version": "1.2.0",
      "description": "Writes filesystem scripts and files in sandbox workspace",
      "capabilities": ["code-write", "local-exec"],
      "mcp_servers": ["stdio:node-fs-mcp"],
      "input_schema": {
        "type": "object",
        "properties": { "prompt": { "type": "string" } }
      },
      "output_schema": {
        "type": "object",
        "properties": { "result": { "type": "string" } }
      }
    },
    {
      "agent_id": "bb2b3b4c-5d6e-7f8a-9b0c-1d2e3f4g5h6i",
      "name": "local-test-agent",
      "version": "1.0.0",
      "description": "Runs test suites and executes shell scripts in sandbox workspace",
      "capabilities": ["code-test"],
      "mcp_servers": ["stdio:bash-shell-mcp"],
      "input_schema": {
        "type": "object",
        "properties": { "test_command": { "type": "string" } }
      },
      "output_schema": {
        "type": "object",
        "properties": { "stdout": { "type": "string" }, "success": { "type": "boolean" } }
      }
    }
  ]
}
```

---

### 3. Distributed Service Bootstrapping

To set up the headless server and client environments:

#### 3.1 Headless Server Docker Compose (`docker-compose-server.yml`)
The central backend server can be run inside a Docker network, pulling in Ollama for offline LLM support:

```yaml
version: '3.8'
services:
  event-broker:
    image: redis:7-alpine
    ports:
      - "6379:6379"

  ollama-service:
    image: ollama/ollama:latest
    ports:
      - "11434:11434"
    volumes:
      - ollama_data:/root/.ollama

  headless-server:
    build:
      context: .
      dockerfile: Dockerfile.server
    ports:
      - "8000:8000"
    environment:
      - REDIS_URL=redis://event-broker:6379/0
      - DATABASE_URL=sqlite:///./state_registry.db
      - OLLAMA_API_URL=http://ollama-service:11434
      - OLLAMA_MODEL=qwen2.5-coder:7b
    depends_on:
      - event-broker
      - ollama-service

volumes:
  ollama_data:
```

#### 3.2 Starting services manually

##### Step A: Run the Headless Server Stack
1.  **Launch the Ollama daemon and pull the model:**
    ```bash
    # On the server host
    ollama serve
    ollama pull qwen2.5-coder:7b
    ```
2.  **Start Redis and the FastAPI control plane:**
    ```bash
    redis-server --protected-mode no
    uvicorn infrastructure.api.main:app --host 0.0.0.0 --port 8000
    ```

##### Step B: Run the Local Workstation Daemon & Sandbox Agents
1.  **Boot the Workstation Daemon with the client JSON config:**
    ```bash
    # This spawns process directories for Coder and Tester, hooks up local MCPs,
    # and registers the batch WorkstationRegistration to the headless server.
    python src/infrastructure/entrypoints/run_workstation_daemon.py --config config/workstation.json
    ```
2.  **Verify Batch Agent Registration:**
    You can query the headless server registry to verify that both sandboxed agents were added:
    ```bash
    curl http://headless-server:8000/api/v1/registry/agents
    ```

##### Step C: Run the Tauri Control Client (GUI)
1.  **Run the Tauri visual interface in development mode:**
    ```bash
    cd src/ui/tauri-app
    npm install
    npm run tauri dev
    ```
    This launches the drag-and-drop workspace UI connecting to the central Headless Server WebSocket. The workspace UI will automatically display the registered sandboxed agents in the nodes list.


