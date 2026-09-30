//! 工作区设置：`<workspace>/settings.json`（默认模型/只读/危险命令/MCP 服务器/v0.4 配置组）。

use crate::mcp::McpServerConfig;
use crate::whitelist::WhitelistEntry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// 语音输入配置（v0.4 D20，默认 SenseVoice-Small 本地转写）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttSettings {
    #[serde(default = "default_stt_model")]
    pub model: String,
    /// SenseVoice 语言（auto / zh / en / ja / ko / yue），可用 OWO_STT_LANGUAGE 覆盖。
    #[serde(default = "default_stt_language")]
    pub language: String,
    /// 是否启用逆文本规范化（ITN），可用 OWO_STT_ITN 覆盖。
    #[serde(default = "default_true")]
    pub itn: bool,
    #[serde(default = "default_false")]
    pub enable_high_accuracy: bool,
    #[serde(default)]
    pub hotwords: Vec<String>,
    #[serde(default = "default_latency_budget")]
    pub latency_budget_ms: u64,
}

impl Default for SttSettings {
    fn default() -> Self {
        Self {
            model: "SenseVoice-Small".to_string(),
            language: "auto".to_string(),
            itn: true,
            enable_high_accuracy: false,
            hotwords: Vec::new(),
            latency_budget_ms: 2000,
        }
    }
}

/// 受限自主探索配置（v0.4 D23，默认 S0 隔离虚拟机层）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExploreSettings {
    #[serde(default = "default_explore_tier")]
    pub default_tier: String,
    #[serde(default = "default_action_budget")]
    pub action_budget: u32,
    #[serde(default = "default_max_duration")]
    pub max_duration_s: u64,
    #[serde(default = "default_false")]
    pub allow_s1: bool,
}

impl Default for ExploreSettings {
    fn default() -> Self {
        Self {
            default_tier: "S0".to_string(),
            action_budget: 50,
            max_duration_s: 600,
            allow_s1: false,
        }
    }
}

/// 主动建议阈值配置（v0.4 D24，默认仅提示不执行）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProactiveSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_weekly_threshold")]
    pub weekly_threshold: u32,
    #[serde(default = "default_daily_threshold")]
    pub daily_threshold: u32,
    #[serde(default = "default_similarity")]
    pub similarity: f64,
    #[serde(default = "default_cooldown_hours")]
    pub cooldown_hours: u32,
    #[serde(default = "default_daily_cap")]
    pub daily_cap: u32,
    #[serde(default = "default_auto_silence_days")]
    pub auto_silence_days: u32,
}

impl Default for ProactiveSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            weekly_threshold: 5,
            daily_threshold: 3,
            similarity: 0.9,
            cooldown_hours: 24,
            daily_cap: 3,
            auto_silence_days: 30,
        }
    }
}

/// 技能包分享/导入配置（v0.4 D26）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsSettings {
    #[serde(default = "default_share_format")]
    pub share_format: String,
    #[serde(default = "default_false")]
    pub require_signature: bool,
    /// 禁用的技能名列表（设置页启用/禁用，重启后从 settings.json 恢复）。
    #[serde(default)]
    pub disabled: Vec<String>,
}

fn default_stt_model() -> String {
    "SenseVoice-Small".to_string()
}

fn default_stt_language() -> String {
    "auto".to_string()
}

fn default_false() -> bool {
    false
}

fn default_true() -> bool {
    true
}

fn default_latency_budget() -> u64 {
    2000
}

fn default_explore_tier() -> String {
    "S0".to_string()
}

fn default_action_budget() -> u32 {
    50
}

fn default_max_duration() -> u64 {
    600
}

fn default_weekly_threshold() -> u32 {
    5
}

fn default_daily_threshold() -> u32 {
    3
}

fn default_similarity() -> f64 {
    0.9
}

fn default_cooldown_hours() -> u32 {
    24
}

fn default_daily_cap() -> u32 {
    3
}

fn default_auto_silence_days() -> u32 {
    30
}

fn default_share_format() -> String {
    "owskill".to_string()
}

impl Default for SkillsSettings {
    fn default() -> Self {
        Self {
            share_format: "owskill".to_string(),
            require_signature: false,
            disabled: Vec::new(),
        }
    }
}

/// 数据出境开关（v0.3 7.5）：关闭后拒绝云端模型调用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EgressSettings {
    #[serde(default = "default_true")]
    pub cloud_enabled: bool,
}

