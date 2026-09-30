//! owo-agent HTTP 客户端：鉴权配对、健康检查、会话、回合（SSE 流式）、审批、diff/回滚。
//!
//! 鉴权（引擎 X03）：`GET /auth/token` 为公开引导端点，返回 `{token}`；
//! 其余业务端点要求 `Authorization: Bearer <token>`。客户端懒加载并缓存 token，
//! 收到 401（引擎重启轮换了 token）时自动刷新并重试一次。

use std::io::BufReader;
use std::sync::Mutex;
use std::time::Duration;

use crate::sse::{read_frames, SseFrame};
use crate::types::{
    CreateSessionRequest, FileDiff, HealthResponse, PermissionResponse, SessionInfo, TurnRequest,
};

/// 桥接错误。
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    /// 引擎未运行 / 端口不可达（UI 应显示"启动引擎"引导）。
    #[error("引擎不可达：{0}")]
    Unreachable(String),
    /// 引擎返回非 2xx。
    #[error("引擎返回 HTTP {status}：{body}")]
    Http { status: u16, body: String },
    /// 响应体无法解析为契约结构。
    #[error("响应解析失败：{0}")]
    Decode(String),
    /// 流式读取中断。
    #[error("流式读取失败：{0}")]
    Stream(String),
}

impl BridgeError {
    /// 是否属于"引擎没起/连不上"，供 UI 决定是否提示启动服务。
    pub fn is_unreachable(&self) -> bool {
        matches!(self, BridgeError::Unreachable(_))
    }

    /// 是否鉴权失败（配对异常，UI 可提示重试或重启引擎）。
    pub fn is_unauthorized(&self) -> bool {
        matches!(self, BridgeError::Http { status: 401, .. })
    }
}

/// owo-agent 桥接客户端（线程安全：`&self` 可跨线程复用）。
pub struct OwoBridgeClient {
    base: String,
    agent: ureq::Agent,
    /// bearer token 缓存（None = 尚未配对）。
    token: Mutex<Option<String>>,
}

impl OwoBridgeClient {
    /// `base_url` 形如 `http://127.0.0.1:4096`。
    pub fn new(base_url: impl Into<String>) -> Self {
        let base = base_url.into().trim_end_matches('/').to_string();
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(3))
            // SSE 空闲读超时：模型思考期间可能几十秒无字节，给足余量。
            .timeout_read(Duration::from_secs(300))
            .build();
        Self {
            base,
            agent,
            token: Mutex::new(None),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// 丢弃缓存的 token（引擎重启后调用；`bearer` 亦会在 401 时自动刷新）。
    pub fn clear_token(&self) {
        *self.token.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// 获取（并缓存）bearer token；`force` 为真时强制重新配对。
    pub fn bearer(&self, force: bool) -> Result<String, BridgeError> {
        if !force {
            if let Some(token) = self.token.lock().unwrap_or_else(|e| e.into_inner()).clone() {
                return Ok(token);
            }
        }
        let response = self
            .agent
            .get(&self.url("/auth/token"))
            .timeout(Duration::from_secs(5))
            .call()
            .map_err(map_ureq)?;
        let value: serde_json::Value = decode(response)?;
        let token = value
            .get("token")
            .and_then(|token| token.as_str())
            .unwrap_or_default()
            .to_string();
        if token.is_empty() {
            return Err(BridgeError::Decode("auth/token 未返回 token".to_string()));
        }
        *self.token.lock().unwrap_or_else(|e| e.into_inner()) = Some(token.clone());
        Ok(token)
    }

    fn authorized(&self, request: ureq::Request) -> Result<ureq::Request, BridgeError> {
        let token = self.bearer(false)?;
        Ok(request.set("Authorization", &format!("Bearer {token}")))
    }

    /// 带鉴权 GET，解析 JSON；401 时刷新 token 重试一次。
    fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        timeout: Duration,
    ) -> Result<T, BridgeError> {
        let value = self.get_value(path, timeout)?;
        serde_json::from_value(value).map_err(|error| BridgeError::Decode(error.to_string()))
    }

    fn get_value(&self, path: &str, timeout: Duration) -> Result<serde_json::Value, BridgeError> {
        let mut refreshed = false;
        loop {
            if refreshed {
                self.clear_token();
            }
            let request = self
                .agent
                .get(&self.url(path))
                .timeout(timeout);
            let result = self.authorized(request).and_then(|request| {
                request
                    .call()
                    .map_err(map_ureq)
                    .and_then(|response| decode::<serde_json::Value>(response))
            });
            match result {
                Err(BridgeError::Http { status: 401, .. }) if !refreshed => {
                    refreshed = true;
                    continue;
                }
                other => return other,
            }
        }
    }

    /// 带鉴权 POST（JSON body），解析 JSON 响应；401 刷新重试一次。
    fn post_json<B: serde::Serialize, T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
        timeout: Duration,
    ) -> Result<T, BridgeError> {
        let mut refreshed = false;
        loop {
            if refreshed {
                self.clear_token();
            }
            let request = self.agent.post(&self.url(path)).timeout(timeout);
            let result = self.authorized(request).and_then(|request| {
                request
                    .send_json(body)
                    .map_err(map_ureq)
                    .and_then(|response| decode::<T>(response))
            });
            match result {
                Err(BridgeError::Http { status: 401, .. }) if !refreshed => {
                    refreshed = true;
                    continue;
                }
                other => return other,
            }
        }
    }

