/* 常用模型服务提供方预设（唯一事实源）。
 *
 * 为什么单独一个文件（而不是写在渲染代码里）：
 *   1. 端点、默认模型、所需环境变量名集中一处，设置页与后续引导页共用，
 *      避免两处默认值漂移；
 *   2. 密钥不在这里，也永远不会在这里：只描述"端点 + 默认模型名 + 该端点
 *      用哪个环境变量放密钥"，密钥本体只存在于系统环境变量。
 *
 * 环境变量契约（见 core/gateway.rs）：OPENAI_BASE_URL / OPENAI_API_KEY /
 * OPENAI_MODEL。前端只生成设置命令供复制，不写入任何密钥或端点。
 */
(function (global) {
  "use strict";

  var OLLAMA_HOST = "127.0.0.1";
  var OLLAMA_PORT = 11434;

  function ollamaBaseUrl() {
    return "http://" + OLLAMA_HOST + ":" + OLLAMA_PORT + "/v1";
  }

  function presets() {
    return [
      {
        id: "bigmodel",
        label: "智谱 BigModel",
        baseUrl: "https://open.bigmodel.cn/api/paas/v4",
        model: "glm-5.2",
        keyEnv: "OPENAI_API_KEY",
        note: "OpenAI 兼容接口，密钥在智谱开放平台创建。",
      },
      {
        id: "deepseek",
        label: "DeepSeek",
        baseUrl: "https://api.deepseek.com/v1",
        model: "deepseek-v4-flash-0731",
        keyEnv: "OPENAI_API_KEY",
        note: "默认模型为 flash 档；如需 pro 档可在模型下拉中改选 deepseek-v4-pro-0813。",
      },
      {
        id: "dashscope",
        label: "阿里 DashScope 兼容",
        baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        model: "qwen3.8-max",
        keyEnv: "OPENAI_API_KEY",
        note: "百炼兼容模式端点，密钥在阿里云百炼控制台创建。",
      },
      {
        id: "ollama",
        label: "本地 Ollama",
        baseUrl: ollamaBaseUrl(),
        model: "",
        keyEnv: "",
        note: "本地端点无需密钥；模型名请按 `ollama list` 的实际名称在模型下拉中改选或手填。",
      },
      {
        id: "custom",
        label: "自定义 / 自建",
        baseUrl: "",
        model: "",
        keyEnv: "OPENAI_API_KEY",
        note: "自建网关请手动填写 OPENAI_BASE_URL，保持 OpenAI 兼容的 /chat/completions 契约。",
      },
    ];
  }

  /// 生成 PowerShell 环境变量设置命令（密钥留占位符，绝不写入真实密钥）。
  function envCommand(preset) {
    if (!preset || !preset.baseUrl) return "";
    var parts = ['$env:OPENAI_BASE_URL="' + preset.baseUrl + '"'];
    if (preset.keyEnv) parts.push("$env:" + preset.keyEnv + '="<你的密钥>"');
    if (preset.model) parts.push('$env:OPENAI_MODEL="' + preset.model + '"');
    return parts.join("; ");
  }

  global.OwoProviderPresets = {
    presets: presets,
    ollamaBaseUrl: ollamaBaseUrl,
    envCommand: envCommand,
  };
})(typeof window !== "undefined" ? window : globalThis);