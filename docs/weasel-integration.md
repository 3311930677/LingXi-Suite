# Weasel + LingXi 集成指南

本文说明如何用小狼毫(Weasel) 作为 TSF 前端，加载 rime-ice 词库，并挂载 LingXi 的 AI 重排 filter 插件。

## 架构

```
Weasel (TSF 前端) → librime (拼音引擎) → lingxi_rerank filter → Rust ime-server
                                                                   ↑ AI 重排/联想
```

## 前置条件

- Windows 10/11
- Git, CMake, Visual Studio (含 C++ 桌面开发)
- Rust (MSVC toolchain)

## 步骤

### 1. Fork 并编译 Weasel

```powershell
git clone https://github.com/rime/weasel.git
cd weasel
# 参考 weasel/README.md 的编译指引，通常：
# - 安装 boost (vcpkg 或手动)
# - cmake -B build -G "Visual Studio 17 2022"
# - cmake --build build --config Release
```

> 注：首次编译 Weasel 可能较复杂（依赖 boost/librime/plum），参考其 CI 脚本。

### 2. 配置 rime-ice 词库

```powershell
# 进入 Weasel 用户数据目录（通常 %AppData%\Rime）
cd $env:APPDATA\Rime

# 克隆 rime-ice
git clone https://github.com/iDvel/rime-ice.git .
# 或只复制 *.dict.yaml / *.schema.yaml 文件
```

重新部署 Weasel（右键托盘图标 → 重新部署），验证能正常打字。

### 3. 启动 LingXi IPC Server

```powershell
cd d:\UGit\cross-app-assistant
cargo run -p ime-server
# 输出: LingXi IME Server - Listening: 127.0.0.1:9527
```

保持运行。可用 curl 测试：
```powershell
echo '{"type":"query","pinyin":"nihao"}' | nc 127.0.0.1 9527
# 或用 PowerShell:
$c = New-Object System.Net.Sockets.TcpClient("127.0.0.1", 9527)
$s = $c.GetStream(); $w = New-Object System.IO.StreamWriter($s)
$w.WriteLine('{"type":"query","pinyin":"nihao"}'); $w.Flush()
$r = New-Object System.IO.StreamReader($s); $r.ReadLine()
$c.Close()
```

### 4. 编译 LingXi RIME Filter 插件

```powershell
cd d:\UGit\cross-app-assistant\plugins\lingxi-rime-filter

# 编译为 DLL（需要 librime 头文件）
cl /EHsc /std:c++17 /I "<path-to-librime>/include" /LD lingxi_rerank.cc ws2_32.lib /Fe:lingxi_filter.dll
```

将生成的 `lingxi_filter.dll` 放入 Weasel 的 plugin 目录。

### 5. 在 RIME schema 中启用 filter

编辑你的输入方案 (如 `rime_ice.schema.yaml`)，在 `engine/filters` 末尾加一行：

```yaml
engine:
  filters:
    - simplifier
    - uniquifier
    - lingxi_rerank    # ← 新增
```

重新部署 Weasel，切换到该方案即可生效。

### 6. 验证

1. 在记事本中打字 `nihao`
2. 观察候选顺序是否被 LingXi 重排
3. ime-server 控制台应打印 `handled request from 127.0.0.1:xxxxx`

## 后续增强

- **神经重排**：在 ime-server 中接入 RoBERTa / 小 LM，对 top-K 候选打分
- **上下文感知**：filter 从 RIME composition 获取已提交文本作为 context 传入
- **AI 联想**：追加一个候选来源（ime-server 返回额外的 AI 联想词）
- **大词库加载**：ime-server 启动时 `--dict` 参数加载 rime-ice 全量词库

## 故障排查

| 问题 | 排查 |
|------|------|
| filter 无效果 | 确认 ime-server 在运行，`nc 127.0.0.1 9527` 能通 |
| DLL 加载失败 | 检查 librime 版本兼容性，用 Dependency Walker 看缺什么 |
| Weasel 崩溃 | filter 里 IPC 超时默认回退原始顺序，不应 crash；看 Weasel 日志 |
