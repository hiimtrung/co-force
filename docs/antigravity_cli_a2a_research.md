# Antigravity CLI A2A Communication & Sandbox Isolation Research
## Feasibility, Architecture, and Implementation Guide

Based on the A2A (Agent-to-Agent) specification and the Google Antigravity CLI (`agy`) runtime features, this document outlines how to execute CLI-to-CLI A2A calls and orchestrate multiple isolated agent instances on a single machine.

---

### 1. CLI-to-CLI A2A Communication

#### 1.1 The Challenge
The default Antigravity CLI (`agy`) runs strictly as a **client-side TUI (Text User Interface)** application. It takes user prompts, processes thoughts, executes local terminal tools, and outputs text. It does *not* listen on HTTP/WebSocket ports for external incoming payloads out-of-the-box.

#### 1.2 The Solution
We can enable A2A communication between two CLI-grade agents on the same machine (or across clouds) using two architectures:

##### Method A: Programmatic SDK Daemon Wrapper (Passive Listener)
Instead of running the TUI `agy` executable, wrap the Antigravity agent process using the Python SDK. We host a lightweight FastAPI server that acts as the A2A receiver and forwards requests to the SDK runtime.

```
[Agent A (Client/TUI)] 
      |
      | (A2A POST /api/v1/a2a/task)
      v
[FastAPI A2A Adapter (Port 8080)]
      |
      | (Async SDK Ingestion)
      v
[Agent B (Antigravity SDK Instance)]
      |
      | (MCP Tools)
      v
[Local Filesystem / Terminal Sandbox]
```

**Implementation Example (`run_a2a_agent.py`):**
```python
import asyncio
from fastapi import FastAPI
from pydantic import BaseModel
from google.antigravity import Agent, LocalAgentConfig, CapabilitiesConfig

app = FastAPI()

class A2ATaskPayload(BaseModel):
    task_id: str
    prompt: str

@app.post("/a2a/task")
async def handle_a2a_task(payload: A2ATaskPayload):
    config = LocalAgentConfig(
        system_instructions="You are a sandboxed testing sub-agent.",
        capabilities=CapabilitiesConfig() # Exposes write tools
    )
    # Spawns a sandboxed instance programmatically
    async with Agent(config) as agent:
        response = await agent.chat(payload.prompt)
        text_output = ""
        async for token in response:
            text_output += token
            
    return {"status": "SUCCESS", "output": text_output}
```

##### Method B: MCP A2A Bridge (Active Caller)
To allow a TUI-based `agy` session to actively call an external agent:
1. Configure `agy` to connect to a local **A2A Bridge MCP Server**.
2. When the user asks the CLI to delegate a task, the CLI calls the MCP tool `send_a2a_task(target_url, payload)`.
3. The bridge server performs A2A negotiation, sends the payload, and returns the response to the TUI session.

---

### 2. Running Multiple CLI Instances on the Same Machine

Yes, you can run multiple independent `antigravity` CLI instances simultaneously on a single computer by enforcing environment and folder isolation.

#### 2.1 Separation of User Data Directories
If you run `agy` directly, both instances will read and write to `~/.gemini/antigravity-cli/`, clashing on history logs, active tokens, and workspace settings. 

To isolate them, launch the CLI using the `--user-data-dir` flag:
```bash
# Start Agent 1
agy --user-data-dir ./agent_1_profile --add-dir ./workspace_1

# Start Agent 2
agy --user-data-dir ./agent_2_profile --add-dir ./workspace_2
```

#### 2.2 Workspace Sandboxing
To ensure Agent 1 cannot read/write files in Agent 2's workspace:
1. Ensure `allowNonWorkspaceAccess` is set to `false` (default) in each profile's `settings.json`. This blocks the agent from traversing folders outside its specified workspace root.
2. Launch with the `--sandbox` flag. This runs bash tool execution inside a secure virtual container boundary:
   ```bash
   agy --user-data-dir ./agent_1_profile --sandbox --add-dir ./workspace_1
   ```

#### 2.3 Simulating 2 Swarm Agents on One Workstation
To expose these two local instances to the Headless Server as separate swarm nodes:
1. Create two sandbox directories: `./sandboxes/agent_1` and `./sandboxes/agent_2`.
2. Configure the Workstation Daemon to define two hosted agents, mapping their `sandbox_path` settings to these folders.
3. When the Headless Server routes a task to Agent 1, the daemon starts the `antigravity` SDK runtime or CLI executor bound to `./sandboxes/agent_1`, separating processes cleanly.
