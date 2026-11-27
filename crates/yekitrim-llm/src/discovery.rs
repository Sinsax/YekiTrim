//! LLM **discovery**：`GET {base}/models` —— 问一个中转"你到底有哪些模型"。
//!
//! # 为什么住在这个 crate
//!
//! 这是**别家的协议知识**（URL 怎么拼、回包有三种形状、失败怎么分类），与 `wire` / `catalog`
//! 同一条理由。业务侧（server）只拿到 `Vec<Discovered>` 或一句**已经分好类**的文案：
//! 它不认识 `Authorization` 头、也不认识 `{"data":…}`。
//!
//! # 三条口径
//!
//! 1. **认识的 route 根本不走这里**：内置能力表命中时由调用方直接回答（零网络请求）；
//!    `discover` 只服务"表里没有的那家"。
//! 2. ⚠️ **失败要分类，不许含糊**：key 不对（401/403）/ 这个地址没有 `/models`（404）/
//!    连不上（超时、建连失败）/ 其它 4xx-5xx —— 四句不同的话。把"key 写错"说成
//!    "这个中转不支持列模型"，用户就会去改 base_url：**改多久都不会好**。
//! 3. **200 + 空列表不是错误**：连上了、地址也对，只是它没报模型（有些中转就是这样）。
//!    这时回 `Ok(vec![])`，由调用方用 `empty_hint()` 补一句人话。

use std::time::Duration;

/// 探测超时：`/models` 是个很小的请求，10s 足够；连不上时要**早点失败**。
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);

/// 文案里带的响应体片段上限（别把整页 HTML 塞进界面）。
const SNIPPET_MAX: usize = 200;

/// 探测到的一支模型：`id` 是请求时**要写的那个名字**，`name` 是给人看的（中转常常不给）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    pub id: String,
    pub name: Option<String>,
    /// **顺手探到的能力**（`/models` 回包里带的话）。`None` = 这份回包没说，别编。
    ///
    /// 形状 = [`crate::caps::PartialCaps`]（读取器注册表的产物：**只包含读到的东西**）。
    /// ⚠️ 只用于展示与"这一趟探到的能力"：**不落盘、不写进 `llm.toml`**（草稿不落盘）。
    pub caps: Option<crate::caps::PartialCaps>,
}

/// 这一趟探测的**全部结果**：模型清单 + 试过哪些路径。
///
/// 为什么要带上"试过哪些路径"：路径级探测（LiteLLM / Ollama）**失败一律静默跳过**
/// （不阻断主流程），可这样一来"为什么这支模型没有能力"就没人说得清了 ——
/// `tried` 让界面/日志能说"我试过 `/model/info` 和 `/api/tags`，都没通"。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Discovery {
    pub models: Vec<Discovered>,
    /// 试过的路径（`/models` + 各路径级探测），**顺序即尝试顺序**。⚠️ "试过"≠"成功"。
    pub tried: Vec<String>,
    /// 真正补进过能力的路径级探测 id（`litellm` / `ollama`）。
    /// 空 = 能力只来自 `/v1/models` 的自描述（或压根没探到）。
    pub caps_from: Vec<&'static str>,
}

/// 路径级探测**怎么打**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fetch {
    /// GET `paths[0]`，一次拿全（LiteLLM `/model/info`）。
    Get,
    /// 先 GET `paths[0]` 拿候选模型名，再**逐支** POST `paths[1]`（Ollama：`/api/tags` → `/api/show`）。
    PostPerModel,
}

/// 一条**路径级**探测：有些服务器不从 `/v1/models` 说能力，得换一条路径去问。
///
/// ⚠️ 与 [`crate::caps::CapsReader`]（**字段级**：读 `/v1/models` 的单个条目）不是一回事，
/// 但产物同构：都是一片 [`crate::caps::PartialCaps`]，合并同样走"只补不抹"。
/// **加一家标准 = 写一个 read 函数 + 加一行表项。**
pub struct PathProbe {
    /// 稳定 id（写日志 / `caps_from` 用）。
    pub id: &'static str,
    /// 给界面看的中文名。
    pub label: &'static str,
    /// 要打的路径（`{root}` 之后、`/` 开头）。
    pub paths: &'static [&'static str],
    pub fetch: Fetch,
    /// (整份载荷, model id) → 能力片段；认不出 / 不是这支 ⇒ None。
    pub read: fn(&serde_json::Value, &str) -> Option<crate::caps::PartialCaps>,
}

/// 逐模型那条路最多问多少支（`PostPerModel` 每条路径一次请求）。
/// 276 支的网关逐个 POST 是**我们自己**在打人家，不是一个"探测"该有的量。
const MAX_PER_MODEL_PROBES: usize = 64;

/// **路径级探测注册表**（与 [`crate::caps::READERS`] 同构）。
///
/// 只在 `/v1/models` **已经答了**之后跑（模型清单的权威始终是它），失败一律静默跳过。
pub const PATH_PROBES: &[PathProbe] = &[
    PathProbe {
        id: "litellm",
        label: "LiteLLM /model/info",
        paths: &["/model/info"],
        fetch: Fetch::Get,
        read: crate::caps::read_litellm_info,
    },
    PathProbe {
        id: "ollama",
        label: "Ollama /api/tags → /api/show",
        paths: &["/api/tags", "/api/show"],
        fetch: Fetch::PostPerModel,
        read: crate::caps::read_ollama_show,
    },
];
/// 把用户填的 `base_url` 归一成**根**。
///
/// * 去掉首尾空白与尾部 `/`；
/// * 已经是 `…/chat/completions`（用户把整条端点粘过来了）就先剥掉那一段。
pub fn api_root(base_url: &str) -> String {
    let b = base_url.trim().trim_end_matches('/');
    b.strip_suffix("/chat/completions")
        .unwrap_or(b)
        .trim_end_matches('/')
        .to_string()
}