impl Default for EgressSettings {
    fn default() -> Self {
        Self {
            cloud_enabled: true,
        }
    }
}

/// v0.4.30 模型用量预算配置（持久化到 settings.json，运行时写回环境变量）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UsageSettings {
    /// 累计 token 上限（None = 不熔断）。
    #[serde(default)]
    pub token_budget: Option<u64>,
    /// 累计成本上限（美元，None = 不熔断）。
    #[serde(default)]
    pub cost_budget_usd: Option<f64>,
    /// 输入单价（美元/百万 token，0 = 不估算成本）。
    #[serde(default)]
    pub input_price_per_mtok: f64,
    /// 输出单价（美元/百万 token）。
    #[serde(default)]
    pub output_price_per_mtok: f64,
}

/// 模型服务接入（设置页「配置」选项卡的唯一事实源，优先于环境变量）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelProviderSettings {
    /// OpenAI 兼容端点（如 `https://api.deepseek.com/v1`）。留空 = 回退环境变量。
    pub base_url: Option<String>,
    /// API 密钥：只写进加密信封 `settings.json.owo-crypt`，明文 settings.json 恒为 None。
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// 默认模型（低于环境变量与命令行参数）。
    #[serde(default)]
    pub model: Option<String>,
    /// 模型服务接入（端点 + 密钥），设置页保存后优先于环境变量。
    #[serde(default)]
    pub provider: ModelProviderSettings,
    /// 启动默认只读（plan）模式。
    #[serde(default)]
    pub read_only: bool,
    /// 额外危险命令片段（deny 优先）。
    #[serde(default)]
    pub deny_commands: Vec<String>,
    /// 启动时自动连接的 MCP 服务器。
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
    /// TUI 主题：dark / light。
    #[serde(default)]
    pub theme: Option<String>,
    /// TUI 键位：action → 按键描述（如 "tab"、"ctrl+c"、"f2"）。
    #[serde(default)]
    pub keybinds: HashMap<String, String>,
    /// v0.4 语音输入配置。
    #[serde(default)]
    pub stt: SttSettings,
    /// v0.4 自主探索配置。
    #[serde(default)]
    pub explore: ExploreSettings,
    /// v0.4 主动建议配置。
    #[serde(default)]
    pub proactive: ProactiveSettings,
    /// v0.4 技能包分享/导入配置。
    #[serde(default)]
    pub skills: SkillsSettings,
    /// v0.4 应用白名单（可被默认清单覆盖，用户增删）。
    #[serde(default)]
    pub whitelist: Vec<WhitelistEntry>,
    /// 数据出境开关。
    #[serde(default)]
    pub egress: EgressSettings,
    /// v0.4.30 模型用量预算。
    #[serde(default)]
    pub usage: UsageSettings,
    /// 推理档位（`reasoning_effort`）：minimal / low / medium / high。
    /// 留空 = 不发送该参数（用模型自身默认）；只有显式选择时才写入请求体，
    /// 避免不支持该字段的 OpenAI 兼容端点直接 400。
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    /// 权限规则（审批卡片「记住」产生；重启后仍生效，硬拒绝类命令永不入表）。
    #[serde(default)]
    pub permissions: PermissionRulesSettings,
    /// Hooks 生命周期扩展点（A2-1）：`hooks: [{event, matcher?, command}]`。
    /// exit code 2 = 阻断并把 stderr 回喂模型；命令经系统 shell 执行。
    #[serde(default)]
    pub hooks: Vec<crate::hooks::HookConfig>,
}

/// 权限规则持久化容器（写入 `settings.json` 的 `permissions` 节）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PermissionRulesSettings {
    /// 用户记住的规则列表。
    pub rules: Vec<crate::permissions::PermissionRule>,
}

