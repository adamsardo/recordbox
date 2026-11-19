# Playback CPU and repeatable checks

Playing a track runs both the audio callback and an animated interface. Loading
an unprepared track can also queue analysis before Play is pressed. These costs
need separate measurements: a fast audio callback does not establish low CPU
usage for the window, graphics driver, decoder or background jobs.

The playback changes retain the existing animation cadence and audio controls:

- Each deck caches the overview waveform's static display data. Track loading,
  analysis replacement, width, display style and native waveform changes
  invalidate it. Playhead position, played shading, cues and markers stay live.
- Waveform layers use a small stack array rather than a heap allocation for
  every displayed column, including the moving detail view.
- A settled play/pause fade skips its unity-gain cosine; moving fades and jump
  transitions retain the original processing.
- Gate Comp reuses coefficients while its side, PARAM and device rate stay
  unchanged. This helps that optional effect; it is not an explanation for
  default playback with effects bypassed.

RustFFT and Candle's matrix backend already select supported SIMD at runtime.
Public builds retain their existing CPU compatibility, dependencies and
numerical backends. The changes keep the existing floating-point mode.

## Simulate one deck without music or audio hardware

From the source checkout, after installing the normal native build dependencies:

```sh
cargo test --manifest-path app/Cargo.toml --release --locked cpu_playback_cost -- --ignored --nocapture --test-threads=1
cargo test --manifest-path app/Cargo.toml --release --locked cpu_overview_cost -- --ignored --nocapture --test-threads=1
```

These opt-in reports have no timing pass/fail threshold. Ordinary CI exercises
audio behavior, cache invalidation and exact waveform geometry separately.

The callback benchmark generates stereo PCM in memory and calls the actual
renderer. It measures five repetitions of 20 simulated seconds, using 512-frame
buffers. The first cases are one deck with default controls, including a
44.1 kHz file on a 48 kHz output. Additional cases cover multiple decks and
effects. It excludes decoding, device/driver scheduling and the window.

The waveform benchmark measures the original and cached overview preparation
and mesh generation at changing playback positions. It distinguishes the first
cache build from later frames. It excludes GPU submission and display-driver
work. The correctness tests compare vertices, indices, colors and heights
against the original drawing path, including source changes and resizing.

## Interpret the measurements

Portable release measurements on an Intel Xeon Platinum 8573C used five-run
medians from adjacent matched runs. The unchanged renderer control varied by
less than 1%.

| Measured component | Before | After |
| --- | --- | --- |
| Default one-deck callback, 44.1 → 48 kHz, 512 frames | 17.93 µs | 14.83 µs |
| Default 3Band overview preparation and mesh, 180-second track, 1,126 columns | 0.194 ms/frame | 0.075 ms/frame |

The callback took 17% less elapsed time; the overview component was 2.60 times
faster. At 48 kHz the 512-frame playback deadline is 10.67 ms, so the callback
already used a small part of that budget. These wall-time measurements on a
shared cloud server exclude GPU/driver work and do not measure total
application CPU utilization. They cannot establish that a large process-wide
CPU increase has been reproduced or eliminated.

The original overview rebuilt whole-track columns and display transforms on
each repaint. Playing requests continuous repaint; the effective cadence can
follow a high-refresh display. Caching this unchanged work addresses a cost
that exists even with one deck and no effects. It does not slow the interface
or promise that every graphics driver will use less CPU by the same amount.

For a local comparison, use the same track, window size, display refresh rate,
device sample rate and controls. Measure after decoding and initial analysis
settle, then compare again immediately after loading an unprepared track.
Keep explicit thread settings and CPU power mode unchanged between runs.
Containers, hybrid Intel CPUs and laptops under thermal limits can behave
differently from the cloud Xeon. Model inference was not measured here.