/// 200 + 空列表时给用户的那句话（**不是错误** —— 见模块头第 3 条）。
pub fn empty_hint(base_url: &str) -> String {
    format!(
        "{} 能连上，但它没有报出任何模型（这个中转可能不提供模型清单；手动填模型名即可）",
        api_root(base_url)
    )
}

/// 探一个 `base_url`：主路径 `GET {root}/models` + **路径级探测**（[`PATH_PROBES`]）。
///
/// * 模型清单的**权威只有 `/v1/models`**：路径级探测**不新增条目**，只给已有条目不的能力；
/// * 路径探测**失败一律静默跳过**（404 / 超时 / 非 JSON / 形状不认识），但会记进 `tried`；
/// * `/v1/models` 自己的失败仍然按四条分类文案报错（那是主路径，不能含糊）。
pub async fn discover(api_key: &str, base_url: &str, protocol_id: &str) -> Result<Discovery, String> {
    let root = api_root(base_url);
    if root.is_empty() {
        return Err("没填 API 基础地址（base_url）".to_string());
    }
    // 探针用**该接口自己的协议**发：路径与鉴权都在注册表里（空 = 默认 openai-chat）
    let proto = if protocol_id.trim().is_empty() {
        crate::protocol::default_protocol()
    } else {
        crate::protocol::protocol(protocol_id)
            .ok_or_else(|| format!("未知的 LLM 协议 id: {protocol_id}"))?
    };
    let mut url = format!("{root}{}", proto.models_path);
    if proto.auth == crate::protocol::AuthStyle::QueryKey {
        url = format!("{url}?key={api_key}");
    }
    // 客户端构造走 client 那一份（UA / connect_timeout / keepalive 只在那里设）
    let client = crate::client::build_http_client(DISCOVERY_TIMEOUT);
    let mut req = client.get(&url);
    req = match proto.auth {
        crate::protocol::AuthStyle::Bearer => req.header("Authorization", format!("Bearer {api_key}")),
        crate::protocol::AuthStyle::QueryKey => req,
    };
    for (k, v) in proto.extra_headers {
        req = req.header(*k, *v);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| connect_error(&url, &e))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(status_error(status.as_u16(), &url, &body));
    }
    let mut models = parse_models(&body, &url)?;

    // ── 路径级探测（P3）────────────────────────────────────────────────────
    // 失败不阻断：它只是"再问一条路"，不是主流程。
    let mut tried = vec![proto.models_path.to_string()];
    let mut caps_from: Vec<&'static str> = Vec::new();
    // 路径级探测的读法全是 OpenAI 族形状；别家协议跳过（404 也问不出东西）
    if proto.id == "openai-chat" {
        for probe in PATH_PROBES {
            apply_probe(&client, &root, api_key, probe, &mut models, &mut tried, &mut caps_from).await;
        }
    }
    Ok(Discovery { models, tried, caps_from })
}

/// 打一条路径级探测，把读到的能力**只补不抹**地并进已有条目。
///
/// ⚠️ 全程**不返回错误**：这条路本来就是"能问到就问、问不到就拉倒"。
async fn apply_probe(
    client: &reqwest::Client,
    root: &str,
    api_key: &str,
    probe: &PathProbe,
    models: &mut [Discovered],
    tried: &mut Vec<String>,
    caps_from: &mut Vec<&'static str>,
) {
    match probe.fetch {
        Fetch::Get => {
            let Some(path) = probe.paths.first() else { return };
            tried.push((*path).to_string());
            let Some(payload) = fetch_json(client, root, path, api_key, None).await else {
                return; // 404 / 超时 / 不是 JSON ⇒ 静默跳过（`tried` 已经留痕）
            };
            let mut hit = false;
            for m in models.iter_mut() {
                if let Some(part) = (probe.read)(&payload, &m.id) {
                    hit |= merge_caps_into(&mut m.caps, part);
                }
            }
            if hit && !caps_from.contains(&probe.id) {
                caps_from.push(probe.id);
            }
        }
        Fetch::PostPerModel => {
            let (list_path, ask_path) = match (probe.paths.first(), probe.paths.get(1)) {
                (Some(a), Some(b)) => (*a, *b),
                // 注册表写错了（缺一条路径）⇒ 跳过，不猜
                _ => return,
            };
            // ① 先问"你有哪些模型"：拿候选名，**顺带**确认这家就是它（问不到就不逐支打）
            tried.push(list_path.to_string());
            let Some(list) = fetch_json(client, root, list_path, api_key, None).await else {
                return;
            };
            let mut candidates = probe_ids(&list);
            // ② 只给**清单里已经有**的那些补能力（模型清单的权威是 /v1/models）
            candidates.retain(|id| models.iter().any(|m| &m.id == id));
            if candidates.is_empty() {
                return;
            }
            candidates.truncate(MAX_PER_MODEL_PROBES);
            tried.push(ask_path.to_string());
            let mut hit = false;
            for id in candidates {
                let body = serde_json::json!({ "model": id });
                let Some(payload) = fetch_json(client, root, ask_path, api_key, Some(body)).await else {
                    continue; // 这一支问不到就换下一支
                };
                let Some(part) = (probe.read)(&payload, &id) else { continue };
                if let Some(m) = models.iter_mut().find(|m| m.id == id) {
                    hit |= merge_caps_into(&mut m.caps, part);
                }
            }
            if hit && !caps_from.contains(&probe.id) {
                caps_from.push(probe.id);
            }
        }
    }
}

