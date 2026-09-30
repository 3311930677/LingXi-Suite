//! E5 验收：`update_plan` 工具（计划写入会话 + PlanUpdate 事件）。
//!
//! 变异验收口径：注释掉 agent 循环里的计划快照对比 → `update_plan_emits_plan_update_event` 变红。

use async_trait::async_trait;
use owo_agent_core::permissions::{AutoApprover, Policy};
use owo_agent_core::tools::ToolRegistry;
use owo_agent_core::{
    Agent, AgentConfig, ChatMessage, ModelOutput, ModelProvider, Session, TokenUsage, ToolCall,
    ToolSpec, TurnEvent,
};
use serde_json::json;
use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

struct ScriptedProvider {
    script: Mutex<VecDeque<ModelOutput>>,
    recorded: Arc<Mutex<Vec<ChatMessage>>>,
}

impl ScriptedProvider {
    fn new(outputs: Vec<ModelOutput>, recorded: Arc<Mutex<Vec<ChatMessage>>>) -> Self {
        Self {
            script: Mutex::new(outputs.into()),
            recorded,
        }
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
        TokenUsage::default()
    }
}

fn call(id: &str, name: &str, args: serde_json::Value) -> ModelOutput {
    ModelOutput::ToolCalls(vec![ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments: args,
    }])
}

fn temp_workspace(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("owo-plan-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn build_agent<P>(workspace: &std::path::Path, provider: P) -> Agent
where
    P: ModelProvider + Send + Sync + 'static,
{
    Agent::new(
        Arc::new(provider),
        ToolRegistry::new(),
        Policy::new(workspace.to_path_buf()),
        AgentConfig::default(),
    )
}

#[tokio::test]
async fn update_plan_writes_session_and_emits_event() {
    let workspace = temp_workspace("write");
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let provider = ScriptedProvider::new(
        vec![
            call(
                "c1",
                "update_plan",
                json!({
                    "steps": [
                        { "content": "阅读代码", "status": "completed" },
                        { "content": "修改实现", "status": "in_progress" },
                        { "content": "运行测试", "status": "pending" }
                    ]
                }),
            ),
            ModelOutput::Text("计划已建立".to_string()),
        ],
        Arc::clone(&recorded),
    );
    let agent = build_agent(&workspace, provider);
    let mut session = Session::new(&workspace, "mock".to_string(), None);
    let abort = AtomicBool::new(false);
    let approver = AutoApprover { allow: true };
    let mut events: Vec<TurnEvent> = Vec::new();

    let outcome = agent
        .run_turn(
            &mut session,
            "处理多步任务",
            &approver,
            &abort,
            &mut |event| events.push(event.clone()),
        )
        .await
        .expect("回合应正常完成");

    assert_eq!(outcome.final_text.as_deref(), Some("计划已建立"));

    // 计划写入会话（整表替换语义）。
    let plan = session.plan.as_ref().expect("计划必须写入会话");
    assert_eq!(plan.steps.len(), 3);
    assert_eq!(plan.steps[0].status, "completed");
    assert_eq!(plan.in_progress(), 1);
    assert_eq!(plan.pending(), 1);
    assert_eq!(plan.completed(), 1);

    // PlanUpdate 事件已发出（UI 进度可渲染）。
    assert!(
        events.iter().any(|event| matches!(
            event,
            TurnEvent::PlanUpdate { steps } if steps.len() == 3
        )),
        "必须发出 PlanUpdate 事件：{events:?}"
    );
}

#[tokio::test]
async fn invalid_plan_is_rejected_and_session_unchanged() {
    let workspace = temp_workspace("invalid");
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let provider = ScriptedProvider::new(
        vec![
            call(
                "c1",
                "update_plan",
                json!({ "steps": [{ "content": "x", "status": "done" }] }),
            ),
            ModelOutput::Text("收到错误".to_string()),
        ],
        Arc::clone(&recorded),
    );
    let agent = build_agent(&workspace, provider);
    let mut session = Session::new(&workspace, "mock".to_string(), None);
    let abort = AtomicBool::new(false);
    let approver = AutoApprover { allow: true };

    let outcome = agent
        .run_turn(&mut session, "写计划", &approver, &abort, &mut |_| {})
        .await
        .expect("工具错误不得让回合失败");
    assert_eq!(outcome.final_text.as_deref(), Some("收到错误"));
    assert!(session.plan.is_none(), "非法计划不得写入会话");

    let texts: Vec<String> = recorded
        .lock()
        .unwrap()
        .iter()
        .filter_map(|message| message.content.clone())
        .collect();
    assert!(
        texts.iter().any(|text| text.contains("status 非法")),
        "工具错误必须回喂模型：{texts:?}"
    );
    assert!(
        !outcome
            .events
            .iter()
            .any(|event| matches!(event, TurnEvent::PlanUpdate { .. })),
        "非法计划不得产生 PlanUpdate 事件"
    );
}

#[tokio::test]
async fn plan_tool_is_exempt_from_approval() {
    // 契约：update_plan 属免审批工具（Level::Read），否则「要审批才能建计划」体验极差。
    assert_eq!(
        Policy::level_for("update_plan"),
        owo_agent_core::permissions::Level::Read
    );
}
