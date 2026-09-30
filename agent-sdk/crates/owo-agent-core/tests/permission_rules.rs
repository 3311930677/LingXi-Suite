//! E2 验收：权限规则（审批「记住」）与持久化。
//!
//! 变异验收口径：把 `Policy::remember_rule` 的写入逻辑注释掉，
//! `remember_allows_same_command_prefix_second_time` 与
//! `settings_roundtrip_persists_rules` 必须变红。

use std::time::Duration;

use owo_agent_core::permissions::{Decision, Policy, RuleDecision};
use owo_agent_core::{PermissionRule, Settings};
use serde_json::json;

fn temp_workspace(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("owo-perm-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("创建临时工作区");
    dir
}

#[test]
fn remember_allows_same_command_prefix_second_time() {
    let policy = Policy::new(".");
    let request = policy.evaluate("run_command", &json!({ "command": "cargo test -p x" }));
    assert_eq!(policy.decision(&request), Decision::Ask, "首次必须询问");

    let rule = policy
        .remember_rule(
            "run_command",
            &json!({ "command": "cargo test -p x" }),
            None,
        )
        .expect("常规命令必须可记住");
    assert_eq!(rule.pattern, "cargo *", "命令类规则取首 token 前缀");

    // 同前缀不再询问；不同命令仍询问。
    let again = policy.evaluate("run_command", &json!({ "command": "cargo build" }));
    assert_eq!(policy.decision(&again), Decision::Allow);
    let other = policy.evaluate("run_command", &json!({ "command": "npm install" }));
    assert_eq!(policy.decision(&other), Decision::Ask);
}

#[test]
fn hard_deny_commands_never_remembered() {
    let policy = Policy::new(".");
    let request = policy.evaluate("run_command", &json!({ "command": "rm -rf /tmp/x" }));
    assert_eq!(policy.decision(&request), Decision::Deny, "危险命令硬拒绝");

    assert!(
        policy
            .remember_rule("run_command", &json!({ "command": "rm -rf /tmp/x" }), None)
            .is_none(),
        "危险命令不可记住"
    );
    assert!(policy.rules().is_empty(), "不可记住的请求不得写入规则表");

    // 即使外部配置强行写入通配 Allow 规则，硬拒绝仍然优先。
    policy.add_rule(PermissionRule {
        tool: "run_command".to_string(),
        pattern: "*".to_string(),
        decision: RuleDecision::Allow,
        expires_at_ms: None,
        note: "强行写入（测试）".to_string(),
    });
    let still_denied = policy.evaluate("run_command", &json!({ "command": "rm -rf /tmp/x" }));
    assert_eq!(
        policy.decision(&still_denied),
        Decision::Deny,
        "硬拒绝优先于用户规则"
    );
    // 通配规则对其余命令仍生效（证明规则确实在表里）。
    let ok = policy.evaluate("run_command", &json!({ "command": "echo hi" }));
    assert_eq!(policy.decision(&ok), Decision::Allow);
}

#[test]
fn path_rule_scoped_to_directory() {
    let workspace = temp_workspace("path-rule");
    let policy = Policy::new(&workspace);
    let target = json!({ "path": "src/lib.rs" });
    assert_eq!(
        policy.decision(&policy.evaluate("write_file", &target)),
        Decision::Ask
    );

    let rule = policy
        .remember_rule("write_file", &target, None)
        .expect("写文件可记住");
    assert_eq!(rule.pattern, "src/**", "路径类规则取父目录前缀");

    assert_eq!(
        policy.decision(&policy.evaluate("write_file", &json!({ "path": "src/main.rs" }))),
        Decision::Allow,
        "同目录放行"
    );
    assert_eq!(
        policy.decision(&policy.evaluate("write_file", &json!({ "path": "docs/a.md" }))),
        Decision::Ask,
        "其他目录仍询问"
    );
    assert_eq!(
        policy.decision(&policy.evaluate("edit_file", &json!({ "path": "src/lib.rs" }))),
        Decision::Ask,
        "规则按工具隔离"
    );
}

#[test]
fn expired_rule_is_ignored() {
    let policy = Policy::new(".");
    let rule = policy
        .remember_rule("run_command", &json!({ "command": "cargo test" }), None)
        .unwrap();
    policy.replace_rules(vec![PermissionRule {
        expires_at_ms: Some(1), // 1970 年即过期
        ..rule
    }]);
    let request = policy.evaluate("run_command", &json!({ "command": "cargo test" }));
    assert_eq!(policy.decision(&request), Decision::Ask, "过期规则不得生效");
}

#[test]
fn ttl_rule_expires_after_deadline() {
    let policy = Policy::new(".");
    policy
        .remember_rule(
            "run_command",
            &json!({ "command": "cargo test" }),
            Some(Duration::from_millis(1)),
        )
        .unwrap();
    std::thread::sleep(Duration::from_millis(25));
    assert_eq!(
        policy.decision(&policy.evaluate("run_command", &json!({ "command": "cargo test" }))),
        Decision::Ask
    );
}

#[test]
fn settings_roundtrip_persists_rules() {
    let workspace = temp_workspace("settings-roundtrip");
    let policy = Policy::new(&workspace);
    policy
        .remember_rule("write_file", &json!({ "path": "src/lib.rs" }), None)
        .unwrap();
    let rules = policy.rules();
    assert_eq!(rules.len(), 1);

    let mut settings = Settings::load(&workspace);
    settings.permissions.rules = rules;
    settings.save(&workspace).expect("保存 settings");

    let reloaded = Settings::load(&workspace);
    assert_eq!(reloaded.permissions.rules.len(), 1, "重启后规则必须仍在");
    assert_eq!(reloaded.permissions.rules[0].tool, "write_file");
    assert_eq!(reloaded.permissions.rules[0].pattern, "src/**");

    // 重新灌入策略 → 命中。
    let restored = Policy::new(&workspace);
    restored.replace_rules(reloaded.permissions.rules);
    assert_eq!(
        restored.decision(&restored.evaluate("write_file", &json!({ "path": "src/main.rs" }))),
        Decision::Allow
    );
}

#[test]
fn scoped_policy_shares_rules() {
    let base = Policy::new(".");
    let session = base.scoped_to("sub");
    session
        .remember_rule("run_command", &json!({ "command": "cargo test" }), None)
        .unwrap();
    // 派生策略共享热状态：基策略也看得到规则并生效。
    assert_eq!(base.rules().len(), 1);
    assert_eq!(
        base.decision(&base.evaluate("run_command", &json!({ "command": "cargo test" }))),
        Decision::Allow
    );
}

#[test]
fn same_tool_pattern_is_overwritten_not_duplicated() {
    let policy = Policy::new(".");
    policy
        .remember_rule("run_command", &json!({ "command": "cargo test" }), None)
        .unwrap();
    policy
        .remember_rule("run_command", &json!({ "command": "cargo build" }), None)
        .unwrap();
    let rules = policy.rules();
    assert_eq!(rules.len(), 1, "同 tool+pattern 必须覆盖");
    assert_eq!(rules[0].pattern, "cargo *");
}

#[test]
fn generic_tools_use_wildcard_pattern() {
    let policy = Policy::new(".");
    let rule = policy
        .remember_rule("desktop_click", &json!({ "x": 10, "y": 20 }), None)
        .expect("桌面工具可记住");
    assert_eq!(rule.pattern, "*", "无结构参数工具按工具粒度记住");
    assert_eq!(
        policy.decision(&policy.evaluate("desktop_click", &json!({ "x": 1, "y": 2 }))),
        Decision::Allow
    );
}
