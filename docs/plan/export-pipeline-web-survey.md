# Export pipeline alternatives — web survey (2026)

Baseline: 26 ms/frame in-page canvas loop, ~75 ms/frame per chunk process × 4, 25–40 fps aggregate at
1080p, ffmpeg `h264_nvenc` on an RTX 4070. (`web_search` was unavailable — no API key — so findings come
from direct fetches.)

**Reframing:** NVENC is *not* the bottleneck — Ada H.264 1080p sustains ~291 fps at P5 per NVENC
engine, ~10× your 30 fps need. Of the 75 ms, 26 ms is the canvas loop; the rest is the JPEG round trip.

## 1. Candidate approaches

| # | Approach | Speed | Effort | Mismatch risk | New deps |
|---|---|---|---|---|---|
| A | Canvas-2D + in-page `VideoEncoder` + MediaBunny mux (no JPEG pipe) | High | Med | Low | `mediabunny` |
| B | Cached ¼-scale backdrop blur, `OffscreenCanvas` layers, Worker loop | Med–high | Low | Low | none |
| C | Real GPU in headless (`--headless=new`, drop SwiftShader flag) | Unknown; large if currently software | Very low | None | none |
| D | wgpu/Vulkan compositor → raw frames → `h264_nvenc` | High ceiling | High | High (text/GIF/blur redone) | none |
| E | Hybrid: wgpu background, libass or canvas captions | High | High | Med | none |
| F | Chrome-free: libass captions + ffmpeg GPU blur/overlay | Med (text ≈ free) | Med–high | High (animated highlight) | none |
| G | NVENC straight from Rust | Marginal over D | High | Low | unverified |

## 2. Concrete numbers and facts

