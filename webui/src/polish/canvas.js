// canvas.js — 输出画布参数的**唯一来源**（WebUI 侧）。
// Rust 侧的 SSOT 是 `_canvas` 契约（ProjectFile::canvas_contract / resolve_stage）：
// 横屏 1920×1080 为基准，竖屏 1080×1920；k = W/1920、kh = H/1080。
// 横屏画布 k=kh=1 —— 所有几何逐像素不变（零回归）。
// 引擎（engine.js）与预览（PolishPreview.vue）一律从这里取 {W,H,k,kh}，
// 与 templates/index.html 顶部的 CW/CH/CK/CKH 同一套换算，改一处必须同步另一处。

export const BASE_W = 1920
export const BASE_H = 1080

/**
 * 从 `_canvas` 契约（或整个 config）解析画布参数。
 * @param {object|null} canvasCfg config._canvas（也可直接传整个 config，内部自动取 _canvas）
 * @returns {{W:number,H:number,k:number,kh:number}}
 */
export function canvasOf(canvasCfg) {
  const c = canvasCfg && canvasCfg._canvas ? canvasCfg._canvas : canvasCfg || {}
  const W = Number(c.width) > 0 ? Number(c.width) : BASE_W
  const H = Number(c.height) > 0 ? Number(c.height) : BASE_H
  return { W, H, k: W / BASE_W, kh: H / BASE_H }
}

/**
 * 贴纸缩放倍率 = 短边/1080（与 placement.rs Frame::sticker_scale 一致）。
 * 横竖画布短边都是 1080 ⇒ 恒为 1，贴纸不随画布缩放而缩水
 * （字幕/弹幕字号同样不缩放，见 polish-vertical-canvas.md 决策表第 8 条）。
 */
export function stickerScale(cv) { return Math.min(cv.W, cv.H) / BASE_H }

/** 贴纸盒子边长（280 的短边缩放，与 placement.rs Frame::sticker_size 一致） */
export function stickerSize(cv) { return 280 * stickerScale(cv) }

/** 贴纸安全边距（32 的短边缩放，与 placement.rs Frame::margin 一致） */
export function stickerMargin(cv) { return 32 * stickerScale(cv) }

/** 贴纸上浮量（18 的短边缩放，与 placement.rs Frame::float_up 一致） */
export function stickerFloatUp(cv) { return 18 * stickerScale(cv) }
