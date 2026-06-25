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

Define variables inside a `.env` file at the root of your deployment node/container.

**Orchestration / Control Plane Configuration:**
```ini
NODE_NAME=orchestrator
PORT=8000
DATABASE_URL=sqlite:///./registry.db
REDIS_URL=redis://localhost:6379/0
SSL_CA_CERT=/etc/ssl/co-force/ca.crt
SSL_CERT=/etc/ssl/co-force/orchestrator.crt
SSL_KEY=/etc/ssl/co-force/orchestrator.key
```

**Worker Swarm Node Configuration:**
```ini
NODE_NAME=worker-node-1
PORT=8080
ORCHESTRATOR_URL=https://orchestrator-host:8000
REDIS_URL=redis://redis-host:6379/0
# MCP Servers this node will spin up or connect to
MCP_SERVERS=["http://localhost:5001/mcp/db", "http://localhost:5002/mcp/fs"]
```

---

### 3. Distributed Service Bootstrapping

To coordinate and manage services, you can containerize the environment.

#### 3.1 Local Multi-Service Compose (Docker)
Example `docker-compose.yml` for unified local testing:

```yaml
version: '3.8'
services:
  event-broker:
    image: redis:7-alpine
    ports:
      - "6379:6379"

  orchestrator:
    build:
      context: .
      dockerfile: Dockerfile.orchestrator
    ports:
      - "8000:8000"
    environment:
      - REDIS_URL=redis://event-broker:6379/0
      - DATABASE_URL=sqlite:///./registry.db
    depends_on:
      - event-broker

  worker-coder:
    build:
      context: .
      dockerfile: Dockerfile.worker
    environment:
      - ORCHESTRATOR_URL=http://orchestrator:8000
      - REDIS_URL=redis://event-broker:6379/0
      - AGENT_TYPE=coder
    depends_on:
      - event-broker
      - orchestrator

  mcp-fs-server:
    image: co-force-mcp-fs:latest
    ports:
      - "5002:5002"
```

#### 3.2 Starting Independent Nodes Manually
If deploying to separate physical or virtual machines, start services as follows:

1.  **Launch the Telemetry Broker:**
    ```bash
    redis-server --protected-mode no
    ```
2.  **Start the Central Orchestrator:**
    ```bash
    # Runs the registry, WebSocket telemetry distribution, and compiled LangGraph workflows
    uvicorn infrastructure.api.main:app --host 0.0.0.0 --port 8000
    ```
3.  **Start a Worker Agent:**
    ```bash
    # Starts the sub-agent loop, publishes its AgentCard, and connects to its MCP servers
    python src/infrastructure/entrypoints/run_agent.py --port 8080 --agent-type coder
    ```
4.  **Audit the A2A network logs:**
    ```bash
    # Listen to raw telemetry logs on the event broker
    redis-cli monitor | grep "A2A_EVENT"
    ```
