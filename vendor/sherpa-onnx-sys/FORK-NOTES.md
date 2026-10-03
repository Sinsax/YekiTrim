# vendor/sherpa-onnx-sys — YekiTrim fork

A verbatim copy of **sherpa-onnx-sys 1.13.3** from crates.io, with **`build.rs` modified only**.
`src/**`, `LICENSE` are byte-for-byte upstream. 63 KB of text, no binaries.

Wired up by the workspace root `Cargo.toml`:

```toml
[patch.crates-io]
sherpa-onnx-sys = { path = "vendor/sherpa-onnx-sys" }
```

## Why fork it

Upstream's `build.rs` resolves the native libs in this order:

```
SHERPA_ONNX_LIB_DIR
  > target/sherpa-onnx-prebuilt/<archive-stem>/lib   (cache — exists => return, wins)
  > SHERPA_ONNX_ARCHIVE_DIR
  > download https://github.com/k2-fsa/sherpa-onnx/releases/download/...
```

Three problems, all of them silent:

1. The official prebuilt archive links **espeak-ng (GPL-3.0-or-later)** unconditionally. YekiTrim is
   ASR-only and never calls TTS, so it is a dead dependency that still turns the shipped binary into
   a GPL combined work.
2. The **cache outranks the archive you point at**. On any machine that has ever built with the
   official archive, `target/sherpa-onnx-prebuilt/` already satisfies the second rule, so swapping
   the archive back changes nothing and nothing says so.
3. Upstream verifies **nothing** about what it downloads, and its URL is a single hardcoded constant.

The previous fix lived outside cargo (a PowerShell wrapper that deleted the cache, ran
`cargo clean -p sherpa-onnx-sys --profile release` — the `--profile` is load-bearing, without it it
cleans the *debug* profile and reports success — and then scanned the produced exe). Correct, but it
only protected people who remembered to call the wrapper. `cargo build` and IDE builds bypassed it
entirely. This fork moves the protection into the one place every build must pass through.

## The deltas, exactly

1. `RELEASE_BASE_URL` → `https://github.com/Sinsax/sherpa-onnx-prebuilt-no-tts/releases/download`
2. New constants `PINNED_ARCHIVE_NAME` / `PINNED_ARCHIVE_SHA256`, and a guard that refuses to build
   for any target/link-mode other than the pinned Windows x64 static archive (which would otherwise
   silently fall back to the official GPL archive).
3. **sha256 verification** (`sha2` build-dependency) on the local-archive copy and on the download;
   a mismatching file is deleted, never left in the cache for the next build to trust.
4. `assert_placeholders_are_placeholders()` — the three stub libs listed unconditionally by
   `SHERPA_ONNX_STATIC_LIBS` (`espeak-ng.lib`, `piper_phonemize.lib`, `ucd.lib`) must exist and be
   **≤ 4096 bytes**. It runs on both paths: after extraction **and on the cache hit**, which is what
   closes problem 2 above.
5. **Transfer timeouts** on the download (`timeout_connect` 30 s, `timeout_read` 60 s) and removal of
   the `.part` file when a transfer fails. Upstream sets no timeouts, so a stalled download hangs the
   build indefinitely without printing anything — measured here behind a local proxy
   (`HTTP_PROXY=127.0.0.1:5441`): the transfer crawled to 66 MiB of 106 MiB, then stopped growing
   entirely, 0 bytes in 20 s, no error. `timeout_read` bounds the gap *between* reads rather than the
   whole transfer, so a slow-but-live link still succeeds. curl against the same URL at the same
   moment did 20 MiB in 1.5 s both directly and through that proxy, i.e. the stall is in the transfer,
   not the network being down — and it must therefore be a loud failure, not a 20-minute silence.

### Resyncing on an upstream bump

```bash
# 1. diff our build.rs against the new upstream one
diff vendor/sherpa-onnx-sys/build.rs <new-crate>/build.rs

# 2. copy src/** and LICENSE over verbatim, re-apply the five deltas above
# 3. update the pinned name/hash and, if the archive was rebuilt,
#    https://github.com/Sinsax/sherpa-onnx-prebuilt-no-tts Release v<version>
# 4. cargo metadata --format-version 1 > /dev/null   # refresh Cargo.lock
# 5. cargo build --release -p yekitrim-cli --locked
```

`build.rs` is 13 KB and the deltas are confined to the constants block and
`download_prebuilt_libs()`, so the diff stays small.

> ⚠️ **Line endings.** This repo has `core.autocrlf = true` and no `.gitattributes`, so the committed
> blobs are LF (faithful to upstream) but a **fresh checkout on Windows rewrites these files to
> CRLF**. Step 1's `diff` would then report every line as changed and hide the real deltas. Diff with
> `--strip-trailing-cr` (`diff --strip-trailing-cr vendor/.../build.rs <new-crate>/build.rs`), or
> normalise first. Cargo itself is unaffected — CRLF in `.rs` compiles fine.

### Verifying the fork does what it claims

```bash
cargo xtask scan                                  # product scan: the exe must carry no espeak-ng
rm -rf target/sherpa-onnx-prebuilt                # force a re-acquire + re-extract
cargo build --release -p yekitrim-cli --locked       # must download, verify, extract, then build
```

Manual bypass, for a deliberate reason only: `SHERPA_ONNX_LIB_DIR` is still honoured first and is
**not** content-checked, so pointing it at a self-built lib dir is the escape hatch. Setting
`SHERPA_ONNX_ARCHIVE_DIR` points at a local copy of the *pinned* archive (same hash required) — that
is the offline path.
