//! 真实引擎联调（需要本机 `owo-agent serve` 在跑）。
//!
//! ```powershell
//! # 终端 A：拉起引擎（debug 构建）
//! $env:OPENAI_API_KEY="sk-..."   # turn 用例需要
//! D:\working OWOWOWOWOWO\OwO\agent-sdk\target\debug\owo-agent.exe serve --port 4096 --workspace <某目录>
//!
//! # 终端 B：
//! cargo test -p owo-bridge --test live_engine -- --ignored --nocapture
//! ```
//!
//! `OWO_AGENT_BASE_URL` 可覆盖地址（默认 `http://127.0.0.1:4096`）。

use owo_bridge::{OwoBridgeClient, TurnRequest};

fn live_client() -> OwoBridgeClient {
    let base =
        std::env::var("OWO_AGENT_BASE_URL").unwrap_or_else(|_| owo_bridge::default_base_url());
    OwoBridgeClient::new(base)
}

/// 会话 workspace **必须等于**引擎启动时的 `--workspace`：
/// 引擎的沙箱根（policy workspace）在启动时确定，会话 workspace 与之一致
/// 才能通过文件工具的越界检查，否则 `write_file` 报“路径越界”。
fn live_workspace() -> std::path::PathBuf {
    let dir = std::env::var("OWO_AGENT_WORKSPACE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("owo-bridge-live-ws"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
#[ignore = "需要本机 owo-agent serve 在线"]
fn health_and_session_lifecycle_against_live_engine() {
    let client = live_client();
    let health = client.health().expect("引擎应在线（先启动 owo-agent serve）");
    assert!(health.healthy);
    println!(
        "[live] health v={} auto_approve={}",
        health.version, health.auto_approve
    );

    let workspace = live_workspace();
    let session = client
        .create_session(&workspace.to_string_lossy(), None, None)
        .expect("建会话应成功");
    assert!(!session.id.is_empty(), "会话 id 不应为空");

    let sessions = client.list_sessions().expect("会话列表应可读");
    assert!(
        sessions.iter().any(|item| item.id == session.id),
        "新会话应出现在列表里"
    );

    let diffs = client.diff(&session.id).expect("diff 应可读");
    assert!(diffs.is_empty(), "新会话不应有改动：{diffs:?}");

    let detail = client.session_detail(&session.id).expect("详情应可读");
    assert_eq!(detail["id"].as_str(), Some(session.id.as_str()));
    println!("[live] session {} 生命周期 OK", session.id);
}

#[test]
#[ignore = "需要本机 owo-agent serve 在线 + 模型密钥（turn 会真实调用模型）"]
fn turn_streams_events_and_engine_writes_file() {
    let client = live_client();
    let workspace = live_workspace();
    let _ = std::fs::remove_file(workspace.join("hello.txt"));
    let session = client
        .create_session(&workspace.to_string_lossy(), None, None)
        .expect("建会话应成功");

    let mut kinds: Vec<String> = Vec::new();
    let mut final_text = String::new();
    let mut permission_requests = 0usize;
    let mut approved = 0usize;
    client
        .turn_stream(
            &session.id,
            &TurnRequest::new("在工作区里创建 hello.txt，内容写 hi 两个字母"),
            |frame| {
                kinds.push(frame.event_name().to_string());
                match frame.json::<owo_bridge::TurnEvent>() {
                    Some(owo_bridge::TurnEvent::Final { text }) => final_text = text,
                    Some(owo_bridge::TurnEvent::ToolUse { tool, args, .. }) => {
                        println!("[live] tool_use {tool} args={args}");
                    }
                    Some(owo_bridge::TurnEvent::ToolResult {
                        tool, ok, error, ..
                    }) => {
                        println!("[live] tool_result {tool} ok={ok} error={error:?}");
                    }
                    Some(owo_bridge::TurnEvent::PermissionRequest { request_id, .. }) => {
                        permission_requests += 1;
                        // 自动放行（联调用；真实桌宠由用户点审批卡片）。
                        if client
                            .respond_permission(&session.id, &request_id, true, Some(false), None)
                            .is_ok()
                        {
                            approved += 1;
                        }
                    }
                    _ => {}
                }
            },
        )
        .expect("turn 流应正常结束");

    println!(
        "[live] 事件序列={kinds:?} 审批={permission_requests} 放行={approved}"
    );
    assert!(kinds.iter().any(|kind| kind == "final"), "应收到 final 事件");
    assert!(!final_text.is_empty(), "final 文本不应为空");
    let diffs = client.diff(&session.id).expect("diff 应可读");
    println!("[live] 改动 = {diffs:?}");
    let listed: Vec<String> = std::fs::read_dir(&workspace)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    println!("[live] workspace 落盘文件 = {listed:?}");
    assert!(
        workspace.join("hello.txt").is_file(),
        "引擎应真实创建 hello.txt（当前目录内容：{listed:?}）"
    );
}
