# webui/src/polish — 精修共享模块

- `api.js` — polish 端点调用 + `watchJob` 任务订阅（SSE 优先、轮询兜底）。页面唯一数据出口。
- `engine.js` — 从 `crates/yekitrim-polish/templates/index.html` Canvas 渲染器（真实出片路径）移植的纯函数特效状态机。
- `audio.js` — SFX 播放器（音量跟随响度曲线，与渲染混音同思路）。
- `heat.js` — 时间轴标尺带上那条**热度曲线**的口径（弹幕按条 + 字幕按字量 → 0~1）。纯函数、零依赖；
  与 Rust `process/punch.rs` 是"继承四条 + 两处有意不同"，详见模块头。

修改特效表现时先改 `templates/index.html`，再同步移植到这里，保持预览与出片一致。
