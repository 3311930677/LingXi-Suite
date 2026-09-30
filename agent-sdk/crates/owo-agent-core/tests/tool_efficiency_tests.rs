//! 工具效能基准：量化「改造前 vs 改造后」同一任务的上下文成本。
//!
//! 任务原型：在一个 500 行的源文件里定位目标函数并做一处局部修改。
//! - 旧路径：`list_dir` 浏览 → `read_file` 整文件读多个候选 → `write_file` 整文件重写
//! - 新路径：`grep` 定位 → `read_file(offset/limit)` 分页读目标区段 → `edit_file` 精准替换
//!
//! 成本口径：工具返回结果的序列化字符数之和（上下文占用的代理指标）。
//! 本基准不依赖模型，可在 CI 中守住「工具信息密度」不退化。

use owo_agent_core::audit::AuditLog;
use owo_agent_core::permissions::Policy;
use owo_agent_core::session::Session;
use owo_agent_core::skill::SkillRegistry;
use owo_agent_core::tools::{ToolContext, ToolRegistry};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

/// 目标文件的总行数。
const TARGET_LINES: usize = 500;
/// 干扰文件数量（模拟真实项目里同类文件很多）。
const NOISE_FILES: usize = 40;

fn fixture_workspace() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("owo-bench-{}", uuid_like_tag()));
    std::fs::create_dir_all(&dir).unwrap();

    // 目标文件：第 300 行附近含可识别函数。
    let mut body = String::new();
    for index in 1..=TARGET_LINES {
        if index == 300 {
            body.push_str("fn reconcile(&self) -> usize {\n    self.pending\n}\n");
        } else {
            body.push_str(&format!(
                "// filler line {index}\n    let _x{index} = {index};\n"
            ));
        }
    }
    std::fs::write(dir.join("target.rs"), &body).unwrap();

    // 干扰文件：同样有很多行，逼迫旧路径要么浏览要么整读。
    for index in 0..NOISE_FILES {
        let mut noise = String::new();
        for line in 0..120 {
            noise.push_str(&format!("pub fn noise_{index}_{line}() {{}}\n"));
        }
        std::fs::write(dir.join(format!("noise_{index}.rs")), noise).unwrap();
    }
    dir
}

fn uuid_like_tag() -> String {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let nonce = COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("{}-{}", std::process::id(), nonce)
}

struct Env {
    workspace: std::path::PathBuf,
    session: Session,
    policy: Policy,
    audit: Arc<Mutex<AuditLog>>,
    skills: SkillRegistry,
    elements: Arc<Mutex<owo_agent_core::ElementRegistry>>,
    registry: ToolRegistry,
    cost_chars: usize,
}

impl Env {
    fn new(workspace: &std::path::Path) -> Self {
        Self {
            workspace: workspace.to_path_buf(),
            session: Session::new(workspace, "bench", None),
            policy: Policy::new(workspace),
            audit: Arc::new(Mutex::new(AuditLog::default())),
            skills: SkillRegistry::default(),
            elements: Arc::new(Mutex::new(owo_agent_core::ElementRegistry::new())),
            registry: ToolRegistry::new(),
            cost_chars: 0,
        }
    }

    /// 调用工具并累计返回字符数作为上下文成本。
    async fn invoke(&mut self, name: &str, args: Value) -> Result<Value, String> {
        let tool = self
            .registry
            .get(name)
            .ok_or_else(|| format!("未知工具：{name}"))?;
        let value = match tool.as_read_only() {
            Some(read_only) => {
                read_only
                    .run_read_only(&self.workspace, &self.policy, args)
                    .await?
            }
            None => {
                let (workspace, session, policy, audit, skills, elements) = (
                    &self.workspace,
                    &mut self.session,
                    &self.policy,
                    &self.audit,
                    &self.skills,
                    &self.elements,
                );
                let mut ctx = ToolContext {
                    workspace,
                    policy,
                    session,
                    audit,
                    subagent: None,
                    skills,
                    elements,
                    questioner: None,
                    fanout: None,
                    abort: None,
                };
                tool.run(&mut ctx, args).await?
            }
        };
        self.cost_chars += serde_json::to_string(&value).map(|s| s.len()).unwrap_or(0);
        Ok(value)
    }
}

/// 旧路径：列目录 → 逐个整读候选文件 → 整文件重写。
async fn legacy_flow(env: &mut Env) {
    env.invoke("list_dir", json!({})).await.unwrap();
    // 模型看不到内容，只能挑候选整读（这里读 4 个文件，含目标文件）。
    for candidate in ["noise_0.rs", "noise_1.rs", "noise_2.rs", "target.rs"] {
        env.invoke("read_file", json!({ "path": candidate }))
            .await
            .unwrap();
    }
    let whole = std::fs::read_to_string(env.workspace.join("target.rs")).unwrap();
    let patched = whole.replace("self.pending", "self.pending + 1");
    env.invoke(
        "write_file",
        json!({ "path": "target.rs", "content": patched }),
    )
    .await
    .unwrap();
}

/// 新路径：grep 定位 → 分页读目标区段 → 精准替换。
async fn modern_flow(env: &mut Env) {
    env.invoke("grep", json!({ "pattern": "fn reconcile", "glob": "*.rs" }))
        .await
        .unwrap();
    env.invoke(
        "read_file",
        json!({ "path": "target.rs", "offset": 295, "limit": 10 }),
    )
    .await
    .unwrap();
    env.invoke(
        "edit_file",
        json!({
            "path": "target.rs",
            "old_str": "    self.pending\n}",
            "new_str": "    self.pending + 1\n}",
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn modern_tooling_cuts_context_cost_by_large_margin() {
    let base = fixture_workspace();
    let mut legacy = Env::new(&base);
    legacy_flow(&mut legacy).await;

    // 同样的工作量必须落到同一个文件内容上。
    let expected = std::fs::read_to_string(base.join("target.rs")).unwrap();
    assert!(expected.contains("self.pending + 1"), "旧路径应完成修改");

    // 还原文件内容，跑新路径。
    let restored = expected.replace("self.pending + 1", "self.pending");
    std::fs::write(base.join("target.rs"), &restored).unwrap();

    let mut modern = Env::new(&base);
    modern_flow(&mut modern).await;
    let final_content = std::fs::read_to_string(base.join("target.rs")).unwrap();
    assert!(
        final_content.contains("self.pending + 1"),
        "新路径应完成同样的修改"
    );

    println!(
        "[tool-bench] legacy={} chars, modern={} chars, ratio={:.1}%",
        legacy.cost_chars,
        modern.cost_chars,
        (modern.cost_chars as f64 / legacy.cost_chars.max(1) as f64) * 100.0
    );

    // 守擂：新路径上下文成本必须显著低于旧路径（经验值 ≥ 5 倍差距）。
    assert!(
        modern.cost_chars * 5 < legacy.cost_chars,
        "工具信息密度退化：modern={} legacy={}",
        modern.cost_chars,
        legacy.cost_chars
    );

    let _ = std::fs::remove_dir_all(&base);
}
