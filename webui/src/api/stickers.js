// 表情包（本地贴纸库）：**用户级资产**，接口在 /api/polish/stickers/* 下，登录用户都能用。
// 库固定在 <程序目录>/resources/stickers/，一个子文件夹 = 一个包；
// 「构建」= LLM 把文件名压成语义 tag 索引。
//
// ⚠️ **2026-10-09 从 `/api/admin/stickers/*` 挪来**（用户口径「素材库是用户级、跟随用户」）：
// 这条路径以及 `fileUrl` 拼的 URL 都改了 —— 改一处必须改另一处，否则 <img> 全部 404
// 而界面上只是"图裂了"，看不出是路由问题。
import { api } from './client'

const q = (s) => encodeURIComponent(s)

/** 上传上限的**兜底值**（服务端 `[stickers] upload_max_mb` 默认 1024 MiB = 1 GiB）。
 *
 * 真正的值由 `limits()` 现取 —— 上限是配置项，写死一份必然漂（旧版写死 64 MiB，
 * 用户 500 MB 的包连前端预检都过不去）。取不到时才用这个兜底。
 *
 * 上传前先在浏览器里挡一道：服务端超限回的是 413 + JSON（Content-Length 预检那条）或
 * 纯文本（axum 硬顶那条），而 client.js 只认 JSON，会把后者报成"接口不存在或返回格式错误"。 */
export const DEFAULT_UPLOAD_MAX_BYTES = 1024 * 1024 * 1024

export const stickersApi = {
  /** 包列表（磁盘为准，手工丢进库里的文件夹也会出现） */
  list: () => api('/api/polish/stickers', { label: '读取表情包列表' }),
  /** 当前账号的贴纸包体积上限（字节）。上传前预检用 —— 值来自 config.toml 的 [stickers]。 */
  limits: () => api('/api/polish/stickers/limits', { label: '读取贴纸包上限' }),
  /** 包内文件 + 每个文件命中的 tags（展开预览时才拉） */
  detail: (pack) => api(`/api/polish/stickers/${q(pack)}`, { label: `读取表情包 ${pack}` }),
  /** 上传一个 zip 建成新包（请求体就是文件原始字节）；build=true 时顺带起 LLM 构建任务。
   *
   * 名字走查询串（body 被字节占满了）。`Content-Type` 必须显式覆盖 —— `api()` 默认发
   * `application/json`，不覆盖就成了"二进制套 JSON 头"。 */
  uploadPack: (file, name, build) =>
    api(`/api/polish/stickers/upload?name=${q(name)}&build=${build ? 1 : 0}`, {
      method: 'POST',
      body: file,
      headers: { 'Content-Type': 'application/octet-stream' },
      label: '上传表情包'
    }),
  /** 从服务器上的一个目录导入成新包（无 UI，留给 curl；见 server/stickers.rs 模块注释） */
  importPack: (dir, name, build) =>
    api('/api/polish/stickers/import', {
      method: 'POST',
      body: JSON.stringify({ dir, name, build }),
      label: '导入表情包'
    }),
  /** LLM 压缩构建（覆盖 <包名>.toml 并注册进索引） */
  build: (pack) =>
    api(`/api/polish/stickers/${q(pack)}/build`, {
      method: 'POST',
      body: '{}',
      label: `构建表情包 ${pack}`
    }),
  /** 删掉整个包（目录 + 清单 + 索引项） */
  remove: (pack) =>
    api(`/api/polish/stickers/${q(pack)}`, { method: 'DELETE', label: `删除表情包 ${pack}` }),
  /** 包内图片 URL（给 <img> 直连，别走 api()） */
  fileUrl: (pack, name) => `/api/polish/stickers/${q(pack)}/files/${q(name)}`
}
