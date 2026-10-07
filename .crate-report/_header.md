# Zed（Windows-only fork）— 198 个 crate 的用途与依赖关系分析

- **数据来源**：`cargo metadata --offline --format-version 1 --filter-platform x86_64-pc-windows-msvc`（host = x86_64-pc-windows-msvc）→ 198 个 workspace 成员、1,844 条内部直接依赖边，落盘为 `%TEMP%\crate-graph-win.csv`（列 `Name,Dir,Desc,In,Rev,InN,RevN`：`In`=直接依赖的内部 crate，`Rev`=反向依赖（谁依赖它），`InN`/`RevN`=计数）。
- **口径提醒**：本视图是 Windows 目标视图，`Rev` 为空/为 0 常表示"仅被非 Windows target 段引用"；`In`/`Rev` **包含 dev-dependencies**（例如 `crates/vim/Cargo.toml:57-77` 里的 git_ui/outline_panel 等全是 dev-deps），判断运行期耦合必须回看 `[dev-dependencies]`。`Cargo.toml` 带 description 的 crate 只有 17/198、`src/lib.rs` 顶部 `//!` 文档只有 2/198，故**用途均由代码（crate 根、模块结构、公开 API）推断**，不是抄注释。
- **仓库状态**：工作树含大量未提交改动（此前的裁剪：AI 云 provider 轴、anthropic/google_ai、内联助手、web search、协作 channel、遥测上传、macOS 专属 crate 与专属文件均已删）。本次分析**未修改仓库任何文件、未运行 cargo build/test**。
- **阅读顺序**：第一节 = 全局结构与统计（自动分析）；第二～十节 = 按 9 组划分的逐 crate 表格（每表 5 列：crate / 用途 / 关键内部依赖 / 被依赖(数量与主要消费者) / 备注）；第十一节 = 汇总（依赖关系规律、裁剪候选、结构风险与死代码清单）。
