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
yekitrim server --open          # 默认 127.0.0.1:26320
```

默认就只监听 loopback，且**全部接口都要登录**（含本机访问，没有免登录分支）。
服务不会主动对外的其他行为：LLM 请求发往你配置的 `base_url`；ASR 模型从 GitHub release 下载
到 `<exe同级>/.yekitrim/models/`。

默认端口 26320 是刻意挑的：老默认 7788 在部分 Windows 上属于 Hyper-V/WSL 的**保留端口段**
（实测会被 `10013 WSAEACCES` 拒绝，且 `netstat` 里看不到任何监听者 —— 它不是"被占用"）。
26320 在跨平台安全带 15001–32767 内（躲开 Windows 动态段/保留段，也在 Linux 动态段 32768–60999 之外）。
换端口用 `--port`；绑不上时程序会打印一个**实测可绑**的替代端口。

### 双击就用：`yekitrim-tray.exe`（Windows）

不想开终端的话，发行包里另有一个 **`yekitrim-tray.exe`**（同目录还有它要的 `yekitrim.ico`）：

- **双击即后台起服务**（无控制台窗口），并**自动打开浏览器**到 WebUI。
- ⚠️ **它是个"薄启动器"（0.5 MB），必须和 `yekitrim.exe` 放在同一个目录**：
  双击后由它去起 `yekitrim.exe server --local-no-auth --open`，真正干活的是后者。
  所以任务管理器里看到的是**两个进程**：`yekitrim-tray.exe`（几 MB，只管托盘）+ `yekitrim.exe`（几十 MB）。
  这么做是因为 Windows 的子系统（有没有控制台）**是每个 exe 链接期定死的**：一个 exe 没法既当 CLI
  （终端里要看到输出、要被脚本等待）又当 GUI 托盘。让托盘重复整个服务曾让它涨到 57 MB，现在是反过来。
- ⚠️ **托盘被强杀（任务管理器「结束任务」）不会留下孤儿服务**：子进程被放进了一个
  `KILL_ON_JOB_CLOSE` 的 Job，父进程一死，系统把服务（以及它拉起的 ffmpeg 等孙进程）一起收走。
  代价是那一刀是硬终止 —— 锁会残留一条 pid，但**下次启动会自动接管**（日志里一句「检测到上次运行残留」）。
- ⚠️ **关停是跨进程的**：点「退出」时托盘会 POST 一个带令牌的 `/api/shutdown`（令牌只经环境变量
  传给子进程），服务按**和 Ctrl+C 一样**的方式优雅关停（锁写成 `pid=0`）。如果 LIMIT 内没停掉，
  托盘会 `taskkill` 兜底。
- **免登录**：它以 `--local-no-auth` 起服务，打开就是管理员，**不需要登录**。
  服务端为此加了两道闸（都只在免登录模式下生效）：
  · `Host` 必须是 loopback 主机名 —— 防 DNS rebinding；
  · 写请求的 `Origin` / `Sec-Fetch-Site` 必须同源 —— 防别的网页跨站调这套 API。
  ⚠️ 也就是说：**免登录 = 这台机器上能访问 127.0.0.1 的任何进程都能操作它**（它本来就是个本机工具）。
  要对外/多人用，请用 `yekitrim server`(**要登录**) 那条路，别用托盘这套。
  ⚠️ 判据（改了这条便利必须跑）：`auth::tests::local_no_auth_guards_reject_foreign_host_and_cross_site`
  （含 `localhost.evil.com` 这类变体）、`routes::tests::shutdown_endpoint_is_404_without_token_env`、
  `auth::tests::shutdown_token_gate_is_exact_and_empty_safe`、
  `unauthenticated_requests_are_401_except_whitelist`（默认不带 flag 时一个字节都没变）。

  **实现锚点**（改之前先读这几处；正文都在这几行注释里，别在本卡抄第二份）：
  · 免登录分支在 `auth::authenticate` 开头：`AppState::local_admin` 是 `Some` 就跳过会话与 CSRF、
    直接注入本机管理员；两道闸在 `auth::guard_local_request`（`Host` 必须 loopback 主机名 +
    写请求 `Origin`/`Sec-Fetch-Site` 同源）。开启条件由 `run_with` 的开机守卫保证
    （**只允许 loopback、且与 `--allow-remote` 互斥** —— 缺一条就是"谁都能免密进来的后台"）。
    哨兵会话 id `LOCAL_SESSION_ID = "local-no-auth"`（**不在 sessions 表里**）⇒ 依赖真实会话的
    功能在本地模式下不可用（最典型是改密码）。
  · 关停通道在 `api::shutdown`：env `YEKITRIM_SHUTDOWN_TOKEN` **没设 ⇒ 404**（不给探测口）、
    对端必须 loopback、头 `x-yekitrim-shutdown-token` 要**常量时间**匹配
    （`auth::shutdown_token_ok` / `auth::token_ok_for`，**别退回 `==`**）。效果**等价于 Ctrl+C**
    （走 `run_with` 的优雅关停、限时 `SHUTDOWN_GRACE`、锁写成 `pid=0`）—— **别改成直接 `process::exit`**。
    ⚠️ 令牌名/头名与 `crates/yekitrim-cli/src/tray.rs` 里**各写一份字面量**（托盘链接不了 server crate），
    **改一处必须改两处**。
- 通知区图标：**左键 = 打开网页**；**右键**菜单 = 当前地址（灰显只读）/ 打开网页 / **退出**。
- 「退出」走的是**优雅关停**：服务收尾后 `yekitrim.lock` 会被写成 `pid=0`，下次启动不会报"残留锁"。
- **已经在跑时再双击**：它会先探测该端口上是不是已有 YekiTrim —— 是就**直接把已有界面打开**再退出，
  不会报"端口被占"，也不会起第二个服务。
- 端口默认 `26320`；要给开发中的实例让路可用环境变量 **`YEKITRIM_PORT`** 覆盖（例：`set YEKITRIM_PORT=26399`）。
- ⚠️ **没有控制台 ⇒ 出错只能靠弹窗与日志**：起不来时会弹一个说明框，细节在
  `<数据目录>/logs/yekitrim-YYYYMMDD.log`。首次运行的管理员初始密码在
  `<数据目录>/initial-admin-password.txt`。
- 想让开机自启：把 `yekitrim-tray.exe` 的快捷方式放进
  `shell:startup`（`%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup`）即可 ——
  不用装服务、不用管理员权限。

## B. 内网 / 可信小圈子

```bash
yekitrim user passwd admin              # 先把初始密码改掉（账本默认在 exe 旁边，见下）
yekitrim server --host 0.0.0.0 --allow-remote
```

> ⚠️ **改密码与启动必须是同一份账本**：`yekitrim user passwd` 是**另一个进程**，它没有 `--data-dir`
> 参数，只认环境变量 `YEKITRIM_DATA_DIR`。所以如果服务端要带 `--data-dir`（容器、系统目录部署都要），
> 改密那条命令也得带上同一个值 —— 否则改的是 exe 旁边那份，服务器上这份还是初始密码，启动照样被拒，
> 而命令行里 `yekitrim user list` 看起来一切正常（"我明明改了密码"的报障形态）。
> 拒绝启动时**日志会直接把整条命令打出来**（含这个变量），照抄即可。

- `--allow-remote` 是必须的：不加它服务会拒绝监听非 loopback 地址。
- **仍在用初始密码时，只要对外暴露就拒绝启动** —— 判据是「非 loopback 监听 **或** 显式加了
  `--allow-remote`」：`--host 127.0.0.1 --allow-remote` 也算暴露（反代就在本机），同样会被拦。
  初始密码会写进日志文件与 `<data_dir>/initial-admin-password.txt`，等于把凭证摊在磁盘上。
- 邀请制注册：`yekitrim user invite --count 5` 生成邀请码并打开注册开关
  （`yekitrim user invites` 查看，`--disable` 关闭）。
- 到这一步仍建议套一层 HTTPS（自签或内网证书），否则口令与 cookie 是明文过网的。

### B-1. 局域网里「精修页很慢」怎么查（2026-11-21）

精修页一次进入会搬**十几到几十 MB**：CJK 字体（单支 5–20 MB）、贴纸动图、`data/danmaku.json`、
以及按原码率取的视频分段。本机 loopback 上感觉不出来，**Wi-Fi 上就是几秒到几十秒**。
服务端已经在做的三件事（都别拆）：

| 层 | 做什么 |
|---|---|
| 压缩 | 静态壳（JS/CSS）gzip + 进内存缓存；`/api/*` 的 `application/json` gzip；字体 `ttf/otf/ttc` gzip；wasm gzip |
| 缓存 | 文件出口一律 `ETag(len+mtime)` + `Cache-Control: no-cache` → **304**；`assets/*` 内容哈希 + `immutable` |
| 代理 | 预览优先读 `_proxy_720.mp4`（720p / GOP10）；出片永远用源；`process` 收尾会按需重建过期的那份 |
| 服务器时间 | `POST /polish/dhampir-project` 的转译结果在**进程内**缓存（键 = `polish.toml`/代理/两个字幕/转译器/字体库指纹，容量 2），改一个字节就失效 |

**用 DevTools 自查（三条，一分钟）**：

1. **冷启动**（Network → 勾 Disable cache → 硬刷新）：看 `Transferred` 合计与最慢的 5 个请求。
2. **热启动**（取消 Disable cache，普通 F5）：精修页那些请求应当全是 **`304`** 或 `(disk cache)`；
   只剩 `index.html`（1 KB）与视频的 Range 请求还有传输。若某个字体/贴纸还是 **200 且几 MB**，
   说明它走了一条没带校验符的出口 —— 那是 bug，不是网络问题。
3. **一次编辑**（时间轴上拖一个块松手）：看请求条数、`polish/process` 与 `polish/dhampir-project`
   各自的墙钟耗时。后者每次都重跑 Node 转译器（几百 ms ~ 2 s），**这是服务器时间，不是带宽**。

### B-2. 硬件编码（可选，默认关）

本仓自己那条 ffmpeg 回路（**屏蔽区间重编码**、剪切）默认用 **CPU `libx264 slow/crf17`**。
想换成核显/独显可以设环境变量（进程级，重启生效）：

| 值 | 行为 |
|---|---|
| 不设 / `auto` | 先探 NVENC（**真编 3 帧验证**），不行回落 `libx264` |
| `cpu` | 强制 `libx264`（跨机器结果一致，画质最好） |
| `vaapi` | 显式用 **VAAPI**（AMD/Intel 核显，走 `/dev/dri/renderD128`，可用 `VTEDIT_VAAPI_DEVICE` 换设备）；探不到自动回落 |

```bash
# /etc/systemd/system/yekitrim.service 的 [Service] 里（用户级服务则 ~/.config/systemd/user/）
Environment=VTEDIT_ENCODER=vaapi
systemctl --user daemon-reload && systemctl --user restart yekitrim
```

⚠️ **默认不开**的理由：VAAPI 只能按 `-qp` 定质量（不像 `crf` 按内容自适应），这是拿一点画质换速度；
实测（Ryzen 7 7735HS / Radeon 680M，1080p60 真实素材 10s）：`libx264 slow/crf17` 8.50s / SSIM 0.9965，
`h264_vaapi -qp19` **3.94s** / SSIM 0.9942 ⇒ **快 2.2×**、画质低 0.0023。
⚠️ 这条只加速**我们自己的 ffmpeg 回路**；出片的主体是底座 Dhampir（它自己编码），不受此开关影响。
⚠️ 放进容器要加 `--device /dev/dri`，否则探不到设备（会静默回落 `libx264`，日志一条 warn）。

> 视频那条腿：DOM 预览默认读 `_proxy_720.mp4`（有就优先）。想看原码率就设 `VTEDIT_NO_PROXY=1`
> 重启服务 —— 预览回落源、拖时间轴更卡，但画质是原始分辨率。

## C. 公网服务（含邀请制 SaaS）

### 1. 反代 + HTTPS（必做）

服务**自身没有 TLS**，只能裸 HTTP。请务必前置 Caddy/nginx 并终止 TLS。

Caddy 最小配置：

```
yekitrim.example.com {
    reverse_proxy 127.0.0.1:26320
}
```

#### 只放行一个端口时（单端口方案，2026-11-14 实践）

有些机器（内网防火墙只开 26320、或云安全组只映射了这一个端口）开不了第二个 https 端口。
这时**把 TLS 搬到 26320 本身**，应用退到 loopback 的另一个端口：

```
浏览器 ──https://<host>:26320──▶ Caddy(TLS) ──▶ 127.0.0.1:26321  (yekitrim)
```

- 应用侧：`--host 127.0.0.1 --port 26321 --allow-remote --trust-proxy`
  （`listening` 会变成 `loopback`；`--trust-proxy` 必须有，否则 cookie 不带 `Secure`）
- Caddy 侧：`https://<host>:26320 { tls internal \n reverse_proxy 127.0.0.1:26321 { flush_interval -1 } }`
- 代价：旧的 `http://<host>:26320` 会回 **400**（把 HTTP 发到 TLS 端口），书签要改成 https；
  自签证书首次要手动信任一次（或导入 Caddy 的 `pki/authorities/local/root.crt`）。
- 为什么值得：WebGPU（`navigator.gpu`）**只在安全上下文**（https / localhost）里存在，
  `http://<内网IP>:26320` 里它根本不存在 —— 浏览器端实时预览会静默退回近似预览。

nginx 要点：`proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;`
与 `proxy_set_header X-Forwarded-Proto $scheme;`，并且**不要**做响应缓冲以外的改写
（SSE 端点 `/api/stream`、`/api/admin/logs/events`、`/api/jobs/:id/events` 需要
`proxy_buffering off;`）。

### 2. 启动参数

```bash
export YEKITRIM_DATA_DIR=/var/lib/yekitrim      # ⚠️ 必须与下面 --data-dir 同值（见「为什么」）
yekitrim user passwd admin --password-file /root/yekitrim-admin.pw   # 强口令，别用初始密码

yekitrim server --host 127.0.0.1 --allow-remote \
             --trust-proxy \
             --data-dir /var/lib/yekitrim
```

> ⚠️ **为什么改密要带 `YEKITRIM_DATA_DIR`**：`--data-dir` 只作用于 `yekitrim server` 那个进程
> （它把值 `set_var` 进自己的环境）；`yekitrim user passwd` 是**另一个进程**，没有 `--data-dir` 参数，
> 只认这个环境变量。不带它 = 改的是 exe 旁边那份账本，服务器上这份仍是初始密码 ——
> `yekitrim user list` 一切正常、重启却被护栏拒绝。
> 全新机器可以**省掉这一步**：启动时直接 `--admin-password-file /root/yekitrim-admin.pw`
> （账本为空时生效，且视为已改密，见 `ensure_admin_bootstrap`）。

| 参数 | 为什么 |
|---|---|
| `--host 127.0.0.1` | 让服务只被本机反代访问，公网入口只有反代一个。多一层隔离。 |
| `--allow-remote` | 不加就不许监听非 loopback（即使 host 写的是 127.0.0.1 也建议显式加上，避免后续误改 host 时被拒启动搞混）。⚠️ 它同时**打开初始密码护栏**：显式加了它就视为对外暴露，未改初始密码会被拒绝启动。 |
| `--trust-proxy` | **反代后面必须加**。不加的话所有请求都被当成来自反代本机：按 IP 的限流退化成全局一份额度，`Secure` cookie 也不会下发。 |
| `--data-dir` | 账号账本、日志、模型的落点。默认在 `<exe_dir>/.yekitrim/`，容器里通常是只读层 → 会启动失败。 |

> `--trust-proxy` 打开后，客户 IP 取 `X-Forwarded-For` 的**最后一段**（自己那台反代追加的
> 真实对端）。别让反代把客户端自带的 `X-Forwarded-For` 原样透传在最外层之外 ——
> Caddy/nginx 的默认配置都会正确追加。

### 3. 数据目录

```
<data_dir>/
├── yekitrim.db                  # 账号 / 会话 / 审计 / 项目登记 / 邀请码（明文）/ LLM 用量
├── config.toml, llm.toml     # 全局配置（llm.toml 含 API key）
├── logs/                     # 运行日志
└── models/                   # ASR 模型（约 1.1GB）
```

- **这个目录不要公开、不要打进镜像层/分享包**。里面的 `yekitrim.db` 含**明文邀请码**与所有项目的
  真实绝对路径；`llm.toml` 含 LLM API key。
- 备份就是拷这个目录（加上各工程目录里的 `yekitrim.toml` 与 `output/`）。
- 容器部署把它挂成 volume；镜像里那份空的 `.yekitrim/` 会在启动时被写入，别用只读层。

### 4. 上线前检查清单

- [ ] HTTPS 反代生效，浏览器地址栏是 https
- [ ] `--allow-remote` 已加，且**初始密码已改**（未改会被拒绝启动；拒绝时的那段日志里直接印着完整改密命令，含 `YEKITRIM_DATA_DIR`）
- [ ] `--trust-proxy` 已加（反代后面）
- [ ] `yekitrim doctor --url https://你的域名` 输出里 `listening=remote`、`tls_terminated=true`
- [ ] 注册开关的状态是你想要的（`yekitrim user invites`；不需要自助注册就用 `--disable`）
- [ ] `data_dir` 不在公开可读的位置，也没有进镜像层
- [ ] 知道限流与配额的存在：超出时是**拒绝**（HTTP 429 / 任务记失败），不是排队
      —— 见 [`job-queue.md`](job-queue.md) §3.5 与 `yekitrim-server/src/ratelimit.rs`

### 5. 已知边界（按此设计使用）

- **没有配额计费**：`users.quota_bytes` 只在入库（录播姬自动管线 + B站下载）那条路上做水位预检；
  手动发起的转写/渲染/剪切没有按账号的存储配额。公开服务请自己盯着磁盘。
- **限流是按 IP 的**（加上任务侧的按账号配额）。同一出口 IP 后面的多个用户共享一份额度；
  额度设得较宽（见 `ratelimit.rs` 常量），正常使用碰不到。
- **`POST /api/open-path` 只在本地免登录模式下存在**（精修页产物卡的「打开输出目录」）。
  本地模式（`yekitrim-tray` 双击起的那份：绑 loopback + `--local-no-auth`，`/api/health`
  回 `local_no_auth=true`）点一下，由**与本机同进程的服务端**去弹系统文件管理器；按钮也**只在那时渲染**。
  服务端模式（`--allow-remote` / 反代）下 `local_admin` 是 `None` ⇒ 这个端点**回 404「未知接口」**，
  前端**不渲染那颗按钮** —— 否则远端的人点一下、**服务器上**就弹出一个资源管理器窗口，
  那是荒唐且有人身风险的能力。实现：`api::open_path`（先 `canonicalize` 再 `is_dir()`，
  `explorer` / `open` / `xdg-open` 各拿一个参数，**不走 shell**）；
  判据：`routes::tests::open_path_is_local_only_and_never_spawns_for_bad_paths`
  （成功那条刻意不测 —— 它会真弹窗口）。
- **日志含项目路径**：`/api/admin/logs` 与 `/api/admin/usage` 因此**仅 admin**。
  给出去的账号一律用普通角色（`yekitrim user add <name> --role user`）。
- 单实例：同一个 `data_dir` 只能跑一个进程（`yekitrim.lock`）。要多副本请各自独立 `data_dir`
  与端口，再在上层做路由 —— 但工程目录也会因此被两个实例同时写，不建议。

---

## 发行给别人（自部署）

1. 用 `cargo xtask package --smoke-test` 打包：会在打包后**清掉**
   解包目录里的 `.yekitrim/` 与 `stickers/`（跑过冒烟测试会生成，含真实绝对路径）。
2. 检查 `release/<包名>/` 里没有 `.yekitrim/` 再分发。`.zip` 本身不含它。
3. 告诉用户需要自备 `ffmpeg` / `ffprobe`（在 `PATH` 中）与 Chrome/Edge（精修出片要用）。
4. 许可：发行二进制是 **Apache-2.0**（宽松许可，**允许闭源再分发**）。打包时已**强制**随包
   附上 `LICENSE` 与 `THIRD-PARTY-NOTICES.md`，分发前确认二者在包内即可 —— 见仓库根
   [`LICENSE`](../LICENSE) 与 [`THIRD-PARTY-NOTICES.md`](../THIRD-PARTY-NOTICES.md)。

### 升级：**换掉 exe 就够了**（2026-10-03 起）

> 要往 Linux 上移植的话，先读 **[`docs/linux-port-handover.md`](linux-port-handover.md)** ——
> 里面把"哪些已经平台无关、哪些是阻塞项（ASR vendor / 托盘 / 数据目录 / 打包）"逐条列了出处。

包里现在只有这么几样：`yekitrim.exe`、`yekitrim-tray.exe`、`yekitrim.ico`、`LICENSE`、
`THIRD-PARTY-NOTICES.md`、`resources/fonts/`（**空目录 + 说明**，字体已编在 exe 里）。
**WebUI 也编在 exe 里**（`crates/yekitrim-server/build.rs` 把 `webui/dist` 用 `include_bytes!` 嵌进去），
所以升级操作是：

> **覆盖式解压**到原目录（把新的 `yekitrim.exe` / `yekitrim-tray.exe` 覆盖旧的），
> 然后重启。**不要**先删整个文件夹。

⚠️ **别删文件夹再解压**：`data_dir()` = `YEKITRIM_DATA_DIR` ?? **exe 所在目录**，
所以 `.yekitrim/`（配置、账号库、**下载好的 ASR 模型**、日志、工程账本）就在 exe 旁边 ——
删文件夹等于把配置、账号和几个 GB 的模型一起清掉。包里本来就没有 `.yekitrim/`，
**覆盖式解压不会碰到它**。

为什么"只换 exe"现在成立（以前**不**成立）：

| | 以前 | 现在 |
|---|---|---|
| 前端 | 运行时读盘，包里带 `webui/dist/`（30 个文件 1.2 MB） | **编进 exe** |
| 只换 exe 的后果 | **静默**端出旧界面（查找链会命中 exe 旁边那份旧 dist，看不出异常） | 前端跟着 exe 一起换，**不存在错配** |
| dist 过期 | 只在 `cargo xtask package` 时告警 | **编译期直接失败**（`dist` 缺失或比 `webui/src` 旧） |

⚠️ 两个例外仍要单独准备：
- **`resources/fonts/`** 里的说明文件（空目录也在包里）；字体不用管，已编进 exe。
- **程序目录以外的东西**：`ffmpeg` / `ffprobe`（`PATH` 里）与出片底座（Dhampir）按各自的方式升级 ——
  它们不在本仓的发行物里。

排查用的开关：`yekitrim server --webui-dir <目录>`（或环境变量 `YEKITRIM_WEBUI_DIR`）可以**临时**
让服务端改读磁盘上的一份界面（生产热修 / 排障）。启动日志会写一行 `静态资源：内嵌（N 个文件）`
或 `静态资源：磁盘 <目录>`，据此判断当前吃的是哪一份。
