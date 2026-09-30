//! E4 验收：循环检测 / 每步工具超时 / 预算熔断。
//!
//! 变异验收口径：
//! - 注释掉 `LoopDetector::observe` 的计数逻辑 → `repeated_identical_tool_call_forces_wrap_up` 变红；
//! - 注释掉 `budget_stop_reason` 调用 → `budget_cap_stops_loop` 变红；
//! - 去掉 `tool_step_timeout` 包装 → `slow_tool_times_out_with_structured_error` 变红。
//!
//! 环境变量类测试需跨 await 持有进程级 env 锁（串行化 env 修改，避免并行测试互相污染）。
#![allow(clippy::await_holding_lock)]

use async_trait::async_trait;
use owo_agent_core::permissions::{AutoApprover, Policy};
use owo_agent_core::tools::{Tool, ToolContext, ToolRegistry};
use owo_agent_core::{
    Agent, AgentConfig, ChatMessage, ModelOutput, ModelProvider, Session, TokenUsage, ToolCall,
    ToolSpec,
};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

/// 环境变量类测试串行化（避免并行测试互相污染）。
static ENV_LOCK: Mutex<()> = Mutex::new(());

struct ScriptedProvider {
    script: Mutex<VecDeque<ModelOutput>>,
    recorded: Arc<Mutex<Vec<ChatMessage>>>,
    usage: TokenUsage,
}

impl ScriptedProvider {
    fn new(outputs: Vec<ModelOutput>, recorded: Arc<Mutex<Vec<ChatMessage>>>) -> Self {
        Self {
            script: Mutex::new(outputs.into()),
            recorded,
            usage: TokenUsage::default(),
        }
    }

    fn with_usage(mut self, total_tokens: u64) -> Self {
        self.usage = TokenUsage {
            prompt_tokens: total_tokens,
            completion_tokens: 0,
            total_tokens,
        };
        self
    }
}

#[async_trait]
impl ModelProvider for ScriptedProvider {
    async fn complete(
        &self,
        messages: &[ChatMessage],
        _tools: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        self.recorded
            .lock()
            .unwrap()
            .extend(messages.iter().cloned());
        self.script
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| "脚本输出耗尽".to_string())
    }

    fn usage_snapshot(&self) -> TokenUsage {
        self.usage
    }
}

/// 慢工具：固定延迟，用于超时验收。
struct SlowTool {
    delay: std::time::Duration,
}

#[async_trait]
impl Tool for SlowTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "slow_tool".to_string(),
            description: "测试用慢工具".to_string(),
            input_schema: json!({ "type": "object", "properties": {} }),
        }
    }

    async fn run(&self, _ctx: &mut ToolContext<'_>, _args: Value) -> Result<Value, String> {
        tokio::time::sleep(self.delay).await;
        Ok(json!({ "done": true }))
    }
}

fn call(id: &str, name: &str, args: Value) -> ModelOutput {
    ModelOutput::ToolCalls(vec![ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments: args,
    }])
}

