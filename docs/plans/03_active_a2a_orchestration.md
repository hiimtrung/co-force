# Implementation Plan: 03 - Active A2A Orchestration

**Status:** Plan
**Target:** `crates/co-force-core/src/orchestration/` và Event Bus

## 1. Overview
Phần này mô tả các cơ chế nâng cao nhất của Co-Force: Quản lý vòng đời agent phụ (Sub-agents), phát sóng sự kiện (Event Bus), và chuyển giao tác vụ (Handover), biến Co-Force thành một Active Orchestrator.

## 2. Giải pháp Kỹ thuật

### 2.1 Event-Driven Architecture (In-Memory Event Bus)
*Tham chiếu:* `URD.md` (Section 14.2 Event-Driven Architecture)

Để tránh các module (như MCP Server, Dashboard, AGENTS.md Generator) gọi chéo lẫn nhau gây code rối rắm (tight coupling):
- Sử dụng `tokio::sync::broadcast` tạo ra một kênh Event Bus toàn cục.
- **Sự kiện (Events):** `AgentCheckedIn`, `FileLocked`, `TaskUpdated`, `ContextShared`.
- Khi MCP tool xử lý xong, Use Case sẽ `send()` event lên kênh này. Các module khác tự `subscribe()` và xử lý độc lập ở chế độ ngầm.

### 2.2 Dynamic AGENTS.md Generation
*Tham chiếu:* `URD.md` (UC-36: Dynamic AGENTS.md Generation)

Một Background Daemon (Daemon loop) subscribe vào Event Bus:
- Thu thập state hiện tại từ SQLite (Agents active, Locks, Pending Tasks).
- Cập nhật `.co-force/AGENTS.md` (dùng Regex/String matching để ghi vào đúng khoảng giữa `<!-- BEGIN CO-FORCE MANAGED BLOCK -->` và `<!-- END CO-FORCE MANAGED BLOCK -->`).
- Đồng bộ cho các file đặc thù của Provider (ví dụ `.cursorrules`, `.clauderules`).

### 2.3 Process Manager & Sub-agent Spawning
*Tham chiếu:* `URD.md` (UC-37: Sub-Agent Spawning)

Tích hợp chức năng quản lý Process trên OS:
- **Module:** `ProcessManager`.
- Khi agent gọi `co_force_spawn_agent`:
  - Dùng `tokio::process::Command` để spawn một lệnh bash (ví dụ `antigravity-cli` hay `claude`) và detached (chạy dưới nền).
  - Thu thập PID, theo dõi trạng thái sống/chết.
  - Xử lý tắt/kill process nếu parent agent yêu cầu.

### 2.4 Task Handover / Fallback System
*Tham chiếu:* `URD.md` (UC-38: Task Handover / Fallback)

Khi agent (ví dụ đang ở 128k context limits) muốn dừng và nhường người khác:
- Agent gọi `co_force_handover`.
- Use Case cập nhật Task Status -> `pending_handover`, lưu trữ `state_summary`.
- Force release (Unlock) tất cả file đang khóa bởi agent đó (để tránh deadlock).
- Gọi `ProcessManager` spawn agent mới và pass thông tin ID task vào (để agent mới resume).