impl Settings {
    pub fn load(workspace: &Path) -> Self {
        let path = workspace.join("settings.json");
        std::fs::read_to_string(path)
            .ok()
            // Windows 编辑器常写 UTF-8 BOM，serde 不识别，先剥离。
            .map(|content| content.trim_start_matches('\u{feff}').to_string())
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, workspace: &Path) -> Result<(), String> {
        // 明文 settings.json 绝不含密钥：先剥掉 api_key 再序列化落盘。
        let mut plain = self.clone();
        plain.provider.api_key = None;
        let path = workspace.join("settings.json");
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&plain).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        // 密钥单独写 DPAPI 信封（绑定当前 Windows 账户，解密不需密码）。
        // 信封持有完整配置（含密钥）且 load_encrypted 优先读它，因此这里必须与明文
        // 同步重写：只在「本次带密钥」时写会留下旧信封，读取时旧值胜出，导致改端点/
        // 模型的保存静默不生效（曾实测 base_url 更新被旧信封覆盖）。
        // 本次未提供密钥（None/空串，读-改-写路径如 egress、白名单同款 save）时，
        // 解密旧信封取已保存的 key 合并，避免把已有密钥冲掉。
        let envelope = workspace.join("settings.json.owo-crypt");
        let provided = self
            .provider
            .api_key
            .as_deref()
            .filter(|key| !key.trim().is_empty())
            .map(str::to_string);
        let existing = if provided.is_none() && envelope.exists() {
            crate::storage_crypto::decrypt_file_envelope(&envelope)
                .ok()
                .and_then(|content| serde_json::from_slice::<Settings>(&content).ok())
                .and_then(|settings| settings.provider.api_key)
                .filter(|key| !key.trim().is_empty())
        } else {
            None
        };
        if let Some(key) = provided.or(existing) {
            let mut full = self.clone();
            full.provider.api_key = Some(key);
            let content = serde_json::to_vec_pretty(&full).map_err(|error| error.to_string())?;
            crate::storage_crypto::encrypt_file_envelope(&envelope, &content)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// 加密落盘（R9）：`settings.json.owo-crypt` 信封加密；settings 仍零明文密钥
    /// （api_key_ref 引用模型不变，加密的是配置整体）。非 Windows 显式失败。
    pub fn save_encrypted(&self, workspace: &Path) -> Result<(), String> {
        let path = workspace.join("settings.json.owo-crypt");
        let content = serde_json::to_vec_pretty(self).map_err(|error| error.to_string())?;
        crate::storage_crypto::encrypt_file_envelope(&path, &content)
            .map_err(|error| error.to_string())
    }

    /// 加密读取（R9）：优先加密文件（解密读取，损坏显式报错不静默回退），
    /// 无加密文件时回退明文 `settings.json`（兼容既有安装）。
    pub fn load_encrypted(workspace: &Path) -> Result<Self, String> {
        let encrypted = workspace.join("settings.json.owo-crypt");
        if encrypted.exists() {
            let content = crate::storage_crypto::decrypt_file_envelope(&encrypted)
                .map_err(|error| format!("settings 解密失败：{error}"))?;
            return serde_json::from_slice(&content).map_err(|error| error.to_string());
        }
        Ok(Self::load(workspace))
    }

    /// 注册已保存的模型接入为进程内覆盖（优先于环境变量）。
    /// 必须在任何 `OpenAiCompatibleConfig::from_env()` 之前调用，否则保存的
    /// 端点/密钥不会生效——CLI 的 build_agent_with_mcp 就在 AppState 之前跑。
    pub fn apply_provider_override(&self) {
        crate::gateway::set_provider_override(
            self.provider.base_url.clone(),
            self.provider.api_key.clone(),
            self.model.clone(),
        );
    }

    /// 把用量预算配置写回环境变量（provider 每次调用前读取，即时生效）。
    /// None 字段清除对应环境变量，避免旧值残留。
    pub fn apply_usage_env(&self) {
        match self.usage.token_budget {
            Some(value) => std::env::set_var("OWO_USAGE_TOKEN_BUDGET", value.to_string()),
            None => std::env::remove_var("OWO_USAGE_TOKEN_BUDGET"),
        }
        match self.usage.cost_budget_usd {
            Some(value) => std::env::set_var("OWO_USAGE_COST_BUDGET_USD", value.to_string()),
            None => std::env::remove_var("OWO_USAGE_COST_BUDGET_USD"),
        }
        std::env::set_var(
            "OWO_MODEL_INPUT_PRICE_PER_MTOK",
            self.usage.input_price_per_mtok.to_string(),
        );
        std::env::set_var(
            "OWO_MODEL_OUTPUT_PRICE_PER_MTOK",
            self.usage.output_price_per_mtok.to_string(),
        );
    }

    /// 把推理档位写回环境变量（provider 每次请求前读取，设置页保存后即时生效）。
    /// 仅接受 minimal/low/medium/high：其余取值（含空串）一律清除变量 = 不下发该参数。
    pub fn apply_reasoning_env(&self) {
        let normalized = self
            .reasoning_effort
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .filter(|value| matches!(value.as_str(), "minimal" | "low" | "medium" | "high"));
        match normalized {
            Some(value) => std::env::set_var("OWO_REASONING_EFFORT", value),
            None => std::env::remove_var("OWO_REASONING_EFFORT"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_settings_from_workspace() {
        let workspace =
            std::env::temp_dir().join(format!("owo-settings-workspace-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            workspace.join("settings.json"),
            r#"{
                "model": "deepseek-v4-flash",
                "read_only": true,
                "deny_commands": ["git push"],
                "mcp_servers": [
                    { "name": "files", "transport": "stdio", "command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "."] }
                ],
                "theme": "light",
                "keybinds": { "toggle_mode": "f2" },
                "stt": { "model": "SenseVoice-Small", "hotwords": ["VSCode", "提交"] },
                "explore": { "default_tier": "S0", "action_budget": 20 },
                "proactive": { "enabled": true, "weekly_threshold": 3 },
                "skills": { "share_format": "owskill" },
                "whitelist": [
                    { "app_id": "code", "name": "VSCode", "tier": "productivity", "learn_allowed": true, "auto_ops_allowed": true }
                ],
                "usage": { "token_budget": 5000, "cost_budget_usd": 1.25, "input_price_per_mtok": 0.3, "output_price_per_mtok": 1.2 }
            }"#,
        )
        .unwrap();
        let settings = Settings::load(&workspace);
        assert_eq!(settings.model.as_deref(), Some("deepseek-v4-flash"));
        assert!(settings.read_only);
        assert_eq!(settings.deny_commands, vec!["git push"]);
        assert_eq!(settings.mcp_servers.len(), 1);
        assert_eq!(settings.mcp_servers[0].name, "files");
        assert_eq!(settings.theme.as_deref(), Some("light"));
        assert_eq!(
            settings.keybinds.get("toggle_mode").map(String::as_str),
            Some("f2")
        );
        assert_eq!(settings.stt.model, "SenseVoice-Small");
        assert_eq!(settings.stt.hotwords, vec!["VSCode", "提交"]);
        assert_eq!(settings.explore.default_tier, "S0");
        assert_eq!(settings.explore.action_budget, 20);
        assert_eq!(settings.proactive.weekly_threshold, 3);
        assert_eq!(settings.proactive.daily_threshold, 3);
        assert_eq!(settings.skills.share_format, "owskill");
        assert_eq!(settings.whitelist.len(), 1);
        assert_eq!(settings.whitelist[0].app_id, "code");
        assert_eq!(settings.usage.token_budget, Some(5000));
        assert_eq!(settings.usage.cost_budget_usd, Some(1.25));
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn loads_settings_with_utf8_bom() {
        let workspace =
            std::env::temp_dir().join(format!("owo-settings-bom-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            workspace.join("settings.json"),
            "\u{feff}{\"model\":\"bom-model\",\"usage\":{\"token_budget\":9000}}",
        )
        .unwrap();
        let settings = Settings::load(&workspace);
        assert_eq!(settings.model.as_deref(), Some("bom-model"));
        assert_eq!(settings.usage.token_budget, Some(9000));
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn missing_settings_returns_defaults() {
        let workspace =
            std::env::temp_dir().join(format!("owo-settings-missing-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let settings = Settings::load(&workspace);
        assert!(settings.model.is_none());
        assert!(!settings.read_only);
        assert!(settings.deny_commands.is_empty());
        assert!(settings.theme.is_none());
        assert!(settings.keybinds.is_empty());
        assert_eq!(settings.stt.model, "SenseVoice-Small");
        assert!(!settings.stt.enable_high_accuracy);
        assert_eq!(settings.explore.default_tier, "S0");
        assert_eq!(settings.explore.action_budget, 50);
        assert!(!settings.explore.allow_s1);
        assert!(settings.proactive.enabled);
        assert_eq!(settings.proactive.weekly_threshold, 5);
        assert_eq!(settings.proactive.similarity, 0.9);
        assert_eq!(settings.skills.share_format, "owskill");
        assert!(!settings.skills.require_signature);
        assert!(settings.whitelist.is_empty());
        assert!(settings.egress.cloud_enabled);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn save_and_load_round_trip_preserves_all_groups() {
        let workspace =
            std::env::temp_dir().join(format!("owo-settings-save-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let settings = Settings {
            model: Some("deepseek-v4-flash".to_string()),
            read_only: true,
            stt: SttSettings {
                model: "Other-Model".to_string(),
                language: "zh".to_string(),
                itn: false,
                ..SttSettings::default()
            },
            proactive: ProactiveSettings {
                enabled: false,
                ..ProactiveSettings::default()
            },
            skills: SkillsSettings {
                disabled: vec!["demo".to_string()],
                ..SkillsSettings::default()
            },
            egress: EgressSettings {
                cloud_enabled: false,
            },
            usage: UsageSettings {
                token_budget: Some(100_000),
                cost_budget_usd: Some(5.0),
                input_price_per_mtok: 0.5,
                output_price_per_mtok: 2.0,
            },
            ..Settings::default()
        };
        settings.save(&workspace).unwrap();
        let loaded = Settings::load(&workspace);
        assert_eq!(loaded.model.as_deref(), Some("deepseek-v4-flash"));
        assert!(loaded.read_only);
        assert_eq!(loaded.stt.model, "Other-Model");
        assert_eq!(loaded.stt.language, "zh");
        assert!(!loaded.stt.itn);
        assert!(!loaded.proactive.enabled);
        assert_eq!(loaded.skills.disabled, vec!["demo"]);
        assert!(!loaded.egress.cloud_enabled);
        assert_eq!(loaded.usage.token_budget, Some(100_000));
        assert_eq!(loaded.usage.cost_budget_usd, Some(5.0));
        assert!((loaded.usage.input_price_per_mtok - 0.5).abs() < 1e-9);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn apply_usage_env_syncs_budget_and_prices() {
        static ENV_LOCK: std::sync::LazyLock<std::sync::Mutex<()>> =
            std::sync::LazyLock::new(|| std::sync::Mutex::new(()));
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let settings = Settings {
            usage: UsageSettings {
                token_budget: Some(42_000),
                cost_budget_usd: Some(3.25),
                input_price_per_mtok: 1.0,
                output_price_per_mtok: 4.0,
            },
            ..Settings::default()
        };
        settings.apply_usage_env();
        assert_eq!(
            std::env::var("OWO_USAGE_TOKEN_BUDGET").as_deref(),
            Ok("42000")
        );
        assert_eq!(
            std::env::var("OWO_USAGE_COST_BUDGET_USD").as_deref(),
            Ok("3.25")
        );
        assert_eq!(
            std::env::var("OWO_MODEL_INPUT_PRICE_PER_MTOK").as_deref(),
            Ok("1")
        );
        assert_eq!(
            std::env::var("OWO_MODEL_OUTPUT_PRICE_PER_MTOK").as_deref(),
            Ok("4")
        );

        // None 清除预算变量（价格始终写回）。
        Settings::default().apply_usage_env();
        assert!(std::env::var("OWO_USAGE_TOKEN_BUDGET").is_err());
        assert!(std::env::var("OWO_USAGE_COST_BUDGET_USD").is_err());
        std::env::remove_var("OWO_MODEL_INPUT_PRICE_PER_MTOK");
        std::env::remove_var("OWO_MODEL_OUTPUT_PRICE_PER_MTOK");
    }

    #[test]
    fn apply_reasoning_env_only_accepts_known_levels() {
        // 与 gateway 的档位下发测试共用同一把锁：两处都读写 OWO_REASONING_EFFORT。
        let _guard = crate::ENV_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut settings = Settings {
            reasoning_effort: Some(" HIGH ".to_string()),
            ..Settings::default()
        };
        settings.apply_reasoning_env();
        assert_eq!(std::env::var("OWO_REASONING_EFFORT").as_deref(), Ok("high"));

        // 非法档位 → 清除变量（等于不下发该参数，回落模型默认）。
        settings.reasoning_effort = Some("unsupported".to_string());
        settings.apply_reasoning_env();
        assert!(std::env::var("OWO_REASONING_EFFORT").is_err());

        settings.reasoning_effort = Some("medium".to_string());
        settings.apply_reasoning_env();
        assert_eq!(
            std::env::var("OWO_REASONING_EFFORT").as_deref(),
            Ok("medium")
        );

        settings.reasoning_effort = None;
        settings.apply_reasoning_env();
        assert!(std::env::var("OWO_REASONING_EFFORT").is_err());
    }
}