fn temp_workspace(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("owo-loop-guard-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn build_agent<P>(workspace: &std::path::Path, provider: P) -> Agent
where
    P: ModelProvider + Send + Sync + 'static,
{
    build_agent_with_registry(workspace, provider, ToolRegistry::new())
}

fn build_agent_with_registry<P>(
    workspace: &std::path::Path,
    provider: P,
    registry: ToolRegistry,
) -> Agent
where
    P: ModelProvider + Send + Sync + 'static,
{
    Agent::new(
        Arc::new(provider),
        registry,
        Policy::new(workspace.to_path_buf()),
        AgentConfig::default(),
    )
}

fn recorded_texts(recorded: &Arc<Mutex<Vec<ChatMessage>>>) -> Vec<String> {
    recorded
        .lock()
        .unwrap()
        .iter()
        .filter_map(|message| message.content.clone())
        .collect()
}

#[tokio::test]
async fn repeated_identical_tool_call_forces_wrap_up() {
    let workspace = temp_workspace("repeat");
    let recorded = Arc::new(Mutex::new(Vec::new()));
    // 5 次同一 list_dir（非豁免工具）+ 收尾文本。
    let mut script = Vec::new();
    for index in 0..5 {
        script.push(call(
            &format!("c{index}"),
            "list_dir",
            json!({ "path": "." }),
        ));
    }
    script.push(ModelOutput::Text("收尾结论".to_string()));
    let provider = ScriptedProvider::new(script, Arc::clone(&recorded));
    let agent = build_agent(&workspace, provider);
    let mut session = Session::new(&workspace, "mock".to_string(), None);
    let abort = AtomicBool::new(false);
    let approver = AutoApprover { allow: true };

    let outcome = agent
        .run_turn(&mut session, "列目录", &approver, &abort, &mut |_| {})
        .await
        .expect("回合必须正常收尾");

    assert_eq!(outcome.final_text.as_deref(), Some("收尾结论"));
    let texts = recorded_texts(&recorded);
    assert!(
        texts.iter().any(|text| text.contains("重复调用同一个工具")),
        "第 3 次必须注入循环提醒：{texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|text| text.contains("重复次数过多") || text.contains("强制结束工具调用")),
        "第 5 次必须注入强制收尾提示：{texts:?}"
    );
}

#[tokio::test]
async fn exempt_tool_repeats_are_allowed() {
    let workspace = temp_workspace("exempt");
    std::fs::write(workspace.join("a.txt"), "hello\n").unwrap();
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let mut script = Vec::new();
    for index in 0..6 {
        script.push(call(
            &format!("c{index}"),
            "read_file",
            json!({ "path": "a.txt" }),
        ));
    }
    script.push(ModelOutput::Text("完成".to_string()));
    let provider = ScriptedProvider::new(script, Arc::clone(&recorded));
    let agent = build_agent(&workspace, provider);
    let mut session = Session::new(&workspace, "mock".to_string(), None);
    let abort = AtomicBool::new(false);
    let approver = AutoApprover { allow: true };

    let outcome = agent
        .run_turn(&mut session, "读文件", &approver, &abort, &mut |_| {})
        .await
        .expect("豁免工具重复调用不得打断回合");
    assert_eq!(outcome.final_text.as_deref(), Some("完成"));
    let texts = recorded_texts(&recorded);
    assert!(
        !texts.iter().any(|text| text.contains("重复调用同一个工具")),
        "豁免工具（read_file）不得触发循环检测"
    );
}

#[tokio::test]
async fn slow_tool_times_out_with_structured_error() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    std::env::set_var("OWO_TOOL_TIMEOUT_SECS", "1");
    let workspace = temp_workspace("timeout");
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ToolRegistry::new();
    registry.register(SlowTool {
        delay: std::time::Duration::from_secs(5),
    });
    let provider = ScriptedProvider::new(
        vec![
            call("c1", "slow_tool", json!({})),
            ModelOutput::Text("超时后收尾".to_string()),
        ],
        Arc::clone(&recorded),
    );
    let agent = build_agent_with_registry(&workspace, provider, registry);
    let mut session = Session::new(&workspace, "mock".to_string(), None);
    let abort = AtomicBool::new(false);
    let approver = AutoApprover { allow: true };

    let outcome = agent
        .run_turn(&mut session, "跑慢工具", &approver, &abort, &mut |_| {})
        .await
        .expect("工具超时不得让回合失败");
    std::env::remove_var("OWO_TOOL_TIMEOUT_SECS");

    assert_eq!(outcome.final_text.as_deref(), Some("超时后收尾"));
    let texts = recorded_texts(&recorded);
    assert!(
        texts.iter().any(|text| text.contains("工具执行超时")),
        "超时必须作为结构化工具错误回喂模型：{texts:?}"
    );
}

#[tokio::test]
async fn budget_cap_stops_loop_with_visible_conclusion() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    std::env::set_var("OWO_USAGE_TOKEN_BUDGET", "1000");
    let workspace = temp_workspace("budget");
    let recorded = Arc::new(Mutex::new(Vec::new()));
    // 脚本第 1 轮调用工具；此时 provider 报告的累计用量已超预算 → 第 2 轮开头熔断。
    let provider = ScriptedProvider::new(
        vec![
            call("c1", "list_dir", json!({ "path": "." })),
            ModelOutput::Text("不应到达这里".to_string()),
        ],
        Arc::clone(&recorded),
    )
    .with_usage(50_000);
    let agent = build_agent(&workspace, provider);
    let mut session = Session::new(&workspace, "mock".to_string(), None);
    let abort = AtomicBool::new(false);
    let approver = AutoApprover { allow: true };

    let outcome = agent
        .run_turn(&mut session, "做点事", &approver, &abort, &mut |_| {})
        .await
        .expect("预算熔断不得让回合失败");
    std::env::remove_var("OWO_USAGE_TOKEN_BUDGET");

    let final_text = outcome.final_text.unwrap_or_default();
    assert!(
        final_text.contains("预算"),
        "熔断必须给出可见结论：{final_text}"
    );
    assert!(
        !final_text.contains("不应到达这里"),
        "超预算后不得继续请求模型"
    );
}

#[tokio::test]
async fn no_budget_configured_does_not_stop() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    std::env::remove_var("OWO_USAGE_TOKEN_BUDGET");
    std::env::remove_var("OWO_USAGE_COST_BUDGET_USD");
    let workspace = temp_workspace("no-budget");
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let provider = ScriptedProvider::new(
        vec![ModelOutput::Text("正常完成".to_string())],
        Arc::clone(&recorded),
    )
    .with_usage(9_999_999);
    let agent = build_agent(&workspace, provider);
    let mut session = Session::new(&workspace, "mock".to_string(), None);
    let abort = AtomicBool::new(false);
    let approver = AutoApprover { allow: true };

    let outcome = agent
        .run_turn(&mut session, "随便", &approver, &abort, &mut |_| {})
        .await
        .unwrap();
    assert_eq!(outcome.final_text.as_deref(), Some("正常完成"));
}
