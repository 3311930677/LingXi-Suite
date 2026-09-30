OwO Agent 输入法桥接包（serve-ime，v0.1.0）

四步接入：
  1. 安装 OwO 输入法 0.2.3 及以上（官网安装包）。
  2. 安装 Agent IPC 连接器：OwO 设置中心 → 插件 → 安装
     org.owo.agent-ipc-1.2.0.owopkg（随 OwO-release 仓库 artifacts/plugins/ 分发）。
  3. 在 OwO 设置中心对该插件【明确授权】；管道端点保持默认
     \\.\pipe\OwO.Agent.External.v1。
  4. 双击 start-serve-ime.cmd，在任意输入框键入 v 前缀拼音
     （如 vbangwozhaowenjian），候选框将出现 Agent 命令。

环境变量（可选，启动前设置）：
  OPENAI_API_KEY / OPENAI_BASE_URL / OPENAI_MODEL   模型凭据（需自行配置）
  OWO_AGENT_DATA                                    数据目录（默认 %LOCALAPPDATA%\OwO\Agent）
  OWO_ONNX_OCR_MODEL_DIR                            本地 OCR 模型目录（默认随包 models/ocr）

说明：
  - 管道面：OwO 输入法连接器调用（协议 v3）；HTTP 面：桌宠/Web 工作台（127.0.0.1:4096）。
  - 高风险命令不会直接执行——候选框返回「打开确认界面」，在 HTTP 面审批。
  - 与 owo-agent serve 共用数据目录，二者不可同时运行（PidFile 互斥）。
