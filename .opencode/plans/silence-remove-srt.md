# silence-remove SRT support

## Files to change

### 1. `crates/vtrim-core/src/cutting/mod.rs`

**Change A**: Add `detect_silence(input, duration) -> Result<Vec<(f64, f64)>>`

Run `ffmpeg -i input -af "silencedetect=n=-30dB:d={duration}" -f null -`, parse stderr:
- `silence_start: X.XXX` → set `current_start`
- `silence_end: X.XXX | silence_duration: X.XXX` → push `(start, end)`

Return list of `(start_sec, end_sec)` silence segments.

**Change B**: Modify `remove_silence` signature from `-> Result<()>` to `-> Result<Vec<(f64, f64)>>`

Body:
1. Call `detect_silence(input, duration)` to get segments
2. If empty, `std::fs::copy(input, output)` and return empty vec
3. Otherwise run same ffmpeg silenceremove command
4. Return the silence segments

**Change C**: Add `adjust_srt(input, output, silence_segments) -> Result<()>`

1. `read_to_string(input)` → `parse_srt()` → `Transcript`
2. For each `TranscriptSegment`, compute cumulative shift:
   - `shift_before_start = sum(e-s for all silence segments where e <= seg.start)`
   - `shift_before_end = sum(e-s for all silence segments where e <= seg.end)`
3. Adjust: `seg.start = (seg.start - shift_before_start).max(0.0)`, same for end
4. `write_srt(&transcript, output)`

### 2. `crates/vtrim-cli/src/main.rs`

**Change D**: In `Command::SilenceRemove` loop, after video processing:

```rust
let segments = vtrim_core::cutting::remove_silence(input, &output_path, *duration)?;
let srt_path = input.with_extension("srt");
if srt_path.exists() {
    let out_srt = output_path.with_extension("srt");
    vtrim_core::cutting::adjust_srt(&srt_path, &out_srt, &segments)?;
    tracing::info!("  字幕已调整: {}", out_srt.display());
}
```