    /// 带鉴权 POST（空 body），忽略响应体；401 刷新重试一次。
    fn post_empty(&self, path: &str, timeout: Duration) -> Result<(), BridgeError> {
        let mut refreshed = false;
        loop {
            if refreshed {
                self.clear_token();
            }
            let request = self.agent.post(&self.url(path)).timeout(timeout);
            let result = self
                .authorized(request)
                .and_then(|request| request.send_string("").map_err(map_ureq).map(|_| ()));
            match result {
                Err(BridgeError::Http { status: 401, .. }) if !refreshed => {
                    refreshed = true;
                    continue;
                }
                other => return other,
            }
        }
    }

    /// `GET /health`：公开端点（不需要 token），3 秒超时。
    pub fn health(&self) -> Result<HealthResponse, BridgeError> {
        let response = self
            .agent
            .get(&self.url("/health"))
            .timeout(Duration::from_secs(3))
            .call()
            .map_err(map_ureq)?;
        decode(response)
    }

    /// `POST /session`：创建会话。
    pub fn create_session(
        &self,
        workspace: &str,
        model: Option<&str>,
        system_prompt: Option<&str>,
    ) -> Result<SessionInfo, BridgeError> {
        self.post_json(
            "/session",
            &CreateSessionRequest {
                workspace,
                model,
                system_prompt,
            },
            Duration::from_secs(10),
        )
    }

    /// `GET /sessions`：会话列表。
    pub fn list_sessions(&self) -> Result<Vec<SessionInfo>, BridgeError> {
        self.get_json("/sessions", Duration::from_secs(10))
    }

    /// `GET /session/{id}`：会话详情（含历史消息，raw JSON 防契约漂移）。
    pub fn session_detail(&self, session_id: &str) -> Result<serde_json::Value, BridgeError> {
        self.get_value(&format!("/session/{session_id}"), Duration::from_secs(10))
    }

    /// `POST /session/{id}/attachments`：上传附件（base64），返回服务端登记信息
    /// （`id` 为 sanitize 后的文件名，发送回合时放进 `attachments`）。
    pub fn upload_attachment(
        &self,
        session_id: &str,
        name: &str,
        mime: Option<&str>,
        data_b64: &str,
    ) -> Result<serde_json::Value, BridgeError> {
        self.post_json(
            &format!("/session/{session_id}/attachments"),
            &serde_json::json!({ "name": name, "mime": mime, "data_b64": data_b64 }),
            Duration::from_secs(60),
        )
    }

