# Polish Render: SFX Priority, GIF Single-Play, Irasu Smart Duration

## Problem

Three issues in the vtrim-polish rendering pipeline:

1. **SFX jarring** — `sfxPlay()` fade-in is only ~30ms (6ms×5 steps); `fireSounds()` fires all same-time SFX in a 0.12s window, causing cacophony when emoji/irasu/shake/etc. trigger simultaneously.
2. **Emoji GIF loops** — `getActiveEmoji()` uses `elapsedMs % total` (modulo), causing GIF animation to loop forever instead of playing once.
3. **Static irasu duration** — current logic in `process.rs` unconditionally extends to the next subtitle end without a base-duration anchor or cap.

## Design

### 1. SFX Priority + Fade-in

**Priority table** (JavaScript, in `fireSounds`/`scheduleSound`):

| Priority | Event types | Default SFX |
|----------|-------------|-------------|
| 5 (highest) | shake, stutter, split | impact-light, pop-poke, flash-of-light-1 |
| 4 | pulse, blur, noise, zoom_bounce | pop-jump, pop-notice, pop-notice, (none) |
| 3 | emoji, irasu | pop-notice |
| 2 | overlay, hue_shift, color_shift, vignette, flash | (sfx field) |
| 1 (lowest) | sfx (pure sound) | (custom) |

**Scheduling:** `fireSounds(t)` groups sounds by time, per-group only plays the highest priority entry. Same priority → first registered wins.

**Fade-in:** 5×6ms → 8×12ms ≈ 96ms fade-in.

**Scope:** Pure frontend (`index.html`).

### 2. GIF Single-Play

**Current:**
```javascript
var loopPos = elapsedMs % total;  // loops forever
```

**New:**
```javascript
var loopPos = Math.min(elapsedMs, total - 1);  // hold last frame
```

Plays once, stays on last frame until element fades out.

**Scope:** Pure frontend (`index.html`).

### 3. Irasu Static Image Smart Duration

Replace unconditional extension in `process.rs` Step 2c with capped adaptive logic:

- Base duration from event config (e.g., LLM-generated 2.5s)
- Look for a subtitle **end** time within `[base_end, base_end + base_duration * 0.5]`
- If found → extend `duration` to that subtitle end
- If not found → keep base duration

**Pseudo:**
```rust
let base_end = et + *dur;
let max_extension = *dur * 0.5;

if let Some(ns) = subtitles.iter()
    .find(|s| s.end > base_end - 0.001
               && s.end <= base_end + max_extension
               && s.start > et)
{
    *dur = ns.end - et;
}
```

**Scope:** `crates/vtrim-polish/src/process.rs`.

## Files Changed

| File | Changes |
|------|---------|
| `crates/vtrim-polish/templates/index.html` | SFX priority + fade-in; GIF single-play |
| `crates/vtrim-polish/src/process.rs` | Irasu duration capped adaptive logic |
