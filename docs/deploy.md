# 部署

三种形态，安全要求差别很大。**先认清你要哪一种**，再照对应清单做。

| 形态 | 谁在访问 | 需要的动作 |
|---|---|---|
| **A. 本机自用** | 只有你自己，`127.0.0.1` | 什么都不用做（默认就是这个） |
| **B. 内网/自己人** | 少数可信的人在局域网或 VPN 里 | 加 `--allow-remote`，改掉初始密码 |
| **C. 公网服务** | 公网可达（邀请制也算） | B + 反代 HTTPS + `--trust-proxy` + 下面整份清单 |

> ⚠️ 这个服务能**读写它所在机器上的工程目录**、调用 `ffmpeg` 与 headless Chrome。
> 把它暴露到不可信网络之前，请当作"暴露一台能跑命令的机器"来对待。

---

## A. 本机自用

```bash
vtrim server --open          # 默认 127.0.0.1:26320
```

默认就只监听 loopback，且**全部接口都要登录**（含本机访问，没有免登录分支）。
服务不会主动对外的其他行为：LLM 请求发往你配置的 `base_url`；ASR 模型从 GitHub release 下载
到 `<exe同级>/.vtrim/models/`。

默认端口 26320 是刻意挑的：老默认 7788 在部分 Windows 上属于 Hyper-V/WSL 的**保留端口段**
（实测会被 `10013 WSAEACCES` 拒绝，且 `netstat` 里看不到任何监听者 —— 它不是"被占用"）。
26320 在跨平台安全带 15001–32767 内（躲开 Windows 动态段/保留段，也在 Linux 动态段 32768–60999 之外）。
换端口用 `--port`；绑不上时程序会打印一个**实测可绑**的替代端口。

## B. 内网 / 可信小圈子

```bash
vtrim user passwd admin              # 先把初始密码改掉
vtrim server --host 0.0.0.0 --allow-remote
```

- `--allow-remote` 是必须的：不加它服务会拒绝监听非 loopback 地址。
- **仍在使用初始密码时会直接拒绝启动** —— 初始密码会写进日志文件与
  `<data_dir>/initial-admin-password.txt`，等于把凭证摊在磁盘上。
- 邀请制注册：`vtrim user invite --count 5` 生成邀请码并打开注册开关
  （`vtrim user invites` 查看，`--disable` 关闭）。
- 到这一步仍建议套一层 HTTPS（自签或内网证书），否则口令与 cookie 是明文过网的。

## C. 公网服务（含邀请制 SaaS）

### 1. 反代 + HTTPS（必做）

服务**自身没有 TLS**，只能裸 HTTP。请务必前置 Caddy/nginx 并终止 TLS。

Caddy 最小配置：

```
vtrim.example.com {
    reverse_proxy 127.0.0.1:26320
}
```

nginx 要点：`proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;`
与 `proxy_set_header X-Forwarded-Proto $scheme;`，并且**不要**做响应缓冲以外的改写
（SSE 端点 `/api/stream`、`/api/admin/logs/events`、`/api/jobs/:id/events` 需要
`proxy_buffering off;`）。

### 2. 启动参数

```bash
vtrim user passwd admin                        # 强口令，别用初始密码
vtrim server --host 127.0.0.1 --allow-remote \
             --trust-proxy \
             --data-dir /var/lib/vtrim
```

| 参数 | 为什么 |
|---|---|
| `--host 127.0.0.1` | 让服务只被本机反代访问，公网入口只有反代一个。多一层隔离。 |
| `--allow-remote` | 不加就不许监听非 loopback（即使 host 写的是 127.0.0.1 也建议显式加上，避免后续误改 host 时被拒启动搞混）。 |
| `--trust-proxy` | **反代后面必须加**。不加的话所有请求都被当成来自反代本机：按 IP 的限流退化成全局一份额度，`Secure` cookie 也不会下发。 |
| `--data-dir` | 账号账本、日志、模型的落点。默认在 `<exe_dir>/.vtrim/`，容器里通常是只读层 → 会启动失败。 |

