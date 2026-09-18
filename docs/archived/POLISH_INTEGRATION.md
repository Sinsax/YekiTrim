# V-Trim Polish Integration Plan

> vtedit → vtrim `polish` 子命令合并方案

## 目标

把 `vtedit/.main/`（独立 GSAP 精修引擎）合并进 V-Trim 单一二进制，作为 `vtrim polish` 子命令。
vtedit 不再作为独立程序存在。

## 最终目录结构

```
<工作区>\
├── vtedit\.main\                保留（历史参考，不再构建）
│
└── V-Trim\
    ├── Cargo.toml               workspace members + "crates/vtrim-polish"
    └── crates\
        ├── vtrim-core\          不变
        ├── vtrim-cli\
        │   ├── Cargo.toml       加 vtrim-polish dep
        │   └── src\
        │       ├── cli.rs       加 Polish 子命令
        │       └── main.rs      加 Polish 路由
        └── vtrim-polish\        从 vtedit 复制并 lib-化
            ├── Cargo.toml       [lib] target, 删 [[bin]]
            ├── src\
            │   ├── lib.rs       pub mod 声明
            │   ├── commands.rs  main.rs 的 cmd_* 全部 pub
            │   ├── project\
            │   ├── parse\
            │   ├── llm\
            │   ├── render\
            │   └── assets\
            └── templates\       (index.html + cover.html 原样)
```

## CLI 设计

| 命令 | 行为 |
|------|------|
| `vtrim polish init` | 初始化项目（创建 config.toml + 解析字幕弹幕） |
| `vtrim polish llm` | LLM 分析生成 segments 和关键词（使用 .vtrim/llm.toml） |
| `vtrim polish process` | 后处理（解析 emoji/irasu keyword、下载资源、计算位置） |
| `vtrim polish render` | 渲染视频（自动先跑 process） |
| `vtrim polish cover` | 生成封面图 |
| `vtrim polish cover --template X` | 设置封面模板预设 + 生成封面 |
| `vtrim polish preview` | 浏览器预览双端口（video + cover） |
| `vtrim polish preview video` | 只预览视频 |
| `vtrim polish preview cover` | 只预览封面 |
| `vtrim polish doctor` | 系统检查 |
| `vtrim polish test` | 测试模式 |
| `vtrim polish init output/clips/clip-01` | 对指定 clip 目录操作 |

所有子命令接受可选的位置参数 `[INPUT]`（默认当前目录）。

## 与 vtrim cut 的对接

```
vtrim cut                          vtrim polish -i output/clips/clip-01
   │                                          │
   ▼                                          ▼
output/clips/clip-01/              output/clips/clip-01/
├── clip.mp4         ◄── 输入 ───  ├── clip.mp4 (原样)
├── clip.srt                      ├── clip.srt
├── clip.dm.srt                   ├── clip.dm.srt
└── clip.json                     ├── clip.json
                                  ├── config.toml      (vtedit 风格)
                                  ├── data/            (parse 产物)
                                  ├── assets/emoji/    (download 产物)
                                  ├── assets/irasu/
                                  ├── sfx/             (SFX 文件)
                                  ├── polished.mp4     (render 输出)
                                  └── cover.jpg        (cover 输出)
```

## 关键决策

1. **独立 crate**：vtrim-polish 不放入 vtrim-core，避免 headless_chrome/axum/image 等无关重依赖污染 core
2. **完整搬运**：vtedit/.main 源码整体复制进 vtrim-polish，零业务逻辑修改
3. **commands.rs**：原 main.rs 的所有 `cmd_*` 标记 `pub` 后搬入
4. **删除 main.rs + cli.rs**：vtedit 不再作为独立二进制
5. **tracing 初始化由 vtrim-cli 负责**：vtrim-polish 不重复初始化
6. **clip.json 复用**：vtrim cut 已写齐 duration/title/event_type/popularity/rank，polish 直接读取

## 执行步骤

1. 复制 vtedit/.main/src/* 到 V-Trim/crates/vtrim-polish/src/
2. 复制 vtedit/.main/templates/ 到 V-Trim/crates/vtrim-polish/templates/
3. 改 Cargo.toml: name → vtrim-polish，加 [lib]，删 [[bin]]
4. 创 src/lib.rs: `pub mod commands; pub mod project; pub mod parse; pub mod llm; pub mod render; pub mod assets;`
5. 把 main.rs 搬成 src/commands.rs:
   - 删 `mod cli;` 和 `use cli::*;`
   - 把 `mod project;` 等改成 `use crate::project;` 等
   - 所有 `cmd_*` 函数加 `pub`
   - 删 `#[tokio::main]` 和 tracing 初始化
6. 删 main.rs 和 cli.rs
7. 根 Cargo.toml 加 `"crates/vtrim-polish"` 到 members
8. vtrim-cli/Cargo.toml 加 `vtrim-polish = { path = "../vtrim-polish" }`
9. vtrim-cli/src/cli.rs 加 `Polish` 枚举变体
10. vtrim-cli/src/main.rs 加 `Command::Polish` 路由
11. `cargo build` 验证
12. 修复编译错误

## 预期耗时

- 复制 + 搬 main.rs：30 分钟
- 改 Cargo.toml + lib.rs：10 分钟
- 接 vtrim-cli：30 分钟
- 编译 + 调试：1-2 小时（vtedit 完整编译约 5-7 分钟，含 headless_chrome）

## 风险

- 编译时间首次较长（headless_chrome 是大依赖）
- vtedit main.rs 内部 `crate::` 引用在 lib-化后路径可能错
- vtedit 与 vtrim-core 的 tracing/serde/tokio 版本可能不一致（workspace 统一后应自动解决）

## 不在本计划范围

- polish.toml 配置生成（vtrim cut 输出 → polish.toml 的自动转换）
- 批量 polish 多个 clip 的并发逻辑
- polish 与 vtrim 状态机集成（polish 是独立子命令，不进 stage machine）
- 端到端测试（vtrim process → vtrim polish 完整流程）
