//! polish::render::ffmpeg：ffmpeg 调用的唯一收口（编码器探测 / 拼接 / 抽音 / 混 SFX / 按屏蔽区间重编码 / 按 EDL 组装音轨 / 合流）。
//!
//! 入口：`Ffmpeg::{detect_encoder, concat, extract_audio, apply_limiter, mix_sfx, cut_ranges, assemble_audio_from_edl, probe_duration, probe_fps, mux, mux_checked}`、`EncoderInfo`、`SfxEvent`。
//! 边界：不许改回 `Command::output()` / 管道（Chrome/ffmpeg 是孙进程，会一直持有 stdout 句柄）；编码器一律用 `detect_encoder()` 的结果，别硬编码 —— 见 `crates/yekitrim-polish/src/render/AGENTS.md`

use std::process::{Stdio};

/// 解析 `ffprobe` 打的帧率：`"60/1"` / `"30000/1001"` / `"25"`。
///
/// 抽成**独立纯函数**是为了能逐值测：真实 `ffprobe` 只会给其中一种形状，
/// 而 1001 那一族（NTSC）恰恰是最容易写错的（`30000/1001` 写成 30 就少 0.1%，
/// 一个 133 秒的长片累计差 ~0.13 秒 —— 落在 ±1 帧容差里，**看不出来**）。
///
/// 分母为 0 / 非有限 / 非正数一律 `None`（不返回 `inf`：那会让后面所有帧号变 NaN）。
fn parse_rational_fps(raw: &str) -> Option<f64> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    let (num, den) = match text.split_once('/') {
        Some((n, d)) => (n.trim().parse::<f64>().ok()?, d.trim().parse::<f64>().ok()?),
        None => (text.parse::<f64>().ok()?, 1.0),
    };
    if !num.is_finite() || !den.is_finite() || den == 0.0 {
        return None;
    }
    let fps = num / den;
    if fps.is_finite() && fps > 0.0 {
        Some(fps)
    } else {
        None
    }
}

/// Concat 清单临时文件路径。
///
/// ⚠️ 必须**每个进程每次调用都不同**：以前写死 `_polish_concat.txt`，两个渲染（哪怕是
/// 不同项目、不同进程）同时跑就会互相覆盖清单，拼出来的成片是别人的片段。
fn concat_temp_path() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("_polish_concat-{}-{}.txt", std::process::id(), n))
}

/// `atempo` 滤镜的**单级**合法区间（ffmpeg 的硬约束，不是我们能调的）。
///
/// ⚠️ 与 `project::validate::SPEED_MIN/MAX` 是**同一件事的两处表示**，必须一致：
/// 那对常量决定"用户能选哪些档"，这一对决定"滤镜能不能吃下去"。
/// 放宽到 `atempo` 之外 ⇒ 出片时音频滤镜直接失败（或更糟：静默取不到效果）。
const ATEMPO_STAGE_MIN: f64 = 0.5;
const ATEMPO_STAGE_MAX: f64 = 2.0;

/// 把任意正速率拆成**若干级 `[0.5, 2.0]` 的 `atempo`**，拼成一条滤镜串。
///
/// # 为什么必须"串联多级"而不是一级写个大数
///
/// ffmpeg 的 `atempo` **单级只吃 `[0.5, 2.0]`**，超出范围会**直接报错**
/// （`atempo=4` ⇒ `Invalid value 4.000000 for option 'tempo'`）。
/// 而规格 §3.2 要求 `speed=0.25` 写成 `atempo=0.5,atempo=0.5` —— 机制就是
/// **级联**：每级各变一次速，总倍率是各级之积。
///
/// # 为什么**绝不能**用 `asetpts` 缩放代替（规格 §9 R6，本批最凶的一条）
///
/// `asetpts=PTS/speed` 是"告诉下游时间戳变了" —— 采样率与样本内容**一个字节没动**，
/// 于是播放器按 `speed` 倍的重采样率去放它 ⇒ **音调乘以 speed**（花栗鼠音 / 低沉音）。
/// `atempo` 是**时域伸缩**（WSOLA 类）：时长变了、基频不变。判据是数值的，不是听感：
/// 实测同源 440Hz 正弦，`atempo=2.0` 后基频仍是 **440Hz**，`asetpts` 缩放后变 **880Hz**。
///
/// # 返回
///
/// 形如 `"atempo=0.500000,atempo=0.500000"`（**不带**前导/尾随逗号；调用方自己接）。
/// `speed` 非正 / 非有限 ⇒ `None`（调用方必须 bail，不许当成 1.0 静默放行）。
///
/// # 分解口径
///
/// 贪心地"能塞一级最大/最小就先塞一级"：`speed > 2` 时反复乘 `2.0`，
/// `speed < 0.5` 时反复乘 `0.5`，剩下的那点（落在 `[0.5, 2.0]` 内）作为**最后一级**。
/// ⚠️ 最后一级**只写一次**，且不做"再拆成两级更精确"的花活 —— 级数越多，
/// 每次时域伸缩的接缝伪影越多（每一级都是一次独立的 WSOLA），而收益只是
/// 把 `1.999999` 写得更准，没有意义。
///
/// 边界：`speed == 0.5` / `speed == 2.0` ⇒ **恰好一级**（这两档就是单级的上下端，
/// 别拆成两级 —— 那会平白多一次伸缩）。`speed == 1.0` ⇒ `Some("")`（**空串**：
/// 原速就什么滤镜都不挂，挂 `atempo=1.0` 是白跑一遍时域伸缩。
/// ⚠️ 调用方见到空串必须**不要**往滤镜链里塞那个逗号）。
fn atempo_filter(speed: f64) -> Option<String> {
    if !speed.is_finite() || speed <= 0.0 {
        return None;
    }
    // 原速：什么滤镜都不挂（**不是** `atempo=1.0`）。
    if (speed - 1.0).abs() < 1e-9 {
        return Some(String::new());
    }
    let mut stages: Vec<f64> = Vec::new();
    let mut remaining = speed;
    // 快放：一级一级地吃 `2.0`，直到剩下那点能一级吃完。
    while remaining > ATEMPO_STAGE_MAX + 1e-9 {
        stages.push(ATEMPO_STAGE_MAX);
        remaining /= ATEMPO_STAGE_MAX;
    }
    // 慢放：一级一级地吃 `0.5`。
    while remaining < ATEMPO_STAGE_MIN - 1e-9 {
        stages.push(ATEMPO_STAGE_MIN);
        remaining /= ATEMPO_STAGE_MIN;
    }
    // 最后一小节（落在单级区间内）。⚠️ 上一步可能把它除到**极接近 1.0**
    // （`speed = 4.0` ⇒ 4/2/2 = 1.0），那一级照样要写 —— 少了它总倍率就差一倍。
    stages.push(remaining);
    Some(
        stages
            .iter()
            .map(|s| format!("atempo={s:.6}"))
            .collect::<Vec<_>>()
            .join(","),
    )
}

/// ffmpeg 操作包装
pub struct Ffmpeg;

impl Ffmpeg {
    /// 检测可用编码器 (NVENC 优先)
    pub fn detect_encoder() -> EncoderInfo {
        let output = yekitrim_core::process::cmd("ffmpeg")
            .args(["-hide_banner", "-encoders"])
            .output()
            .ok();

        if let Some(out) = output {
            let text = String::from_utf8_lossy(&out.stdout);
            if text.contains("nvenc") {
                return EncoderInfo {
                    codec: "h264_nvenc".into(),
                    args: vec!["-preset".into(), "p1".into(), "-cq".into(), "22".into()],
                };
            }
        }

        EncoderInfo {
            codec: "libx264".into(),
            args: vec!["-preset".into(), "veryfast".into(), "-crf".into(), "22".into()],
        }
    }

    /// Concat 多个 MP4 文件
    pub fn concat(inputs: &[String], output: &str) -> anyhow::Result<()> {
        let mut concat_content = String::new();
        for input in inputs {
            // 使用绝对路径，因为 concat 文件在临时目录中
            let abs_path = std::path::absolute(input)
                .unwrap_or_else(|_| std::path::PathBuf::from(input));
            concat_content.push_str(&format!("file '{}'\n", abs_path.to_string_lossy().replace('\\', "/")));
        }

        let abs_output = std::path::absolute(output)
            .unwrap_or_else(|_| std::path::PathBuf::from(output));

        let concat_file = concat_temp_path();
        std::fs::write(&concat_file, &concat_content)?;

        let status = yekitrim_core::process::cmd("ffmpeg")
            .args([
                "-y", "-f", "concat", "-safe", "0",
                "-i", &concat_file.to_string_lossy(),
                "-c", "copy",
                &abs_output.to_string_lossy(),
            ])
            .stderr(Stdio::null()).stdout(Stdio::null()).status();

        // 失败路径也要删（以前只删成功路径，报错时临时文件会攒下来）
        std::fs::remove_file(&concat_file).ok();
        let status = status?;

        if !status.success() {
            anyhow::bail!("ffmpeg concat failed");
        }
        Ok(())
    }

