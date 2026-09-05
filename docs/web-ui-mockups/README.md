# V-Trim Web UI 风格样稿

**[`sakura-cream.html`](sakura-cream.html)（樱花奶油 · 浅色）是 V-Trim Web UI 的正式设计标准** —— 正式 UI 直接采用该浅色主题与组件语言（顶部 pill 导航 + 手账贴纸卡片 + 大圆角），不再映射为暗色。设计规范见主设计文档 [`docs/web-ui-design.md`](../web-ui-design.md) v0.2（第 4 章 Design Tokens 全量取自该样稿）。

| 目录 | 定位 | 演示页面 | 布局特征 | 预览 PNG |
|------|------|----------|----------|----------|
| [`sakura-cream.html`](sakura-cream.html) | **设计标准**（配色/组件/布局基准） | P2 项目列表 | 顶部 pill 导航 + 手账贴纸卡片网格 + 丝带角标 + 圆点纹理 | [`sakura-cream.png`](sakura-cream.png) |
| [`mint-soda.html`](mint-soda.html) | 布局参考（数据表格） | P3 事件管理 (Tab2) | 左侧导航 + 紧凑数据表格 + KV 概览条 | [`mint-soda.png`](mint-soda.png) |
| [`sky-glow.html`](sky-glow.html) | 布局参考（仪表盘） | P1 总览仪表盘 | 玻璃拟态卡片 + 双列仪表网格 | [`sky-glow.png`](sky-glow.png) |

> mint-soda / sky-glow 仅借鉴其**数据表格 / 仪表盘的布局组织方式**，配色、圆角与组件语言一律以 sakura-cream 为准。

## 快速查看

```bash
# 浏览器直接打开 HTML（推荐）
start sakura-cream.html

# 或重新生成 PNG（需本机 Chrome）
"C:\Program Files\Google\Chrome\Application\chrome.exe" --headless=new --disable-gpu \
  --screenshot="sakura-cream.png" --window-size=1440,900 \
  "file:///F:/para/Code/V-Trim/docs/web-ui-mockups/sakura-cream.html"
```

## 三份为何不同（不只是换色）

| 维度 | Sakura Cream（标准） | Mint Soda（参考） | Sky Glow（参考） |
|------|--------------|-----------|----------|
| 信息密度 | 低 · 大卡片突出内容 | 高 · 表格容纳 12+ 事件 | 中 · 仪表盘快览 |
| 布局骨架 | 顶部导航 + 卡片网格 | 左导航 + 全宽工具表 | 左导航 + 双列仪表 |
| 组件语言 | 贴纸/丝带/虚线描边 | 方形圆角/行内操作/hover | 毛玻璃/渐变色条/填充动画 |
| 适合场景 | 浏览型项目概览 | 数据型事件/片段管理 | 运营型监控仪表 |
| 主色 | #F682B8 樱花粉 | #45BEAA 青碧 | #7FA8F0 星蓝 |