    /// 带鉴权 DELETE，解析 JSON 响应；401 刷新重试一次。
    fn delete_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        timeout: Duration,
    ) -> Result<T, BridgeError> {
        let mut refreshed = false;
        loop {
            if refreshed {
                self.clear_token();
            }
            let request = self.agent.delete(&self.url(path)).timeout(timeout);
            let result = self.authorized(request).and_then(|request| {
                request
                    .call()
                    .map_err(map_ureq)
                    .and_then(|response| decode::<T>(response))
            });
            match result {
                Err(BridgeError::Http { status: 401, .. }) if !refreshed => {
                    refreshed = true;
                    continue;
                }
                other => return other,
            }
        }
    }

    /// `GET /automations`：定时任务列表（raw JSON 透传，契约由引擎维护）。
    pub fn list_automations(&self) -> Result<serde_json::Value, BridgeError> {
        self.get_value("/automations", Duration::from_secs(10))
    }

    /// `POST /automations`：创建任务（schedule/action 为引擎协议 JSON）。
    pub fn create_automation(
        &self,
        name: &str,
        schedule: serde_json::Value,
        action: serde_json::Value,
    ) -> Result<serde_json::Value, BridgeError> {
        self.post_json(
            "/automations",
            &serde_json::json!({ "name": name, "schedule": schedule, "action": action }),
            Duration::from_secs(10),
        )
    }

    /// `POST /automations/{id}/toggle`：启停切换。
    pub fn toggle_automation(&self, id: &str) -> Result<serde_json::Value, BridgeError> {
        self.post_json(
            &format!("/automations/{id}/toggle"),
            &serde_json::json!({}),
            Duration::from_secs(10),
        )
    }

    /// `DELETE /automations/{id}`：删除任务。
    pub fn delete_automation(&self, id: &str) -> Result<serde_json::Value, BridgeError> {
        self.delete_json(&format!("/automations/{id}"), Duration::from_secs(10))
    }

    /// `GET /activity`（A8-2）：引擎活跃回合快照——桌宠等外部进度面板的数据源
    /// （哪个会话在跑、阶段、待审批计数）。
    pub fn activity(&self) -> Result<serde_json::Value, BridgeError> {
        self.get_value("/activity", Duration::from_secs(5))
    }

    /// `POST /desktop/pet/report`（A8-3）：桌宠显隐心跳——上报本机实际状态，
    /// 返回工作台写入的期望值（`{desired: bool|null}`）。
    pub fn report_pet_visible(&self, visible: bool) -> Result<serde_json::Value, BridgeError> {
        self.post_json(
            "/desktop/pet/report",
            &serde_json::json!({ "visible": visible }),
            Duration::from_secs(10),
        )
    }

    /// `GET /desktop/pet`（A8-3）：桌宠显隐状态（desired/actual/overlay_online）。
    pub fn pet_state(&self) -> Result<serde_json::Value, BridgeError> {
        self.get_value("/desktop/pet", Duration::from_secs(10))
    }

    /// `POST /desktop/pet`（A8-3）：写入期望显隐（桌面端本地切换时同步，保持两端一致）。
    pub fn set_pet_desired(&self, visible: bool) -> Result<serde_json::Value, BridgeError> {
        self.post_json(
            "/desktop/pet",
            &serde_json::json!({ "visible": visible }),
            Duration::from_secs(10),
        )
    }

    /// `GET /automations/runs`：执行记录（时间倒序；task_id/limit 可选）。
    pub fn automation_runs(
        &self,
        task_id: Option<&str>,
        limit: Option<u32>,
    ) -> Result<serde_json::Value, BridgeError> {
        let mut path = "/automations/runs".to_string();
        let mut params: Vec<String> = Vec::new();
        if let Some(id) = task_id {
            params.push(format!("task_id={id}"));
        }
        if let Some(limit) = limit {
            params.push(format!("limit={limit}"));
        }
        if !params.is_empty() {
            path.push('?');
            path.push_str(&params.join("&"));
        }
        self.get_value(&path, Duration::from_secs(10))
    }

    /// `POST /session/{id}/turn`：执行一个回合，逐帧回调 SSE 事件。
    ///
    /// **阻塞**直到回合结束（`final` 或连接关闭）；调用方应放在后台线程，
    /// 并把帧通过 Tauri event 转发到前端。回合不自动重试（避免重复执行副作用）。
    pub fn turn_stream(
        &self,
        session_id: &str,
        request: &TurnRequest,
        mut on_frame: impl FnMut(SseFrame),
    ) -> Result<(), BridgeError> {
        let builder = self
            .agent
            .post(&self.url(&format!("/session/{session_id}/turn")))
            .set("Accept", "text/event-stream");
        let response = self
            .authorized(builder)?
            .send_json(request)
            .map_err(map_ureq)?;
        read_frames(BufReader::new(response.into_reader()), |frame| {
            on_frame(frame)
        })
        .map_err(|error| BridgeError::Stream(error.to_string()))
    }

    /// `POST /session/{id}/permission/{request_id}`：审批回传。
    ///
    /// `remember_scope`（A6-1 三档授权）：`None` = 不记住；`Some("session")` =
    /// 本会话内允许（引擎内存临时规则，重启失效）；`Some("forever")` = 永久规则。
    pub fn respond_permission(
        &self,
        session_id: &str,
        request_id: &str,
        allow: bool,
        remember: Option<bool>,
        remember_scope: Option<&str>,
    ) -> Result<(), BridgeError> {
        let response: serde_json::Value = self.post_json(
            &format!("/session/{session_id}/permission/{request_id}"),
            &PermissionResponse {
                allow,
                remember,
                remember_scope: remember_scope.map(str::to_string),
            },
            Duration::from_secs(10),
        )?;
        let _ = response;
        Ok(())
    }

    /// `GET /permissions/rules`：权限规则列表（B4-1：持久 + 会话临时）。
    pub fn permission_rules(&self) -> Result<serde_json::Value, BridgeError> {
        self.get_value("/permissions/rules", Duration::from_secs(10))
    }

    /// `POST /permissions/rules/remove`：删除一条权限规则。
    pub fn remove_permission_rule(
        &self,
        tool: &str,
        pattern: &str,
        session: bool,
    ) -> Result<bool, BridgeError> {
        let response: serde_json::Value = self.post_json(
            "/permissions/rules/remove",
            &serde_json::json!({
                "tool": tool,
                "pattern": pattern,
                "scope": if session { "session" } else { "forever" },
            }),
            Duration::from_secs(10),
        )?;
        Ok(response.get("removed").and_then(|v| v.as_bool()).unwrap_or(false))
    }

    /// `POST /session/{id}/answer/{question_id}`：提交 ask_user 提问的回答，唤醒挂起回合。
    pub fn answer_question(
        &self,
        session_id: &str,
        question_id: &str,
        answer: &str,
    ) -> Result<(), BridgeError> {
        let response: serde_json::Value = self.post_json(
            &format!("/session/{session_id}/answer/{question_id}"),
            &serde_json::json!({ "answer": answer }),
            Duration::from_secs(10),
        )?;
        let _ = response;
        Ok(())
    }

    /// `GET /session/{id}/diff`：本回合改动列表。
    pub fn diff(&self, session_id: &str) -> Result<Vec<FileDiff>, BridgeError> {
        self.get_json(&format!("/session/{session_id}/diff"), Duration::from_secs(15))
    }

    /// `POST /session/{id}/revert`：回滚全部改动。
    pub fn revert(&self, session_id: &str) -> Result<(), BridgeError> {
        self.post_empty(&format!("/session/{session_id}/revert"), Duration::from_secs(60))
    }

    /// `POST /session/{id}/abort`：中止当前回合。
    pub fn abort(&self, session_id: &str) -> Result<(), BridgeError> {
        self.post_empty(&format!("/session/{session_id}/abort"), Duration::from_secs(10))
    }
}

