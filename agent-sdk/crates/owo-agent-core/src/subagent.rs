//! 子代理：主 Agent 派生的嵌套会话（explore 只读 / subagent 通用 / fan-out 并行）。

use crate::agent::{Agent, AgentConfig, TurnEvent};
use crate::gateway::ModelProvider;
use crate::permissions::{Approver, AutoApprover, Policy};
use crate::session::Session;
use crate::tools::ToolRegistry;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub const MAX_SUBAGENT_DEPTH: usize = 2;

/// 只读子代理系统提示（单任务 explore 与 fan-out 共用）。
pub(crate) const READ_ONLY_SUBAGENT_PROMPT: &str =
    "你是只读探索子代理：只能读取/搜索工作区文件，禁止写入或执行命令；\
     调查完成后用简洁中文汇报发现。";

/// fan-out 工具的注入通道（A5-1）：与 [`SubagentRunner`] 不同，它是 **owned 数据**
/// 且要求 `'static`（[`crate::fleet::fan_out_cfg`] 的闭包约束），因此单独成型。
#[derive(Clone)]
pub struct FanOutRunner {
    pub provider: Arc<dyn ModelProvider>,
    pub workspace: PathBuf,
    pub model: String,
    pub depth: usize,
    pub max_turns: usize,
}

/// A5-1 并行 fan-out：同时派出多个**只读**子代理，返回部分成功报告。
///
/// - 并发上限 / 单任务超时 / 整体时长预算 / 取消传播由
///   [`crate::fleet::FanOutConfig`] 控制（`fan_out_cfg` 已具备完整仲裁语义）；
/// - **只读限定**：多子代理并行写同一工作区存在冲突风险，写类任务请串行走
///   `subagent`；只读策略同时天然禁掉网络/执行类工具（Policy::read_only 拒非 Read）；
/// - 每个子代理独立会话、独立失败（单失败不影响其余，结果按输入顺序返回）。
pub async fn fan_out_subagents(
    provider: Arc<dyn ModelProvider>,
    workspace: PathBuf,
    model: String,
    depth: usize,
    max_turns: usize,
    prompts: Vec<String>,
    mut out_config: crate::fleet::FanOutConfig,
) -> Result<crate::fleet::FanOutReport, String> {
    if depth >= MAX_SUBAGENT_DEPTH {
        return Err(format!("子代理深度超限（最多 {MAX_SUBAGENT_DEPTH} 层）"));
    }
    if prompts.is_empty() {
        return Err("fan-out 任务列表为空".to_string());
    }
    // 取消标志：调用方未提供时内建一个——主回合取消经工具层桥接到该标志，
    // fan_out_cfg 停止调度新子任务并 abort 在飞者（已成功结果保留）。
    let cancelled = out_config
        .cancelled
        .clone()
        .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
    out_config.cancelled = Some(Arc::clone(&cancelled));
    let workers: Vec<String> = (0..prompts.len())
        .map(|index| format!("subagent-{index}"))
        .collect();
    let prompts = Arc::new(prompts);
    let report = crate::fleet::fan_out_cfg(
        &workers,
        out_config,
        "subagent-fanout",
        move |worker| {
            let provider = Arc::clone(&provider);
            let workspace = workspace.clone();
            let model = model.clone();
            let prompts = Arc::clone(&prompts);
            let cancelled = Arc::clone(&cancelled);
            async move {
                let index: usize = worker
                    .strip_prefix("subagent-")
                    .and_then(|value| value.parse().ok())
                    .ok_or_else(|| format!("worker 名解析失败：{worker}"))?;
                let prompt = prompts
                    .get(index)
                    .cloned()
                    .ok_or_else(|| format!("任务下标越界：{index}"))?;
                let policy = Policy::read_only(workspace.clone());
                let registry = ToolRegistry::read_only();
                let config = AgentConfig {
                    max_turns: max_turns.min(12),
                    subagent_depth: depth + 1,
                    ..Default::default()
                };
                let agent = Agent::new(provider, registry, policy, config);
                let mut session = Session::new(
                    &workspace,
                    model,
                    Some(READ_ONLY_SUBAGENT_PROMPT.to_string()),
                );
                let approver = AutoApprover { allow: true };
                let mut on_event = |_event: &TurnEvent| {};
                let outcome = agent
                    .run_turn(
                        &mut session,
                        &prompt,
                        &approver,
                        cancelled.as_ref(),
                        &mut on_event,
                    )
                    .await
                    .map_err(|error| format!("子代理失败：{error}"))?;
                Ok(outcome
                    .final_text
                    .unwrap_or_else(|| format!("（无最终文本，共 {} 步）", outcome.steps)))
            }
        },
    )
    .await;
    Ok(report)
}

