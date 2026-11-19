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
- Expanded Collection categories reuse their grouped counts until library
  metadata changes. Rows outside the scroll viewport skip painting and text
  layout while retaining their widget IDs and keyboard-focus registration.
- Smoothed channel/crossfader, trim and isolator gains targeting zero clear a
  subnormal tail at the end of the buffer. Normal gain values and the audible
  fade remain unchanged. This avoids repeatedly processing a tiny, silent gain
  on older Intel processors without changing the floating-point mode.

RustFFT and Candle's matrix backend already select supported SIMD at runtime.
Public builds retain their existing CPU compatibility, dependencies and
numerical backends. The changes keep the existing floating-point mode.

## Simulate one deck without music or audio hardware

From the source checkout, after installing the normal native build dependencies:

```sh
cargo test --manifest-path app/Cargo.toml --release --locked cpu_playback_cost -- --ignored --nocapture --test-threads=1
cargo test --manifest-path app/Cargo.toml --release --locked cpu_overview_cost -- --ignored --nocapture --test-threads=1
cargo test --manifest-path app/Cargo.toml --release --locked cpu_zero_gain_cost -- --ignored --nocapture --test-threads=1
cargo test --manifest-path app/Cargo.toml --release --locked cpu_collection_categories_cost -- --ignored --nocapture --test-threads=1
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

The follow-up reports use the same portable release toolchain on that Xeon,
pinned to one logical CPU. The category report generates 100,000 tracks and
compares the former row painter/count grouping with the cached implementation,
including actual egui widgets and tessellation. Three runs of five warm frames
follow the separately measured cold frame. Every row still participates in
keyboard focus; this is painting culling rather than widget virtualization.

| Measured component | Before | After |
| --- | --- | --- |
| Expanded Artist category, warm frame | 275.934 ms | 74.730 ms |
| All Collection categories expanded, warm frame | 570.498 ms | 114.116 ms |
| Zero channel fader, four-state callback, 20 simulated seconds, 44.1 → 48 kHz | 0.292 s | 0.031 s |

The category component was 3.7–5 times faster. The muted callback case used
about nine times less elapsed time after two seconds of warmup; ordinary unity
controls produced identical output checksums. Its absolute compute cost was
about 1.46% of one core before and 0.16% after, excluding the device and window.
This silent-gain case does not establish a cause for high CPU with untouched
controls. Category savings depend on the expanded groups and library contents;
100,000 distinct artists is a scaling stress case.

## Check sustained Linux process CPU

In the usual Linux `top` display, 100% means one logical CPU. On a four-core
machine, a process at 80% uses roughly 0.8 of one core; monitors using a
whole-machine percentage have a different scale.

Let loading and initial analysis settle, then compare the same track and audio
settings with a smaller window, the Simple Player, and expanded versus collapsed
Collection categories. These comparisons distinguish display and library work
from the audio callback. Keep screen refresh and CPU power settings fixed.

`glxinfo -B` identifies the renderer. `llvmpipe` or `softpipe` indicates software
graphics; an `NVE4` renderer identifies the Nouveau NVIDIA hardware path. Graphics
driver CPU time belongs to Recordbox's process too, so high process CPU alone
does not identify which part of the application is busy.

Inspect threads during playback with:

```sh
top -H -p $(pgrep -n -x recordbox)
```

The main thread has the same thread ID as the process ID. Compare it with
`cpal_alsa_out` and any `llvmpipe-*` workers. A busy main thread points toward
interface or driver work; a busy ALSA output thread needs device/callback
investigation. This is a diagnostic distinction, not an automatic diagnosis.

The screenshot benchmark requests continuous repaint itself. Its update timings
exclude GPU/driver work and cannot establish idle CPU usage. Whole-process tests
on a virtual display with software graphics also do not reproduce an older
physical GPU or its driver. Collection and zero-gain reports measure specific
components, not a promised reduction in total process CPU.