/// GET / POST 一个小 JSON 路径。**任何失败都返回 `None`**（探测这条路的口径）。
async fn fetch_json(
    client: &reqwest::Client,
    root: &str,
    path: &str,
    api_key: &str,
    body: Option<serde_json::Value>,
) -> Option<serde_json::Value> {
    for url in probe_urls(root, path) {
        let req = match &body {
            // ⚠️ Ollama 的 `/api/show` 只认 POST（GET 会 405）
            Some(v) => client.post(&url).json(v),
            None => client.get(&url),
        };
        let Ok(resp) = req
            .header("Authorization", format!("Bearer {api_key}"))
            .send()
            .await
        else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        let Ok(text) = resp.text().await else { continue };
        if let Ok(v) = serde_json::from_str(&text) {
            return Some(v);
        }
    }
    None
}

/// 一条探测路径要试的 URL：先 `{root}{path}`，再**服务根** `{origin}{path}`。
///
/// 为什么两个都试：用户填的 `base_url` 一般是 `https://host/v1`（OpenAI 兼容面），而
/// **LiteLLM 的 `/model/info` 与 Ollama 的 `/api/tags`+`/api/show` 都在服务根上**（不在 `/v1` 下）——
/// 只试 `{root}` 的话真机上一次都问不到（实测 7863：两种变体都 404）。
/// 失败的那个变体**不额外记 `tried`**：记的是"这条路径试过"，不是"试了几个 URL"。
fn probe_urls(root: &str, path: &str) -> Vec<String> {
    let first = format!("{root}{path}");
    let mut out = vec![first.clone()];
    if let Some(origin) = origin_of(root) {
        let alt = format!("{origin}{path}");
        if alt != first {
            out.push(alt);
        }
    }
    out
}

/// `scheme://host[:port]`（砍掉路径）—— 用户填的 `base_url` 的服务根。
fn origin_of(root: &str) -> Option<String> {
    let (scheme, rest) = root.split_once("://")?;
    let host = rest.split('/').next()?;
    (!scheme.is_empty() && !host.is_empty()).then(|| format!("{scheme}://{host}"))
}

/// **只补不抹**：把路径级探测读到的能力并进已有的片段。
///
/// 返回"是不是真的补进去了"（拿它决定 `caps_from` 要不要记这一笔）。
/// ⚠️ 与 [`crate::caps::merge_into`] 同一条口径：**已有值一律保留** ——
/// 那条路只说了 `supports_vision` 时，表里/`/v1/models` 给的档位与上下文必须照留。
fn merge_caps_into(slot: &mut Option<crate::caps::PartialCaps>, part: crate::caps::PartialCaps) -> bool {
    if part.is_empty() {
        return false;
    }
    match slot {
        Some(acc) => {
            let before = acc.clone();
            crate::caps::merge_into(acc, part);
            *acc != before
        }
        None => {
            *slot = Some(part);
            true
        }
    }
}

/// 从"列模型"那条路的载荷里取候选模型名（Ollama `/api/tags`：`models[].name`，也认裸字符串）。
fn probe_ids(payload: &serde_json::Value) -> Vec<String> {
    let arr = payload
        .get("models")
        .and_then(|m| m.as_array())
        .or_else(|| payload.as_array());
    let Some(arr) = arr else { return Vec::new() };
    let mut out: Vec<String> = Vec::new();
    for item in arr {
        let name = match item {
            serde_json::Value::String(s) => Some(s.as_str()),
            other => other
                .get("name")
                .and_then(|n| n.as_str())
                .or_else(|| other.get("model").and_then(|n| n.as_str())),
        };
        if let Some(n) = name {
            let n = n.trim();
            if !n.is_empty() && !out.iter().any(|x| x == n) {
                out.push(n.to_string());
            }
        }
    }
    out
}

/// 传输层失败：**连不上**与"对方答了但状态码不对"是两回事，文案不能混。
fn connect_error(url: &str, e: &reqwest::Error) -> String {
    let kind = if e.is_timeout() {
        TransportError::Timeout
    } else if e.is_connect() {
        TransportError::Connect
    } else {
        TransportError::Other
    };
    connect_error_text(url, kind, &e.to_string())
}

/// 传输层失败的种类（只为把文案拆成**纯函数**，好离线钉住）。
enum TransportError {
    Timeout,
    Connect,
    Other,
}

/// 传输层失败的文案。
///
/// ⚠️ **为什么单拆一个纯函数**：本机常常挂着代理（Clash/Mihomo 的 TUN，DNS 回 fake-ip），
/// "连一个没人听的端口"在那种环境下会被代理接走、回它自己的 502 —— 端到端用例**钉不住**
/// 这条分支（实测：断言"连不上"却收到"返回 HTTP 502"）。判据要能翻面，所以三种文案走纯函数，
/// 端到端只钉"传输层失败没被说成状态码失败"。
fn connect_error_text(url: &str, kind: TransportError, detail: &str) -> String {
    match kind {
        TransportError::Timeout => format!(
            "连不上 {url}：请求超时（{}s）—— 地址不对，或那台机器/代理不响应",
            DISCOVERY_TIMEOUT.as_secs()
        ),
        TransportError::Connect => format!("连不上 {url}：{detail}"),
        TransportError::Other => format!("请求 {url} 失败：{detail}"),
    }
}

/// 状态码失败：**key 不对** / **这个地址没有 /models** / 其它，三句不同的话。
fn status_error(code: u16, url: &str, body: &str) -> String {
    match code {
        401 | 403 => format!(
            "API key 不对或没有权限（HTTP {code}）—— 检查 llm.toml 里的 api_key{}",
            snippet(body)
        ),
        404 => format!(
            "这个地址没有 /models 接口（HTTP 404）—— {url} 可能不是 OpenAI 兼容的根地址（一般形如 https://host/v1）"
        ),
        _ => format!("{url} 返回 HTTP {code}{}", snippet(body)),
    }
}