fn map_ureq(error: ureq::Error) -> BridgeError {
    match error {
        ureq::Error::Status(status, response) => {
            let body = response.into_string().unwrap_or_default();
            let body: String = body.chars().take(500).collect();
            BridgeError::Http { status, body }
        }
        ureq::Error::Transport(transport) => BridgeError::Unreachable(transport.to_string()),
    }
}

fn decode<T: serde::de::DeserializeOwned>(response: ureq::Response) -> Result<T, BridgeError> {
    response
        .into_json::<T>()
        .map_err(|error| BridgeError::Decode(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    fn json_response(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn status_response(status: u16, reason: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn sse_response(frames: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{frames}"
        )
    }

    /// 路由式 mock：key 匹配请求首行（如 `"POST /session"`）；
    /// 同一 key 的多次调用按顺序取响应，用尽后重复最后一个。
    fn spawn_mock(
        routes: Vec<(&'static str, Vec<String>)>,
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let received = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&received);
        let routes: Vec<(String, Vec<String>)> = routes
            .into_iter()
            .map(|(key, responses)| (key.to_string(), responses))
            .collect();
        thread::spawn(move || {
            let mut counters: HashMap<String, usize> = HashMap::new();
            loop {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let request = read_http_request(&mut stream);
                sink.lock().unwrap().push(request.clone());
                let first_line = request.lines().next().unwrap_or_default().to_string();
                let response = routes
                    .iter()
                    .find(|(key, _)| first_line.contains(key.as_str()))
                    .map(|(key, responses)| {
                        let index = counters.entry(key.clone()).or_insert(0);
                        let response =
                            responses[(*index).min(responses.len() - 1)].clone();
                        *index += 1;
                        response
                    })
                    .unwrap_or_else(|| status_response(404, "Not Found", "{}"));
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
        });
        (format!("http://{addr}"), received)
    }

    /// 读满一个完整 HTTP 请求（头 + Content-Length 指定的 body）。
    fn read_http_request(stream: &mut std::net::TcpStream) -> String {
        let mut raw = Vec::new();
        let mut buffer = [0u8; 4096];
        let header_end = loop {
            match stream.read(&mut buffer) {
                Ok(0) => break raw.len(),
                Ok(n) => {
                    raw.extend_from_slice(&buffer[..n]);
                    if let Some(position) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        break position + 4;
                    }
                }
                Err(_) => break raw.len(),
            }
        };
        let head = String::from_utf8_lossy(&raw[..header_end.min(raw.len())]).into_owned();
        let content_length = head
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        while raw.len() < header_end + content_length {
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => raw.extend_from_slice(&buffer[..n]),
                Err(_) => break,
            }
        }
        String::from_utf8_lossy(&raw).into_owned()
    }

    fn token_route() -> (&'static str, Vec<String>) {
        ("GET /auth/token", vec![json_response(r#"{"token":"test-token"}"#)])
    }

    #[test]
    fn health_is_public_and_parses_auto_approve() {
        let (base, received) = spawn_mock(vec![(
            "GET /health",
            vec![json_response(
                r#"{"healthy":true,"version":"0.7.0","auto_approve":false}"#,
            )],
        )]);
        let client = OwoBridgeClient::new(&base);
        let health = client.health().unwrap();
        assert!(health.healthy);
        assert_eq!(health.version, "0.7.0");
        assert!(!health.auto_approve);
        // 健康检查是公开端点：不应触发 token 配对。
        assert_eq!(received.lock().unwrap().len(), 1);
    }

    #[test]
    fn protected_calls_pair_via_auth_token_and_carry_bearer_header() {
        let (base, received) = spawn_mock(vec![
            token_route(),
            (
                "POST /session",
                vec![json_response(r#"{"id":"s-1","workspace":"D:/work"}"#)],
            ),
            (
                "GET /sessions",
                vec![json_response(r#"[{"id":"s-1"}]"#)],
            ),
        ]);
        let client = OwoBridgeClient::new(&base);
        let session = client.create_session("D:/work", None, None).unwrap();
        assert_eq!(session.id, "s-1");
        let sessions = client.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);

        let requests = received.lock().unwrap().clone();
        // 只配对一次（token 缓存复用）。
        assert_eq!(
            requests
                .iter()
                .filter(|raw| raw.starts_with("GET /auth/token"))
                .count(),
            1,
            "{requests:?}"
        );
        assert!(
            requests[1].contains("Authorization: Bearer test-token")
                || requests[1].contains("authorization: Bearer test-token"),
            "业务请求应带 bearer：{}",
            requests[1]
        );
    }

    #[test]
    fn unauthorized_response_refreshes_token_and_retries_once() {
        let (base, received) = spawn_mock(vec![
            token_route(),
            (
                "POST /session",
                vec![
                    status_response(401, "Unauthorized", r#"{"error":"未授权"}"#),
                    json_response(r#"{"id":"s-2","workspace":"D:/work"}"#),
                ],
            ),
        ]);
        let client = OwoBridgeClient::new(&base);
        let session = client.create_session("D:/work", None, None).unwrap();
        assert_eq!(session.id, "s-2");
        let requests = received.lock().unwrap().clone();
        // 首次 401 后应强制重新配对（两次 /auth/token）。
        assert_eq!(
            requests
                .iter()
                .filter(|raw| raw.starts_with("GET /auth/token"))
                .count(),
            2,
            "{requests:?}"
        );
        assert_eq!(
            requests
                .iter()
                .filter(|raw| raw.starts_with("POST /session"))
                .count(),
            2
        );
    }

    #[test]
    fn turn_stream_emits_frames_in_order_and_json_parses() {
        let frames = concat!(
            "event: tool_use\ndata: {\"type\":\"tool_use\",\"id\":\"c1\",\"tool\":\"grep\",\"args\":{},\"v\":1}\n\n",
            "event: permission_request\ndata: {\"type\":\"permission_request\",\"request_id\":\"r1\",\"tool\":\"edit_file\",\"args\":{},\"reason\":\"write\",\"v\":1}\n\n",
            "event: final\ndata: {\"type\":\"final\",\"text\":\"完成\",\"v\":1}\n\n",
        );
        let (base, received) = spawn_mock(vec![
            token_route(),
            ("POST /session/s-1/turn", vec![sse_response(frames)]),
        ]);
        let client = OwoBridgeClient::new(&base);
        let mut kinds = Vec::new();
        let mut permission_seen = false;
        client
            .turn_stream("s-1", &TurnRequest::new("列一下目录"), |frame| {
                kinds.push(frame.event_name().to_string());
                if let Some(crate::TurnEvent::PermissionRequest { request_id, .. }) =
                    frame.json::<crate::TurnEvent>()
                {
                    assert_eq!(request_id, "r1");
                    permission_seen = true;
                }
            })
            .unwrap();
        assert_eq!(kinds, vec!["tool_use", "permission_request", "final"]);
        assert!(permission_seen);
        let requests = received.lock().unwrap().clone();
        let turn_request = requests
            .iter()
            .find(|raw| raw.starts_with("POST /session/s-1/turn"))
            .expect("应发出 turn 请求");
        assert!(turn_request.contains("text/event-stream"), "{turn_request}");
        assert!(turn_request.contains("Bearer test-token"), "{turn_request}");
        assert!(turn_request.contains("列一下目录"), "{turn_request}");
    }

    #[test]
    fn diff_and_mutations_hit_expected_paths_with_auth() {
        let (base, received) = spawn_mock(vec![
            token_route(),
            (
                "GET /session/s-1/diff",
                vec![json_response(r#"[{"path":"a.txt","before":"old","after":"new"}]"#)],
            ),
            (
                "POST /session/s-1/permission/req-9",
                vec![json_response("{}")],
            ),
            ("POST /session/s-1/revert", vec![json_response("{}")]),
            ("POST /session/s-1/abort", vec![json_response("{}")]),
        ]);
        let client = OwoBridgeClient::new(&base);
        let diffs = client.diff("s-1").unwrap();
        assert_eq!(diffs[0].path, "a.txt");
        assert_eq!(diffs[0].after.as_deref(), Some("new"));
        client
            .respond_permission("s-1", "req-9", true, Some(true), Some("forever"))
            .unwrap();
        client.revert("s-1").unwrap();
        client.abort("s-1").unwrap();

        let requests = received.lock().unwrap().clone();
        let permission = requests
            .iter()
            .find(|raw| raw.starts_with("POST /session/s-1/permission/req-9"))
            .unwrap();
        assert!(permission.contains("\"allow\":true"), "{permission}");
        assert!(permission.contains("Bearer test-token"), "{permission}");
    }

    #[test]
    fn http_error_status_carries_body() {
        let (base, _received) = spawn_mock(vec![
            token_route(),
            (
                "POST /session",
                vec![status_response(
                    400,
                    "Bad Request",
                    r#"{"error":"workspace 不存在"}"#,
                )],
            ),
        ]);
        let client = OwoBridgeClient::new(&base);
        let error = client.create_session("D:/nope", None, None).unwrap_err();
        match error {
            BridgeError::Http { status, body } => {
                assert_eq!(status, 400);
                assert!(body.contains("workspace 不存在"), "{body}");
            }
            other => panic!("应为 Http 错误：{other}"),
        }
    }

    #[test]
    fn unreachable_engine_maps_to_unreachable_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let client = OwoBridgeClient::new(format!("http://{addr}"));
        let error = client.health().unwrap_err();
        assert!(error.is_unreachable(), "应归类为不可达：{error}");
    }
}