1. **Ada NVENC H.264, 1920×1080 YUV420 8-bit, fps per engine:** P1 CBR/LL 910 · P3 VBR/HQ 647 · P5 CBR/LL 291 · P5 VBR/HQ 283 · P7 VBR/HQ 211 (RTX 4090 @2415 MHz; scales with clock) — <https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvenc-application-note/index.html>
2. **RTX 4070 = 1 NVENC engine.** NVIDIA's matrix lists **12** max concurrent sessions; the SDK 13.0 app note still says **8 per system** for non-qualified (GeForce) GPUs — the sources disagree. **Split Frame Encoding** is Ada + SDK 12.0, **HEVC/AV1 only**, needs ≥2 engines (4070 Ti up), never H.264 — <https://developer.nvidia.com/video-encode-and-decode-gpu-support-matrix-new> · <https://obsproject.com/kb/advanced-nvenc-options>
4. **WebCodecs hardware H.264 on Windows:** a Chromium issue reports `prefer-hardware` `isConfigSupported` rejecting **>1080p@30** on tested GTX 1080 / RTX 2070S / 3070 Ti — <https://issues.chromium.org/issues/40249559> (old; re-verify)
5. **GPU-accelerated canvas blur is the slow path:** Chromium bug "Blur filters on GPU accelerated canvas are very slow" (ex crbug 847594) — <https://issues.chromium.org/issues/41391683>
6. **`--headless=new` (Chrome 112+) is the real browser** — it only skips showing platform windows; everything else is available. Old headless became the separate `chrome-headless-shell` binary in Chrome 132 — <https://developer.chrome.com/docs/automation-and-testing/headless>
7. **Puppeteer silently injects `--use-angle=swiftshader-webgl`** (software GL), overriding your ANGLE choice; drop it via `ignoreDefaultArgs`. Working set: `--headless=new --ignore-gpu-blocklist --use-angle=d3d11 --disable-gpu-sandbox` — <https://tigerabrodi.blog/how-to-get-webgpu-in-headless-chrome-on-cloud-gpus>
8. **`VideoEncoder`** is secure-context-only, Chrome 94+, and **works in a Dedicated Worker**; `encodeQueueSize` + `dequeue` are the documented backpressure signals — <https://developer.mozilla.org/en-US/docs/Web/API/VideoEncoder>
9. **`mp4-muxer` is deprecated** for **Mediabunny** — `CanvasSource({codec:'avc', quality})`, frame-rate-snapped timestamps, streaming `StreamTarget` — <https://github.com/Vanilagy/mp4-muxer> · <https://mediabunny.dev/guide/writing-media-files>
10. **Ships in production:** VideoFlow renders in headless Chromium, encodes via WebCodecs, muxes with MediaBunny, and calls the screenshot→ffmpeg-stdin pattern "fundamentally inefficient" (round trip + JPEG→H.264 re-encode). No numbers published — <https://videoflow.dev/blog/zero-ffmpeg-video-pipelines-webcodecs>
10. **libplacebo has no blur primitive** (no built-in blur; custom GLSL shaders only); ffmpeg's `libplacebo` filter adds `inputs`/`idx` compositing, still no blur — <https://github.com/haasn/libplacebo> · <https://ayosec.github.io/ffmpeg-filters-docs/8.0/Filters/Video/libplacebo.html>
11. **Native building blocks:** BtbN Windows builds enable libplacebo (<https://raw.githubusercontent.com/BtbN/FFmpeg-Builds/master/scripts.d/50-libplacebo.sh>); ffmpeg ships GPU blurs `gblur_vulkan`/`avgblur_vulkan`/`boxblur_opencl` plus `scale_cuda`/`overlay_cuda` (<https://ffmpeg.org/ffmpeg-filters.html>); **libass** does `\bord` outline, `\blur` (capped at 100), per-span colour, CJK (<https://github.com/libass/libass>); **cosmic-text** has per-span styles and CJK but **no stroke**, and glyphon wraps it for wgpu with **no benchmarks** (<https://github.com/pop-os/cosmic-text>)

## 3. Could NOT verify (do not assume)

- Whether current Chrome on Windows returns NVENC-backed H.264 from `isConfigSupported` at 1080p60; item 4 is years old.
- Whether `--enable-accelerated-2d-canvas` / `--enable-gpu-rasterization` speed up `ctx.filter='blur(Npx)'` in `--headless=new`. No benchmark either way; the one Chromium bug found says GPU canvas blur is *slow*.
- Any ms/frame for canvas blur at 1080p, any OffscreenCanvas+Worker fps delta, whether blur is quadratic in radius, and a primary-source NVENC-vs-libx264 BD-rate/VMAF number (rigaya's `vq_results` was unreachable; a Gough SSIM PDF exists but yielded no figures).
- Whether **gyan.dev** builds include libplacebo (BtbN confirmed; gyan not), and whether *your* ffmpeg has `libplacebo`/`gblur_vulkan`/`scale_cuda` — check `ffmpeg -filters`.
- libass burn-in cost per frame; Rust NVENC binding maturity; whether `file://` is a WebCodecs secure context (spec says potentially trustworthy; item 7 says it failed).
- Whether `EncodedVideoChunk` streams from separate per-chunk renders concatenate cleanly — parameter-set/GOP continuity unverified.

## 4. Recommended experiment order

**1. Is the browser on the GPU? (hours).** Add `ignoreDefaultArgs:['--use-angle=swiftshader-webgl']`,
`--headless=new`, `--ignore-gpu-blocklist`, `--disable-gpu-sandbox`; dump `chrome://gpu` from inside that
process. *Kill if:* it already reported hardware canvas and the 26 ms does not move.

**2. Kill the JPEG round trip (days).** Same draw code; `new VideoFrame(canvas,{timestamp})` →
`VideoEncoder('avc', prefer-hardware)` → MediaBunny `Output`/`StreamTarget` → one MP4 blob.
*Kill if:* it lands on software (log the `hardwareAcceleration` `isConfigSupported()` returns; check
`nvidia-smi encodersessions` during encode) — CPU pegged, zero sessions.

**3. Cheap canvas wins (hours).** Blur the backdrop once at ¼ scale; cache static layers in an
`OffscreenCanvas`; move the loop to a Worker. *Kill if:* a per-operation timer shows the 26 ms did not
move. Target <12 ms.

**4. Only if 1–3 miss realtime: wgpu compositor (weeks).** Incrementally — backdrop, blur, drawImage,
stickers first, captions last. Feed NVENC via `scale_cuda` to NV12, not CPU RGBA→NV12 swscale.
*Kill if:* wgpu render ms/frame is not ≥3× the Step-3 canvas number — re-implementing text and GIF
timing will not pay for a 1.5× win.
