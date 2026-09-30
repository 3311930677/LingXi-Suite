use std::path::Path;

const RULE_FILES: &[&str] = &["AGENTS.md", "CLAUDE.md"];

/// 读取工作区项目规则（AGENTS.md / CLAUDE.md），作为系统指令的一部分。
pub fn load_project_rules(workspace: &Path) -> String {
    let mut rules = Vec::new();
    for name in RULE_FILES {
        let path = workspace.join(name);
        if let Ok(content) = std::fs::read_to_string(&path) {
            rules.push(format!("### {} 规则（必须遵守）\n{}", name, content.trim()));
        }
    }
    rules.join("\n\n")
}

pub fn build_system_prompt(configured: Option<&str>, rules: &str) -> String {
    let mut parts = Vec::new();
    if let Some(configured) = configured {
        parts.push(configured.to_string());
    }
    if !rules.is_empty() {
        parts.push(rules.to_string());
    }
    parts.push(
        "你是 OwO Agent SDK 驱动的智能体。工具调用必须经过权限审批；\
         被拒绝的操作不要重试同一参数，应寻找更安全的替代方案；\
         完成工作后给出简洁的最终汇报。"
            .to_string(),
    );
    parts.push(
        "工程纪律（必须遵守）：\n\
         1. 修改文件前必须先用 read_file 读到目标区段；edit_file 的 old_str 以读到的内容为准（含缩进与空行）。\n\
         2. 查找代码、定义、用法优先用 grep 按内容搜索，不要用 list_dir 盲目遍历目录。\n\
         3. 长文件用 offset/limit 分页读取；引用代码位置时使用行号。\n\
         4. 局部修改用 edit_file（old_str 必须唯一）；只有新建文件或整文件重写才用 write_file。\n\
         5. 改动代码后，如工作区有测试/构建脚本，先运行验证再汇报结果。\n\
         6. 工具报错是信息不是失败：读错误详情、调整参数后重试；同一错误连续出现 2 次必须换思路。\n\
         7. 不要假装成功；每个结论都应有工具结果支撑。"
            .to_string(),
    );
    parts.push(
        "回合收尾（必须遵守）：\n\
         1. 每个回合都必须以可见的最终回答结束：给出结论、产出物或明确的下一步，不允许停在中途没有任何输出。\n\
         2. 审查、分析、调研类任务必须产出结构化 Markdown 报告（结论 / 证据 / 风险或问题 / 建议）；\
         用户要求报告文件时写入工作区并在回答里说明路径。\n\
         3. 需求含糊、信息不足或方案取舍需要用户决定时，直接在回答里提出具体问题并给出候选项，\
         不要以「等待用户」为由静默停下。"
            .to_string(),
    );
    parts.join("\n\n")
}