pub struct SubagentRunner<'a> {
    pub provider: Arc<dyn ModelProvider>,
    pub approver: &'a dyn Approver,
    pub abort: &'a AtomicBool,
    pub depth: usize,
    pub max_turns: usize,
    pub model: String,
}

impl SubagentRunner<'_> {
    /// 在只读或完整模式下运行一个子会话，返回最终文本。
    pub async fn run(
        &self,
        workspace: &Path,
        prompt: &str,
        read_only: bool,
    ) -> Result<String, String> {
        if self.depth >= MAX_SUBAGENT_DEPTH {
            return Err(format!("子代理深度超限（最多 {MAX_SUBAGENT_DEPTH} 层）"));
        }
        let policy = if read_only {
            Policy::read_only(workspace.to_path_buf())
        } else {
            Policy::new(workspace.to_path_buf())
        };
        let registry = if read_only {
            ToolRegistry::read_only()
        } else {
            ToolRegistry::new()
        };
        let config = AgentConfig {
            max_turns: self.max_turns.min(12),
            subagent_depth: self.depth + 1,
            ..Default::default()
        };
        let agent = Agent::new(Arc::clone(&self.provider), registry, policy, config);
        let system_prompt = if read_only {
            READ_ONLY_SUBAGENT_PROMPT
        } else {
            "你是通用子代理：独立完成委派任务，工具调用仍需审批，完成后汇报结果。"
        };
        let mut session = Session::new(
            workspace,
            self.model.clone(),
            Some(system_prompt.to_string()),
        );
        let mut on_event = |_event: &TurnEvent| {};
        let outcome = agent
            .run_turn(
                &mut session,
                prompt,
                self.approver,
                self.abort,
                &mut on_event,
            )
            .await
            .map_err(|error| format!("子代理执行失败：{error}"))?;
        Ok(outcome
            .final_text
            .unwrap_or_else(|| format!("（子代理无最终文本，共 {} 步）", outcome.steps)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::{ChatMessage, ModelOutput, ModelProvider};
    use crate::permissions::AutoApprover;
    use crate::tools::ToolSpec;
    use async_trait::async_trait;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    struct FixedProvider;

    #[async_trait]
    impl ModelProvider for FixedProvider {
        async fn complete(
            &self,
            _messages: &[ChatMessage],
            _tools: &[ToolSpec],
        ) -> Result<ModelOutput, String> {
            Ok(ModelOutput::Text("ok".to_string()))
        }
    }

    #[tokio::test]
    async fn depth_limit_blocks_nested_run() {
        let workspace = std::env::temp_dir();
        let runner = SubagentRunner {
            provider: Arc::new(FixedProvider),
            approver: &AutoApprover { allow: true },
            abort: &AtomicBool::new(false),
            depth: MAX_SUBAGENT_DEPTH,
            max_turns: 5,
            model: "mock".to_string(),
        };
        let result = runner.run(&workspace, "x", true).await;
        assert!(result.unwrap_err().contains("深度超限"));
    }

    /// 可编排 mock：按 prompt 内容决定 sleep 时长与是否失败；记录峰值并发。
    struct ScriptedFanoutProvider {
        delay_ms: u64,
        peak: Arc<std::sync::atomic::AtomicUsize>,
        inflight: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait]
    impl ModelProvider for ScriptedFanoutProvider {
        async fn complete(
            &self,
            messages: &[ChatMessage],
            _tools: &[ToolSpec],
        ) -> Result<ModelOutput, String> {
            let current = self.inflight.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(current, Ordering::SeqCst);
            // 只匹配最后一条消息（当前用户 prompt）——system prompt 文本不可作为
            // 编排信号（否则「失败」等词会误伤全部子代理）。
            let prompt = messages
                .last()
                .and_then(|message| message.content.clone())
                .unwrap_or_default();
            if prompt.contains("慢") {
                tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            }
            self.inflight.fetch_sub(1, Ordering::SeqCst);
            if prompt.contains("失败") {
                return Err("scripted failure".to_string());
            }
            Ok(ModelOutput::Text(format!("结论：{prompt}")))
        }
    }

    fn fanout_provider(
        delay_ms: u64,
    ) -> (
        Arc<dyn ModelProvider>,
        Arc<std::sync::atomic::AtomicUsize>,
    ) {
        let peak = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let provider = Arc::new(ScriptedFanoutProvider {
            delay_ms,
            peak: Arc::clone(&peak),
            inflight: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        });
        (provider, peak)
    }

    fn fanout_config(max_parallel: usize) -> crate::fleet::FanOutConfig {
        crate::fleet::FanOutConfig {
            max_parallel,
            budget: crate::fleet::Budget {
                max_duration_secs: 60,
                ..Default::default()
            },
            per_worker_timeout: Some(Duration::from_secs(10)),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn fan_out_runs_subagents_in_parallel_and_isolates_failure() {
        let (provider, peak) = fanout_provider(300);
        let prompts = vec![
            "慢任务一".to_string(),
            "慢任务二".to_string(),
            "失败任务".to_string(),
        ];
        let start = Instant::now();
        let report = fan_out_subagents(
            provider,
            std::env::temp_dir(),
            "mock".to_string(),
            0,
            4,
            prompts,
            fanout_config(3),
        )
        .await
        .unwrap();
        let elapsed = start.elapsed();
        // 失败隔离：2 成功 1 失败，顺序与输入一致。
        assert_eq!(report.succeeded().len(), 2, "{report:?}");
        assert_eq!(report.failed().len(), 1, "{report:?}");
        assert_eq!(report.outcomes[0].worker, "subagent-0");
        assert_eq!(report.outcomes[2].status, crate::fleet::FanOutStatus::Failed);
        // 并行：3 个 300ms 任务串行需 ~900ms，并发（上限 3）应明显更快。
        assert!(
            elapsed < Duration::from_millis(750),
            "应并行执行，实际耗时 {elapsed:?}"
        );
        assert!(peak.load(Ordering::SeqCst) >= 2, "并发度未体现");
    }

    #[tokio::test]
    async fn fan_out_respects_max_parallel() {
        let (provider, peak) = fanout_provider(200);
        let prompts = vec!["慢A".to_string(), "慢B".to_string(), "慢C".to_string()];
        let report = fan_out_subagents(
            provider,
            std::env::temp_dir(),
            "mock".to_string(),
            0,
            4,
            prompts,
            fanout_config(1),
        )
        .await
        .unwrap();
        assert_eq!(report.succeeded().len(), 3);
        assert_eq!(peak.load(Ordering::SeqCst), 1, "max_parallel=1 应串行");
    }

    #[tokio::test]
    async fn fan_out_cancel_keeps_completed_results() {
        let (provider, _) = fanout_provider(2_000);
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut config = fanout_config(1);
        config.cancelled = Some(Arc::clone(&cancelled));
        // 第一个任务立即完成，第二个在飞时取消。
        let mut prompts = vec!["快任务".to_string()];
        prompts.push("慢任务一".to_string());
        prompts.push("慢任务二".to_string());
        let handle = tokio::spawn({
            let cancelled = Arc::clone(&cancelled);
            async move {
                tokio::time::sleep(Duration::from_millis(300)).await;
                cancelled.store(true, Ordering::SeqCst);
            }
        });
        let report = fan_out_subagents(
            provider,
            std::env::temp_dir(),
            "mock".to_string(),
            0,
            4,
            prompts,
            config,
        )
        .await
        .unwrap();
        handle.await.unwrap();
        // 已成功的结果保留；被取消的子任务不在成功集合。
        assert_eq!(report.succeeded().len(), 1, "{report:?}");
        assert!(report.failed().len() >= 2, "{report:?}");
    }

    #[tokio::test]
    async fn fan_out_rejects_depth_overflow_and_empty_tasks() {
        let (provider, _) = fanout_provider(10);
        let error = fan_out_subagents(
            Arc::clone(&provider),
            std::env::temp_dir(),
            "mock".to_string(),
            MAX_SUBAGENT_DEPTH,
            4,
            vec!["a".to_string(), "b".to_string()],
            fanout_config(2),
        )
        .await
        .unwrap_err();
        assert!(error.contains("深度超限"), "{error}");
        let error = fan_out_subagents(
            provider,
            std::env::temp_dir(),
            "mock".to_string(),
            0,
            4,
            Vec::new(),
            fanout_config(2),
        )
        .await
        .unwrap_err();
        assert!(error.contains("任务列表为空"), "{error}");
    }
}
