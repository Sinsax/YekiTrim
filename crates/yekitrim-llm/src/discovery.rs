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
    pub caps: Option<DiscoverCaps>,
}

/// 探测时**顺带**读到的能力（"能获取当然最好"那条）。
///
/// ⚠️ **只用于展示与"这一趟探到的能力"**：不落盘、不写进 `llm.toml`（既有口径：草稿不落盘）。
/// 每个字段都可能是 `None`（回包没给）—— 调用方要保持表里的值，别拿"没探到"当"不支持"。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiscoverCaps {
    /// 输入模态（`["text","image"]` 之类）。
    pub input: Option<crate::params::InputModalities>,
    /// 推理写法（注册表 id：`openai_effort` / `thinking_type` …）；`None` = 没探出来。
    pub dialect: Option<&'static str>,
    /// 档位 → wire 拼写（只放**探到的**那几档）。
    pub levels: Vec<(crate::params::ReasoningLevel, Option<String>)>,
    /// 能不能带 `temperature`；`None` = 没探出来（保持表里的值）。
    pub temperature: Option<bool>,
}

impl DiscoverCaps {
    pub fn is_empty(&self) -> bool {
        self.input.is_none()
            && self.dialect.is_none()
            && self.levels.is_empty()
            && self.temperature.is_none()
    }

    /// 给界面用的形状（键名与 `catalog::caps_json` 同源，前端一套归一化就能吃）。
    pub fn to_json(&self) -> serde_json::Value {
        let mut levels = serde_json::Map::new();
        for (level, wire) in &self.levels {
            levels.insert(
                level.as_str().to_string(),
                match wire {
                    Some(w) => serde_json::Value::String(w.clone()),
                    None => serde_json::Value::Null,
                },
            );
        }
        serde_json::json!({
            "input": self.input.map(|m| m.to_json()),
            "dialect": self.dialect,
            "levels": levels,
            "temperature": self.temperature.map(|ok| if ok { "supported" } else { "unsupported" }),
        })
    }
}

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