/// 从一支模型的回包元素里**尽量**读能力；一个都读不到 ⇒ `None`。
///
/// 具体认哪些形状归**读取器注册表**（[`crate::caps::READERS`]）：这里只负责"依次过一遍、求并集"。
/// 一条铁律写在注册表那边：**认不出的字段忽略，绝不当成"支持"**。
fn caps_of(item: &serde_json::Value) -> Option<crate::caps::PartialCaps> {
    crate::caps::read_all(item)
}

/// 解析回包：`{"data":[…]}`（OpenAI）/ `{"models":[…]}`（部分中转）/ **裸数组**。
///
/// 数组元素两种写法都认：`{"id":…,"name":…}` 与裸字符串（少数网关只给名字）。
fn parse_models(body: &str, url: &str) -> Result<Vec<Discovered>, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| {
        format!(
            "{url} 回了 200，但不是 JSON（{e}）—— 中转可能插了一页 HTML{}",
            snippet(body)
        )
    })?;
    let arr = v
        .as_array()
        .or_else(|| v.get("data").and_then(|d| d.as_array()))
        .or_else(|| v.get("models").and_then(|d| d.as_array()))
        .ok_or_else(|| format!("{url} 的 JSON 里既没有 data 也没有 models 数组（形状不认识）"))?;

    let mut out: Vec<Discovered> = Vec::new();
    for item in arr {
        let (id, name) = match item {
            serde_json::Value::String(s) => (s.as_str(), None),
            other => {
                // id 缺了就用 name（Gemini/Ollama 只有 name；Gemini 还带 `models/` 前缀）
                let raw_id = other
                    .get("id")
                    .and_then(|i| i.as_str())
                    .or_else(|| other.get("name").and_then(|n| n.as_str()))
                    .unwrap_or_default();
                let id = raw_id.strip_prefix("models/").unwrap_or(raw_id);
                // 人名：displayName（Gemini）/ display_name（Anthropic）优先；
                // `name` 与 id 相同时（Ollama/Gemini）它就是 id，别当名字
                let name = other
                    .get("displayName")
                    .and_then(|n| n.as_str())
                    .or_else(|| other.get("display_name").and_then(|n| n.as_str()))
                    .or_else(|| {
                        let n = other.get("name").and_then(|n| n.as_str())?;
                        (n != id && n != raw_id).then_some(n)
                    });
                (id, name)
            }
        };
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        // 能力：带就取、不带就 None（**别编** —— "没探到"不等于"不支持"）
        let caps = caps_of(item);
        match out.iter_mut().find(|m| m.id == id) {
            // 去重：同 id 出现两次时留下**有名字**的那条
            Some(prev) => {
                if prev.name.is_none() {
                    prev.name = name.map(str::to_string);
                }
                // 同 id 重复出现时，能力也"留信息量大的那条"
                if prev.caps.is_none() {
                    prev.caps = caps;
                }
            }
            None => out.push(Discovered {
                id: id.to_string(),
                name: name.map(str::to_string),
                caps,
            }),
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

fn snippet(body: &str) -> String {
    let s: String = body.trim().chars().take(SNIPPET_MAX).collect();
    if s.is_empty() {
        String::new()
    } else {
        format!("：{s}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{ReasoningLevel, Support};
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// 假 `/models` 服务：按 script 依次回，顺带记下**第一个请求行**（钉 URL 怎么拼的）。
    ///
    /// ⚠️ script 用完后**继续 accept** 并回 500（同 `client.rs::fake_server` 的坑）：
    /// 直接 break 会让 listener 还在 listen 而没人 accept ⇒ 客户端"连得上、永远收不到响应"。
    async fn fake(script: Vec<(u16, String)>) -> (Arc<TcpListener>, String, Arc<Mutex<String>>) {
        let listener = Arc::new(TcpListener::bind("127.0.0.1:0").await.unwrap());
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(String::new()));
        let serve = listener.clone();
        let seen_srv = seen.clone();
        tokio::spawn(async move {
            let mut script = script.into_iter();
            loop {
                let Ok((mut sock, _)) = serve.accept().await else { break };
                let (code, body) = script
                    .next()
                    .unwrap_or((500, r#"{"error":"fake script exhausted"}"#.to_string()));
                let mut buf = [0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let line = String::from_utf8_lossy(&buf[..n])
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string();
                if let Ok(mut s) = seen_srv.lock() {
                    if s.is_empty() {
                        *s = line;
                    }
                }
                let reason = if code == 200 { "OK" } else { "ERR" };
                let resp = format!(
                    "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });
        (listener, format!("http://{addr}/v1"), seen)
    }

    /// 三种回包形状 + 去重 + 升序。少认一种，界面上就是"这个中转没有模型"。
    #[tokio::test]
    async fn 三种回包形状都认并且去重升序() {
        // ① OpenAI：{"data":[…]}，同名重复一条（**没有** name）—— 去重后要留下有名字的那条
        let openai = r#"{"object":"list","data":[
            {"id":"b-model","name":"B 模型"},
            {"id":"a-model"},
            {"id":"b-model"},
            {"id":""}
        ]}"#;
        let (_s, base, _) = fake(vec![(200, openai.to_string())]).await;
        let got = discover("k", &base, "").await.unwrap().models;
        assert_eq!(
            got,
            vec![
                Discovered { id: "a-model".into(), name: None, caps: None },
                Discovered { id: "b-model".into(), name: Some("B 模型".into()), caps: None },
            ],
            "data 形状 / 去重留名字 / id 升序"
        );

        // ② 部分中转：{"models":[…]}，元素是裸字符串
        let relay = r#"{"models":["z-model","a-model"]}"#;
        let (_s2, base2, _) = fake(vec![(200, relay.to_string())]).await;
        let got = discover("k", &base2, "").await.unwrap().models;
        assert_eq!(got.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["a-model", "z-model"]);

        // ③ 裸数组
        let bare = r#"[{"id":"only-one","name":"唯一"}]"#;
        let (_s3, base3, _) = fake(vec![(200, bare.to_string())]).await;
        let got = discover("k", &base3, "").await.unwrap().models;
        assert_eq!(got, vec![Discovered { id: "only-one".into(), name: Some("唯一".into()), caps: None }]);
    }

    /// 用户把整条端点粘过来（`…/chat/completions`）时，要探的是**根**上的 `/models`。
    #[tokio::test]
    async fn chat_completions_结尾的地址先剥成根() {
        assert_eq!(api_root("  https://api.deepseek.com/v1/  "), "https://api.deepseek.com/v1");
        assert_eq!(api_root("https://api.deepseek.com/v1/chat/completions"), "https://api.deepseek.com/v1");
        assert_eq!(api_root(""), "");

        let (_s, base, seen) = fake(vec![(200, r#"{"data":[{"id":"m"}]}"#.to_string())]).await;
        discover("k", &format!("{base}/chat/completions"), "").await.unwrap();
        let line = seen.lock().unwrap().clone();
        assert!(line.starts_with("GET /v1/models "), "请求行应为 GET /v1/models，实际：{line}");
    }

    /// ⚠️ **验收项**：HTTP 401（key 不对）与 200 空数组必须是**两句不同的话**。
    ///
    /// 把"key 写错"判成"这个中转不支持列模型"，用户就会去改 base_url —— 改多久都不会好。
    #[tokio::test]
    async fn key_不对与空列表是两句不同的话() {
        let (_s, base, _) = fake(vec![(401, r#"{"error":{"message":"invalid api key"}}"#.to_string())]).await;
        let err = discover("bad-key", &base, "").await.unwrap_err();
        assert!(err.contains("401"), "401 要出现在文案里：{err}");
        assert!(err.contains("key") || err.contains("api_key"), "要点名 key：{err}");
        assert!(!err.contains("/models 接口"), "别把 key 不对说成地址不对：{err}");

        // 200 + 空数组：**连上了、地址也对**，只是没有模型 ⇒ Ok(空)，不是 Err
        let (_s2, base2, _) = fake(vec![(200, r#"{"data":[]}"#.to_string())]).await;
        let got = discover("k", &base2, "").await.expect("200 空数组是合法答案").models;
        assert!(got.is_empty());
        let hint = empty_hint(&base2);
        assert_ne!(hint, err, "空列表与 key 不对必须是两句不同的话");
        assert!(hint.contains("没有报出任何模型"), "{hint}");
    }

    /// 404 = **这个地址没有 /models**（地址不对），与 401（key 不对）分开。
    #[tokio::test]
    async fn 状态码_404_说的是这个地址没有_models() {
        let (_s, base, _) = fake(vec![(404, "not found".to_string())]).await;
        let err = discover("k", &base, "").await.unwrap_err();
        assert!(err.contains("404"), "{err}");
        assert!(err.contains("/models"), "{err}");
        assert!(!err.contains("api_key"), "地址不对别去怪 key：{err}");
    }

    /// 传输层失败的三句文案（超时 / 连不上 / 其它）互不串味，且都不带 HTTP 状态码。
    #[test]
    fn 传输层失败的三句文案互不串味() {
        let url = "http://127.0.0.1:1/v1/models";
        let refused = connect_error_text(url, TransportError::Connect, "tcp connect error: 拒绝连接");
        let timeout = connect_error_text(url, TransportError::Timeout, "operation timed out");
        let other = connect_error_text(url, TransportError::Other, "builder error: relative URL without a base");
        assert!(refused.starts_with("连不上 "), "{refused}");
        assert!(timeout.contains("超时"), "{timeout}");
        assert!(other.starts_with("请求 "), "{other}");
        for t in [&refused, &timeout, &other] {
            assert!(!t.contains("HTTP"), "传输层失败不该报状态码：{t}");
        }
        assert_ne!(refused, timeout);
    }

    /// 端到端那半条：**传输层失败不会被说成状态码失败**。
    ///
    /// ⚠️ 这里刻意不拿"没人听的端口"试（见 `connect_error_text` 的说明：本机挂代理时
    /// 会被代理接走并回 502）。给一个 reqwest 在**发请求之前**就拒掉的地址，
    /// 拿到的必然是"请求 … 失败"，而不是任何 HTTP 状态码文案。
    #[tokio::test]
    async fn 建连失败不会被说成状态码失败() {
        let err = discover("k", "not-a-url", "").await.unwrap_err();
        assert!(err.starts_with("请求 "), "{err}");
        assert!(!err.contains("HTTP"), "{err}");
    }

    /// **能获取就获取**：OpenRouter 那种回包（architecture.input_modalities + supported_parameters）
    /// 里的能力必须被读出来，而不是只留 id/name。
    #[tokio::test]
    async fn openrouter_style_caps_are_read_from_the_same_payload() {
        let body = r#"{"data":[
            {"id":"router-a","name":"A",
             "architecture":{"input_modalities":["text","image"]},
             "supported_parameters":["temperature","reasoning_effort","max_tokens"]},
            {"id":"plain-b","name":"B"}
        ]}"#;
        let (_s, base, _) = fake(vec![(200, body.to_string())]).await;
        let got = discover("k", &base, "").await.unwrap().models;
        let a = got.iter().find(|m| m.id == "router-a").unwrap();
        let caps = a.caps.as_ref().expect("这支回包给了能力，必须读出来");
        assert!(caps.input.unwrap().image, "input_modalities 里的 image");
        assert_eq!(caps.dialect, Some("openai_effort"), "有 reasoning_effort");
        let levels = caps.levels.as_ref().expect("档位清单");
        assert_eq!(levels.len(), 2, "至少 off + high 两档：{levels:?}");
        assert!(levels.iter().any(|(lv, w)| *lv == ReasoningLevel::High && w.as_deref() == Some("high")));
        assert_eq!(caps.temperature, Some(Support::Supported), "列表里有 temperature");
        // 没有 supported_parameters 的那支：**不下结论**（caps = None，别编）
        let b = got.iter().find(|m| m.id == "plain-b").unwrap();
        assert!(b.caps.is_none(), "这家什么都没说 ⇒ 一个结论都不下：{:?}", b.caps);
    }

    /// 只有 reasoning ⇒ 开/关两档（thinking_type）；include_reasoning 也算推理信号。
    #[tokio::test]
    async fn reasoning_only_means_on_off() {
        let body = r#"{"models":[
            {"id":"m-onoff","supported_parameters":["reasoning"]},
            {"id":"m-inc","supported_parameters":["include_reasoning"]}
        ]}"#;
        let (_s, base, _) = fake(vec![(200, body.to_string())]).await;
        let got = discover("k", &base, "").await.unwrap().models;
        let caps = got[0].caps.as_ref().expect("reasoning ⇒ 有结论");
        assert_eq!(caps.dialect, Some("thinking_type"));
        assert_eq!(
            caps.temperature,
            Some(Support::Unsupported),
            "列表在但没有 temperature ⇒ 判不支持"
        );
        let inc = got[1].caps.as_ref().expect("include_reasoning 也算");
        assert_eq!(inc.dialect, Some("thinking_type"));
    }

    /// 另外两种常见形状：顶层 input_modalities / capabilities.modalities。
    #[tokio::test]
    async fn other_modality_shapes_are_recognised() {
        let body = r#"{"data":[
            {"id":"top-level","input_modalities":["text","image"]},
            {"id":"nested","capabilities":{"modalities":["text"]}}
        ]}"#;
        let (_s, base, _) = fake(vec![(200, body.to_string())]).await;
        let got = discover("k", &base, "").await.unwrap().models;
        // ⚠️ discover 按 id 升序返回（"nested" < "top-level"）—— 按 id 取，别按下标
        let top = got.iter().find(|m| m.id == "top-level").unwrap();
        assert!(top.caps.as_ref().unwrap().input.unwrap().image, "顶层 input_modalities");
        let nested = got.iter().find(|m| m.id == "nested").unwrap();
        let nested = nested.caps.as_ref().unwrap().input.unwrap();
        assert!(nested.text && !nested.image, "只写 text ⇒ 不吃图片");
    }

    /// to_json 的形状（服务端直接回给界面；键名与 caps_json 同源）。
    #[test]
    fn discovered_caps_json_shape() {
        let caps = crate::caps::PartialCaps {
            input: Some(crate::params::InputModalities { text: true, image: true }),
            dialect: Some("openai_effort"),
            levels: Some(vec![
                (ReasoningLevel::Off, None),
                (ReasoningLevel::High, Some("high".into())),
            ]),
            temperature: Some(Support::Supported),
            ..Default::default()
        };
        let v = caps.to_json();
        assert_eq!(v["input"], serde_json::json!({"text": true, "image": true}));
        assert_eq!(v["dialect"], "openai_effort");
        assert_eq!(v["levels"]["high"], "high");
        assert!(v["levels"]["off"].is_null(), "off = 不发这个参数");
        assert_eq!(v["temperature"], "supported");
        assert!(!caps.is_empty());
        assert!(crate::caps::PartialCaps::default().is_empty());
    }

    /// 形状不认识（既没有 data 也没有 models）要给一句能定位的话，而不是"没有模型"。
    #[tokio::test]
    async fn 形状不认识时报形状() {
        let (_s, base, _) = fake(vec![(200, r#"{"object":"list"}"#.to_string())]).await;
        let err = discover("k", &base, "").await.unwrap_err();
        assert!(err.contains("形状不认识"), "{err}");
    }
    // ── P3：路径级探测（LiteLLM / Ollama）───────────────────────────────────

    /// LiteLLM `/model/info` 的**载荷形状**（离线，不连网）：三种形状都认，且 `mode` 非 chat 不下结论。
    #[test]
    fn litellm_载荷三种形状与模式门() {
        let item = r#"{"model_name":"gpt-4o","model_info":{
            "key":"openai/gpt-4o","mode":"chat","supports_reasoning":true,
            "supports_vision":true,"supports_function_calling":true,
            "max_input_tokens":128000,"max_output_tokens":16384}}"#;
        let one: serde_json::Value = serde_json::from_str(item).unwrap();
        // ① 裸对象
        let c = crate::caps::read_litellm_info(&one, "gpt-4o").expect("裸对象也认");
        assert_eq!(c.context_window, Some(128000));
        assert_eq!(c.max_output_ceiling, Some(16384));
        assert!(c.input.unwrap().image, "supports_vision");
        assert_eq!(c.tool_call, Some(true), "supports_function_calling");
        assert_eq!(c.dialect, Some("openai_effort"), "支持推理 ⇒ 落 OpenAI 方言");
        assert!(c.levels.is_none(), "**说不出档位就别编**");

        // ② `{"data":[…]}`（LiteLLM 的常规回包）——按 model_name 对上 id
        let wrapped = serde_json::json!({ "data": [ one.clone() ] });
        assert!(crate::caps::read_litellm_info(&wrapped, "gpt-4o").is_some());
        assert!(
            crate::caps::read_litellm_info(&wrapped, "别的模型").is_none(),
            "对不上 id ⇒ None（**别把别人的能力安到这支头上**）"
        );

        // ③ 裸数组
        let arr = serde_json::Value::Array(vec![one.clone()]);
        assert!(crate::caps::read_litellm_info(&arr, "gpt-4o").is_some());

        // ④ 非聊天模型（mode=embedding）⇒ None：那不是聊天模型，推理/视觉都谈不上
        let emb = serde_json::json!({"model_name":"m","model_info":{"mode":"embedding","supports_reasoning":true}});
        assert!(crate::caps::read_litellm_info(&emb, "m").is_none(), "embedding 不下结论");

        // ⑤ 明确说不支持推理 ⇒ **空清单**（"知道没有"，与服务端清单同一条语义）
        let no = serde_json::json!({"model_name":"m","model_info":{"mode":"chat","supports_reasoning":false}});
        let c = crate::caps::read_litellm_info(&no, "m").unwrap();
        assert_eq!(c.levels, Some(Vec::new()));
        assert!(c.dialect.is_none(), "说不支持就不该给方言");
    }

    /// Ollama `/api/show` 的**载荷形状**（离线）：`thinking` ⇒ 支持推理；`vision`/`modalities` ⇒ image。
    #[test]
    fn ollama_载荷读出thinking与vision() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"capabilities":["completion","tools","thinking","vision"],
               "model_info":{"llama.context_length":131072,"llama.vision.block_count":32}}"#,
        )
        .unwrap();
        let c = crate::caps::read_ollama_show(&v, "llava:latest").expect("这是聊天模型");
        assert_eq!(c.dialect, Some("openai_effort"), "capabilities 里的 thinking ⇒ 支持推理");
        assert!(c.levels.is_none(), "它不说档位 ⇒ 不编");
        assert_eq!(c.tool_call, Some(true));
        assert!(c.input.unwrap().image, "capabilities 里的 vision ⇒ 收图");
        assert_eq!(c.context_window, Some(131072), "model_info.*.context_length");

        // 只有 text 的模型：能力清单在、但既不 thinking 也不 vision
        let plain: serde_json::Value =
            serde_json::from_str(r#"{"capabilities":["completion"],"model_info":{"x.context_length":8192}}"#).unwrap();
        let c = crate::caps::read_ollama_show(&plain, "llama3.2:latest").unwrap();
        assert_eq!(c.tool_call, Some(false), "清单在、没 tools ⇒ 明确不支持");
        assert!(!c.input.unwrap().image);
        assert!(c.dialect.is_none(), "没 thinking ⇒ 不给方言");

        // `modalities: ["text","image"]` 也算视觉信号（有的版本这么写）
        let mods: serde_json::Value =
            serde_json::from_str(r#"{"capabilities":["completion"],"modalities":["text","image"]}"#).unwrap();
        assert!(crate::caps::read_ollama_show(&mods, "m").unwrap().input.unwrap().image);

        // 不是聊天模型（没有 completion）⇒ None
        let emb: serde_json::Value = serde_json::from_str(r#"{"capabilities":["embedding"]}"#).unwrap();
        assert!(crate::caps::read_ollama_show(&emb, "nomic-embed").is_none());
    }

    /// **端到端：能力来自另一条路径**（LiteLLM）—— 且**只补不抹**。
    ///
    /// 这一条就是"变异自证"要打红的那条：把 `merge_caps_into` 改成覆盖式赋值，
    /// `/v1/models` 读到的档位会被 `/model/info` 只说了 `supports_vision` 的那份**抹掉** ⇒ 这里红。
    #[tokio::test]
    async fn litellm_路径探测补能力且只补不抹() {
        let models = r#"{"data":[{"id":"gpt-4o","supported_parameters":["temperature","reasoning_effort"]}]}"#;
        let info = r#"{"data":[{"model_name":"gpt-4o","model_info":{
            "mode":"chat","supports_vision":true,"max_input_tokens":128000,"max_output_tokens":16384}}]}"#;
        // 第三笔给 Ollama 的 `/api/tags`（这条测试里它必然 404 —— 网关不是 Ollama）
        let (_s, base, _) = fake(vec![
            (200, models.to_string()),
            (200, info.to_string()),
            (404, "not found".to_string()),
        ])
        .await;
        let d = discover("k", &base, "").await.unwrap();
        assert_eq!(d.models.len(), 1);
        let caps = d.models[0].caps.as_ref().expect("两条路都给了能力");
        // 来自 `/v1/models` 的（reader 注册表）
        let levels = caps.levels.as_ref().expect("档位清单必须**还在**（只补不抹）");
        assert!(levels.iter().any(|(lv, _)| *lv == ReasoningLevel::High), "{levels:?}");
        assert_eq!(caps.temperature, Some(Support::Supported));
        // 来自 `/model/info` 的
        assert!(caps.input.unwrap().image, "supports_vision 补进来了");
        assert_eq!(caps.context_window, Some(128000));
        assert_eq!(caps.max_output_ceiling, Some(16384));
        assert_eq!(d.caps_from, vec!["litellm"], "能力来自哪条路径要说得出");
        assert_eq!(d.tried, vec!["/models", "/model/info", "/api/tags"]);
    }

    /// **服务根上的路径也认**：`base_url` 是 `…/v1`，而 LiteLLM 的 `/model/info` 在**根**上。
    ///
    /// 判据形状：`{root}/model/info` 先 404（第一笔），同一条路径**换到服务根**再打一次就 200。
    #[tokio::test]
    async fn 路径探测会退到服务根再试一次() {
        let models = r#"{"data":[{"id":"m1"}]}"#;
        let info = r#"{"data":[{"model_name":"m1","model_info":{"mode":"chat","supports_vision":true}}]}"#;
        let (_s, base, _) = fake(vec![
            (200, models.to_string()),
            (404, "not here".to_string()), // {root}/model/info
            (200, info.to_string()),       // {origin}/model/info
            (404, "not here".to_string()), // {root}/api/tags
            (404, "not here".to_string()), // {origin}/api/tags
        ])
        .await;
        let d = discover("k", &base, "").await.unwrap();
        assert_eq!(d.caps_from, vec!["litellm"], "根上的 /model/info 必须被用上");
        assert!(d.models[0].caps.as_ref().unwrap().input.unwrap().image);
        // `tried` 记的是**路径**（不因两个 URL 变体重复）
        assert_eq!(d.tried, vec!["/models", "/model/info", "/api/tags"]);
    }

    /// 拼 URL 的两个变体（纯函数）：`{root}{path}` + `{origin}{path}`，重复只留一个。
    #[test]
    fn 探测路径会拼出根与服务根两个变体() {
        assert_eq!(
            probe_urls("https://h/v1", "/model/info"),
            vec!["https://h/v1/model/info", "https://h/model/info"]
        );
        // `base_url` 本来就是服务根 ⇒ 只一个，不重复打
        assert_eq!(probe_urls("https://h", "/api/tags"), vec!["https://h/api/tags"]);
        assert_eq!(origin_of("https://h:8080/v1"), Some("https://h:8080".into()));
        assert_eq!(origin_of("not-a-url"), None);
    }
    /// **端到端：Ollama** —— 先 `/api/tags` 拿候选名，再逐支 POST `/api/show`；认不出的跳过。
    #[tokio::test]
    async fn ollama_路径探测逐支问且认不出就跳过() {
        let models = r#"{"data":[{"id":"llama3.2:latest"},{"id":"nomic-embed-text:latest"}]}"#;
        let tags = r#"{"models":[{"name":"llama3.2:latest"},{"name":"nomic-embed-text:latest"}]}"#;
        let show_llama = r#"{"capabilities":["completion","tools","thinking"],
            "model_info":{"llama.context_length":131072}}"#;
        let show_embed = r#"{"capabilities":["embedding"],"model_info":{"bert.context_length":512}}"#;
        // ⚠️ 每条路径都会**先试 `{root}`（…/v1）再试服务根**，所以每一笔要按两下写：
        //    Ollama 的原生面在根上 ⇒ `{root}/api/...` 404、根上 200。
        let (_s, base, _) = fake(vec![
            (200, models.to_string()),
            (404, "not here".to_string()), // {root}/model/info（LiteLLM 那条：两下都 404）
            (404, "not here".to_string()), // {origin}/model/info
            (404, "not here".to_string()), // {root}/api/tags
            (200, tags.to_string()),       // {origin}/api/tags
            (404, "not here".to_string()), // {root}/api/show（llama）
            (200, show_llama.to_string()), // {origin}/api/show（llama）
            (404, "not here".to_string()), // {root}/api/show（embedding）
            (200, show_embed.to_string()), // {origin}/api/show（embedding）
        ])
        .await;
        let d = discover("k", &base, "").await.unwrap();
        let llama = d.models.iter().find(|m| m.id == "llama3.2:latest").unwrap();
        let c = llama.caps.as_ref().expect("Ollama 那条路给了能力");
        assert_eq!(c.dialect, Some("openai_effort"), "thinking ⇒ 支持推理");
        assert_eq!(c.context_window, Some(131072));
        assert_eq!(c.tool_call, Some(true));
        // embedding 那支：read 返回 None ⇒ **一个结论都不下**（不是"不支持推理"）
        let emb = d.models.iter().find(|m| m.id == "nomic-embed-text:latest").unwrap();
        assert!(emb.caps.is_none(), "非聊天模型不下结论：{:?}", emb.caps);
        assert_eq!(d.caps_from, vec!["ollama"]);
        assert!(d.tried.contains(&"/api/tags".to_string()));
        assert!(d.tried.contains(&"/api/show".to_string()));
    }

    /// **路径不通不阻断**：404 / 垃圾 JSON 都不许影响主流程，但要在 `tried` 里留痕。
    #[tokio::test]
    async fn 路径探测失败不阻断主流程但留痕() {
        let models = r#"{"data":[{"id":"a-model"}]}"#;
        let (_s, base, _) = fake(vec![
            (200, models.to_string()),
            (404, "no such path".to_string()),
            (200, "<html>不是 JSON</html>".to_string()),
        ])
        .await;
        let d = discover("k", &base, "").await.expect("路径探测失败不该让整趟探测失败");
        assert_eq!(d.models.len(), 1, "模型清单照常返回");
        assert!(d.models[0].caps.is_none(), "两条路都没读到 ⇒ 不下结论");
        assert!(d.caps_from.is_empty());
        assert_eq!(d.tried, vec!["/models", "/model/info", "/api/tags"], "试过哪些路径要说得清");
    }

    /// 注册表本身：每条 probe 的 read 都必须能对着**垃圾载荷**返回 None（不 panic）。
    #[test]
    fn 路径探测注册表对垃圾载荷返回none() {
        for probe in PATH_PROBES {
            for junk in [serde_json::json!(null), serde_json::json!([]), serde_json::json!({"nope":1}), serde_json::json!("字符串")] {
                assert!(
                    (probe.read)(&junk, "any-id").is_none(),
                    "{} 对垃圾载荷必须返回 None",
                    probe.id
                );
            }
            assert!(!probe.id.is_empty() && !probe.label.is_empty());
            assert!(!probe.paths.is_empty(), "每条 probe 至少要声明一条路径");
            assert!(probe.paths.iter().all(|p| p.starts_with('/')), "路径要 / 开头");
        }
        // PostPerModel 要求**两条**路径（先列模型、再逐支问）
        for probe in PATH_PROBES.iter().filter(|p| p.fetch == Fetch::PostPerModel) {
            assert_eq!(probe.paths.len(), 2, "{}：PostPerModel 需要 两条路径", probe.id);
        }
    }
}