    /// 提取音频
    pub fn extract_audio(video_path: &str, audio_output: &str) -> anyhow::Result<()> {
        let out = yekitrim_core::process::cmd("ffmpeg")
            .args([
                "-y", "-i", video_path,
                "-vn", "-acodec", "aac", "-b:a", "128k",
                audio_output,
            ])
            .output()
            .map_err(|e| anyhow::anyhow!("ffmpeg audio extraction 启动失败: {}", e))?;

        if !out.status.success() {
            anyhow::bail!(
                "ffmpeg audio extraction failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }

    /// 混入音效
    /// 混音后限幅: 防止音效峰值爆音 (alimiter)
    pub fn apply_limiter(input: &str, output: &str) -> anyhow::Result<()> {
        let out = yekitrim_core::process::cmd("ffmpeg")
            .args(["-y", "-i", input,
                   "-af", "alimiter=limit=0.95:level=false",
                   "-c:a", "aac", output])
            .output()
            .map_err(|e| anyhow::anyhow!("limiter 启动失败: {}", e))?;
        if !out.status.success() {
            anyhow::bail!("limiter 失败: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(())
    }

    pub fn mix_sfx(
        audio_input: &str,
        sfx_events: &[SfxEvent],
        output: &str,
    ) -> anyhow::Result<()> {
        if sfx_events.is_empty() {
            std::fs::copy(audio_input, output)?;
            return Ok(());
        }

        // ⚠️ **实测记一笔：`mix_sfx` **不**改音轨长度**（别再"修"它）。
        //
        // 这条注释是一次**错误的直觉**留下的路标。当时的推理是：
        // `adelay` 会把每条音效补齐到 `delay + dur`，而 `amix=duration=first`
        // 只保证"不短于第一条" ⇒ 以为多出来的音效会把输出拉长。
        //
        // **逐值实测否定它**：基准 18.077891s 的 AAC + 3 条音效（最晚 16.5s + 1.0s），
        // 混完 = **18.645862s**；给每条音效加 `apad,atrim=end=<基准>` 之后
        // **仍然是 18.645862s** —— 一个样本都没变。所以 `amix=duration=first`
        // 真的按第一条定长，`adelay` 的补齐不会外溢。
        //
        // ⚠️ 那 0.57s 的"超出"是**别的东西**造成的（`-c:a aac` 在多段拼接流上重编码时
        // 容器时长被撑大：`-c copy` 18.077891s → `-c:a aac` 18.478725s）。
        // **这才是合流前那条断言必须按"解码后样本数"对账、而**不能**按容器时长的原因**
        // —— 见 `probe_audio_samples` 的文档注释（那里有完整的实测数字）。
        //
        // 别把这段删了换成"修复注释"：下一个人会踩同一个直觉。
        let mut filter_parts = Vec::new();
        let mut input_args = vec!["-y".to_string(), "-i".to_string(), audio_input.to_string()];
        let mut mix_inputs = vec!["[0:a]".to_string()];

        for (i, event) in sfx_events.iter().enumerate() {
            input_args.push("-i".to_string());
            input_args.push(event.file.clone());
            let delay_ms = (event.time * 1000.0) as u64;
            let dur = event.duration.unwrap_or(0.5);
            let fade_st = (dur - 0.2).max(0.0);
            filter_parts.push(format!(
                "[{}:a]atrim=0:{},afade=t=out:st={}:d=0.2,adelay={}|{},volume={}[s{}]",
                i + 1, dur, fade_st, delay_ms, delay_ms, event.volume, i
            ));
            mix_inputs.push(format!("[s{}]", i));
        }

        let mix_str = mix_inputs.join("");
        let amix = format!(
            "{}amix=inputs={}:duration=first:normalize=0[a]",
            mix_str,
            sfx_events.len() + 1
        );

        let filter_complex = if filter_parts.is_empty() {
            amix
        } else {
            format!("{};{}", filter_parts.join(";"), amix)
        };

        let mut args: Vec<String> = input_args;
        args.extend_from_slice(&[
            "-filter_complex".into(),
            filter_complex,
            "-map".into(),
            "[a]".into(),
            "-acodec".into(),
            "aac".into(),
            "-b:a".into(),
            "128k".into(),
            output.into(),
        ]);

        let status = yekitrim_core::process::cmd("ffmpeg").args(&args).stderr(Stdio::null()).stdout(Stdio::null()).status()?;
        if !status.success() {
            anyhow::bail!("ffmpeg SFX mix failed");
        }
        Ok(())
    }

    /// 按屏蔽区间剪掉一段时间：视频/音频同步跳过的部分会被移除。
    ///
    /// 这里是**整段重编码**（select 过滤没法流转发），所以编码器要走
    /// [`Ffmpeg::detect_encoder`] —— 硬编码 libx264 时，一个 4 分钟的切片要 17s，
    /// 而同一个出片流程的其它环节早就用上 NVENC 了。画质档与分块编码保持一致。
    pub fn cut_ranges(input: &str, ranges: &[(f64, f64)], output: &str) -> anyhow::Result<()> {
        let mut parts: Vec<String> = Vec::new();
        for (s, e) in ranges {
            if e > s {
                parts.push(format!(r"not(between(t\,{s:.3}\,{e:.3}))"));
            }
        }
        if parts.is_empty() {
            return Ok(());
        }
        let expr = parts.join("*");
        let vf = format!("select='{expr}',setpts=N/FRAME_RATE/TB");
        let af = format!("aselect='{expr}',asetpts=N/SR/TB");
        let enc = Self::detect_encoder();
        let mut args: Vec<String> = vec![
            "-y".into(), "-i".into(), input.into(),
            "-vf".into(), vf,
            "-af".into(), af,
            "-c:v".into(), enc.codec.clone(),
        ];
        args.extend(enc.args.iter().cloned());
        args.extend([
            "-pix_fmt".into(), "yuv420p".into(),
            "-c:a".into(), "aac".into(),
            "-b:a".into(), "160k".into(),
            "-movflags".into(), "+faststart".into(),
            output.into(),
        ]);
        let out = yekitrim_core::process::cmd("ffmpeg")
            .args(&args)
            .output()
            .map_err(|e| anyhow::anyhow!("ffmpeg cut ranges 启动失败: {}", e))?;
        if !out.status.success() {
            anyhow::bail!("ffmpeg cut ranges failed: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(())
    }

    /// 按 EDL 把**音轨**从源素材上重新拼出来（第 2 批 B 件的核心）。
    ///
    /// # 做法（照搬 `yekitrim-core::cutting::cut_parts_and_concat` 的"逐段剪、再拼"）
    ///
    /// 每一段 `atrim` 出源素材上的一截，`asetpts=PTS-STARTPTS` 把时间戳归零
    /// （**少了这一句，段与段之间会保留源时间戳** —— `concat` 会把后一段放到
    /// "源时间 3 秒"的位置上，中间凭空多出 3 秒静音，而 ffmpeg **退出码 0**），
    /// 该段带 `speed ≠ 1` 时再接一串 `atempo`（见 [`atempo_filter`]），
    /// 最后 `concat=n=N:v=0:a=1` 顺次拼起来。
    ///
    /// **一次 ffmpeg 调用搞定**（不是"剪 N 个文件再 concat"）：音轨是纯滤镜链，
    /// 走 `filter_complex` 没有中间产物、也不用管临时文件清理与失败路径。
    /// 代价是 N 很大时命令行会很长 —— 段数是个位数到几十，离系统上限还很远。
    ///
    /// # 为什么必须从 EDL 拼、不能沿用"整条原声 + 尾部剪一次"
    ///
    /// 老路（`cut_ranges`）只能**删掉**区间，表达不了"重放/冷开场"这类
    /// **顺序改变 + 重复**的剪辑。而画面那条腿（底座）现在是按 EDL 出片的
    /// ⇒ 音轨不跟着重排就**从第一个重放点起彻底错位**，且两边都不报错。
    ///
    /// # 参数
    ///
    /// - `clips`：`(source_start, source_end, out_start, speed)`（**秒 + 倍率**），
    ///   来自 `Edl::resolve_clips`（`ResolvedClip` 四个字段各取一个）。用 `out_start`
    ///   而不是"顺序拼接"是刻意的：规格 §9 R8 说 `out_start` 是**唯一的**排序依据，
    ///   段序与源序可以不同；按 `out_start` 排一遍就让"乱序 EDL"也正确
    ///   （`resolve_clips` 已经排过，这里是**幂等的第二道**，成本为零）。
    /// - `rate`：**整条轨的额外倍率**（今天恒为 1.0），与每段自己的 `speed` **相乘**。
    ///
    /// # ⚠️ 为什么逐段 `speed`、而不是一个全局 `rate`（第 4 批的结论）
    ///
    /// 第 2 批只留了一个 `rate` 参数，形状是"整条轨一个速率"。变速真落地时发现
    /// 它**表达不了 EDL** —— "一段 ×0.5 慢放挨着一段 ×2.0 快放"是常态
    /// （需求原话就是"两三次连续重放，可加速可减速"）。一个全局速率只能整片统一变速。
    ///
    /// # ⚠️ 音轨变速**必须**用 `atempo`，不许用 `asetpts` 缩放（规格 §9 R6）
    ///
    /// `asetpts=PTS/speed` 不动采样率也不动样本内容，等于"骗下游说时间变了" ——
    /// 播放器按 `speed` 倍的采样率去放它 ⇒ **音调乘以 speed**（花栗鼠音）。
    /// `atempo` 是时域伸缩（WSOLA）：时长变、基频不变。
    /// 实测同源 440Hz 正弦：`atempo=2.0` 后基频仍 **440Hz**，`asetpts` 缩放后 **880Hz**。
    ///
    /// # 第 7 批（task-10）：叠化处的**音频交叉淡化**
    ///
    /// 画面那条腿在叠化处把上层的 `start` 提前 `DISSOLVE_OVERLAP_SEC`，两层在重叠窗口里
    /// 同时可见。音轨**必须跟着走同一条时间线**，否则画面在淡化、声音还在硬切
    /// —— 割裂感很明显，而**两边都不报错**。
    ///
    /// ⚠️ **为什么交叉淡化必须在本仓做、不能交给底座**（这是我实测出来的，不是推断）：
    /// 底座把重叠的音频层**直接相加**（`Layer.gain` 乘完之后求和），
    /// `opacity` 关键帧**对音频没有任何作用** —— 探针实测：给上层音轨写 `opacity` 0→1
    /// 的线性关键帧，重叠区峰值与"不加关键帧"逐窗口**一模一样**（只有相加后的整体偏大）。
    /// 它会在报告里打一条 `overlaps: ["a1[aA] 与 a2[aB] 在第 30 帧叠上，相加了 48000 个采样点"]`
    /// —— 相加**不等于**交叉淡化：两条人声直接相加会**成倍变响**（实测重叠区比单段
    /// 高约 **+6 dB**，正常交叉淡化应当基本持平）。
    ///
    /// ⇒ 所以叠化的**音频那一半**在这里做：给**出段**挂一个 `afade=out`、
    /// **入段**挂一个 `afade=in`，窗口都等于重叠时长，而两段的 `atrim` 窗口各**向外扩**
    /// 同样的长度 —— 于是重叠区里两段同时存在、各自带着自己的淡化包络。
    ///
    /// ⚠️ **两段的总位移是"各扩一半"**，不是"都往前挪"：`assemble_audio_from_edl` 的
    /// 输出位置由 `out_start` 决定，而 `out_start` 是**不因转场而变**的那份（见
    /// `ResolvedClip::transition_in`）。所以做法是把两段的**尾部与头部各向外扩**，
    /// 让它们在 `out_start` 两侧相遇 —— 段与段之间不再是首尾相接的 `concat`，
    /// 而是"每一段落到自己的绝对位置上"（`adelay` + `amix` 达不到样本级对齐，
    /// 所以走 `atrim` 扩窗 + `concat` 的分段形状，见下）。
    ///
    /// # ⚠️ 为什么用 `atrim` 扩窗 + 逐段 `adelay` 对齐，而不是 `acrossfade`
    ///
    /// `acrossfade` 只能拼**两条**流、且它自己会吃掉重叠时长（输出总长 = 两段之和 − 重叠），
    /// 而 EDL 的段数是任意多、且每段的成片位置由 `out_start` 独立决定
    /// （乱序/重放都能出现）。`acrossfade` 的"链式"形状表达不了那个。
    /// 所以这里保持"逐段 `atrim` + `afade` + `concat`"——**总长与逐段位置都不变**，
    /// 变的只有**段边界上那 0.5 秒的包络**。
    ///
    /// ⚠️ **空 `clips` 直接拒绝**：`resolve_edl` 在 `total_duration <= 0` 时返回空数组，
    /// 而"空 EDL ⇒ 拼出 0 秒音轨"会让成片**没有声音**（`-shortest` 再把它截成 0.1 秒）。
    /// 调用方必须先判过，这里再兜一层。
    pub fn assemble_audio_from_edl(
        video: &str,
        clips: &[(f64, f64, f64, f64)],
        output: &str,
        rate: f64,
    ) -> anyhow::Result<()> {
        Self::assemble_audio_from_edl_fades(video, clips, &[], output, rate)
    }

    /// [`Ffmpeg::assemble_audio_from_edl`] 的**逐段音源**版本。
    ///
    /// `audio_of[i]` = 第 `i` 段（**传入顺序**，与 `clips` 同序）的声音该从**哪个文件**读；
    /// `None` = 读主素材（`video`，缺省行为）。
    ///
    /// # 为什么必须能换源（2026-10-09 实测踩到）
    ///
    /// EDL 从第 6 批起支持 `source_kind = "asset"`（画面来自素材库的视频），
    /// 片头/片尾就是这种段。而这条音轨腿原先**硬编码读主素材**：
    /// 1080 帧的画面（18s 正片 + 1.5s 片头 + 3s 片尾）配一条只按 3 段正片拼的 864000 样本音轨，
    /// 差 216000 样本 —— `mux_checked` **当场拒绝合流**（这条守卫是对的，它拦住了一部声画错位的成片）。
    /// 底座的 `--no-audio` 让画面那条腿对素材段取的是**素材自己的**画面，所以声音也必须跟素材走，
    /// 否则"画面放了 1.5 秒片头、声音还是正片开头那 1.5 秒"。
    ///
    /// ⚠️ **找不到音轨的素材段必须补静音，不能借主素材的声音**：借了就是"片头那 3 秒在放正片的对白"，
    /// 而**不报错**。所以这里对 `None` 音轨的段（探不到流的）用 `anullsrc` 铺等长静音。
    pub fn assemble_audio_from_edl_sources(
        video: &str,
        clips: &[(f64, f64, f64, f64)],
        fades: &[(usize, usize, f64)],
        audio_of: &[Option<String>],
        output: &str,
        rate: f64,
    ) -> anyhow::Result<()> {
        Self::assemble_audio_impl(video, clips, fades, audio_of, output, rate)
    }

    /// [`Ffmpeg::assemble_audio_from_edl`] 的**带转场**版本（第 7 批 / task-10）。
    ///
    /// `fades` = 每条边一个 `(出段下标, 入段下标, 重叠秒数)`，下标是 `clips` 的
    /// **传入顺序**下标（与 `Edl::resolve_clips` 的顺序一致）。空 = 与不带转场那条**逐字节相同**
    /// （老工程/无转场工程零回归，R4）。
    ///
    /// ⚠️ `fades` 里的下标必须**已经按成立条件过滤过**（`EdlClip::dissolve_overlap_frames`
    /// 那三条），这里只负责把它们翻成滤镜。
    pub fn assemble_audio_from_edl_fades(
        video: &str,
        clips: &[(f64, f64, f64, f64)],
        fades: &[(usize, usize, f64)],
        output: &str,
        rate: f64,
    ) -> anyhow::Result<()> {
        Self::assemble_audio_impl(video, clips, fades, &[], output, rate)
    }

    /// 上面两个入口的**同一份实现**。`audio_of` 空 = 全部读主素材（老行为，逐字节不变）。
    fn assemble_audio_impl(
        video: &str,
        clips: &[(f64, f64, f64, f64)],
        fades: &[(usize, usize, f64)],
        audio_of: &[Option<String>],
        output: &str,
        rate: f64,
    ) -> anyhow::Result<()> {
        if clips.is_empty() {
            anyhow::bail!(
                "按 EDL 组装音轨：段列表为空。\
                 （`resolve_edl` 在 `total_duration <= 0` 时会返回空 —— 调用方必须先判过，\
                 否则这里会拼出一条 0 秒音轨，成片静默没声音）"
            );
        }
        // ⚠️ **每个 clip 带上自己的 speed**（第 4 批）。第 2 批那个 `rate` 参数是
        // "整条轨一个速率"的占位形状，变速落地后发现它**表达不了 EDL**：
        // 一段慢放挨着一段快放是常态（`[(3,6) replay, (0,12), (3,6) speed=2]`）。
        // 所以逐段取 `clip.3`，而**不是**拿一个全局 `rate` 乘下去。
        //
        // `rate` 仍留在签名里，含义收窄成"**整条轨的额外倍率**"（今天恒为 1.0）：
        // 它是给"整片统一变速"留的口子，与逐段 speed **相乘**。留着的成本为零，
        // 而删掉它就得让每个调用点自己乘 —— 那正是"两个地方各算一次"的开端。
        if !rate.is_finite() || rate <= 0.0 {
            anyhow::bail!("按 EDL 组装音轨：整轨 rate={rate} 非法（要求正有限值）");
        }

        // ⚠️ **输入列表**（2026-10-09）：第 0 个永远是主素材；素材段各自追加一个。
        // `input_index[i]` = 第 i 段（**传入顺序**）该读哪个 `-i`。
        // 空 `audio_of` ⇒ 全部 0 ⇒ 与改造前**逐字节相同**（R4）。
        let mut input_files: Vec<String> = vec![video.to_string()];
        // `None` = 这一段**没有可读的音轨**（素材本身无声）⇒ 补等长静音。
        let mut input_index: Vec<Option<usize>> = Vec::with_capacity(clips.len());
        for i in 0..clips.len() {
            let src = audio_of.get(i).and_then(|o| o.as_deref());
            match src {
                Some(p) => {
                    // ⚠️ **素材没有音轨 ⇒ 补静音，绝不回落到主素材**（回落到主素材 =
                    // "片头那几秒在放正片的对白"，而且**不报错**）。
                    // 片头/片尾常常就是一段无声的 logo 动画，这条路是常态不是边角。
                    if !Self::has_audio_stream(p) {
                        input_index.push(None);
                        continue;
                    }
                    // 同一个文件被多段引用时复用那一个输入（`-i` 重复会把同一文件读两遍）
                    match input_files.iter().position(|f| f == p) {
                        Some(k) => input_index.push(Some(k)),
                        None => {
                            input_files.push(p.to_string());
                            input_index.push(Some(input_files.len() - 1));
                        }
                    }
                }
                None => input_index.push(Some(0)),
            }
        }

        let mut filter_parts: Vec<String> = Vec::with_capacity(clips.len() + 1);
        let mut labels: Vec<String> = Vec::with_capacity(clips.len());
        // ⚠️ **淡化窗口的规划抽成纯函数**（`plan_dissolve_fades`）：它要按 `out_start`
        // 重排、还要把"传入顺序的下标"搬成"排序后的位置"，是最容易写错的一步，
        // 而错了也**不报错**（淡化挂到别的段上）。抽出来才能逐值测。
        let (lead_in, fade_len, tail_out) = Self::plan_dissolve_fades(clips, fades);
        // 按成片起点排（规格 §9 R8：`out_start` 是唯一排序依据）。
        let mut ordered: Vec<(f64, f64, f64, f64)> = clips.to_vec();
        ordered.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
        // ⚠️ **有没有转场决定走哪条图**（第 7 批 / task-10）：
        //   · **没有转场** ⇒ 老的 `concat` 那条路，**逐字节不变**（R4 存量零回归）。
        //   · **有转场** ⇒ `adelay` + `amix`：让两段在**时间上真的重叠**
        //     （`concat` 是首尾相接，各自在自己边缘上淡只是"接了个寂寞" ——
        //     实测：A 尾 0.25s 淡出 + B 头 0.25s 淡入拼起来，边界处 4000Hz 的过零数
        //     与硬切**一模一样** 19999。交叉淡化要求两段**同时**在响）。
        let crossfading = fade_len.iter().any(|&d| d > 0.0);

        for (i, (s, e, out_start, cspeed)) in ordered.iter().enumerate() {
            // ⚠️ **入段往前多留 `lead_in`、出段往后多播 `tail_out`** —— 这就是"两侧各让一半"
            // （见 `plan_dissolve_fades` 的几何图）。`atrim` 是按**源时间**切的，
            // 而 `lead_in`/`tail_out` 是**成片时间**上的秒：`speed ≠ 1` 时两者差 `speed` 倍，
            // 所以进 `atrim` 之前要乘回去。
            let speed = cspeed * rate;
            if !speed.is_finite() || speed <= 0.0 {
                anyhow::bail!(
                    "按 EDL 组装音轨：第 {} 段的 speed={speed} 非法（要求正有限值）",
                    i + 1
                );
            }
            let start = (s - lead_in[i] * speed).max(0.0);
            // `atrim` 的 `end` 是**源时间轴上的绝对时刻**（不是时长）—— 写成 `-t` 语义就全错。
            let end = (e + tail_out[i] * speed).max(start + 1e-6);
            // ⚠️ `atempo` **必须排在 `atrim` 之后**：先按源时间轴切出这一段
            // （此刻时间戳还是源时间），再对这一截做时域伸缩。反过来先伸缩再切，
            // `atrim` 的 `start`/`end` 就落在**伸缩后**的时间轴上 —— 切出来的是别的内容。
            let tempo = atempo_filter(speed).ok_or_else(|| {
                anyhow::anyhow!(
                    "按 EDL 组装音轨：第 {} 段的 speed={speed} 非法（要求正有限值）",
                    i + 1
                )
            })?;
            // 原速时 `tempo` 是空串 —— **不能**留下一个尾随逗号（`atrim=…,` 直接语法错）。
            let tempo_part = if tempo.is_empty() {
                String::new()
            } else {
                format!(",{tempo}")
            };
            // ⚠️ **叠化的 `afade` 排在 `atempo` 之后**：`afade` 的 `st`/`d` 是
            // **伸缩之后**的时间轴上的秒（`atempo` 已经改过时长），排在前面会把淡化窗口
            // 落在错的位置上。`speed ≠ 1` 时那两者差整整 `speed` 倍。
            //
            // 该段**扩窗之后**的实际时长（秒，伸缩后）：`(end - start) / speed`。
            let out_dur = (end - start) / speed;
            let mut fade_part = String::new();
            let flen = fade_len[i];
            if flen > 0.0 {
                // 入段（`lead_in > 0`）：从头淡入，窗口 `flen`。
                if lead_in[i] > 0.0 {
                    let d = flen.min(out_dur);
                    fade_part.push_str(&format!(",afade=t=in:st=0:d={d:.6}"));
                }
                // 出段（`tail_out > 0`）：淡出，窗口 `flen`，**末点落在扩窗后的段末**。
                if tail_out[i] > 0.0 && out_dur > flen {
                    let st = out_dur - flen;
                    fade_part.push_str(&format!(",afade=t=out:st={st:.6}:d={flen:.6}"));
                }
            }
            // ⚠️ **无声素材段**：用 `anullsrc` 铺**等长**静音（长度 = 该段在成片上的时长）。
            // 不能省掉这一段 —— 省了 `concat`/`amix` 的总长就短了，`mux_checked` 会当场拒绝
            // （它是按"派生帧数 ⇒ 期望样本数"卡死的）。
            let src_label = match input_index[i] {
                Some(k) => format!("[{k}:a]atrim=start={start:.6}:end={end:.6},asetpts=PTS-STARTPTS{tempo_part}{fade_part}"),
                None => {
                    // 静音长度 = 成片时长 = (end-start)/speed（伸缩之后的那份）
                    let sil = (out_dur).max(0.0);
                    format!(
                        "anullsrc=r=48000:cl=stereo,atrim=start=0:end={sil:.6},asetpts=PTS-STARTPTS{fade_part}"
                    )
                }
            };
            if crossfading {
                // ⚠️ **`adelay` 的延迟 = 该段在成片时间轴上的起点减掉它往前多留的那半**。
                // 毫秒整数（`adelay` 只吃毫秒）；`all=1` 套全部声道（比手写 `N|N` 稳）。
                let place = (out_start - lead_in[i]).max(0.0);
                let delay_ms = (place * 1000.0).round() as u64;
                filter_parts.push(format!(
                    "{src_label},adelay=delays={delay_ms}:all=1[a{i}]"
                ));
            } else {
                filter_parts.push(format!("{src_label}[a{i}]"));
            }
            labels.push(format!("[a{i}]"));
        }
        if crossfading {
            // ⚠️ **`normalize=0` 是必须的**：`amix` 默认会把总音量按输入数**归一化**
            // （2 路 ⇒ 各乘 0.5），那会让**整条音轨**（不只是重叠区）整体变轻一半
            // —— 一个"成片没以前响"的静默回归。淡化包络已经是我们自己控的增益了。
            // ⚠️ **`duration=longest`**：各段已经用 `adelay` 摆在绝对位置上，
            // 输出长度必须取"最后一段的结束"，`shortest` 会把它截掉。
            filter_parts.push(format!(
                "{}amix=inputs={}:duration=longest:normalize=0[out]",
                labels.join(""),
                labels.len()
            ));
        } else {
            // `v=0` 是**必须**的：不写的话 ffmpeg 会去找每段的视频流，而这里是纯音频图。
            filter_parts.push(format!(
                "{}concat=n={}:v=0:a=1[out]",
                labels.join(""),
                labels.len()
            ));
        }

        let mut args: Vec<String> = vec!["-y".into()];
        for f in &input_files {
            args.push("-i".into());
            args.push(f.clone());
        }
        args.push("-filter_complex".into());
        if std::env::var("VTEDIT_DEBUG_AUDIO").is_ok() {
            eprintln!("DEBUG audio inputs={:?}", input_files);
            eprintln!("DEBUG audio filter={}", filter_parts.join(";"));
        }
        args.push(filter_parts.join(";"));
        args.push("-map".into());
        args.push("[out]".into());
        args.push("-c:a".into());
        args.push("aac".into());
        args.push("-b:a".into());
        args.push("160k".into());
        args.push(output.into());
        let out = yekitrim_core::process::cmd("ffmpeg")
            .args(&args)
            .output()
            .map_err(|e| anyhow::anyhow!("ffmpeg 按 EDL 组装音轨启动失败: {e}"))?;
        if !out.status.success() {
            anyhow::bail!(
                "ffmpeg 按 EDL 组装音轨失败（{} 段）: {}",
                ordered.len(),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }

    /// 量一条视频流的**真实帧率**。
    ///
    /// # 为什么必须真探（这是 R6，一个"数字看着很合理"的错）
    ///
    /// `pipeline.rs` 里那句 `fps = 30` 是**猜的**，而底座那侧早就写明白了：
    /// 权威是**素材自己的时间基**（`dhampir.rs` 的 `sequence_frames` 就是从转译产物里读的）。
    /// EDL 的推导（`resolve_clips` / `out_start_frame` 累加）**整条都建立在 fps 上**，
    /// 拿 30 去推一个 60fps 的工程，每一段的成片起点都会差一半，而且：
    ///
    /// - 不报错（`resolve_clips` 对任何正数 fps 都能算）；
    /// - 报告里那两个数看着都对（它们都是用同一个错 fps 算的）；
    /// - 症状是"重放点对不上、字幕整体偏"。
    ///
    /// ⚠️ **不去新增一个 fps 字段落盘** —— 那就是第二份真相（规格 §9 明说"不新增字段"）。
    /// 每次出片现探一次，成本是几十毫秒。
    ///
    /// 取 `r_frame_rate`（"30000/1001" 这种分数），解析不出来退 `avg_frame_rate`。
    pub fn probe_fps(path: &str) -> anyhow::Result<f64> {
        for entry in ["stream=r_frame_rate", "stream=avg_frame_rate"] {
            let out = yekitrim_core::process::cmd("ffprobe")
                .args([
                    "-v", "error",
                    "-select_streams", "v:0",
                    "-show_entries", entry,
                    "-of", "default=noprint_wrappers=1:nokey=1",
                    path,
                ])
                .output()
                .map_err(|e| anyhow::anyhow!("ffprobe 启动失败: {e}"))?;
            if !out.status.success() {
                continue;
            }
            if let Some(fps) = parse_rational_fps(&String::from_utf8_lossy(&out.stdout)) {
                return Ok(fps);
            }
        }
        anyhow::bail!("ffprobe 量不出帧率（r_frame_rate / avg_frame_rate 两条都失败）: {path}")
    }

    /// 量一条音频流的**解码后样本数**（不是容器时长）。
    ///
    /// # 为什么不能用容器时长对账（实测踩出来的）
    ///
    /// AAC 多段拼接、重编码之后**容器时长会被撑大**，而样本内容是对的：
    ///
    /// ```text
    /// 18.077891s 的 AAC  →  -c copy         →  18.077891s   （保留）
    ///                    →  -c:a aac 重编码  →  18.478725s   （+0.401s 纯补齐）
    /// ```
    ///
    /// 那 0.401s 是编码器在**多段边界**上各自的 priming/补齐（单段 12s 只涨 0.024s）。
    /// 拿容器时长卡 ±1 帧 ⇒ **每一次带 EDL 的出片都会红**，而那是误报；
    /// 一条习惯性误报的断言就不再是防线了（真漏拼一段时也没人看）。
    ///
    /// # 为什么必须**解码后数**、不能读 `nb_samples`
    ///
    /// ⚠️ 实测：ADTS 裸 `.aac`（`extract_audio` / `assemble_audio_from_edl` 的产物）
    /// 的 `nb_samples` 是**空的 / N/A** —— 那个字段只有 MP4/MKV 这类容器才记。
    /// 所以只能真解一遍再数：`-f wav -ar <rate> -ac 1` 到临时文件，
    /// 用 WAV 头里的 `data` 长度换算。**代价是一条纯音频解码**（这段音轨几十秒，
    /// 实测 <0.3s），换的是"对账口径真的可比"。
    ///
    /// ⚠️ **必须 `-ar` 到固定采样率再数**：不写的话采样率跟源走，
    /// 而源可能是 44.1k —— 那样样本数就不能直接与 `按 fps 推出来的秒数 × 48000` 比。
    pub fn probe_audio_samples(path: &str, sample_rate: u32) -> anyhow::Result<u64> {
        let tmp = std::env::temp_dir().join(format!(
            "_polish_probe_{}_{}.wav",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let out = yekitrim_core::process::cmd("ffmpeg")
            .args([
                "-v", "error",
                "-y",
                "-i", path,
                "-vn",
                "-f", "wav",
                "-ar", &sample_rate.to_string(),
                "-ac", "1",
            ])
            .arg(&tmp)
            .output()
            .map_err(|e| anyhow::anyhow!("ffmpeg 启动失败: {e}"))?;
        if !out.status.success() {
            let _ = std::fs::remove_file(&tmp);
            anyhow::bail!(
                "解码音轨以数样本失败: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        let bytes = std::fs::read(&tmp)
            .map_err(|e| anyhow::anyhow!("读不回解出来的 wav（{}）: {e}", tmp.display()))?;
        let _ = std::fs::remove_file(&tmp);
        // **不解析 WAV 头**：`-ac 1` + `-f wav` 的输出就是 44 字节标准头 + PCM16 data。
        // 但 44 这个数不该写死（不同 ffmpeg 的 `LIST` 块长度会变），所以找 `data` 块。
        let data = bytes
            .windows(4)
            .position(|w| w == b"data")
            .ok_or_else(|| anyhow::anyhow!("解出来的 wav 里找不到 data 块"))?;
        if data + 8 > bytes.len() {
            anyhow::bail!("wav 的 data 块头被截断");
        }
        let len = u32::from_le_bytes([bytes[data + 4], bytes[data + 5], bytes[data + 6], bytes[data + 7]])
            as u64;
        // PCM16 单声道 ⇒ 每样本 2 字节。
        Ok(len / 2)
    }

    /// 视频 + 音频合流
    ///
    /// ⚠️ **`-shortest` 会静默截断**（R5）。这条命令原本就是"谁短听谁的"，而两条轨
    /// 不等长的成因极多（音轨组装时少拼了一段、画面比派生时长多出一帧、某一段被屏蔽切掉……），
    /// 每一条都**不报错** —— 用户只会发现"成片比预期短一截"。
    /// 所以：
    ///
    /// 1. 合流**之前**先用 [`Ffmpeg::probe_duration`] 量一次两条轨，差超过一帧就**拒绝出片**
    ///    （`bail!`）并把两个数写进错误里。宁可让任务失败，也不要一个看不出来的短成片。
    /// 2. 判据是**帧数相等或 ±1 帧**（半帧量化是 EDL 的固有代价，见规格 §2.4）——
    ///    别按毫秒卡死，那会把浮点舍入当成错误。
    ///
    /// # 参数
    ///
    /// - `check_frames`：期望的成片帧数（`Edl::total_frames` 派生值）。`None` = 不做断言
    ///   （老工程 / 时长未知时别拦 —— 没有派生值就没有比较基准）。
    /// - `fps`：断言用的帧率。
    pub fn mux_checked(
        video: &str,
        audio: &str,
        output: &str,
        check_frames: Option<u64>,
        fps: f64,
    ) -> anyhow::Result<()> {
        if let Some(expected) = check_frames {
            if fps > 0.0 {
                let vd = Self::probe_duration(video)
                    .map_err(|e| anyhow::anyhow!("量成片画面时长失败（合流前的断言需要它）: {e:#}"))?;
                let ad = Self::probe_duration(audio)
                    .map_err(|e| anyhow::anyhow!("量成片音轨时长失败（合流前的断言需要它）: {e:#}"))?;
                let vf = (vd * fps).round() as i64;
                let want = expected as i64;
                // 画面：派生值 ±1 帧（`-shortest` 之后成片就是画面那么长）。
                if (vf - want).abs() > 1 {
                    anyhow::bail!(
                        "成片画面帧数与成片时间线不一致：派生 {want} 帧（{:.3}s），实得 {vf} 帧（{vd:.3}s，fps={fps}）。\
                         差超过 1 帧 ⇒ 画面那条腿没有按 EDL 出全（转译器/底座）。**不继续合流** —— \
                         `-shortest` 会把成片静默截到较短的那条轨上。",
                        expected as f64 / fps
                    );
                }
                // 音轨：**用解码后的样本数对账**，不用容器时长。
                //
                // ⚠️ 为什么（实测）：AAC 多段拼接后**重编码会把容器时长撑大**
                // ——18.077891s `-c copy` 原样保留，`-c:a aac` 重编码变 **18.478725s**
                // （+0.401s，是每段边界的 priming/补齐；单段只涨 0.024s）。
                // 拿容器时长卡 ±1 帧 ⇒ **每一次带 EDL 的出片都红**，而那是**误报**。
                // 一条习惯性误报的断言就不再是防线了：真漏拼一段时也没人会去看它。
                //
                // `nb_samples` 不受补齐影响 —— 补齐的是容器里多出来的静音帧，不是有效样本。
                // ⚠️ 采样率取流上的真实值（不是写死 48000）：`extract_audio` / `assemble`
                // 都写 `-c:a aac` 而**没指定 `-ar`**，采样率跟着源走。
                // ⚠️ `nb_samples` 读不到（N/A）时**退回容器时长**并放宽到 AAC 补齐的量级
                // —— 不能因为"探不到"就跳过整条断言（那正好是最该拦的时候）。
                // 统一按 48000 数（`probe_audio_samples` 会 `-ar 48000` 重采样后再数），
                // 这样样本数与"派生的秒数"直接可比，不受源采样率影响。
                let sr = 48000.0_f64;
                let want_samples = (expected as f64 / fps * sr).round() as i64;
                // 容差：**按 AAC 帧的个数给**，逐值标定过（实测数据在下面）。
                //
                // 实测（60fps、18.000s 目标、48000Hz ⇒ 期望 864000 样本）：
                //   · EDL 拼出来的基准轨（3 段 concat + `-c:a aac`）   → **865280**（+1280 = 1.25 AAC 帧）
                //   · 再过一道 `mix_sfx`（3 条 SFX）                    → **867328**（+3328 = 3.25 AAC 帧）
                //
                // 那 3.25 个 AAC 帧是**编码器自己的**priming/边界补齐，不是内容缺失 ——
                // 所以判据要按"几个 AAC 帧"给，而不是按视频帧（1 视频帧 = 800 样本，
                // 比 1 个 AAC 帧还小，拿它卡必然误报）。
                //
                // ⚠️ **取 8 个 AAC 帧**：比实测的 3.25 帧留一倍余量，但仍然
                // **远小于最短的一段**（最短也有一帧视频 ≈ 800 样本 —— 不，那是 1 帧；
                // 真正要拦的是"整段丢了"，那至少 800 样本 × 段长，量级差 3 个数量级）。
                // 换句话说：**漏拼一整段一定红**，编码器补几个帧不会红。
                let slack = 8 * 1024i64;
                match Self::probe_audio_samples(audio, 48000) {
                    Ok(samples) => {
                        let got = samples as i64;
                        if (got - want_samples).abs() > slack {
                            anyhow::bail!(
                                "音轨样本数与成片时间线不一致：派生 {want} 帧 ⇒ 期望 {want_samples} 样本\
                                 （{:.3}s @ {sr:.0}Hz），实得 {got} 样本（{:.3}s）。\
                                 差超过 {slack} 样本（1 个 AAC 帧 + 1 帧视频）⇒ \
                                 音轨组装漏段/多段（`assemble_audio_from_edl`），或 SFX 混音把轨拉长了。\
                                 **不继续合流** —— 音轨短会被 `-shortest` 截断成片。",
                                expected as f64 / fps,
                                got as f64 / sr
                            );
                        }
                        tracing::info!(
                            "  ✓ 成片时长断言通过：画面 {vf} 帧 / 音轨 {got} 样本（期望 {want_samples}，@ {sr:.0}Hz）/ 派生 {want} 帧"
                        );
                    }
                    Err(e) => {
                        // 退回容器时长口径（重解码这一步就失败了）。余量按实测的
                        // "AAC 多段重编码补齐"给（3 段 18s 实测 +0.401s ⇒ 每段 ~0.14s，给 8 段）。
                        let af = (ad * fps).round() as i64;
                        let pad_slack = (0.15 * fps).ceil() as i64 * 8 + 2;
                        if af < want - pad_slack {
                            anyhow::bail!(
                                "音轨**短于**成片时间线：派生 {want} 帧（{:.3}s），实得 {af} 帧（{ad:.3}s）。\
                                 音轨短 ⇒ `-shortest` 会把成片截到音轨上。\
                                 （另：量样本数也失败了 —— {e:#}）",
                                expected as f64 / fps
                            );
                        }
                        tracing::warn!(
                            "  ⚠ 量不到音轨样本数（{e:#}），退回容器时长口径：{af} 帧（期望 {want} 帧）"
                        );
                    }
                }
            }
        }
        Self::mux(video, audio, output)
    }

    /// 量一个媒体文件的时长（秒）。探不到就**报错**，不返回 0
    /// —— 0 会让上面那条断言把"探不到"当成"成片是空的"。
    ///
    /// 这个文件**有没有音轨**。
    ///
    /// 用在 EDL 的素材段上（片头/片尾常是一段无声的 logo 动画）：
    /// 有没有音轨决定那一段是"读它的声音"还是"补等长静音"。
    /// ⚠️ **不探就会硬绑 `[N:a]` 失败**：ffmpeg 报
    /// `Stream specifier ':a' ... matches no streams`，整条音轨组装直接失败
    /// （实测：一个无声的 intro.mp4 让整部片子出不来）。
    /// 探不出来（ffprobe 失败）时**当"没有"**，即补静音 —— 宁可少一段素材声，
    /// 也不要因为探不出而让整片渲染失败。
    pub fn has_audio_stream(path: &str) -> bool {
        let Ok(out) = yekitrim_core::process::cmd("ffprobe")
            .args([
                "-v", "error",
                "-select_streams", "a:0",
                "-show_entries", "stream=index",
                "-of", "default=noprint_wrappers=1:nokey=1",
                path,
            ])
            .output()
        else {
            return false;
        };
        out.status.success() && !String::from_utf8_lossy(&out.stdout).trim().is_empty()
    }

    /// 走 `ffprobe` 的 `format=duration`（容器级，不解码）；它读不出来时退到
    /// `stream=duration`（有些流式容器只在流上写）。
    pub fn probe_duration(path: &str) -> anyhow::Result<f64> {
        for entry in ["format=duration", "stream=duration"] {
            let out = yekitrim_core::process::cmd("ffprobe")
                .args([
                    "-v", "error",
                    "-show_entries", entry,
                    "-of", "default=noprint_wrappers=1:nokey=1",
                    path,
                ])
                .output()
                .map_err(|e| anyhow::anyhow!("ffprobe 启动失败: {e}"))?;
            if !out.status.success() {
                continue;
            }
            if let Ok(v) = String::from_utf8_lossy(&out.stdout).trim().parse::<f64>() {
                if v.is_finite() && v > 0.0 {
                    return Ok(v);
                }
            }
        }
        anyhow::bail!("ffprobe 量不出时长（format/stream 两条都失败）: {path}")
    }

    /// 视频 + 音频合流
    pub fn mux(video: &str, audio: &str, output: &str) -> anyhow::Result<()> {
        let out = yekitrim_core::process::cmd("ffmpeg")
            .args([
                "-y", "-i", video,
                "-i", audio,
                "-c:v", "copy",
                "-c:a", "aac",
                "-b:a", "128k",
                "-shortest",
                output,
            ])
            .output()
            .map_err(|e| anyhow::anyhow!("ffmpeg mux 启动失败: {}", e))?;

        if !out.status.success() {
            anyhow::bail!(
                "ffmpeg mux failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }
    /// 把「转场边」翻成**每段的淡化与扩窗**（纯函数，为了能逐值测）。
    ///
    /// 输入：`clips` = 段表（`(source_start, source_end, out_start, speed)`）；
    /// `fades` = `(出段下标, 入段下标, 重叠秒数)`（下标是 `clips` 的**传入顺序**）。
    ///
    /// 输出：`(lead_in, fade_len, tail_out)` 三个长度 `clips.len()` 的向量，
    /// 下标是**按 `out_start` 排序后**的位置，单位秒：
    /// - `lead_in[i]`：第 i 段要**往前多留**多久（该段起点提前、前一段多播这么久）；
    /// - `fade_len[i]`：该段上要挂的淡化包络长度（`0` = 不挂）；
    /// - `tail_out[i]`：第 i 段要**往后多播**多久（供下一段淡入时压在上面）。
    ///
    /// # 几何（一次转场 `O` 秒，边界在 `t`）
    ///
    /// 转场**骑在边界上**，两侧各让 `O/2`：
    /// ```text
    /// A 的音频：…… ═══════════╗（fade out 铺满 O，末点 = t + O/2）
    /// B 的音频：        ╔═══════════ ……（fade in 铺满 O，起点 = t − O/2）
    ///                   t−O/2      t      t+O/2
    /// ```
    /// ⇒ A 往后多播 `O/2`、B 往前多留 `O/2`，两段的包络**在 `[t−O/2, t+O/2]` 上同时存在**
    /// （这才是"交叉"淡化）。总长不变：B 的**结束**位置没动。
    ///
    /// ⚠️ **不能各在自己那一侧的边缘上淡**（我第一版的错法）：那样 A 的淡出与 B 的淡入
    /// 只是**首尾相接**，窗口里任何时刻只有**一个**频率在响 —— 实测过零数与硬切
    /// **一模一样**（19999 vs 19999），也就是"淡了个寂寞"。必须让两段**同时在响**。
    ///
    /// ⚠️ **必须按 `out_start` 重排后再找位置**：`fades` 的下标是传入顺序，
    /// 而消费侧用的是排序后的顺序。不搬下标 ⇒ 淡化挂到**别的段**上
    /// （乱序 EDL 下才会暴露，而 EDL 的卖点恰恰是乱序）。
    fn plan_dissolve_fades(
        clips: &[(f64, f64, f64, f64)],
        fades: &[(usize, usize, f64)],
    ) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
        let n = clips.len();
        let mut lead_in = vec![0.0_f64; n];
        let mut fade_len = vec![0.0_f64; n];
        let mut tail_out = vec![0.0_f64; n];
        // 传入顺序 → 按 out_start 排序后的位置。
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| {
            clips[a].2.partial_cmp(&clips[b].2).unwrap_or(std::cmp::Ordering::Equal)
        });
        for &(prev, next, secs) in fades {
            if !secs.is_finite() || secs <= 0.0 {
                continue;
            }
            if prev >= n || next >= n {
                continue;
            }
            let (Some(p_pos), Some(n_pos)) =
                (order.iter().position(|&x| x == prev), order.iter().position(|&x| x == next))
            else {
                continue;
            };
            // 出段：往后多播半个窗口 + 挂 O 长的淡出。
            tail_out[p_pos] = secs / 2.0;
            fade_len[p_pos] = secs;
            // 入段：往前多留半个窗口 + 挂 O 长的淡入。
            lead_in[n_pos] = secs / 2.0;
            fade_len[n_pos] = secs;
        }
        (lead_in, fade_len, tail_out)
    }
}

#[derive(Debug)]
pub struct EncoderInfo {
    pub codec: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SfxEvent {
    pub time: f64,
    pub file: String,
    pub volume: f64,
    pub duration: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 帧率解析：**1001 那一族必须不丢精度**。
    ///
    /// 写成"取整"的实现在 29.97/59.94 上少 0.1% —— 一个 133 秒的长片累计差 ~0.13s，
    /// 正好落在 `mux_checked` 的 ±1 帧容差里，所以**它不会红**，只会一直小声偏着。
    #[test]
    fn rational_fps_keeps_ntsc_precision() {
        assert_eq!(parse_rational_fps("60/1"), Some(60.0));
        assert_eq!(parse_rational_fps("25"), Some(25.0));
        let ntsc = parse_rational_fps("30000/1001").unwrap();
        assert!((ntsc - 29.970_029_97).abs() < 1e-6, "实得 {ntsc}");
        let ntsc60 = parse_rational_fps("60000/1001").unwrap();
        assert!((ntsc60 - 59.940_059_94).abs() < 1e-6, "实得 {ntsc60}");
    }

    /// 垃圾输入一律 `None`，**绝不返回 `inf`/`NaN`** ——
    /// 那两个值会让 `resolve_clips` 的帧号全变 NaN，然后 `as u64` 得到 0，
    /// 表现是"成片时长 0"而不是报错。
    #[test]
    fn rational_fps_rejects_garbage() {
        assert_eq!(parse_rational_fps(""), None);
        assert_eq!(parse_rational_fps("   "), None);
        assert_eq!(parse_rational_fps("0/0"), None);
        assert_eq!(parse_rational_fps("60/0"), None);
        assert_eq!(parse_rational_fps("0/1"), None);
        assert_eq!(parse_rational_fps("-30/1"), None);
        assert_eq!(parse_rational_fps("abc"), None);
        assert_eq!(parse_rational_fps("N/A"), None);
    }

    /// 空段列表必须**报错**，不许拼出 0 秒音轨。
    ///
    /// 这是 B 件最凶的一条静默失败：`resolve_edl` 在时长未知时返回空数组，
    /// 空数组拼出来的音轨是 0 秒，而 `-shortest` 会把成片截到 0.1 秒 —— 全程无报错。
    #[test]
    fn assemble_audio_rejects_empty_clips() {
        let err = Ffmpeg::assemble_audio_from_edl("in.mp4", &[], "out.m4a", 1.0).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("段列表为空"), "错误信息要说清是空的：{msg}");
    }

    /// 整轨 `rate` 非法（0 / 负 / NaN）必须**当场拒绝**。
    ///
    /// ⚠️ 第 2 批这里钉的是"`rate != 1.0` 就拒绝"——因为那时还没有 `atempo`，
    /// 按原速拼就是音画不同步。第 4 批把 `atempo` 实现掉之后那条判据**整条作废**：
    /// `rate = 1.5` 今天是**合法且必须成功**的（见 `atempo_filter` 的单测）。
    /// 留着它会把正确行为判成失败 —— 这是"断言过期"的典型形态，所以直接改写而不是保留。
    #[test]
    fn assemble_audio_rejects_non_positive_rate() {
        let clips = [(0.0, 1.0, 0.0, 1.0)];
        for bad in [0.0_f64, -1.0, f64::NAN] {
            let err =
                Ffmpeg::assemble_audio_from_edl("in.mp4", &clips, "out.m4a", bad).unwrap_err();
            let msg = format!("{err}");
            assert!(msg.contains("rate="), "错误信息要点名那个 rate：{msg}");
        }
    }

    /// 逐段 `speed` 非法也必须**当场拒绝**，不许当成 1.0 静默放行。
    #[test]
    fn assemble_audio_rejects_non_positive_clip_speed() {
        let clips = [(0.0_f64, 1.0_f64, 0.0_f64, 0.0_f64)];
        let err = Ffmpeg::assemble_audio_from_edl("in.mp4", &clips, "out.m4a", 1.0).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("speed=0"), "错误信息要点名那个 speed：{msg}");
        assert!(msg.contains("第 1 段"), "要点名是哪一段：{msg}");
    }

    // =====================================================================
    // `atempo` 分解（第 4 批的核心可测单元）
    // =====================================================================

    /// **边界**：`0.5` / `2.0` 就是单级的两端，**必须恰好一级**。
    ///
    /// 拆成两级也是"数学上对的"（`0.5 = 0.5×1.0`），但每多一级就多一次独立的
    /// 时域伸缩接缝 —— 白付伪影。所以边界值钉成"一级"。
    #[test]
    fn atempo_boundaries_are_single_stage() {
        let lo = atempo_filter(0.5).expect("0.5 是合法速率");
        assert_eq!(lo, "atempo=0.500000", "单级下界必须只有一级：{lo}");
        assert_eq!(lo.matches("atempo=").count(), 1, "恰好一级：{lo}");

        let hi = atempo_filter(2.0).expect("2.0 是合法速率");
        assert_eq!(hi, "atempo=2.000000", "单级上界必须只有一级：{hi}");
        assert_eq!(hi.matches("atempo=").count(), 1, "恰好一级：{hi}");

        // 单级区间内的普通值也是一级。
        assert_eq!(atempo_filter(0.75).unwrap().matches("atempo=").count(), 1);
        assert_eq!(atempo_filter(1.5).unwrap().matches("atempo=").count(), 1);
    }

    /// **原速 = 不挂任何滤镜**（空串，不是 `atempo=1.0`）。
    ///
    /// 判据落在"空串"上：调用方要靠它决定**加不加那个逗号**。
    #[test]
    fn atempo_unit_speed_is_empty() {
        assert_eq!(atempo_filter(1.0), Some(String::new()));
        // 极近 1 也算原速（浮点残留不该多挂一级伸缩）。
        assert_eq!(atempo_filter(1.0 + 1e-12), Some(String::new()));
    }

    /// **超界向下（慢放）串联**：规格 §3.2 的原话是 `0.25` ⇒ `atempo=0.5,atempo=0.5`。
    #[test]
    fn atempo_chains_below_range() {
        let f = atempo_filter(0.25).expect("0.25 合法");
        assert_eq!(f, "atempo=0.500000,atempo=0.500000", "0.25 要串两级 0.5：{f}");
        assert_eq!(f.matches("atempo=").count(), 2);
        // **每一级都必须在单级区间内** —— 这才是"串联"这条规矩的全部意义。
        for stage in f.split(',') {
            let v: f64 = stage.trim_start_matches("atempo=").parse().unwrap();
            assert!(
                (ATEMPO_STAGE_MIN..=ATEMPO_STAGE_MAX).contains(&v),
                "每一级都要落在 [{ATEMPO_STAGE_MIN}, {ATEMPO_STAGE_MAX}] 内，越界 ffmpeg 会直接报错：{stage}"
            );
        }
    }

    /// **超界向上（快放）串联**：规格同段的 `4` ⇒ `atempo=2.0,atempo=2.0`。
    #[test]
    fn atempo_chains_above_range() {
        let f = atempo_filter(4.0).expect("4.0 合法");
        assert_eq!(f, "atempo=2.000000,atempo=2.000000", "4.0 要串两级 2.0：{f}");
        assert_eq!(f.matches("atempo=").count(), 2);
        for stage in f.split(',') {
            let v: f64 = stage.trim_start_matches("atempo=").parse().unwrap();
            assert!((ATEMPO_STAGE_MIN..=ATEMPO_STAGE_MAX).contains(&v), "越界：{stage}");
        }
    }

    /// 多级串联时**每一级都在单级区间内**，且**各级之积 == 请求的速率**。
    ///
    /// ⚠️ 这条是"串联"唯一真正要保证的事：级数怎么拆都行，但**乘起来必须是原值** ——
    /// 差一点点，音轨长度就与画面那条腿对不上（而 `mux_checked` 的容差是 8 个 AAC 帧，
    /// 累计误差大了会**红在一条与变速无关的断言上**，排查方向全错）。
    #[test]
    fn atempo_chain_product_equals_speed_and_stages_in_range() {
        for speed in [
            0.1_f64, 0.2, 0.25, 0.3, 0.4, 0.5, 0.6, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0,
        ] {
            let f = atempo_filter(speed).unwrap_or_else(|| panic!("{speed} 应当合法"));
            if f.is_empty() {
                assert!((speed - 1.0).abs() < 1e-9, "只有原速才能是空串：{speed}");
                continue;
            }
            let mut product = 1.0_f64;
            let mut n = 0;
            for stage in f.split(',') {
                let v: f64 = stage.trim_start_matches("atempo=").parse().unwrap();
                assert!(
                    (ATEMPO_STAGE_MIN..=ATEMPO_STAGE_MAX).contains(&v),
                    "speed={speed} 有一级越界（{v}）：{f}"
                );
                product *= v;
                n += 1;
            }
            assert!(n >= 1, "非原速至少要有一级：{speed}");
            assert!(
                (product - speed).abs() / speed < 1e-6,
                "各级之积 {product} 必须等于请求的 {speed}：{f}"
            );
        }
    }

    /// 垃圾速率一律 `None`（**绝不 `Some`**：那会让调用方按原速静默出片）。
    #[test]
    fn atempo_rejects_garbage() {
        assert_eq!(atempo_filter(0.0), None);
        assert_eq!(atempo_filter(-2.0), None);
        assert_eq!(atempo_filter(f64::NAN), None);
        assert_eq!(atempo_filter(f64::INFINITY), None);
    }

    /// ⚠️ **重放段必须各自成段**：EDL 里同一段源出现两次时，音轨要拼**两遍**。
    ///
    /// 判据落在"段列表被原样消费"上：`assemble_audio_from_edl` 的 `clips` 参数是
    /// `(source_start, source_end, out_start)`，同一对 `(source_start, source_end)`
    /// 出现两次就是重放。这里钉住"它不会去重、也不会按 `out_start` 之前的顺序重排错"。
    ///
    /// 用真实媒体跑（`ffmpeg` 不在 PATH 时跳过）—— 纯逻辑的桩测不出 `atrim`/`concat` 的行为。
    #[test]
    fn assemble_audio_keeps_replayed_segment_twice() {
        if which_ffmpeg().is_none() {
            eprintln!("跳过：PATH 上没有 ffmpeg");
            return;
        }
        let dir = std::env::temp_dir().join(format!("_polish_replay_audio_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let src = dir.join("src.mp4");
        // 造一条 2 秒、每 0.5s 换一次音高的源（这样"拼了几段、段的顺序"能从时长看出来）。
        let made = yekitrim_core::process::cmd("ffmpeg")
            .args([
                "-v", "error", "-y",
                "-f", "lavfi", "-i", "sine=frequency=440:duration=2",
                "-f", "lavfi", "-i", "color=c=black:s=64x64:d=2",
                "-shortest", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac",
            ])
            .arg(&src)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !made {
            eprintln!("跳过：造不出测试素材");
            return;
        }

        // 同一段源 [0,1) 放两次：out_start = 0 和 1（speed 都是 1）。
        let clips = [
            (0.0_f64, 1.0_f64, 0.0_f64, 1.0_f64),
            (0.0, 1.0, 1.0, 1.0),
        ];
        let out = dir.join("replay.m4a");
        Ffmpeg::assemble_audio_from_edl(
            src.to_str().unwrap(),
            &clips,
            out.to_str().unwrap(),
            1.0,
        )
        .expect("重放段应当能拼出来");

        let secs = Ffmpeg::probe_duration(out.to_str().unwrap()).expect("量得出时长");
        // **两段 1 秒 = 2 秒**。去重的话会是 1 秒 —— 那正是要拦的静默错误。
        assert!(
            (secs - 2.0).abs() < 0.15,
            "重放段应当拼两遍（约 2.0s），实得 {secs:.3}s —— 去重了就会是 1.0s"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ⚠️ **变速段真的变了、而且没变调**（第 4 批的核心准入判据，用真媒体跑）。
    ///
    /// # 判据为什么是"基频"而不是"听感"
    ///
    /// 把音轨变快有两条路，**只有一条对**：
    /// - `atempo`：**时域伸缩**（WSOLA）—— 时长变、基频不变；
    /// - `asetpts` / `asetrate` 缩放：改的是"时间戳/采样率"，**样本内容一个字节没动**
    ///   ⇒ 播放器按 `speed` 倍的速率去重采样 ⇒ **基频乘以 speed**（花栗鼠音）。
    ///
    /// 两条路的**时长可能一样**，所以"时长对了"根本区分不出它们 —— 唯一能区分的是基频。
    /// 实测（同源 440Hz 正弦，`target/speed-fixture`）：
    ///
    /// ```text
    /// 源                  基频 440.00Hz   12.00s
    /// atempo=0.5          基频 440.04Hz   23.98s   ✓
    /// atempo=2.0          基频 440.07Hz    6.00s   ✓
    /// asetpts=PTS/2       基频 440.00Hz   12.00s   ✗ 时长压根没变（只是骗了时间戳）
    /// asetrate=96000      基频 880.00Hz    6.00s   ✗ 花栗鼠音（440×2）
    /// ```
    ///
    /// # 判据（两条，各拦一类错法）
    ///
    /// - **时长**：用 `probe_audio_samples` 量**解码后的样本数**（与 `mux_checked` 同一口径），
    ///   期望 `2.0 / speed × 48000`。容器时长**不算数** —— 见下面 `asetpts` 那条实测。
    /// - **基频**：落在 440±3%（`atempo` 的 WSOLA 实测偏移 <0.02%）。
    ///   `asetrate` 那一路会落在 880 —— 判据离它 100%，绝不会混淆。
    ///
    /// 基频用**过零率**估：正弦过零率 × 采样率 / 2 = 基频，纯逻辑、不引 FFT 依赖。
    /// 素材段的声音**从素材自己读**、无声素材**补等长静音**（2026-10-09，片头/片尾带出来的）。
    ///
    /// 为什么必须有这条：`assemble_audio_from_edl` 原先**硬编码 `[0:a]`**（只读主素材）。
    /// EDL 从第 6 批起支持 `source_kind = "asset"`，而底座那条腿对素材段取的是**素材自己的**画面 ⇒
    /// 声音必须同源。不修的症状是 `mux_checked` 拒绝合流（实测 1080 帧画面 vs 864000 样本音轨）。
    ///
    /// 两条判据（缺一条这段就能静默退化）：
    /// - **音源真的换了**：主素材是 440Hz、素材是 880Hz ⇒ 量素材那一段的过零率，必须落在 880±5%
    ///   （回落成主素材就是 440，判据离它 100%）。
    /// - **无声素材补静音、且长度对得上**：素材只有视频流时整条音轨必须仍然成功、且总样本数
    ///   等于派生时长 —— 少了这一段 `mux_checked` 会拒（这正是当初暴露问题的形状）。
    #[test]
    fn assemble_audio_reads_asset_segments_from_their_own_file() {
        if which_ffmpeg().is_none() {
            eprintln!("跳过：PATH 上没有 ffmpeg");
            return;
        }
        let dir = std::env::temp_dir().join(format!("_polish_asset_audio_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        // 主素材：440Hz；素材：880Hz（两者都能看出"读的是谁"）
        let mk = |name: &str, hz: u32, with_audio: bool| -> Option<std::path::PathBuf> {
            let p = dir.join(name);
            let mut c = yekitrim_core::process::cmd("ffmpeg");
            c.args(["-v", "error", "-y", "-f", "lavfi", "-i"])
                .arg(format!("color=c=black:s=64x64:d=2"))
                .args(["-f", "lavfi", "-i"])
                .arg(format!("sine=frequency={hz}:duration=2"))
                .args(["-shortest", "-c:v", "libx264", "-pix_fmt", "yuv420p"]);
            if with_audio {
                c.args(["-c:a", "aac"]);
            } else {
                c.arg("-an");
            }
            c.arg(&p).status().map(|s| s.success()).unwrap_or(false).then_some(p)
        };
        let (Some(src), Some(asset)) = (mk("src.mp4", 440, true), mk("asset.mp4", 880, true)) else {
            eprintln!("跳过：造不出测试素材");
            return;
        };
        let src_s = src.to_str().unwrap().to_string();
        let asset_s = asset.to_str().unwrap().to_string();

        // 三段（各 1 秒）：主素材 [0,1) → **素材** [0,1) → 主素材 [0,1)。
        // `out_start` = 0 / 1 / 2。中段的声音必须来自素材（880Hz）。
        let clips = [
            (0.0_f64, 1.0_f64, 0.0_f64, 1.0_f64),
            (0.0, 1.0, 1.0, 1.0),
            (0.0, 1.0, 2.0, 1.0),
        ];
        let audio_of = [None, Some(asset_s.clone()), None];
        let out = dir.join("mixed.m4a");
        Ffmpeg::assemble_audio_from_edl_sources(
            &src_s, &clips, &[], &audio_of, out.to_str().unwrap(), 1.0,
        )
        .expect("按 EDL 组装（含素材段）必须成功");

        let samples = Ffmpeg::probe_audio_samples(out.to_str().unwrap(), 48000).expect("量样本数") as i64;
        let want = (3.0_f64 * 48000.0).round() as i64;
        assert!(
            (samples - want).abs() <= 8 * 1024,
            "总样本数应当 ≈ 3 秒（{want}），实得 {samples} —— 素材段没被拼进去 / 拼长了"
        );

        // **整体**看上去是"混了两路"：它必须**不是**纯 440（那说明中段回落成了主素材）、
        // 也**不是**纯 880。真正逐段判据在下一条（单独拼素材那一段）。
        let mixed_hz = estimate_fundamental_hz(out.to_str().unwrap()).expect("估得出基频");
        assert!(
            (mixed_hz - 440.0).abs() > 20.0,
            "首尾是 440Hz 的源、中段是 880Hz 的素材，整条不该测成纯 440（实得 {mixed_hz:.0}Hz）\
             —— 这就是'音源没换'（片头放的是正片的对白）"
        );

        // **逐段确证**：只拼素材那一段，基频必须落在 880（±5%）。
        let only_asset = dir.join("only-asset.m4a");
        Ffmpeg::assemble_audio_from_edl_sources(
            &src_s,
            &[(0.0, 1.0, 0.0, 1.0)],
            &[],
            &[Some(asset_s.clone())],
            only_asset.to_str().unwrap(),
            1.0,
        )
        .expect("只拼素材段");
        let asset_hz = estimate_fundamental_hz(only_asset.to_str().unwrap()).expect("估得出基频");
        assert!(
            (asset_hz - 880.0).abs() / 880.0 < 0.05,
            "素材段必须是 880Hz（素材自己的声音），实测约 {asset_hz:.0}Hz —— \
             回落成 440 就是'音源没换'（片头放的是正片的对白）"
        );

        // ---- ② 无声素材：补等长静音，整条音轨仍要能成功且长度对得上 ----
        let Some(silent) = mk("silent.mp4", 440, false) else {
            eprintln!("跳过无声素材那一条");
            return;
        };
        let audio_of_silent = [None, Some(silent.to_str().unwrap().to_string()), None];
        let out2 = dir.join("silent.m4a");
        Ffmpeg::assemble_audio_from_edl_sources(
            &src_s, &clips, &[], &audio_of_silent, out2.to_str().unwrap(), 1.0,
        )
        .expect("无声素材段必须补静音而不是整条失败（片头常是无音轨的 logo 动画）");
        let samples2 = Ffmpeg::probe_audio_samples(out2.to_str().unwrap(), 48000).expect("量样本数") as i64;
        assert!(
            (samples2 - want).abs() <= 8 * 1024,
            "无声素材段也要补出等长静音：期望 ≈ {want} 样本，实得 {samples2}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn assemble_audio_changes_speed_without_shifting_pitch() {
        if which_ffmpeg().is_none() {
            eprintln!("跳过：PATH 上没有 ffmpeg");
            return;
        }
        let dir = std::env::temp_dir().join(format!("_polish_speed_audio_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let src = dir.join("src.mp4");
        // 2 秒 / 440Hz 纯音 + 一张黑图（音轨组装要有视频流才成 mp4）。
        let made = yekitrim_core::process::cmd("ffmpeg")
            .args([
                "-v", "error", "-y",
                "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:duration=2",
                "-f", "lavfi", "-i", "color=c=black:s=64x64:d=2",
                "-shortest", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac",
            ])
            .arg(&src)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !made {
            eprintln!("跳过：造不出测试素材");
            return;
        }

        for speed in [0.5_f64, 2.0_f64] {
            let out = dir.join(format!("sped-{speed}.m4a"));
            Ffmpeg::assemble_audio_from_edl(
                src.to_str().unwrap(),
                &[(0.0, 2.0, 0.0, speed)],
                out.to_str().unwrap(),
                1.0,
            )
            .unwrap_or_else(|e| panic!("speed={speed} 应当能拼出来: {e:#}"));

            // -- 时长：**用解码后的样本数**（`mux_checked` 的同一口径）--
            //
            // ⚠️ **不能用容器时长**：`asetpts=PTS/speed` 那种错法会让**容器时长**
            // 看着像对的（实测 `PTS/0.5` 的容器 3.98s、`PTS/2.0` 的 1.01s，
            // 与 `atempo` 的 3.99s / 1.01s 几乎一样），而**解出来的样本数一个都没变**
            // —— 因为它根本没动样本，只是骗了时间戳。
            // 我第一版就是拿容器时长卡，结果 `asetpts` 变异**照样绿**（自己实测到并推翻）。
            // 换成解码后样本数之后，`asetpts` 变异必红：恒为 96000（2.000s），
            // 而期望是 192000 / 48000（speed=0.5 / 2.0）。
            let want_samples = (2.0 / speed * 48000.0).round() as i64;
            let got = Ffmpeg::probe_audio_samples(out.to_str().unwrap(), 48000)
                .expect("数得出样本") as i64;
            // 容差照 `mux_checked` 的量级：8 个 AAC 帧（编码器边界补齐实测 ≤3.25 帧）。
            let slack = 8 * 1024;
            assert!(
                (got - want_samples).abs() <= slack,
                "speed={speed}：音轨应当约 {want_samples} 个样本，实得 {got}                  （96000 = 换了 asetpts 缩放，只改时间戳、样本没动；规格 §9 R6）"
            );

            // -- 基频：必须还是 440（±3%）--
            let hz = estimate_fundamental_hz(out.to_str().unwrap()).expect("估得出基频");
            assert!(
                (hz - 440.0).abs() / 440.0 < 0.03,
                "speed={speed}：基频应当仍是 440Hz，实得 {hz:.1}Hz                  —— 偏离说明改了采样率（`asetrate` 那一路会把基频乘 speed = 花栗鼠音）"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 估一条音轨的**基频**（过零率法，够用且零依赖）。
    ///
    /// 取中段 0.5 秒（避开编解码的起止毛刺），数过零次数：
    /// `频率 = 过零次数 / 2 / 时长`。只对**准正弦**有效，而夹具就是正弦
    /// —— 换成音乐信号要改用 FFT 峰值，那是另一件事。
    fn estimate_fundamental_hz(path: &str) -> Option<f64> {
        let dir = std::env::temp_dir();
        let wav = dir.join(format!(
            "_polish_pitch_{}_{}.wav",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let ok = yekitrim_core::process::cmd("ffmpeg")
            .args(["-v", "error", "-y", "-i", path, "-vn", "-f", "wav", "-ar", "48000", "-ac", "1"])
            .arg(&wav)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            let _ = std::fs::remove_file(&wav);
            return None;
        }
        let bytes = std::fs::read(&wav).ok()?;
        let _ = std::fs::remove_file(&wav);
        let data = bytes.windows(4).position(|w| w == b"data")?;
        let pcm = &bytes[data + 8..];
        let samples: Vec<i16> = pcm
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect();
        if samples.len() < 48000 {
            return None;
        }
        // 中段 0.5 秒。
        let mid = samples.len() / 2;
        let seg = &samples[mid..mid + 24000];
        let mut crossings = 0usize;
        for w in seg.windows(2) {
            // 过零（含"正好落在 0"的退化情况：按符号位判，与音频惯例一致）。
            if (w[0] >= 0) != (w[1] >= 0) {
                crossings += 1;
            }
        }
        // 0.5 秒里有 `crossings` 次过零 ⇒ 每周期两次 ⇒ 频率 = crossings / 2 / 0.5。
        Some(crossings as f64 / 2.0 / 0.5)
    }

    // =====================================================================
    // 叠化的音轨交叉淡化（第 7 批 / task-10）
    // =====================================================================

    /// **`plan_dissolve_fades` 的逐值判据**（纯逻辑，不依赖 ffmpeg）。
    ///
    /// 钉四件事：
    ///  ① 转场**骑在边界上**：出段往后多播半窗、入段往前多留半窗（`O/2` 各一份）；
    ///  ② 两段都挂**整窗**（`fade_len = O`）—— 各挂半窗就等于"各在自己那侧边缘淡"，
    ///     实测那不是交叉淡化（过零数与硬切一模一样）；
    ///  ③ 下标按 `out_start` **排序后**的位置落，不是传入顺序；
    ///  ④ 空 `fades` ⇒ 全 0（无转场工程与非转场那条腿**逐字节相同**）。
    #[test]
    fn plan_dissolve_fades_straddles_the_boundary() {
        // 传入顺序 = 成片顺序（out_start 0 / 4 / 8）。
        let clips = [
            (0.0_f64, 2.0_f64, 0.0_f64, 1.0_f64),
            (2.0, 4.0, 4.0, 1.0),
            (4.0, 6.0, 8.0, 1.0),
        ];
        // 只有第 2→3 段之间有叠化，重叠 0.5s。
        let (lead, flen, tail) = Ffmpeg::plan_dissolve_fades(&clips, &[(1, 2, 0.5)]);
        assert_eq!(lead, vec![0.0, 0.0, 0.25], "入段往前多留半窗：{lead:?}");
        assert_eq!(flen, vec![0.0, 0.5, 0.5], "两段都挂整窗：{flen:?}");
        assert_eq!(tail, vec![0.0, 0.25, 0.0], "出段往后多播半窗：{tail:?}");

        // ③ 乱序：传入顺序是 [0,2s)@0 / [0,1s)@8 / [1,2s)@4，而 out_start 排序后是 0,2,1。
        //    fades 用的是**传入**下标，落点必须是**排序后**的位置。
        let shuf = [
            (0.0_f64, 2.0_f64, 0.0_f64, 1.0_f64),
            (0.0, 1.0, 8.0, 1.0),
            (1.0, 2.0, 4.0, 1.0),
        ];
        let (lead, flen, tail) = Ffmpeg::plan_dissolve_fades(&shuf, &[(1, 2, 0.5)]);
        // 排序后顺序 = [传入 0, 传入 2, 传入 1]。
        // 入段 = 传入 2 ⇒ 位置 1（往前多留）；出段 = 传入 1 ⇒ 位置 2（往后多播）。
        assert_eq!(lead, vec![0.0, 0.25, 0.0], "乱序下入段要落在排序后的位置：{lead:?}");
        assert_eq!(flen, vec![0.0, 0.5, 0.5], "乱序下两段都挂整窗：{flen:?}");
        assert_eq!(tail, vec![0.0, 0.0, 0.25], "乱序下出段要落在排序后的位置：{tail:?}");

        // ④ 空 = 全 0。
        let (lead, flen, tail) = Ffmpeg::plan_dissolve_fades(&clips, &[]);
        assert_eq!(lead, vec![0.0, 0.0, 0.0]);
        assert_eq!(flen, vec![0.0, 0.0, 0.0]);
        assert_eq!(tail, vec![0.0, 0.0, 0.0]);

        // 垃圾输入不许 panic、也不许留下窗口。
        let (lead, flen, tail) =
            Ffmpeg::plan_dissolve_fades(&clips, &[(9, 9, 0.5), (0, 1, 0.0), (0, 1, f64::NAN)]);
        assert_eq!(lead, vec![0.0, 0.0, 0.0], "越界/0/NaN 的边必须被忽略：{lead:?}");
        assert_eq!(flen, vec![0.0, 0.0, 0.0], "越界/0/NaN 的边必须被忽略：{flen:?}");
        assert_eq!(tail, vec![0.0, 0.0, 0.0], "越界/0/NaN 的边必须被忽略：{tail:?}");
    }

    /// ⚠️ **真媒体的交叉淡化判据**：重叠窗口里两段的频率**同时在响**（不是硬切），
    /// 而**总样本数一个都不变**。
    ///
    /// # 为什么"总长不变"是这条判据的一半
    ///
    /// 交叉淡化最容易写错的方式是"把窗扩出去" —— 那会让音轨变长，
    /// 而合流前那条 `mux_checked` 断言立刻翻面（成片被 `-shortest` 截）。
    /// 所以这里同时钉：**淡化发生了** 且 **总长 = 各段之和**。
    ///
    /// # 判据怎么读（⚠️ 素材要**特意设计**，否则这条用例会**假绿**）
    ///
    /// 源是 8 秒：`[0,4)` = 1000Hz、`[4,8)` = 4000Hz。
    /// A = 源 `[2,4)`（整段 1000Hz）、B = 源 `[4.25, 6.25)`（整段 4000Hz）。
    ///
    /// ⚠️ **B 必须从 `4.25` 起，不能从 `4.0` 起**：交叉淡化会把 B 的 `atrim` 往前
    /// **多留 0.25s**（`lead_in`），从 4.0 起就会读到 `[3.75,4.0)` —— 那还是 **1000Hz**，
    /// 于是重叠窗口里两段响的是**同一个频率**，过零数与硬切**一模一样**，
    /// 这条判据就变成空转（我第一版素材正是这个错，跑出来 19999 == 19999）。
    /// 从 4.25 起，B 的整个扩窗 `[4.0, 6.25)` 都是 4000Hz。
    ///
    /// 于是重叠窗口 `[1.75, 2.25]` 里：A 是 1000Hz、B 是 4000Hz，**两个都在**
    /// ⇒ 整条过零数**明显高于**硬切版（硬切版里任一瞬间只有一个频率）。
    #[test]
    fn assemble_audio_crossfades_without_changing_length() {
        if which_ffmpeg().is_none() {
            eprintln!("跳过：PATH 上没有 ffmpeg");
            return;
        }
        let dir = std::env::temp_dir().join(format!("_polish_xfade_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let src = dir.join("src8.mp4");
        let made = yekitrim_core::process::cmd("ffmpeg")
            .args([
                "-v", "error", "-y",
                "-f", "lavfi", "-i", "sine=frequency=1000:sample_rate=48000:duration=4",
                "-f", "lavfi", "-i", "sine=frequency=4000:sample_rate=48000:duration=4",
                "-f", "lavfi", "-i", "color=c=black:s=64x64:d=8",
                "-filter_complex", "[0:a][1:a]concat=n=2:v=0:a=1[a]",
                "-map", "[a]", "-map", "2:v", "-shortest",
                "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac",
            ])
            .arg(&src)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(made, "造不出测试素材（造不出来就必须失败，不能静默跳过 —— 那会让本用例假绿）");

        // A = 源 [2,4)（1000Hz）→ 成片 [0,2)；B = 源 [4.25,6.25)（4000Hz）→ 成片 [2,4)。
        let clips = [
            (2.0_f64, 4.0_f64, 0.0_f64, 1.0_f64),
            (4.25, 6.25, 2.0, 1.0),
        ];
        // 重叠 0.5s ⇒ A 往后多播 0.25s、B 往前多留 0.25s。
        let out = dir.join("xfade.m4a");
        Ffmpeg::assemble_audio_from_edl_fades(
            src.to_str().unwrap(),
            &clips,
            &[(0, 1, 0.5)],
            out.to_str().unwrap(),
            1.0,
        )
        .expect("交叉淡化应当能拼出来");

        // ① 总长不变：**用解码后的样本数**（与 `mux_checked` 同一口径），不是容器时长。
        let got = Ffmpeg::probe_audio_samples(out.to_str().unwrap(), 48000).expect("数得出样本");
        let want = 4.0 * 48000.0;
        let slack = 8 * 1024; // 8 个 AAC 帧（与另两条用例同量级）
        assert!(
            (got as f64 - want).abs() <= slack as f64,
            "交叉淡化不许改总长：期望约 {want} 样本（4.0s），实得 {got}              （扩窗不改回来会让它变长，合流前那条断言会翻面）"
        );

        // ② 重叠窗口里**两个频率都在** ⇒ 整条过零数明显高于硬切版。
        let zc_all = count_zero_crossings(out.to_str().unwrap()).expect("数得出过零");
        let hard = dir.join("hard.m4a");
        Ffmpeg::assemble_audio_from_edl_fades(
            src.to_str().unwrap(),
            &clips,
            &[],
            hard.to_str().unwrap(),
            1.0,
        )
        .expect("硬切版应当能拼出来");
        let zc_hard = count_zero_crossings(hard.to_str().unwrap()).expect("数得出过零");
        // ⚠️ **阈值的来处**（不是拍的）：多出来的过零只出现在边界**前**那半个窗口
        // `[1.75, 2.00)` —— 边界**后**那半（`[2.00,2.25)`）A 自己的尾巴也已经是 4000Hz
        // （A 的扩窗读到了源里跨过音调边界的那一段），两边同频，过零数不变。
        //
        // 该半窗 0.25s、基频 1000Hz ⇒ 硬切版约 `1000×2×0.25 = 500` 次（实测 500）；
        // 淡化版里 B 以 4000Hz 叠进来，实测 **814**。所以阈值取 `+200`：
        // 远离两侧（硬切 500 / 淡化 814），又小到"只多几个样本的假象"骗不过去。
        // 实测数据（本机）：淡化 20307 / 硬切 19993，差 **314**。
        assert!(
            zc_all > zc_hard + 200,
            "交叉淡化窗口里 1000Hz 与 4000Hz 要**同时存在** ⇒ 过零数应高于硬切版；             实得 淡化={zc_all} / 硬切={zc_hard}（差不到 200 说明没真的叠上）"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 数一条音轨的**过零次数**（解码成 WAV 后逐样本判符号）。
    ///
    /// `afade` 只改振幅、不改频率 ⇒ 过零数不受淡化包络影响，
    /// 于是它能当"窗口里有哪些频率在响"的判据（见上一条用例）。
    fn count_zero_crossings(path: &str) -> Option<usize> {
        let wav = std::env::temp_dir().join(format!(
            "_polish_zc_{}_{}.wav",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let ok = yekitrim_core::process::cmd("ffmpeg")
            .args(["-v", "error", "-y", "-i", path, "-vn", "-f", "wav", "-ar", "48000", "-ac", "1"])
            .arg(&wav)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            let _ = std::fs::remove_file(&wav);
            return None;
        }
        let bytes = std::fs::read(&wav).ok()?;
        let _ = std::fs::remove_file(&wav);
        let data = bytes.windows(4).position(|w| w == b"data")?;
        let pcm = &bytes[data + 8..];
        let mut crossings = 0usize;
        let mut prev: Option<i16> = None;
        for c in pcm.chunks_exact(2) {
            let v = i16::from_le_bytes([c[0], c[1]]);
            if let Some(p) = prev {
                if (p >= 0) != (v >= 0) {
                    crossings += 1;
                }
            }
            prev = Some(v);
        }
        Some(crossings)
    }

    /// `ffmpeg` 在不在 PATH 上（不在就跳过真实媒体的用例，而不是误报失败）。
    fn which_ffmpeg() -> Option<()> {
        yekitrim_core::process::cmd("ffmpeg")
            .arg("-version")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|_| ())
    }
}