/// 探一个 `base_url` 上的模型清单：`GET {root}/models` + `Authorization: Bearer`。
///
/// 返回按 `id` 升序、去重（同 id 出现两次时留下**有名字**的那条）。
pub async fn discover(api_key: &str, base_url: &str) -> Result<Vec<Discovered>, String> {
    let root = api_root(base_url);
    if root.is_empty() {
        return Err("没填 API 基础地址（base_url）".to_string());
    }
    let url = format!("{root}/models");
    // 客户端构造走 client 那一份（UA / connect_timeout / keepalive 只在那里设）
    let resp = crate::client::build_http_client(DISCOVERY_TIMEOUT)
        .get(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .send()
        .await
        .map_err(|e| connect_error(&url, &e))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(status_error(status.as_u16(), &url, &body));
    }
    parse_models(&body, &url)
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
/// 认这几种常见形状（都在 data/models/裸数组的元素上）：
/// * OpenRouter：`architecture.input_modalities: ["text","image"]`
/// * 通用：`input_modalities` / `modalities` / `capabilities.{input_modalities,modalities}`
/// * 参数面：`supported_parameters: ["reasoning_effort","reasoning","temperature",…]`
///
/// ⚠️ **没有 `supported_parameters` 时不下结论**：那说明这家根本不说自己支持什么，
/// "没说"不等于"不支持" —— 保持表里的值（这条路径什么都不回）。
/// ⚠️ `temperature` 同理：只有那串**存在**时才判断有没有它。
fn caps_of(item: &serde_json::Value) -> Option<DiscoverCaps> {
    use crate::params::{InputModalities, ReasoningLevel};

    let mut caps = DiscoverCaps::default();

    // ① 输入模态
    let list = item
        .get("architecture")
        .and_then(|a| a.get("input_modalities"))
        .or_else(|| item.get("input_modalities"))
        .or_else(|| item.get("modalities"))
        .or_else(|| item.get("capabilities").and_then(|c| c.get("input_modalities")))
        .or_else(|| item.get("capabilities").and_then(|c| c.get("modalities")))
        .and_then(|v| v.as_array());
    if let Some(arr) = list {
        caps.input = Some(InputModalities::parse(arr.iter().filter_map(|v| v.as_str())));
    }

    // ② 参数面
    if let Some(params) = item.get("supported_parameters").and_then(|v| v.as_array()) {
        let has = |needle: &str| {
            params
                .iter()
                .any(|p| p.as_str().map(|s| s.eq_ignore_ascii_case(needle)).unwrap_or(false))
        };
        // 推理：先看"能指定档位"（reasoning_effort），再看"只能开关"（reasoning / include_reasoning）
        if has("reasoning_effort") {
            caps.dialect = Some("openai_effort");
            caps.levels = vec![
                (ReasoningLevel::Off, None),                 // 关 = 不发这个参数
                (ReasoningLevel::High, Some("high".into())), // 至少 high 档
            ];
        } else if has("reasoning") || has("include_reasoning") {
            caps.dialect = Some("thinking_type");
            caps.levels = vec![
                (ReasoningLevel::Off, None),
                (ReasoningLevel::High, Some("high".into())),
            ];
        }
        // 温度：**列表存在才下结论**（没有它不代表不支持）
        caps.temperature = Some(has("temperature"));
    }

    (!caps.is_empty()).then_some(caps)
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
            other => (
                other.get("id").and_then(|i| i.as_str()).unwrap_or_default(),
                other.get("name").and_then(|n| n.as_str()),
            ),
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
    use crate::params::ReasoningLevel;
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
                    body.as_bytes().len()
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
        let got = discover("k", &base).await.unwrap();
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
        let got = discover("k", &base2).await.unwrap();
        assert_eq!(got.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["a-model", "z-model"]);

        // ③ 裸数组
        let bare = r#"[{"id":"only-one","name":"唯一"}]"#;
        let (_s3, base3, _) = fake(vec![(200, bare.to_string())]).await;
        let got = discover("k", &base3).await.unwrap();
        assert_eq!(got, vec![Discovered { id: "only-one".into(), name: Some("唯一".into()), caps: None }]);
    }

    /// 用户把整条端点粘过来（`…/chat/completions`）时，要探的是**根**上的 `/models`。
    #[tokio::test]
    async fn chat_completions_结尾的地址先剥成根() {
        assert_eq!(api_root("  https://api.deepseek.com/v1/  "), "https://api.deepseek.com/v1");
        assert_eq!(api_root("https://api.deepseek.com/v1/chat/completions"), "https://api.deepseek.com/v1");
        assert_eq!(api_root(""), "");

        let (_s, base, seen) = fake(vec![(200, r#"{"data":[{"id":"m"}]}"#.to_string())]).await;
        discover("k", &format!("{base}/chat/completions")).await.unwrap();
        let line = seen.lock().unwrap().clone();
        assert!(line.starts_with("GET /v1/models "), "请求行应为 GET /v1/models，实际：{line}");
    }

    /// ⚠️ **验收项**：HTTP 401（key 不对）与 200 空数组必须是**两句不同的话**。
    ///
    /// 把"key 写错"判成"这个中转不支持列模型"，用户就会去改 base_url —— 改多久都不会好。
    #[tokio::test]
    async fn key_不对与空列表是两句不同的话() {
        let (_s, base, _) = fake(vec![(401, r#"{"error":{"message":"invalid api key"}}"#.to_string())]).await;
        let err = discover("bad-key", &base).await.unwrap_err();
        assert!(err.contains("401"), "401 要出现在文案里：{err}");
        assert!(err.contains("key") || err.contains("api_key"), "要点名 key：{err}");
        assert!(!err.contains("/models 接口"), "别把 key 不对说成地址不对：{err}");

        // 200 + 空数组：**连上了、地址也对**，只是没有模型 ⇒ Ok(空)，不是 Err
        let (_s2, base2, _) = fake(vec![(200, r#"{"data":[]}"#.to_string())]).await;
        let got = discover("k", &base2).await.expect("200 空数组是合法答案");
        assert!(got.is_empty());
        let hint = empty_hint(&base2);
        assert_ne!(hint, err, "空列表与 key 不对必须是两句不同的话");
        assert!(hint.contains("没有报出任何模型"), "{hint}");
    }

    /// 404 = **这个地址没有 /models**（地址不对），与 401（key 不对）分开。
    #[tokio::test]
    async fn 状态码_404_说的是这个地址没有_models() {
        let (_s, base, _) = fake(vec![(404, "not found".to_string())]).await;
        let err = discover("k", &base).await.unwrap_err();
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
        let err = discover("k", "not-a-url").await.unwrap_err();
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
        let got = discover("k", &base).await.unwrap();
        let a = got.iter().find(|m| m.id == "router-a").unwrap();
        let caps = a.caps.as_ref().expect("这支回包给了能力，必须读出来");
        assert!(caps.input.unwrap().image, "input_modalities 里的 image");
        assert_eq!(caps.dialect, Some("openai_effort"), "有 reasoning_effort");
        assert_eq!(caps.levels.len(), 2, "至少 off + high 两档：{:?}", caps.levels);
        assert!(caps.levels.iter().any(|(lv, w)| *lv == ReasoningLevel::High && w.as_deref() == Some("high")));
        assert_eq!(caps.temperature, Some(true), "列表里有 temperature");
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
        let got = discover("k", &base).await.unwrap();
        let caps = got[0].caps.as_ref().expect("reasoning ⇒ 有结论");
        assert_eq!(caps.dialect, Some("thinking_type"));
        assert_eq!(caps.temperature, Some(false), "列表在但没有 temperature ⇒ 判不支持");
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
        let got = discover("k", &base).await.unwrap();
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
        let caps = DiscoverCaps {
            input: Some(crate::params::InputModalities { text: true, image: true }),
            dialect: Some("openai_effort"),
            levels: vec![(ReasoningLevel::Off, None), (ReasoningLevel::High, Some("high".into()))],
            temperature: Some(true),
        };
        let v = caps.to_json();
        assert_eq!(v["input"], serde_json::json!({"text": true, "image": true}));
        assert_eq!(v["dialect"], "openai_effort");
        assert_eq!(v["levels"]["high"], "high");
        assert!(v["levels"]["off"].is_null(), "off = 不发这个参数");
        assert_eq!(v["temperature"], "supported");
        assert!(!caps.is_empty());
        assert!(DiscoverCaps::default().is_empty());
    }

    /// 形状不认识（既没有 data 也没有 models）要给一句能定位的话，而不是"没有模型"。
    #[tokio::test]
    async fn 形状不认识时报形状() {
        let (_s, base, _) = fake(vec![(200, r#"{"object":"list"}"#.to_string())]).await;
        let err = discover("k", &base).await.unwrap_err();
        assert!(err.contains("形状不认识"), "{err}");
    }
}
