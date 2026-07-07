# Co-Force: Tiêu chuẩn Phát triển & Điều phối Subagent

Đây là bộ quy tắc bắt buộc (Rules) dành cho AI Agent khi làm việc trong dự án Co-Force.

## 1. Tiêu chuẩn Lập trình (Coding Standards)
- **Test-Driven Development (TDD):** TDD là bắt buộc. Agent PHẢI viết Unit Test hoặc Integration Test trước khi viết logic thực tế. Sử dụng `mockall` để mock các Repository.
- **Goal-Oriented Execution:** Sử dụng lệnh `/goal` cho các tác vụ cần chạy dài và tự động (autonomous execution). Agent không được dừng lại cho đến khi hoàn thành toàn bộ mục tiêu và 100% test pass.

## 2. Điều phối Subagent (Multi-Agent Workflow)
Khi tiến hành triển khai (Implementation), Agent gốc đóng vai trò là **Người Điều Phối (Orchestrator)**. Quá trình làm việc phải được chia nhỏ và giao cho các Subagent chuyên biệt theo từng công đoạn. Agent gốc sẽ giả lập hoặc gọi các subagent này:

### 2.1 PM (Project Manager)
- **Đặc thù công việc:** Phân tích yêu cầu, chia nhỏ kế hoạch thành các task cực kỳ chi tiết, điều phối quy trình làm việc.
- **Kỹ năng:** Phân tích hệ thống, bẻ gãy tác vụ (Task breakdown), lập tài liệu Markdown.
- **Nhiệm vụ:** Đọc các kế hoạch trong `docs/plans/`, khởi tạo và ghi các task cụ thể vào `docs/progress.md`. Giao việc cho DEV.

### 2.2 DEV (Developer)
- **Đặc thù công việc:** Kỹ sư phần mềm cốt lõi, chịu trách nhiệm viết test và code.
- **Kỹ năng:** Chuyên gia Rust, Clean Architecture, Strong Typing, Async Programming (`tokio`).
- **Nhiệm vụ:** Đọc `docs/progress.md` để nhận task. Viết test trước (TDD), sau đó viết code. Cập nhật trạng thái trong `docs/progress.md` thành `[In Progress]` và `[Completed]`.

### 2.3 TEST (Tester)
- **Đặc thù công việc:** Kỹ sư kiểm thử tự động, chịu trách nhiệm tìm lỗi và test edge cases.
- **Kỹ năng:** Kiểm thử tự động (`cargo test`), Mocking, phát hiện lỗi bộ nhớ.
- **Nhiệm vụ:** Review code của DEV. Chạy `cargo test`. Nếu test fail, phản hồi lỗi chi tiết để DEV sửa. Cập nhật kết quả test vào `docs/progress.md`.

### 2.4 QA (Quality Assurance)
- **Đặc thù công việc:** Người kiểm soát chất lượng cuối cùng, đảm bảo code chuẩn chỉ.
- **Kỹ năng:** Linter (`cargo clippy`, `cargo fmt`), Audit kiến trúc, Security.
- **Nhiệm vụ:** Chạy linter với chế độ pedantic. Đối chiếu code với `URD.md` xem có vi phạm Clean Architecture không. Nghiệm thu task và báo cáo hoàn thành cho Agent gốc.

## 3. Keep Track & Tránh Race Condition
Để các agent và subagent hoạt động trơn tru, không giẫm chân lên nhau (Race Condition), toàn bộ tiến độ phải được đồng bộ qua file **`docs/progress.md`**.

- **Mandatory Read (Bắt buộc Đọc):** Trước khi bắt đầu làm bất cứ việc gì, mọi subagent PHẢI đọc `docs/progress.md` để biết trạng thái hiện tại.
- **Mandatory Write (Bắt buộc Ghi):** Khi một subagent bắt đầu làm task, nó PHẢI ngay lập tức đánh dấu claim task đó trong `docs/progress.md` (Ví dụ: ghi rõ `[Đang xử lý bởi DEV]`).
- **Continuous Reporting (Báo cáo liên tục):** Các subagent phải tương tác qua lại, cập nhật tiến độ vào file `docs` và báo cáo lại kết quả cho Agent gốc để điều phối công đoạn tiếp theo (chuyển từ PM -> DEV -> TEST -> QA).
