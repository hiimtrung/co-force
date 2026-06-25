# Official A2A Production Deployment Blueprint
## Programmatic Agent Orchestration, Microservices, and Standards

To deploy an Agent2Agent (A2A) network in production, it is critical to separate interactive developer interfaces from backend execution engines.

---

### 1. What is "Official" (Chính Thống) vs. "Non-Standard" (Hacky)

When working with Google Antigravity and the A2A Protocol, we must differentiate between UI wrappers and programmatic SDKs:

| Category | Non-Standard / Brittle (Hacky) | Production Standard (Chính Thống) |
| :--- | :--- | :--- |
| **Execution** | Programmatically invoking the TUI command `agy` inside shell scripts or wrapping terminal inputs/outputs. | Spawning and calling the agent using the official **Google Antigravity Python SDK** (`google-antigravity` on PyPI). |
| **A2A Interface** | Writing custom HTTP parsing wrappers for terminal consoles. | Implementing the official **A2A ADK (Agent Development Kit)** specifications and JSON schemas over HTTP/2. |
| **Sandboxing** | Running multiple local profiles in arbitrary folders via `--user-data-dir`. | Running isolated agent runner processes in **Docker containers** with read-only filesystems and dedicated workspace mounts. |
| **Tool Execution** | Directly exposing system terminal access to agents. | Enforcing strict capability scopes using local or remote **MCP (Model Context Protocol) Servers** acting as security boundaries. |

---

### 2. A2A Production Deployment Architecture

In a production environment, each agent operates as an independent **Headless Microservice**. The standard deployment layout consists of the following components:

```
                  +--------------------------------+
                  |  Tauri Control Client (GUI)    |
                  +--------------------------------+
                                  |
                                  | (REST / WebSocket)
                                  v
                  +--------------------------------+
                  |     API Gateway / Router       |
                  +--------------------------------+
                      |                        |
     (A2A Delegation) |                        | (A2A Delegation)
                      v                        v
         +-------------------------+      +-------------------------+
         |     Coder Agent Pod     |      |    Tester Agent Pod     |
         |  - A2A HTTP Controller  |      |  - A2A HTTP Controller  |
         |  - Antigravity SDK Core |      |  - Antigravity SDK Core |
         +-------------------------+      +-------------------------+
            |                   |            |                   |
  (stdio)   v           (SSE)   v   (stdio)  v           (SSE)   v
  +------------------+  +---------------+ +------------------+  +---------------+
  | local workspace  |  | Local MCP tool| | local workspace  |  | Local MCP tool|
  |  (Docker Mount)  |  | (Bash/Docker) | |  (Docker Mount)  |  | (PyTest/Bash) |
  +------------------+  +---------------+ +------------------+  +---------------+
```

#### 2.1 The Agent Container Structure (Microservice Pod)
Each agent is packaged in a Docker container exposing:
1.  **Port 80 (A2A Port):** A web server conforming to A2A Protocol standards (e.g., exposing `/agentcard` for capabilities and `/a2a/task` for delegation).
2.  **Antigravity Runtime Process:** Runs the python script importing `google.antigravity` to execute logic.
3.  **MCP Sidecar:** A companion process handling file edits or terminal commands, sandboxed via container policy.

#### 2.2 Telemetry and Decoupling
*   **Asynchronous Event Streams:** Agents must not block during execution. Progress and execution milestones are published as events to an **Event Broker (e.g. Redis Streams or RabbitMQ)**.
*   **WebSocket Gateway:** The central API Gateway reads events from the broker and streams them to the Tauri Control Client for visualization.

---

### 3. Production Deployment Code Layout

Here is how the A2A Microservice is officially built in Python using FastAPI and the Antigravity SDK:

#### 3.1 Dockerfile (`Dockerfile.agent`)
```dockerfile
FROM python:3.11-slim

# Install system dependencies (compilers, git)
RUN apt-get update && apt-get install -y git build-essential && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

COPY src/ /app/src/

# Expose A2A server port
EXPOSE 80

CMD ["uvicorn", "src.a2a_server:app", "--host", "0.0.0.0", "--port", "80"]
```

#### 3.2 Programmatic A2A Server (`src/a2a_server.py`)
```python
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel
from google.antigravity import Agent, LocalAgentConfig, CapabilitiesConfig
import uuid
import datetime

app = FastAPI()

class A2ATask(BaseModel):
    message_id: str
    sender_id: str
    payload: dict

@app.get("/agentcard")
def get_agent_card():
    # Return official A2A AgentCard JSON schema
    return {
        "agent_id": "aa1a2a3b-4c5d-6e7f-8a9b-0c1d2e3f4g5h",
        "name": "Production Coder Agent",
        "version": "1.0.0",
        "capabilities": ["code-writing"],
        "endpoints": {
            "a2a_endpoint": "/a2a/task"
        },
        "input_schema": {
            "type": "object",
            "properties": { "prompt": { "type": "string" } }
        },
        "output_schema": {
            "type": "object",
            "properties": { "code": { "type": "string" } }
        }
    }

@app.post("/a2a/task")
async def run_task(task: A2ATask):
    try:
        # 1. Configure the agent using the official SDK
        config = LocalAgentConfig(
            system_instructions="You are a production code writer agent.",
            capabilities=CapabilitiesConfig() # Enables write capabilities
        )
        
        # 2. Run the agent programmatically inside its isolated container workspace
        async with Agent(config) as agent:
            response = await agent.chat(task.payload.get("prompt", ""))
            
            output_tokens = []
            async for token in response:
                output_tokens.append(token)
                # Here you can also publish telemetry events to Redis
                
        return {
            "message_id": str(uuid.uuid4()),
            "parent_message_id": task.message_id,
            "timestamp": datetime.datetime.utcnow().isoformat(),
            "message_type": "TASK_COMPLETE",
            "payload": {"code": "".join(output_tokens)}
        }
    except Exception as e:
        raise HTTPException(status_code=500, detail=str(e))
```

---

### 4. Implementation Steps for Production

1.  **Package Agents as Images:** Build a Docker image for each agent type using `Dockerfile.agent`.
2.  **Define Workspace Volumes:** Mount distinct host folders or Kubernetes Persistent Volume Claims (PVC) to `/app/workspace` in the container. This provides isolated physical sandboxes for file changes.
3.  **Deploy on Cloud or Local VM Clusters:** Run containers on Kubernetes, AWS ECS, Google Cloud Run, or a local Docker Swarm.
4.  **Enforce Private Mesh Routing:** Ensure the central router resolves agent URLs through private internal DNS (e.g., `http://agent-coder.local`) rather than public endpoints.
