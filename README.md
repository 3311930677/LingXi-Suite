# LingXi Suite

灵犀桌面智能体套件。本仓库用**分支**划分两个项目：

| 分支 | 内容 |
|---|---|
| [`desktop`](/tree/desktop) | 灵犀桌面智能体：桌宠 overlay（Tauri，点击即弹出操作菜单）+ 工作台前端（统一卡片网格 / 分区目录 / 扩展面板体系）+ 引擎分发脚本 + 项目文档 |
| [`engine`](/tree/engine) | OwO Agent SDK：Rust 引擎（agent / server / CLI）——多模型网关、MCP、权限三档审批、自动化、可观测性、终端 Markdown 渲染与管道模式 |

## 克隆指定分支

```bash
git clone -b desktop https://github.com/3311930677/LingXi-Suite.git LingXi-DesktopAgent
git clone -b engine  https://github.com/3311930677/LingXi-Suite.git OwO
```

> 桌宠（desktop）构建时会把 engine 产出的 `owo-agent.exe` 复制进
> `apps/overlay/engine-dist/`（见 `apps/overlay/scripts/prepare-engine-dist.ps1`），
> 该目录不入库，需先在 engine 分支 `cargo build -p owo-agent-cli` 生成。
