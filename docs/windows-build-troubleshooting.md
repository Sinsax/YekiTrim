# Windows 构建排障：cl 报错被吞 + 工作区完整性标签

2026-10-05 实测踩过的两个坑，都会表现成**同一句毫无信息量的报错**：

```
warning: bzip2-sys@0.1.13+1.0.8: ToolExecError: command did not execute successfully
         (status code exit code: 2): "cl.exe" "-nologo" ... "-c" "bzip2-1.0.8/bzlib.c"
error: failed to run custom build command for `bzip2-sys v0.1.13+1.0.8`
```

`bzip2-sys` / `ring` / `libwebp-sys` / `libsqlite3-sys` / sherpa 预编译归档，任何一个 C 编译单元
失败都长这样。**先别怀疑代码**，按下面两步走。

## 坑 1：cl 把诊断写 stdout，cc-rs 只回显命令行

MSVC 的 `cl.exe` **把诊断写到 stdout**（实测：清空 `INCLUDE` 后编译，`exit 2`、**stderr 为空**、
stdout 里才是 `fatal error C1083: 无法打开包括文件: "stdlib.h"`）。而 cc-rs 1.2.65 报告失败时
（`cc-1.2.65/src/command_helpers.rs:276` 的 `Error::CommandExec`）**只 format 命令行**，捕获到的
stdout 被丢掉。于是**真正的原因永远到不了终端**。

要看见它，用一个「假 cl」包一层（文件名必须带 `cl`，例如 `cl.exe`，否则 cc-rs 会按文件名把它
判成 GNU 编译器、喂一堆 `-O0 -ffunction-sections`）：

1. 写个极小的转发程序：`cl.exe <args…>` → 起真正的 `cl.exe`、把它的 **stdout/stderr 原样转发**、
   退出码照抄，同时把 stdout 的**原始字节**追加到一个文件里（本机控制台是 GBK，用 UTF-8
   lossy 转一遍会把中文原因烧成乱码）。
2. `set CC=<那个假 cl 的绝对路径>` 再 `cargo build`。
3. ⚠️ 只在 `%TEMP%` 之类**可写**的位置记日志：见坑 2，受限上下文里可能只有 `target\` 可写。
   日志记不上就静默丢数据（本项目第一版探针就这么白跑了两轮）。

## 坑 2：工作区带 **Low 完整性标签** ⇒ `D8050 cannot execute c1.dll`

`cl: 命令行 error D8050 :无法执行"...\bin\HostX64\x64\c1.dll"` 的真身是**完整性级别**问题：

- Windows 建进程时取「**父进程令牌**」与「**可执行文件标签**」里**较低**的一级（实测：同一个
  Medium shell，跑 `target\labeltest.exe`（文件标签 Low）出来的进程是 `Low S-1-16-4096`，
  跑 `.workbuddy\...\cl.exe`（文件标签 Medium）出来是 `Medium S-1-16-8192`）。
- 本项目一旦 `F:\...\YekiTrim`（仓库根）带 `Mandatory Label\Low Mandatory Level:(OI)(CI)(NW)`，
  cargo 生成的 `target\debug\build\<pkg>\build-script-build.exe` **全是 Low 标签** ⇒ build script
  跑在 Low ⇒ 它拉起的 `cl.exe` 也是 Low ⇒ Low 进程**不能执行** Medium 标签的 `c1.dll`
  （no-read-up）⇒ 每个 `.c` 都 D8050，而坑 1 把这句话藏了。
- ⚠️ 这类标签是**文件系统状态，不是代码问题**：`git status` 永远干净，换个终端同样失败，
  只有量一下进程令牌才看得见。来源通常是文件沙箱/受限模式在工作区留下的残留。

### 修复（不需要管理员权限，自己拥有的目录就能改标签）

```bat
:: 1) 根目录标签改回 Medium（(OI)(CI) 让新建的文件/目录都跟着是 Medium）
icacls F:\para\Code\YekiTrim /setintegritylevel (OI)(CI)M

:: 2) 旧的 target 必须「改名后删」，不能就地重标：
::    那棵树里带着 Everyone:(DENY)(S,DC)，icacls 拿不到句柄
ren F:\para\Code\YekiTrim\target target-poisoned
rmdir /s /q F:\para\Code\YekiTrim\target-poisoned

:: 3) 重建（此时的 build script 才是 Medium）
cargo build --bin yekitrim
```

验证（必须显式量，别只看构建成功）：

```bat
icacls F:\para\Code\YekiTrim            :: Mandatory Label\Medium Mandatory Level
icacls F:\para\Code\YekiTrim\target\debug\build\bzip2-sys-*\build-script-build.exe
```

## 别照抄的两句错话（本次走过的弯路）

- ❌「是 cargo 在给 build script 降权」：`cargo.exe` 里搜不到任何沙箱/低完整性/`CreateRestrictedToken`
  字符串，o 换成工具链里的真 cargo 全路径也一样。**降级来自可执行文件的标签**。
- ❌「是 harness / 沙箱改了进程令牌」：在证据只到「cargo 的子进程是 Low、我直接起的进程是 Medium」
  时这么归因是**没验完**。做一次「同父进程、只换文件标签」的对拍就能立刻分清真凶。