> `--trust-proxy` 打开后，客户 IP 取 `X-Forwarded-For` 的**最后一段**（自己那台反代追加的
> 真实对端）。别让反代把客户端自带的 `X-Forwarded-For` 原样透传在最外层之外 ——
> Caddy/nginx 的默认配置都会正确追加。

### 3. 数据目录

```
<data_dir>/
├── vtrim.db                  # 账号 / 会话 / 审计 / 项目登记 / 邀请码（明文）/ LLM 用量
├── config.toml, llm.toml     # 全局配置（llm.toml 含 API key）
├── logs/                     # 运行日志
└── models/                   # ASR 模型（约 1.1GB）
```

- **这个目录不要公开、不要打进镜像层/分享包**。里面的 `vtrim.db` 含**明文邀请码**与所有项目的
  真实绝对路径；`llm.toml` 含 LLM API key。
- 备份就是拷这个目录（加上各工程目录里的 `vtrim.toml` 与 `output/`）。
- 容器部署把它挂成 volume；镜像里那份空的 `.vtrim/` 会在启动时被写入，别用只读层。

### 4. 上线前检查清单

- [ ] HTTPS 反代生效，浏览器地址栏是 https
- [ ] `--allow-remote` 已加，且**初始密码已改**（未改会被拒绝启动）
- [ ] `--trust-proxy` 已加（反代后面）
- [ ] `vtrim doctor --url https://你的域名` 输出里 `listening=remote`、`tls_terminated=true`
- [ ] 注册开关的状态是你想要的（`vtrim user invites`；不需要自助注册就用 `--disable`）
- [ ] `data_dir` 不在公开可读的位置，也没有进镜像层
- [ ] 知道限流与配额的存在：超出时是**拒绝**（HTTP 429 / 任务记失败），不是排队
      —— 见 [`job-queue.md`](job-queue.md) §3.5 与 `vtrim-server/src/ratelimit.rs`

### 5. 已知边界（按此设计使用）

- **没有配额计费**：`users.quota_bytes` 只在入库（录播姬自动管线）那条路上做水位预检；
  手动发起的转写/渲染/剪切没有按账号的存储配额。公开服务请自己盯着磁盘。
- **限流是按 IP 的**（加上任务侧的按账号配额）。同一出口 IP 后面的多个用户共享一份额度；
  额度设得较宽（见 `ratelimit.rs` 常量），正常使用碰不到。
- **日志含项目路径**：`/api/admin/logs` 与 `/api/admin/usage` 因此**仅 admin**。
  给出去的账号一律用普通角色（`vtrim user add <name> --role user`）。
- 单实例：同一个 `data_dir` 只能跑一个进程（`vtrim.lock`）。要多副本请各自独立 `data_dir`
  与端口，再在上层做路由 —— 但工程目录也会因此被两个实例同时写，不建议。

---

## 发行给别人（自部署）

1. 用 `cargo xtask package --smoke-test` 打包：会在打包后**清掉**
   解包目录里的 `.vtrim/` 与 `stickers/`（跑过冒烟测试会生成，含真实绝对路径）。
2. 检查 `release/<包名>/` 里没有 `.vtrim/` 再分发。`.zip` 本身不含它。
3. 告诉用户需要自备 `ffmpeg` / `ffprobe`（在 `PATH` 中）与 Chrome/Edge（精修出片要用）。
4. 许可：发行二进制是 **Apache-2.0**（宽松许可，**允许闭源再分发**）。打包时已**强制**随包
   附上 `LICENSE` 与 `THIRD-PARTY-NOTICES.md`，分发前确认二者在包内即可 —— 见仓库根
   [`LICENSE`](../LICENSE) 与 [`THIRD-PARTY-NOTICES.md`](../THIRD-PARTY-NOTICES.md)。
