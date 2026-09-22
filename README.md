# framestrip

Turn a video into a short, deduplicated, timestamped frame strip.

`framestrip` decodes a video through the system `ffmpeg` binary
(`ffmpeg-sidecar`, no build-time linking), samples it at a fixed rate,
perceptually hashes every sampled frame, keeps a frame only when it differs
from the last kept one, thins to a hard frame cap by dropping the smallest
transitions, and returns every kept frame as JPEG with its source timestamp
and how long that screen stayed on.

It exists so a language model can look at a screen recording as "screen A held
16 s, toast at 00:24, screen B" instead of sixty near-identical frames.

```rust
let strip = framestrip::extract(Path::new("recording.mp4"), &framestrip::Options::default())?;
for frame in &strip.frames {
    println!("t={}ms held={}ms {} bytes", frame.ts_ms, frame.held_ms, frame.bytes.len());
}
```

Requires `ffmpeg` and `ffprobe` on `PATH`. Without them `framestrip::tooling()`
returns `Error::ToolingUnavailable` and nothing else runs.

## Design notes

- Consecutive-only comparison: returning to an earlier screen later in the
  recording is itself a fact, so it stays.
- Pure selection helpers (`select`) are unit-tested without ffmpeg; the
  integration tests generate fixtures with `ffmpeg -f lavfi` and skip when the
  binary is missing.

License: MIT.
