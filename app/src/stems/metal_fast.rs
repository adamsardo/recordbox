//! HTDemucs on Metal with a handful of purpose-built GPU kernels.
//!
//! Candle's stock Metal kernels for strided copies, broadcasts and im2col do 64-bit index
//! arithmetic per element and are about ten times slower than its contiguous kernels and its
//! matrix multiply. This module runs the same graph as `htdemucs::Graph::forward` using only
//! contiguous tensors: small kernels build the matrix-multiply inputs directly in the layout
//! the multiply wants (channels first, bias as an extra row of ones), and normalisation,
//! activation, layer scale and the residual add are fused into one pass.
//!
//! All activations are f32, batch 1, laid out (channels, frequency, time); the time branch
//! uses frequency = 1. Transformer tokens are (tokens, channels) in (frequency, time) order.
//!
//! Since 2026-11-27 (checks/stem-speed-study.md, second pass): matrix multiplies go through
//! Metal 4's tensor operations on M5-generation GPUs (`mm`), the dconv layers of the two
//! finest levels of both branches run as fused kernels (`dconv_row_*`, `tdconv_*`) that keep
//! their intermediates out of memory, layer norm uses one SIMD group per token, and the
//! network's inputs and outputs are read and written in place in shared memory (`Slots`,
//! `with_view`). Every change keeps f32 throughout; outputs stay within rounding of the
//! earlier code (>= 107 dB per stem on real tracks).

use candle_core::backend::BackendStorage;
use candle_core::{CpuStorage, CustomOp1, CustomOp2, DType, Layout, MetalDevice, MetalStorage, Result, Shape, Storage, Tensor};
use candle_metal_kernels::metal::{Buffer, ComputePipeline, Library};
use objc2_metal::{MTLResourceOptions, MTLResourceUsage, MTLSize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const KERNELS: &str = r#"
#include <metal_stdlib>
using namespace metal;

// Same formulas as candle's own kernels, so results match its gelu_erf and sigmoid.
static inline float erf_as(float x) {
    const float a1 = 0.254829592, a2 = -0.284496736, a3 = 1.421413741, a4 = -1.453152027, a5 = 1.061405429, p = 0.3275911;
    float s = x < 0 ? -1.0 : 1.0;
    x = fabs(x);
    float t = 1.0 / (1.0 + p * x);
    float y = 1.0 - (((((a5 * t + a4) * t) + a3) * t + a2) * t + a1) * t * exp(-x * x);
    return s * y;
}
static inline float gelu_erf(float x) { return x * (1 + erf_as(x * M_SQRT1_2_F)) / 2; }
static inline float sigmoid(float x) { return 1.0 / (1 + exp(-x)); }

// src (C, Fin, Tin) -> dst (C*kf*kt [+1 row of ones], Fo, To), rows in weight order.
kernel void im2col(device const float *src [[buffer(0)]], device float *dst [[buffer(1)]], constant uint *p [[buffer(2)]],
                   uint3 gid [[thread_position_in_grid]]) {
    uint C = p[0], Fin = p[1], Tin = p[2], kf = p[3], kt = p[4], sf = p[5], st = p[6], pf = p[7], pt = p[8], df = p[9], dt = p[10], Fo = p[11], To = p[12];
    uint r = gid.z;
    uint o = (r * Fo + gid.y) * To + gid.x;
    uint kk = kf * kt;
    if (r >= C * kk) { dst[o] = 1.0; return; }
    uint c = r / kk;
    uint k = r - c * kk;
    uint kfi = k / kt;
    uint kti = k - kfi * kt;
    int f = int(gid.y * sf + kfi * df) - int(pf);
    int t = int(gid.x * st + kti * dt) - int(pt);
    dst[o] = (f < 0 || f >= int(Fin) || t < 0 || t >= int(Tin)) ? 0.0 : src[(c * Fin + uint(f)) * Tin + uint(t)];
}

// Partial sums for group norm: x (C, F, T) -> out (2, F, C, nb): sum and sum of squares of
// each block of `blk` samples along T.
kernel void row_stats(device const float *x [[buffer(0)]], device float *out [[buffer(1)]], constant uint *p [[buffer(2)]],
                      uint3 gid [[thread_position_in_grid]]) {
    uint C = p[0], F = p[1], T = p[2], blk = p[3], nb = p[4];
    uint b = gid.x, c = gid.y, f = gid.z;
    uint t0 = b * blk, t1 = min(T, t0 + blk);
    uint base = (c * F + f) * T;
    float s = 0, q = 0;
    for (uint t = t0; t < t1; t++) { float v = x[base + t]; s += v; q += v * v; }
    uint o = (f * C + c) * nb + b;
    out[o] = s;
    out[F * C * nb + o] = q;
}

// Group norm (one group; statistics per frequency row when G == F, over everything when
// G == 1, none when G == 0), then activation (1 GELU, 2 GLU over the channel halves), then
// optionally res + layer_scale * y, with an optional extra channel of ones.
// stats: [sum(G), sum of squares(G)]; cst: [weight(Cin), bias(Cin), layer scale(Cout)].
kernel void norm_act(device const float *x [[buffer(0)]], device const float *stats [[buffer(1)]], device const float *cst [[buffer(2)]],
                     device const float *res [[buffer(3)]], device float *out [[buffer(4)]], constant uint *p [[buffer(5)]],
                     uint3 gid [[thread_position_in_grid]]) {
    uint Cin = p[0], F = p[1], T = p[2], G = p[3], act = p[4], has_res = p[5];
    float n = as_type<float>(p[6]), eps = as_type<float>(p[7]);
    uint Cout = act == 2 ? Cin / 2 : Cin;
    uint c = gid.z, f = gid.y;
    uint o = (c * F + f) * T + gid.x;
    if (c >= Cout) { out[o] = 1.0; return; }
    float a = x[o];
    float m = 0, inv = 1;
    if (G > 0) {
        uint g = G == 1 ? 0 : f;
        m = stats[g] / n;
        float var = max(stats[G + g] / n - m * m, 0.0f);
        inv = 1.0 / sqrt(var + eps);
        a = (a - m) * inv * cst[c] + cst[Cin + c];
    }
    if (act == 1) {
        a = gelu_erf(a);
    } else if (act == 2) {
        float b = x[o + Cout * F * T];
        if (G > 0) { b = (b - m) * inv * cst[c + Cout] + cst[Cin + c + Cout]; }
        a = a * sigmoid(b);
    }
    if (has_res != 0) { a = res[o] + cst[2 * Cin + c] * a; }
    out[o] = a;
}

// Transposed convolution, kernel 8, stride 4, cropped by 2 at the start: cols (Co*8, F, T)
// from the matrix multiply -> out (Co, Fo, To), plus bias and optional GELU. axis 0: along
// frequency, 1: along time.
kernel void tr_col2im(device const float *cols [[buffer(0)]], device const float *bias [[buffer(1)]], device float *out [[buffer(2)]],
                      constant uint *p [[buffer(3)]], uint3 gid [[thread_position_in_grid]]) {
    uint F = p[1], T = p[2], axis = p[3], Fo = p[4], To = p[5], act = p[6];
    uint co = gid.z;
    uint pos = (axis == 0 ? gid.y : gid.x) + 2;
    uint j = pos / 4, i = pos % 4;
    uint n = axis == 0 ? F : T;
    float v = bias[co];
    if (j < n) {
        uint f = axis == 0 ? j : gid.y, t = axis == 0 ? gid.x : j;
        v += cols[((co * 8 + i) * F + f) * T + t];
    }
    if (j >= 1 && j - 1 < n) {
        uint f = axis == 0 ? j - 1 : gid.y, t = axis == 0 ? gid.x : j - 1;
        v += cols[((co * 8 + i + 4) * F + f) * T + t];
    }
    if (act == 1) { v = gelu_erf(v); }
    out[(co * Fo + gid.y) * To + gid.x] = v;
}

// a + b, same size.
kernel void add2(device const float *a [[buffer(0)]], device const float *b [[buffer(1)]], device float *out [[buffer(2)]],
                 constant uint *p [[buffer(3)]], uint3 gid [[thread_position_in_grid]]) {
    uint o = gid.y * p[0] + gid.x;
    if (o < p[1]) { out[o] = a[o] + b[o]; }
}

// x (C, F, T) + e (C, F).
kernel void add_cf(device const float *x [[buffer(0)]], device const float *e [[buffer(1)]], device float *out [[buffer(2)]],
                   constant uint *p [[buffer(3)]], uint3 gid [[thread_position_in_grid]]) {
    uint F = p[1], T = p[2];
    uint o = (gid.z * F + gid.y) * T + gid.x;
    out[o] = x[o] + e[gid.z * F + gid.y];
}

// Layer norm along the channels of x (L, C); cst: [weight(C), bias(C)]; optional last
// column of ones. One SIMD group (32 threads) per row: x is (32, L) threads.
kernel void ln_rows(device const float *x [[buffer(0)]], device const float *cst [[buffer(1)]], device float *out [[buffer(2)]],
                    constant uint *p [[buffer(3)]], uint3 gid [[thread_position_in_grid]], uint lane [[thread_index_in_simdgroup]]) {
    uint C = p[0], ones = p[1], L = p[3];
    float eps = as_type<float>(p[2]);
    // A whole SIMD group shares one row, so it leaves together.
    if (gid.y >= L) { return; }
    uint base = gid.y * C;
    float s = 0;
    for (uint c = lane; c < C; c += 32) { s += x[base + c]; }
    float m = simd_sum(s) / float(C);
    float q = 0;
    for (uint c = lane; c < C; c += 32) { float d = x[base + c] - m; q += d * d; }
    float inv = 1.0 / sqrt(simd_sum(q) / float(C) + eps);
    uint ob = gid.y * (C + ones);
    for (uint c = lane; c < C; c += 32) { out[ob + c] = (x[base + c] - m) * inv * cst[c] + cst[C + c]; }
    if (ones != 0 && lane == 0) { out[ob + C] = 1.0; }
}

// Attention heads: src (L, W), columns [off, off + H*D) -> dst (H, L, D).
kernel void split_heads(device const float *src [[buffer(0)]], device float *dst [[buffer(1)]], constant uint *p [[buffer(2)]],
                        uint3 gid [[thread_position_in_grid]]) {
    uint L = p[0], W = p[1], off = p[2], D = p[3];
    uint d = gid.x, l = gid.y, h = gid.z;
    dst[(h * L + l) * D + d] = src[l * W + off + h * D + d];
}

// src (H, L, D) -> dst (L, H*D + 1); the last column is ones.
kernel void merge_heads(device const float *src [[buffer(0)]], device float *dst [[buffer(1)]], constant uint *p [[buffer(2)]],
                        uint3 gid [[thread_position_in_grid]]) {
    uint L = p[0], H = p[1], D = p[2];
    uint c = gid.x, l = gid.y;
    uint W = H * D + 1;
    dst[l * W + c] = c >= H * D ? 1.0 : src[((c / D) * L + l) * D + (c % D)];
}

// x (L, C) -> GELU(x) with a last column of ones: (L, C + 1).
kernel void row_gelu(device const float *x [[buffer(0)]], device float *out [[buffer(1)]], constant uint *p [[buffer(2)]],
                     uint3 gid [[thread_position_in_grid]]) {
    uint C = p[0];
    out[gid.y * (C + 1) + gid.x] = gid.x >= C ? 1.0 : gelu_erf(x[gid.y * C + gid.x]);
}

// Group norm (one group over the whole of x (L, C)) with per-channel weight and bias;
// stats: [sum, sum of squares]; optional last column of ones.
kernel void row_norm(device const float *x [[buffer(0)]], device const float *stats [[buffer(1)]], device const float *cst [[buffer(2)]],
                     device float *out [[buffer(3)]], constant uint *p [[buffer(4)]], uint3 gid [[thread_position_in_grid]]) {
    uint C = p[0], ones = p[1];
    float n = as_type<float>(p[2]), eps = as_type<float>(p[3]);
    uint c = gid.x;
    uint o = gid.y * (C + ones) + c;
    if (c >= C) { out[o] = 1.0; return; }
    float m = stats[0] / n;
    float var = max(stats[1] / n - m * m, 0.0f);
    out[o] = (x[gid.y * C + c] - m) / sqrt(var + eps) * cst[c] + cst[C + c];
}

// One dconv layer of the spectrogram branch, one frequency row per thread group and one
// time step per thread, with nothing in between written to memory: conv C -> H (kernel 3
// along time, dilation D) + group norm over the row + GELU, then the 1x1 expansion H -> 2C +
// group norm over the row + GLU, times the layer scale, plus x. x, out: (C [+1 ones], F, T).
// w1 (H, 3C + 1) and w2 (2C, H + 1) have the bias last; n1 = [weight(H), bias(H)];
// n2 = [weight(2C), bias(2C), scale(C)]. p = [C, F, T, H, D, ones, eps]; H is 6 or 12, T <= threads.
static inline float2 group_sum(float2 v, threadgroup float2 *red, uint lane, uint sg, uint nsg) {
    v = float2(simd_sum(v.x), simd_sum(v.y));
    if (lane == 0) { red[sg] = v; }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    float2 t = float2(0);
    for (uint i = 0; i < nsg; i++) { t += red[i]; }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    return t;
}
// H is a compile-time constant so the per-thread values stay in registers; C = 8 H.
template <uint H>
static inline void dconv_row_h(device const float *x, device const float *w1g, device const float *n1, device const float *w2g,
                               device const float *n2, device float *out, constant uint *p, uint f, uint t, uint nt, uint lane, uint sg, uint nsg,
                               threadgroup float *w1, threadgroup float *w2, threadgroup float2 *red) {
    constexpr uint C = 8 * H, K1 = 3 * C;
    uint F = p[1], T = p[2], D = p[4], ones = p[5];
    float eps = as_type<float>(p[6]);
    for (uint i = t; i < H * (K1 + 1); i += nt) { w1[i] = w1g[i]; }
    for (uint i = t; i < 2 * C * (H + 1); i += nt) { w2[i] = w2g[i]; }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    bool live = t < T;
    float y[H];
    for (uint j = 0; j < H; j++) { y[j] = 0; }
    float2 acc = float2(0);
    if (live) {
        for (uint c = 0; c < C; c++) {
            device const float *row = x + (c * F + f) * T;
            for (uint k = 0; k < 3; k++) {
                int tt = int(t) + (int(k) - 1) * int(D);
                float v = (tt >= 0 && tt < int(T)) ? row[tt] : 0.0;
                for (uint j = 0; j < H; j++) { y[j] += w1[j * (K1 + 1) + c * 3 + k] * v; }
            }
        }
        for (uint j = 0; j < H; j++) {
            y[j] += w1[j * (K1 + 1) + K1];
            acc += float2(y[j], y[j] * y[j]);
        }
    }
    float2 st = group_sum(acc, red, lane, sg, nsg);
    float n = float(H * T);
    float m = st.x / n;
    float inv = 1.0 / sqrt(max(st.y / n - m * m, 0.0f) + eps);
    for (uint j = 0; j < H; j++) { y[j] = gelu_erf((y[j] - m) * inv * n1[j] + n1[H + j]); }
    // The expansion's statistics, then the expansion again for the output.
    acc = float2(0);
    if (live) {
        for (uint o = 0; o < 2 * C; o++) {
            float z = 0;
            for (uint j = 0; j < H; j++) { z += w2[o * (H + 1) + j] * y[j]; }
            z += w2[o * (H + 1) + H];
            acc += float2(z, z * z);
        }
    }
    st = group_sum(acc, red, lane, sg, nsg);
    n = float(2 * C * T);
    m = st.x / n;
    inv = 1.0 / sqrt(max(st.y / n - m * m, 0.0f) + eps);
    if (!live) { return; }
    for (uint o = 0; o < C; o++) {
        float a = 0, b = 0;
        for (uint j = 0; j < H; j++) { a += w2[o * (H + 1) + j] * y[j]; b += w2[(o + C) * (H + 1) + j] * y[j]; }
        a += w2[o * (H + 1) + H];
        b += w2[(o + C) * (H + 1) + H];
        a = (a - m) * inv * n2[o] + n2[2 * C + o];
        b = (b - m) * inv * n2[o + C] + n2[2 * C + o + C];
        uint i = (o * F + f) * T + t;
        out[i] = x[i] + n2[4 * C + o] * (a * sigmoid(b));
    }
    if (ones != 0) { out[(C * F + f) * T + t] = 1.0; }
}
#define DCONV_ROW(H) \
kernel void dconv_row_##H(device const float *x [[buffer(0)]], device const float *w1 [[buffer(1)]], device const float *n1 [[buffer(2)]], \
                          device const float *w2 [[buffer(3)]], device const float *n2 [[buffer(4)]], device float *out [[buffer(5)]], \
                          constant uint *p [[buffer(6)]], uint f [[threadgroup_position_in_grid]], uint t [[thread_index_in_threadgroup]], \
                          uint nt [[threads_per_threadgroup]], uint lane [[thread_index_in_simdgroup]], uint sg [[simdgroup_index_in_threadgroup]], \
                          uint nsg [[simdgroups_per_threadgroup]]) { \
    threadgroup float tw1[H * (24 * H + 1)]; \
    threadgroup float tw2[16 * H * (H + 1)]; \
    threadgroup float2 red[32]; \
    dconv_row_h<H>(x, w1, n1, w2, n2, out, p, f, t, nt, lane, sg, nsg, tw1, tw2, red); \
}
DCONV_ROW(6)
DCONV_ROW(12)

// The same dconv layer for the time branch, where the group norms take one group over the
// whole (C, T) sequence: three passes over T in thread groups of up to 1024 steps, writing only
// the small conv output y (H, T) and per-group partial sums in between.
// a: y and the partial sums (sum, sum of squares) of y: out = [y (H*T), partials (2*nb)].
// b: the expansion's partial sums, from y normalised with a's totals: out = partials (2*nb).
// c: the output: x + scale * GLU(norm(expansion)), (C [+1 ones], 1, T).
// p = [T, D, nb, ones, eps].
static inline float2 total_of(device const float *part, uint nb, uint t, threadgroup float2 *tot) {
    if (t == 0) {
        float2 s = float2(0);
        for (uint i = 0; i < nb; i++) { s += float2(part[2 * i], part[2 * i + 1]); }
        tot[0] = s;
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    return tot[0];
}
template <uint H>
static inline void tconv_y(device const float *x, threadgroup const float *w1, uint T, uint D, uint t, thread float *y) {
    constexpr uint C = 8 * H, K1 = 3 * C;
    for (uint j = 0; j < H; j++) { y[j] = 0; }
    for (uint c = 0; c < C; c++) {
        device const float *row = x + c * T;
        for (uint k = 0; k < 3; k++) {
            int tt = int(t) + (int(k) - 1) * int(D);
            float v = (tt >= 0 && tt < int(T)) ? row[tt] : 0.0;
            for (uint j = 0; j < H; j++) { y[j] += w1[j * (K1 + 1) + c * 3 + k] * v; }
        }
    }
    for (uint j = 0; j < H; j++) { y[j] += w1[j * (K1 + 1) + K1]; }
}
template <uint H>
static inline void tnorm_y(device const float *ybuf, device const float *n1, float2 st, uint T, uint t, float eps, thread float *y) {
    float n = float(H * T);
    float m = st.x / n;
    float inv = 1.0 / sqrt(max(st.y / n - m * m, 0.0f) + eps);
    for (uint j = 0; j < H; j++) { y[j] = gelu_erf((ybuf[j * T + t] - m) * inv * n1[j] + n1[H + j]); }
}
#define TDCONV(H) \
kernel void tdconv_a_##H(device const float *x [[buffer(0)]], device const float *w1g [[buffer(1)]], device float *out [[buffer(2)]], \
                         constant uint *p [[buffer(3)]], uint g [[threadgroup_position_in_grid]], uint tl [[thread_index_in_threadgroup]], \
                         uint nt [[threads_per_threadgroup]], uint lane [[thread_index_in_simdgroup]], uint sg [[simdgroup_index_in_threadgroup]], \
                         uint nsg [[simdgroups_per_threadgroup]]) { \
    threadgroup float w1[H * (24 * H + 1)]; \
    threadgroup float2 red[32]; \
    uint T = p[0], D = p[1]; \
    for (uint i = tl; i < H * (24 * H + 1); i += nt) { w1[i] = w1g[i]; } \
    threadgroup_barrier(mem_flags::mem_threadgroup); \
    uint t = g * nt + tl; \
    float y[H]; \
    float2 acc = float2(0); \
    if (t < T) { \
        tconv_y<H>(x, w1, T, D, t, y); \
        for (uint j = 0; j < H; j++) { out[j * T + t] = y[j]; acc += float2(y[j], y[j] * y[j]); } \
    } \
    float2 st = group_sum(acc, red, lane, sg, nsg); \
    if (tl == 0) { out[H * T + 2 * g] = st.x; out[H * T + 2 * g + 1] = st.y; } \
} \
kernel void tdconv_b_##H(device const float *ybuf [[buffer(0)]], device const float *n1 [[buffer(1)]], device const float *w2g [[buffer(2)]], \
                         device float *out [[buffer(3)]], constant uint *p [[buffer(4)]], uint g [[threadgroup_position_in_grid]], \
                         uint tl [[thread_index_in_threadgroup]], uint nt [[threads_per_threadgroup]], uint lane [[thread_index_in_simdgroup]], \
                         uint sg [[simdgroup_index_in_threadgroup]], uint nsg [[simdgroups_per_threadgroup]]) { \
    constexpr uint C = 8 * H; \
    threadgroup float w2[2 * C * (H + 1)]; \
    threadgroup float2 red[32]; \
    threadgroup float2 tot[1]; \
    uint T = p[0], nb = p[2]; \
    float eps = as_type<float>(p[4]); \
    for (uint i = tl; i < 2 * C * (H + 1); i += nt) { w2[i] = w2g[i]; } \
    float2 st = total_of(ybuf + H * T, nb, tl, tot); \
    uint t = g * nt + tl; \
    float2 acc = float2(0); \
    if (t < T) { \
        float y[H]; \
        tnorm_y<H>(ybuf, n1, st, T, t, eps, y); \
        for (uint o = 0; o < 2 * C; o++) { \
            float z = 0; \
            for (uint j = 0; j < H; j++) { z += w2[o * (H + 1) + j] * y[j]; } \
            z += w2[o * (H + 1) + H]; \
            acc += float2(z, z * z); \
        } \
    } \
    float2 s2 = group_sum(acc, red, lane, sg, nsg); \
    if (tl == 0) { out[2 * g] = s2.x; out[2 * g + 1] = s2.y; } \
} \
kernel void tdconv_c_##H(device const float *x [[buffer(0)]], device const float *ybuf [[buffer(1)]], device const float *part2 [[buffer(2)]], \
                         device const float *n1 [[buffer(3)]], device const float *w2g [[buffer(4)]], device const float *n2 [[buffer(5)]], \
                         device float *out [[buffer(6)]], constant uint *p [[buffer(7)]], uint g [[threadgroup_position_in_grid]], \
                         uint tl [[thread_index_in_threadgroup]], uint nt [[threads_per_threadgroup]]) { \
    constexpr uint C = 8 * H; \
    threadgroup float w2[2 * C * (H + 1)]; \
    threadgroup float2 tot1[1]; \
    threadgroup float2 tot2[1]; \
    uint T = p[0], nb = p[2], ones = p[3]; \
    float eps = as_type<float>(p[4]); \
    for (uint i = tl; i < 2 * C * (H + 1); i += nt) { w2[i] = w2g[i]; } \
    float2 st = total_of(ybuf + H * T, nb, tl, tot1); \
    float2 st2 = total_of(part2, nb, tl, tot2); \
    uint t = g * nt + tl; \
    if (t >= T) { return; } \
    float y[H]; \
    tnorm_y<H>(ybuf, n1, st, T, t, eps, y); \
    float n = float(2 * C * T); \
    float m = st2.x / n; \
    float inv = 1.0 / sqrt(max(st2.y / n - m * m, 0.0f) + eps); \
    for (uint o = 0; o < C; o++) { \
        float a = 0, b = 0; \
        for (uint j = 0; j < H; j++) { a += w2[o * (H + 1) + j] * y[j]; b += w2[(o + C) * (H + 1) + j] * y[j]; } \
        a += w2[o * (H + 1) + H]; \
        b += w2[(o + C) * (H + 1) + H]; \
        a = (a - m) * inv * n2[o] + n2[2 * C + o]; \
        b = (b - m) * inv * n2[o + C] + n2[2 * C + o + C]; \
        out[o * T + t] = x[o * T + t] + n2[4 * C + o] * (a * sigmoid(b)); \
    } \
    if (ones != 0) { out[C * T + t] = 1.0; } \
}
TDCONV(6)
TDCONV(12)
"#;

/// Where the GPU time goes, for profiling only (`profile_begin` / `profile_take`; nothing is
/// measured otherwise). While on, every stage boundary, and with `ops` every operation, waits
/// for the GPU to finish, so each row is the GPU time of that part plus a fixed cost per mark
/// (`profile_take` reports it as the "(empty mark)" row so it can be subtracted).
struct Prof {
    ops: bool,
    last: std::time::Instant,
    rows: Vec<(String, f64, usize)>,
}

static PROF: Mutex<Option<Prof>> = Mutex::new(None);

pub fn profile_begin(ops: bool) {
    if let Ok(mut p) = PROF.lock() {
        *p = Some(Prof { ops, last: std::time::Instant::now(), rows: vec![] });
    }
}

/// Rows (label, seconds, count) in first-seen order; profiling stops.
pub fn profile_take() -> Vec<(String, f64, usize)> {
    PROF.lock().ok().and_then(|mut p| p.take()).map(|p| p.rows).unwrap_or_default()
}

fn profiling(ops: bool) -> bool {
    PROF.lock().map(|p| p.as_ref().is_some_and(|p| p.ops == ops)).unwrap_or(false)
}

/// Wait for the GPU and book the time since the last mark under `label`.
fn mark_on(dev: &MetalDevice, ops: bool, label: impl FnOnce() -> String) {
    if !profiling(ops) {
        return;
    }
    let _ = dev.wait_until_completed();
    if let Ok(mut g) = PROF.lock() {
        if let Some(p) = g.as_mut() {
            let now = std::time::Instant::now();
            let dt = (now - p.last).as_secs_f64();
            let label = label();
            match p.rows.iter_mut().find(|r| r.0 == label) {
                Some(r) => {
                    r.1 += dt;
                    r.2 += 1;
                }
                None => p.rows.push((label, dt, 1)),
            }
            p.last = std::time::Instant::now();
        }
    }
}

/// A stage boundary (no effect unless profiling stages).
pub fn mark(t: &Tensor, label: impl FnOnce() -> String) {
    mark_kind(t, false, label)
}

/// The end of one operation that is not one of this module's kernels (profiling operations).
fn op_mark(t: &Tensor, label: impl FnOnce() -> String) {
    mark_kind(t, true, label)
}

fn mark_kind(t: &Tensor, ops: bool, label: impl FnOnce() -> String) {
    if !profiling(ops) {
        return;
    }
    let dev = match &*t.storage_and_layout().0 {
        Storage::Metal(s) => s.device().clone(),
        _ => return,
    };
    mark_on(&dev, ops, label);
}

static LIBRARIES: Mutex<Vec<(u64, &'static str, Arc<Library>)>> = Mutex::new(Vec::new());
static PIPELINES: Mutex<Vec<(u64, &'static str, Arc<ComputePipeline>)>> = Mutex::new(Vec::new());

fn msg<E: std::fmt::Display>(e: E) -> candle_core::Error {
    candle_core::Error::Msg(e.to_string())
}

fn pipeline(dev: &MetalDevice, name: &'static str) -> Result<Arc<ComputePipeline>> {
    pipeline_in(dev, KERNELS, name)
}

/// A kernel of the Metal source `src` (compiled once per GPU).
fn pipeline_in(dev: &MetalDevice, src: &'static str, name: &'static str) -> Result<Arc<ComputePipeline>> {
    let id = dev.metal_device().registry_id();
    let mut pipes = PIPELINES.lock().map_err(msg)?;
    if let Some(p) = pipes.iter().find(|p| p.0 == id && p.1 == name) {
        return Ok(p.2.clone());
    }
    let mut libs = LIBRARIES.lock().map_err(msg)?;
    let lib = match libs.iter().find(|l| l.0 == id && std::ptr::eq(l.1, src)) {
        Some(l) => l.2.clone(),
        None => {
            let lib = Arc::new(dev.metal_device().new_library_with_source(src, None).map_err(msg)?);
            libs.push((id, src, lib.clone()));
            lib
        }
    };
    let func = lib.get_function(name, None).map_err(msg)?;
    let pipe = Arc::new(dev.metal_device().new_compute_pipeline_state_with_function(&func).map_err(msg)?);
    pipes.push((id, name, pipe.clone()));
    Ok(pipe)
}

/// Output buffers of this module's operations. Candle's own pool releases every unused
/// buffer each time it commits a command buffer (every 50 operations), so the large
/// intermediates of a segment would be freed and allocated again several times per segment
/// (measured: about 15 % of a segment's time). They are kept here until `release_buffers`.
/// Buffers are only reused on the device (command queue) that made them, since that is
/// what keeps the order of use.
static POOL: Mutex<Vec<(candle_core::metal_backend::DeviceId, Arc<Buffer>)>> = Mutex::new(Vec::new());

fn alloc(dev: &MetalDevice, elements: usize) -> Result<Arc<Buffer>> {
    let bytes = elements.max(1) * 4;
    let mut pool = POOL.lock().map_err(msg)?;
    // A buffer nobody else holds is free: commands already encoded against it run before
    // anything encoded later.
    let id = dev.id();
    let free = pool.iter().filter(|(d, b)| *d == id && Arc::strong_count(b) == 1 && b.length() >= bytes && b.length() <= bytes + bytes / 4 + 4096).min_by_key(|(_, b)| b.length());
    if let Some((_, b)) = free {
        return Ok(b.clone());
    }
    let buffer = Arc::new(dev.metal_device().new_buffer(bytes, MTLResourceOptions::StorageModeShared).map_err(msg)?);
    pool.push((id, buffer.clone()));
    Ok(buffer)
}

/// Give back the memory held for intermediates (call when a separation ends).
pub fn release_buffers() {
    if let Ok(mut pool) = POOL.lock() {
        // Buffers still referenced by live tensors are freed with those tensors.
        pool.clear();
    }
}

/// The values of a contiguous f32 tensor, copied straight out of its (shared-memory) buffer
/// once the GPU has finished. `Tensor::to_vec1` copies the whole underlying buffer first,
/// which for one source of four is four times too much.
pub fn read(t: &Tensor) -> Result<Vec<f32>> {
    t.device().synchronize()?;
    let (storage, layout) = t.storage_and_layout();
    let Storage::Metal(s) = &*storage else { candle_core::bail!("read: tensor is not on Metal") };
    let count = layout.shape().elem_count();
    if !layout.is_contiguous() || s.dtype() != DType::F32 || (layout.start_offset() + count) * 4 > s.buffer().length() {
        candle_core::bail!("read: tensor must be contiguous f32")
    }
    // SAFETY: the buffer is CPU-visible shared memory of at least `start + count` f32s (checked
    // above), it stays alive through `storage`, and every command writing it has completed.
    Ok(unsafe { std::slice::from_raw_parts((s.buffer().contents() as *const f32).add(layout.start_offset()), count) }.to_vec())
}

/// Run `f` on the values of a contiguous f32 tensor read straight out of its shared-memory
/// buffer, without a copy. The GPU must have finished writing it (the caller waited).
pub fn with_view<R>(t: &Tensor, f: impl FnOnce(&[f32]) -> R) -> Result<R> {
    let (storage, layout) = t.storage_and_layout();
    let Storage::Metal(s) = &*storage else { candle_core::bail!("view: tensor is not on Metal") };
    let count = layout.shape().elem_count();
    if !layout.is_contiguous() || s.dtype() != DType::F32 || (layout.start_offset() + count) * 4 > s.buffer().length() {
        candle_core::bail!("view: tensor must be contiguous f32")
    }
    // SAFETY: as in `read`; the slice lives only during `f`, while `storage` holds the buffer.
    let v = unsafe { std::slice::from_raw_parts((s.buffer().contents() as *const f32).add(layout.start_offset()), count) };
    Ok(f(v))
}

/// Input buffers the CPU fills in place (shared memory), so a prepared segment is not
/// copied again on its way to the GPU. Each slot holds a spectrogram and a waveform; a slot
/// is used by one stage at a time (the caller passes them around through a free list).
pub struct Slots {
    dev: MetalDevice,
    dummy: Tensor,
    bufs: Vec<(Arc<Buffer>, Arc<Buffer>)>,
    lens: (usize, usize),
}

// SAFETY: the buffers are plain shared memory; who may touch a slot when is the caller's
// contract (`fill`).
unsafe impl Send for Slots {}
unsafe impl Sync for Slots {}

impl Slots {
    pub fn new(device: &candle_core::Device, n: usize, spec: usize, wave: usize) -> Result<Self> {
        let candle_core::Device::Metal(dev) = device else { candle_core::bail!("slots need a Metal device") };
        let mk = |count: usize| -> Result<Arc<Buffer>> { Ok(Arc::new(dev.metal_device().new_buffer(count.max(1) * 4, MTLResourceOptions::StorageModeShared).map_err(msg)?)) };
        let bufs = (0..n).map(|_| Ok((mk(spec)?, mk(wave)?))).collect::<Result<Vec<_>>>()?;
        Ok(Self { dev: dev.clone(), dummy: Tensor::zeros(1, DType::F32, device)?, bufs, lens: (spec, wave) })
    }

    /// The two buffers of slot `i` to write.
    ///
    /// SAFETY: the caller must have slot `i` to itself: no GPU work that uses it is pending
    /// and no other thread holds it.
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn fill(&self, i: usize) -> (&mut [f32], &mut [f32]) {
        let (a, b) = &self.bufs[i];
        (std::slice::from_raw_parts_mut(a.contents() as *mut f32, self.lens.0), std::slice::from_raw_parts_mut(b.contents() as *mut f32, self.lens.1))
    }

    /// Slot `i` as two tensors of the given shapes (no copy).
    pub fn tensors(&self, i: usize, spec: (usize, usize, usize), wave: (usize, usize, usize)) -> Result<(Tensor, Tensor)> {
        let (a, b) = &self.bufs[i];
        let wrap = |buf: &Arc<Buffer>, shape: (usize, usize, usize), count: usize| {
            if shape.0 * shape.1 * shape.2 != count {
                candle_core::bail!("slot shape")
            }
            self.dummy.apply_op1_no_bwd(&Wrap { buf: buf.clone(), dev: self.dev.clone(), shape: vec![shape.0, shape.1, shape.2] })
        };
        Ok((wrap(a, spec, self.lens.0)?, wrap(b, wave, self.lens.1)?))
    }
}

/// A tensor over an existing buffer (no GPU work).
struct Wrap {
    buf: Arc<Buffer>,
    dev: MetalDevice,
    shape: Vec<usize>,
}

impl CustomOp1 for Wrap {
    fn name(&self) -> &'static str {
        "wrap"
    }
    fn cpu_fwd(&self, _: &CpuStorage, _: &Layout) -> Result<(CpuStorage, Shape)> {
        candle_core::bail!("wrap runs on Metal only")
    }
    fn metal_fwd(&self, _: &MetalStorage, _: &Layout) -> Result<(MetalStorage, Shape)> {
        let count = self.shape.iter().product();
        Ok((MetalStorage::new(self.buf.clone(), self.dev.clone(), count, DType::F32), Shape::from(self.shape.clone())))
    }
}

/// Candle's matrix multiply kernel, writing into a pooled buffer. Operands (.., m, k) and
/// (.., k, n) with the same leading dimensions; either may be a transposed view.
struct Gemm;

impl CustomOp2 for Gemm {
    fn name(&self) -> &'static str {
        "gemm"
    }
    fn cpu_fwd(&self, _: &CpuStorage, _: &Layout, _: &CpuStorage, _: &Layout) -> Result<(CpuStorage, Shape)> {
        candle_core::bail!("gemm runs on Metal only")
    }
    fn metal_fwd(&self, lhs: &MetalStorage, ll: &Layout, rhs: &MetalStorage, rl: &Layout) -> Result<(MetalStorage, Shape)> {
        let (ld, rd) = (ll.dims(), rl.dims());
        let rank = ld.len();
        if rank < 2 || rd.len() != rank || ld[..rank - 2] != rd[..rank - 2] || ld[rank - 1] != rd[rank - 2] {
            candle_core::bail!("gemm: shapes {ld:?} and {rd:?} do not multiply")
        }
        let (b, m, k, n) = (ld[..rank - 2].iter().product::<usize>(), ld[rank - 2], ld[rank - 1], rd[rank - 1]);
        let dev = lhs.device().clone();
        let out = alloc(&dev, b * m * n)?;
        let enc = dev.command_encoder()?;
        enc.set_label("gemm");
        candle_metal_kernels::call_mlx_gemm(dev.metal_device(), &enc, dev.kernels(), candle_metal_kernels::GemmDType::F32, (b, m, n, k), ll.stride(), ll.start_offset() * 4, lhs.buffer(), rl.stride(), rl.start_offset() * 4, rhs.buffer(), &out)
            .map_err(msg)?;
        drop(enc);
        mark_on(&dev, true, || format!("gemm {m}x{k} * {k}x{n}{}", if b > 1 { format!(" (x{b})") } else { String::new() }));
        let mut shape = ld[..rank - 2].to_vec();
        shape.extend([m, n]);
        Ok((MetalStorage::new(out, dev, b * m * n, DType::F32), Shape::from(shape)))
    }
}

/// Matrix multiply through Metal 4's tensor operations (`matmul2d` from Metal Performance
/// Primitives), which the M5's GPU runs on its neural accelerators. In f32 with
/// `relaxed_precision` off it is a full-precision product (the same error against an f64
/// product as Candle's kernel, measured in `probe::matmul_speed_and_precision`) and 1.2–1.7×
/// faster on this network's shapes. Compiled at run time from the system's headers.
const MPP_KERNELS: &str = r#"
#include <metal_stdlib>
#include <metal_tensor>
#include <MetalPerformancePrimitives/MetalPerformancePrimitives.h>
using namespace metal;
using namespace mpp::tensor_ops;

// c (M, N) = a (M, K) * b (K, N). a is stored M x K row-major, or K x M when TA; b is K x N,
// or N x K when TB. p = [M, N, K]. Thread groups of 4 SIMD groups, one TM x TN tile each.
template <int TM, int TN, bool TA, bool TB>
static inline void mm_f32(device float *a, device float *b, device float *c, constant uint *p, uint2 tgid) {
    int M = p[0], N = p[1], K = p[2];
    auto A = tensor<device float, dextents<int32_t, 2>, tensor_inline>(a, TA ? dextents<int32_t, 2>(M, K) : dextents<int32_t, 2>(K, M));
    auto B = tensor<device float, dextents<int32_t, 2>, tensor_inline>(b, TB ? dextents<int32_t, 2>(K, N) : dextents<int32_t, 2>(N, K));
    auto C = tensor<device float, dextents<int32_t, 2>, tensor_inline>(c, dextents<int32_t, 2>(N, M));
    constexpr auto desc = matmul2d_descriptor(TM, TN, static_cast<int>(dynamic_extent), TA, TB, false);
    matmul2d<desc, execution_simdgroups<4>> op;
    auto mA = TA ? A.slice(tgid.y * TM, 0) : A.slice(0, tgid.y * TM);
    auto mB = TB ? B.slice(0, tgid.x * TN) : B.slice(tgid.x * TN, 0);
    auto mC = C.slice(tgid.x * TN, tgid.y * TM);
    op.run(mA, mB, mC);
}
#define MM(name, TA, TB) \
kernel void name(device float *a [[buffer(0)]], device float *b [[buffer(1)]], device float *c [[buffer(2)]], \
                 constant uint *p [[buffer(3)]], uint2 tgid [[threadgroup_position_in_grid]]) { mm_f32<32, 128, TA, TB>(a, b, c, p, tgid); }
MM(mpp_mm_nn, false, false)
MM(mpp_mm_tn, true, false)
MM(mpp_mm_nt, false, true)
"#;

/// Whether the tensor-op multiply is used: it must compile (macOS 26 or later) and the GPU
/// must be one with neural accelerators (Apple family 10, the M5 generation); on older GPUs
/// the same code runs without them and Candle's kernel is kept. `PREPDECK_STEM_MPP=0` turns
/// it off.
fn mpp_ok(dev: &MetalDevice) -> bool {
    static OK: Mutex<Vec<(u64, bool)>> = Mutex::new(Vec::new());
    let id = dev.metal_device().registry_id();
    let Ok(mut known) = OK.lock() else { return false };
    if let Some(k) = known.iter().find(|k| k.0 == id) {
        return k.1;
    }
    let off = std::env::var("PREPDECK_STEM_MPP").is_ok_and(|v| v == "0");
    let family = gpu_generation(&dev.metal_device().architecture_name()).is_some_and(|g| g >= 17);
    let ok = !off && family && pipeline_in(dev, MPP_KERNELS, "mpp_mm_nn").is_ok();
    known.push((id, ok));
    ok
}

/// Apple GPU generation from Metal's architecture name ("applegpu_g17s" -> 17; the M1 is 13,
/// the M4 16, the M5 17).
fn gpu_generation(arch: &str) -> Option<u32> {
    let rest = arch.strip_prefix("applegpu_g")?;
    rest.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()
}

struct MppGemm;

impl CustomOp2 for MppGemm {
    fn name(&self) -> &'static str {
        "mpp-gemm"
    }
    fn cpu_fwd(&self, _: &CpuStorage, _: &Layout, _: &CpuStorage, _: &Layout) -> Result<(CpuStorage, Shape)> {
        candle_core::bail!("mpp-gemm runs on Metal only")
    }
    fn metal_fwd(&self, lhs: &MetalStorage, ll: &Layout, rhs: &MetalStorage, rl: &Layout) -> Result<(MetalStorage, Shape)> {
        let (&[m, k], &[k2, n]) = (ll.dims(), rl.dims()) else { candle_core::bail!("mpp-gemm: operands must be matrices") };
        let ta = match ll.stride() {
            [s, 1] if *s == k => false,
            [1, s] if *s == m => true,
            _ => candle_core::bail!("mpp-gemm: left operand layout"),
        };
        let tb = match rl.stride() {
            [s, 1] if *s == n => false,
            [1, s] if *s == k => true,
            _ => candle_core::bail!("mpp-gemm: right operand layout"),
        };
        if k != k2 || lhs.dtype() != DType::F32 || rhs.dtype() != DType::F32 {
            candle_core::bail!("mpp-gemm: shapes or types")
        }
        let dev = lhs.device().clone();
        let out = alloc(&dev, m * n)?;
        let name = match (ta, tb) {
            (false, false) => "mpp_mm_nn",
            (true, false) => "mpp_mm_tn",
            (false, true) => "mpp_mm_nt",
            (true, true) => candle_core::bail!("mpp-gemm: both operands transposed"),
        };
        let pipe = pipeline_in(&dev, MPP_KERNELS, name)?;
        let params = [m as u32, n as u32, k as u32];
        let enc = dev.command_encoder()?;
        enc.set_label(name);
        enc.set_compute_pipeline_state(&pipe);
        enc.set_buffer(0, Some(lhs.buffer()), ll.start_offset() * 4);
        enc.use_resource(lhs.buffer(), MTLResourceUsage::Read);
        enc.set_buffer(1, Some(rhs.buffer()), rl.start_offset() * 4);
        enc.use_resource(rhs.buffer(), MTLResourceUsage::Read);
        enc.set_buffer(2, Some(&out), 0);
        enc.use_resource(&*out, MTLResourceUsage::Write);
        enc.set_bytes_directly(3, 12, params.as_ptr() as *const std::ffi::c_void);
        enc.dispatch_thread_groups(MTLSize { width: n.div_ceil(128), height: m.div_ceil(32), depth: 1 }, MTLSize { width: 128, height: 1, depth: 1 });
        drop(enc);
        mark_on(&dev, true, || format!("mpp gemm {m}x{k} * {k}x{n}"));
        Ok((MetalStorage::new(out, dev, m * n, DType::F32), Shape::from(vec![m, n])))
    }
}

/// a (m, k) * b (k, n): the tensor-op multiply where it is used (see `mpp_ok`), else
/// Candle's.
fn mm(a: &Tensor, b: &Tensor) -> Result<Tensor> {
    let mpp = match &*a.storage_and_layout().0 {
        Storage::Metal(s) => a.rank() == 2 && b.rank() == 2 && mpp_ok(s.device()),
        _ => false,
    };
    if mpp {
        if let Ok(y) = a.apply_op2_no_bwd(b, &MppGemm) {
            return Ok(y);
        }
    }
    a.apply_op2_no_bwd(b, &Gemm)
}

/// Candle's matrix multiply (for comparisons).
#[cfg(test)]
fn mm_candle(a: &Tensor, b: &Tensor) -> Result<Tensor> {
    a.apply_op2_no_bwd(b, &Gemm)
}

/// a + b for contiguous tensors of the same shape.
fn add(a: &Tensor, b: &Tensor) -> Result<Tensor> {
    if a.dims() != b.dims() {
        candle_core::bail!("add: shapes differ")
    }
    let n = a.elem_count();
    let width = n.min(1 << 14);
    run("add2", a, &[b], vec![width as u32, n as u32], a.dims(), (width, n.div_ceil(width), 1))
}

/// One kernel call: buffers are the input, then `extra`, then the output, then `params`.
struct Kernel {
    name: &'static str,
    /// Metal source the kernel is in.
    src: &'static str,
    extra: Vec<Tensor>,
    params: Vec<u32>,
    out: Vec<usize>,
    /// Threads along x, y, z (innermost dimension first).
    grid: (usize, usize, usize),
    /// Thread-group size when the kernel needs a particular one (then `grid` counts
    /// thread groups, not threads).
    group: Option<(usize, usize, usize)>,
}

impl CustomOp1 for Kernel {
    fn name(&self) -> &'static str {
        self.name
    }
    fn cpu_fwd(&self, _: &CpuStorage, _: &Layout) -> Result<(CpuStorage, Shape)> {
        candle_core::bail!("{} runs on Metal only", self.name)
    }
    fn metal_fwd(&self, storage: &MetalStorage, layout: &Layout) -> Result<(MetalStorage, Shape)> {
        // Inputs are f32 (f16 only for the probes of other kernel sources).
        let half_ok = !std::ptr::eq(self.src, KERNELS);
        let dtype_ok = |d: DType| d == DType::F32 || (half_ok && d == DType::F16);
        if !layout.is_contiguous() || !dtype_ok(storage.dtype()) {
            candle_core::bail!("{}: input must be contiguous f32", self.name)
        }
        let dev = storage.device().clone();
        let count: usize = self.out.iter().product();
        let out = alloc(&dev, count)?;
        let pipe = pipeline_in(&dev, self.src, self.name)?;
        // Hold the extra tensors' storage for the duration of the encoding.
        let extra: Vec<_> = self.extra.iter().map(|t| t.storage_and_layout()).collect();
        let enc = dev.command_encoder()?;
        enc.set_label(self.name);
        enc.set_compute_pipeline_state(&pipe);
        enc.set_buffer(0, Some(storage.buffer()), layout.start_offset() * storage.dtype().size_in_bytes());
        enc.use_resource(storage.buffer(), MTLResourceUsage::Read);
        for (i, (s, l)) in extra.iter().enumerate() {
            let Storage::Metal(s) = &**s else { candle_core::bail!("{}: inputs must be on Metal", self.name) };
            if !l.is_contiguous() || !dtype_ok(s.dtype()) {
                candle_core::bail!("{}: input {} must be contiguous f32", self.name, i + 1)
            }
            enc.set_buffer(i + 1, Some(s.buffer()), l.start_offset() * s.dtype().size_in_bytes());
            enc.use_resource(s.buffer(), MTLResourceUsage::Read);
        }
        let k = extra.len() + 1;
        enc.set_buffer(k, Some(&out), 0);
        enc.use_resource(&*out, MTLResourceUsage::Write);
        enc.set_bytes_directly(k + 1, self.params.len() * 4, self.params.as_ptr() as *const std::ffi::c_void);
        if let Some((w, h, d)) = self.group {
            enc.dispatch_thread_groups(MTLSize { width: self.grid.0, height: self.grid.1, depth: self.grid.2 }, MTLSize { width: w, height: h, depth: d });
        } else {
            // Thread group: powers of two per axis, within the pipeline's limit.
            let limit = pipe.max_total_threads_per_threadgroup().max(1);
            let mut group = candle_metal_kernels::utils::get_block_dims(self.grid.0, self.grid.1, self.grid.2);
            while group.width * group.height * group.depth > limit {
                if group.depth > 1 {
                    group.depth /= 2
                } else if group.height > 1 {
                    group.height /= 2
                } else {
                    group.width /= 2
                }
            }
            enc.dispatch_threads(MTLSize { width: self.grid.0, height: self.grid.1, depth: self.grid.2 }, group);
        }
        drop(enc);
        mark_on(&dev, true, || format!("{} {:?}", self.name, self.out));
        Ok((MetalStorage::new(out, dev, count, DType::F32), Shape::from(self.out.clone())))
    }
}

fn run(name: &'static str, x: &Tensor, extra: &[&Tensor], params: Vec<u32>, out: &[usize], grid: (usize, usize, usize)) -> Result<Tensor> {
    x.apply_op1_no_bwd(&Kernel { name, src: KERNELS, extra: extra.iter().map(|t| (*t).clone()).collect(), params, out: out.to_vec(), grid, group: None })
}

/// As `run`, for a kernel of source `src` dispatched as `groups` thread groups of `group`.
#[allow(clippy::too_many_arguments)]
fn run_groups(src: &'static str, name: &'static str, x: &Tensor, extra: &[&Tensor], params: Vec<u32>, out: &[usize], groups: (usize, usize, usize), group: (usize, usize, usize)) -> Result<Tensor> {
    x.apply_op1_no_bwd(&Kernel { name, src, extra: extra.iter().map(|t| (*t).clone()).collect(), params, out: out.to_vec(), grid: groups, group: Some(group) })
}

/// Convolution geometry along (frequency, time).
#[derive(Clone, Copy)]
struct Geo {
    k: (usize, usize),
    s: (usize, usize),
    p: (usize, usize),
    d: (usize, usize),
}

/// The matrix-multiply input of a convolution over x (C, F, T), with a last row of ones for
/// the bias: (C*kf*kt + 1, Fo*To). Returns it with (Fo, To).
fn im2col(x: &Tensor, g: Geo, out: (usize, usize)) -> Result<Tensor> {
    let (c, f, t) = x.dims3()?;
    let rows = c * g.k.0 * g.k.1 + 1;
    let params = [c, f, t, g.k.0, g.k.1, g.s.0, g.s.1, g.p.0, g.p.1, g.d.0, g.d.1, out.0, out.1].map(|v| v as u32).to_vec();
    run("im2col", x, &[], params, &[rows, out.0 * out.1], (out.1, out.0, rows))
}

/// Group-norm statistics of x (C, F, T): [sums, sums of squares] per group (per frequency
/// row when `per_f`, else one group).
fn stats(x: &Tensor, per_f: bool) -> Result<Tensor> {
    let (c, f, t) = x.dims3()?;
    let blk = if t <= 512 { t } else { 256 };
    let nb = t.div_ceil(blk);
    let s = run("row_stats", x, &[], [c, f, t, blk, nb].map(|v| v as u32).to_vec(), &[2, f, c, nb], (nb, c, f))?;
    let groups = if per_f { f } else { 1 };
    let r = s.reshape((2 * groups, (f / groups) * c * nb))?.sum_keepdim(1)?.flatten_all()?;
    op_mark(&r, || format!("candle sum {}x{}", 2 * groups, (f / groups) * c * nb));
    Ok(r)
}

struct Norm<'a> {
    /// [weight, bias] of the group norm (+ layer scale when there is a residual).
    cst: &'a Tensor,
    per_f: bool,
}

/// norm (optional) -> activation (0 none, 1 GELU, 2 GLU) -> optional residual with layer
/// scale; `ones` appends a channel of ones for the next layer's bias.
fn norm_act(x: &Tensor, norm: Option<Norm>, act: u32, res: Option<&Tensor>, ones: bool) -> Result<Tensor> {
    let (c, f, t) = x.dims3()?;
    let cout = if act == 2 { c / 2 } else { c };
    let (st, cst, groups, n) = match &norm {
        Some(nm) => {
            let groups = if nm.per_f { f } else { 1 };
            (stats(x, nm.per_f)?, nm.cst.clone(), groups, (c * f * t / groups) as f32)
        }
        None => (x.clone(), x.clone(), 0, 1.0),
    };
    let params = vec![c as u32, f as u32, t as u32, groups as u32, act, res.is_some() as u32, n.to_bits(), 1e-5f32.to_bits()];
    let ch = cout + ones as usize;
    run("norm_act", x, &[&st, &cst, res.unwrap_or(x)], params, &[ch, f, t], (t, f, ch))
}

struct DconvLayer {
    conv: Tensor,
    norm1: Tensor,
    expand: Tensor,
    norm2: Tensor,
    dilation: usize,
}

struct Dconv(Vec<DconvLayer>);

/// Whether the fused per-row dconv kernel is used (`PREPDECK_STEM_FUSED=0` turns it off,
/// for comparisons).
fn fused_dconv() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    #[cfg(test)]
    if let Some(v) = probe::FUSED.with(|f| f.get()) {
        return v;
    }
    *ON.get_or_init(|| !std::env::var("PREPDECK_STEM_FUSED").is_ok_and(|v| v == "0"))
}

impl Dconv {
    /// x (C, F, T) -> (C [+1 ones], F, T). In the frequency branch the block runs along time
    /// separately for every frequency row.
    fn forward(&self, x: &Tensor, per_f: bool, ones: bool) -> Result<Tensor> {
        let (c, f, t) = x.dims3()?;
        let mut x = x.clone();
        for (i, l) in self.0.iter().enumerate() {
            let h = l.conv.dim(0)?;
            let last_ones = ones && i + 1 == self.0.len();
            // Spectrogram branch at the two finest levels: the whole layer in one pass per
            // frequency row (the unfused passes below are memory-bound there).
            let kernel = match h {
                6 => Some("dconv_row_6"),
                12 => Some("dconv_row_12"),
                _ => None,
            };
            // Time branch (one row, the norms over the whole sequence): three passes.
            if let (false, 1, true, true) = (per_f, f, kernel.is_some() && c == 8 * h, fused_dconv()) {
                let group = 256;
                let nb = t.div_ceil(group);
                let params = vec![t as u32, l.dilation as u32, nb as u32, last_ones as u32, 1e-5f32.to_bits()];
                let (a, b, cn) = match h {
                    6 => ("tdconv_a_6", "tdconv_b_6", "tdconv_c_6"),
                    _ => ("tdconv_a_12", "tdconv_b_12", "tdconv_c_12"),
                };
                let ys = run_groups(KERNELS, a, &x, &[&l.conv], params.clone(), &[h * t + 2 * nb], (nb, 1, 1), (group, 1, 1))?;
                let part = run_groups(KERNELS, b, &ys, &[&l.norm1, &l.expand], params.clone(), &[2 * nb], (nb, 1, 1), (group, 1, 1))?;
                x = run_groups(KERNELS, cn, &x, &[&ys, &part, &l.norm1, &l.expand, &l.norm2], params, &[c + last_ones as usize, 1, t], (nb, 1, 1), (group, 1, 1))?;
                continue;
            }
            if let (true, Some(kernel), true, true) = (per_f, kernel, c == 8 * h && t <= 1024, fused_dconv()) {
                let params = vec![c as u32, f as u32, t as u32, h as u32, l.dilation as u32, last_ones as u32, 1e-5f32.to_bits()];
                let threads = t.div_ceil(32) * 32;
                x = run_groups(KERNELS, kernel, &x, &[&l.conv, &l.norm1, &l.expand, &l.norm2], params, &[c + last_ones as usize, f, t], (f, 1, 1), (threads, 1, 1))?;
                continue;
            }
            let geo = Geo { k: (1, 3), s: (1, 1), p: (0, l.dilation), d: (1, l.dilation) };
            let y = mm(&l.conv, &im2col(&x, geo, (f, t))?)?;
            let y = norm_act(&y.reshape((h, f, t))?, Some(Norm { cst: &l.norm1, per_f }), 1, None, true)?;
            let z = mm(&l.expand, &y.reshape((h + 1, f * t))?)?;
            let c2 = z.dim(0)?;
            x = norm_act(&z.reshape((c2, f, t))?, Some(Norm { cst: &l.norm2, per_f }), 2, Some(&x), last_ones)?;
        }
        Ok(x)
    }
}

struct Encoder {
    conv: Tensor,
    dconv: Dconv,
    rewrite: Tensor,
}

struct Decoder {
    rewrite: Tensor,
    dconv: Dconv,
    /// Transposed convolution as a matrix (Co*8, C) and its bias.
    tr: Tensor,
    tr_bias: Tensor,
}

const HEADS: usize = 8;

fn ln_rows(x: &Tensor, cst: &Tensor, ones: bool) -> Result<Tensor> {
    let (l, c) = x.dims2()?;
    run_groups(KERNELS, "ln_rows", x, &[cst], vec![c as u32, ones as u32, 1e-5f32.to_bits(), l as u32], &[l, c + ones as usize], (1, l.div_ceil(8), 1), (32, 8, 1))
}

/// Columns [off, off + c) of x (L, W) as attention heads (1, heads, L, c / heads).
fn split_heads(x: &Tensor, off: usize, c: usize) -> Result<Tensor> {
    let (l, w) = x.dims2()?;
    let d = c / HEADS;
    run("split_heads", x, &[], [l, w, off, d].map(|v| v as u32).to_vec(), &[1, HEADS, l, d], (d, l, HEADS))
}

/// One transformer layer (self- or cross-attention) with its weights as transposed matrices
/// carrying the bias as a last row; layer scales are folded into `out` and `lin2`, the
/// attention scale into the query projection.
struct Layer {
    norm1: Tensor,
    norm_k: Option<Tensor>,
    in_q: Tensor,
    in_kv: Option<Tensor>,
    out: Tensor,
    norm_ff: Tensor,
    lin1: Tensor,
    lin2: Tensor,
    norm_out: Tensor,
}

/// (Co, Ci) weight and bias -> (Ci + 1, Co), rows of the weight scaled by `scale` (Co).
fn linear_t(w: &Tensor, b: &Tensor, scale: Option<&Tensor>) -> Result<Tensor> {
    let co = w.dim(0)?;
    let m = Tensor::cat(&[w.clone(), b.reshape((co, 1))?], 1)?;
    let m = match scale {
        Some(s) => m.broadcast_mul(&s.reshape((co, 1))?)?,
        None => m,
    };
    m.t()?.contiguous()
}

impl Layer {
    fn new(w: &Weights, name: &str, cross: bool) -> Result<Self> {
        let norm = |n: &str| Tensor::cat(&[get(w, &format!("{name}.{n}.weight"))?, get(w, &format!("{name}.{n}.bias"))?], 0);
        let attn = format!("{name}.{}", if cross { "cross_attn" } else { "self_attn" });
        let (iw, ib) = (get(w, &format!("{attn}.in_proj_weight"))?, get(w, &format!("{attn}.in_proj_bias"))?);
        let c = iw.dim(1)?;
        let dev = iw.device();
        let q_scale = Tensor::full((1.0 / ((c / HEADS) as f64).sqrt()) as f32, c, dev)?;
        let (in_q, in_kv) = if cross {
            (linear_t(&iw.narrow(0, 0, c)?, &ib.narrow(0, 0, c)?, Some(&q_scale))?, Some(linear_t(&iw.narrow(0, c, 2 * c)?, &ib.narrow(0, c, 2 * c)?, None)?))
        } else {
            let scale = Tensor::cat(&[q_scale, Tensor::ones(2 * c, DType::F32, dev)?], 0)?;
            (linear_t(&iw, &ib, Some(&scale))?, None)
        };
        Ok(Self {
            norm1: norm("norm1")?,
            norm_k: if cross { Some(norm("norm2")?) } else { None },
            in_q,
            in_kv,
            out: linear_t(&get(w, &format!("{attn}.out_proj.weight"))?, &get(w, &format!("{attn}.out_proj.bias"))?, Some(&get(w, &format!("{name}.gamma_1.scale"))?))?,
            norm_ff: norm(if cross { "norm3" } else { "norm2" })?,
            lin1: linear_t(&get(w, &format!("{name}.linear1.weight"))?, &get(w, &format!("{name}.linear1.bias"))?, None)?,
            lin2: linear_t(&get(w, &format!("{name}.linear2.weight"))?, &get(w, &format!("{name}.linear2.bias"))?, Some(&get(w, &format!("{name}.gamma_2.scale"))?))?,
            norm_out: norm("norm_out")?,
        })
    }

    /// q (L, C), k (S, C) for cross-attention -> (L, C [+1 ones]).
    fn forward(&self, q: &Tensor, k: Option<&Tensor>, ones: bool) -> Result<Tensor> {
        let (l, c) = q.dims2()?;
        let nq = ln_rows(q, &self.norm1, true)?;
        let (qh, kh, vh) = match (k, &self.norm_k, &self.in_kv) {
            (Some(k), Some(norm_k), Some(in_kv)) => {
                let qp = mm(&nq, &self.in_q)?;
                let kv = mm(&ln_rows(k, norm_k, true)?, in_kv)?;
                (split_heads(&qp, 0, c)?, split_heads(&kv, 0, c)?, split_heads(&kv, c, c)?)
            }
            _ => {
                let p = mm(&nq, &self.in_q)?;
                (split_heads(&p, 0, c)?, split_heads(&p, c, c)?, split_heads(&p, 2 * c, c)?)
            }
        };
        // Candle's fused attention kernel (scores, softmax and the weighted sum in one pass,
        // without storing the L × S score matrix); the 1/sqrt(d) scale is already in `in_q`.
        let attn = candle_nn::ops::sdpa(&qh, &kh, &vh, None, false, 1.0, 1.0)?;
        op_mark(&attn, || format!("sdpa {l}x{}", kh.dim(2).unwrap_or(0)));
        let d = c / HEADS;
        let merged = run("merge_heads", &attn, &[], [l, HEADS, d].map(|v| v as u32).to_vec(), &[l, c + 1], (c + 1, l, 1))?;
        let x = add(q, &mm(&merged, &self.out)?)?;
        let y = mm(&ln_rows(&x, &self.norm_ff, true)?, &self.lin1)?;
        let h = y.dim(1)?;
        let y = mm(&run("row_gelu", &y, &[], vec![h as u32], &[l, h + 1], (h + 1, l, 1))?, &self.lin2)?;
        let x = add(&x, &y)?;
        // norm_out: group norm with one group over all tokens and channels.
        let st = run("row_stats", &x, &[], [l, 1, c, c, 1].map(|v| v as u32).to_vec(), &[2, l], (1, l, 1))?.sum_keepdim(1)?.flatten_all()?;
        op_mark(&st, || format!("candle sum 2x{l}"));
        let params = vec![c as u32, ones as u32, ((l * c) as f32).to_bits(), 1e-5f32.to_bits()];
        run("row_norm", &x, &[&st, &self.norm_out], params, &[l, c + ones as usize], (c + ones as usize, l, 1))
    }
}

/// Prepared weights of one network.
pub struct Fast {
    enc: Vec<Encoder>,
    tenc: Vec<Encoder>,
    dec: Vec<Decoder>,
    tdec: Vec<Decoder>,
    /// Frequency embedding, already scaled: (48, 512).
    freq_emb: Tensor,
    /// Upsamplers to the transformer width as (385, 512); downsamplers as (384, 513).
    up: Tensor,
    up_t: Tensor,
    down: Tensor,
    down_t: Tensor,
    norm_in: Tensor,
    norm_in_t: Tensor,
    layers: Vec<Layer>,
    layers_t: Vec<Layer>,
    /// Positional embeddings for the two token sequences, made on first use.
    pos: std::sync::OnceLock<Tensor>,
    pos_t: std::sync::OnceLock<Tensor>,
}

type Weights = HashMap<String, Tensor>;

fn get(w: &Weights, name: &str) -> Result<Tensor> {
    w.get(name).cloned().ok_or_else(|| candle_core::Error::Msg(format!("missing weight {name}")))
}

/// Convolution weight (Co, C, k..) and bias as one matrix (Co, C*k + 1).
fn conv_matrix(w: &Weights, name: &str) -> Result<Tensor> {
    let weight = get(w, &format!("{name}.weight"))?;
    let co = weight.dim(0)?;
    Tensor::cat(&[weight.reshape((co, ()))?, get(w, &format!("{name}.bias"))?.reshape((co, 1))?], 1)?.contiguous()
}

fn dconv(w: &Weights, name: &str) -> Result<Dconv> {
    let mut layers = vec![];
    for i in 0..2 {
        let p = format!("{name}.layers.{i}");
        layers.push(DconvLayer {
            conv: conv_matrix(w, &format!("{p}.0"))?,
            norm1: Tensor::cat(&[get(w, &format!("{p}.1.weight"))?, get(w, &format!("{p}.1.bias"))?], 0)?,
            expand: conv_matrix(w, &format!("{p}.3"))?,
            norm2: Tensor::cat(&[get(w, &format!("{p}.4.weight"))?, get(w, &format!("{p}.4.bias"))?, get(w, &format!("{p}.6.scale"))?], 0)?,
            dilation: 1 << i,
        });
    }
    Ok(Dconv(layers))
}

impl Fast {
    pub fn new(w: &Weights) -> Result<Self> {
        let encoder = |name: String| -> Result<Encoder> {
            Ok(Encoder { conv: conv_matrix(w, &format!("{name}.conv"))?, dconv: dconv(w, &format!("{name}.dconv"))?, rewrite: conv_matrix(w, &format!("{name}.rewrite"))? })
        };
        let decoder = |name: String| -> Result<Decoder> {
            // (C, Co, 8[, 1]) -> (Co*8, C)
            let tr = get(w, &format!("{name}.conv_tr.weight"))?;
            let (c, co) = (tr.dim(0)?, tr.dim(1)?);
            let tr = tr.reshape((c, co * 8))?.t()?.contiguous()?;
            Ok(Decoder { rewrite: conv_matrix(w, &format!("{name}.rewrite"))?, dconv: dconv(w, &format!("{name}.dconv"))?, tr, tr_bias: get(w, &format!("{name}.conv_tr.bias"))? })
        };
        Ok(Self {
            enc: (0..4).map(|i| encoder(format!("encoder.{i}"))).collect::<Result<_>>()?,
            tenc: (0..4).map(|i| encoder(format!("tencoder.{i}"))).collect::<Result<_>>()?,
            dec: (0..4).map(|i| decoder(format!("decoder.{i}"))).collect::<Result<_>>()?,
            tdec: (0..4).map(|i| decoder(format!("tdecoder.{i}"))).collect::<Result<_>>()?,
            freq_emb: (get(w, "freq_emb.embedding.weight")?.t()? * 2.0)?.contiguous()?,
            up: conv_matrix(w, "channel_upsampler")?.t()?.contiguous()?,
            up_t: conv_matrix(w, "channel_upsampler_t")?.t()?.contiguous()?,
            down: conv_matrix(w, "channel_downsampler")?,
            down_t: conv_matrix(w, "channel_downsampler_t")?,
            norm_in: Tensor::cat(&[get(w, "crosstransformer.norm_in.weight")?, get(w, "crosstransformer.norm_in.bias")?], 0)?,
            norm_in_t: Tensor::cat(&[get(w, "crosstransformer.norm_in_t.weight")?, get(w, "crosstransformer.norm_in_t.bias")?], 0)?,
            layers: (0..5).map(|i| Layer::new(w, &format!("crosstransformer.layers.{i}"), i % 2 == 1)).collect::<Result<_>>()?,
            layers_t: (0..5).map(|i| Layer::new(w, &format!("crosstransformer.layers_t.{i}"), i % 2 == 1)).collect::<Result<_>>()?,
            pos: std::sync::OnceLock::new(),
            pos_t: std::sync::OnceLock::new(),
        })
    }

    /// Encoder layer over x (C, F, T); `freq` picks the axis of the stride-4 convolution.
    /// The result carries a channel of ones when `ones`.
    fn encode(&self, e: &Encoder, x: &Tensor, freq: bool, ones: bool) -> Result<Tensor> {
        let (_, f, t) = x.dims3()?;
        let (geo, out) = if freq { (Geo { k: (8, 1), s: (4, 1), p: (2, 0), d: (1, 1) }, (f / 4, t)) } else { (Geo { k: (1, 8), s: (1, 4), p: (0, 2), d: (1, 1) }, (1, t.div_ceil(4))) };
        let y = mm(&e.conv, &im2col(x, geo, out)?)?;
        let c = y.dim(0)?;
        let y = norm_act(&y.reshape((c, out.0, out.1))?, None, 1, None, false)?;
        let y = e.dconv.forward(&y, freq, true)?;
        let z = mm(&e.rewrite, &y.reshape((c + 1, out.0 * out.1))?)?;
        norm_act(&z.reshape((2 * c, out.0, out.1))?, None, 2, None, ones)
    }

    /// Decoder layer; `length` is the output size along the stride axis.
    fn decode(&self, d: &Decoder, x: &Tensor, skip: &Tensor, freq: bool, length: usize, last: bool) -> Result<Tensor> {
        let (c, f, t) = x.dims3()?;
        let y = add(x, skip)?;
        let geo = if freq { Geo { k: (3, 3), s: (1, 1), p: (1, 1), d: (1, 1) } } else { Geo { k: (1, 3), s: (1, 1), p: (0, 1), d: (1, 1) } };
        let z = mm(&d.rewrite, &im2col(&y, geo, (f, t))?)?;
        let z = norm_act(&z.reshape((2 * c, f, t))?, None, 2, None, false)?;
        let z = d.dconv.forward(&z, freq, false)?;
        let cols = mm(&d.tr, &z.reshape((c, f * t))?)?;
        let co = cols.dim(0)? / 8;
        let (fo, to) = if freq { (length, t) } else { (1, length) };
        let params = [co, f, t, if freq { 0 } else { 1 }, fo, to, if last { 0 } else { 1 }].map(|v| v as u32).to_vec();
        run("tr_col2im", &cols, &[&d.tr_bias], params, &[co, fo, to], (to, fo, co))
    }

    /// The whole network between the transforms: x is the normalised spectrogram
    /// (4, 2048, frames), xt the normalised mix (2, 1, L). Returns the spectrogram branch
    /// (16, 2048, frames) and the time branch (8, 1, L), both still normalised.
    pub fn forward(&self, x: &Tensor, xt: &Tensor) -> Result<(Tensor, Tensor)> {
        mark(x, || "upload".into());
        let (x, xt, saved, saved_t, lengths) = self.encoders(x, xt)?;
        let (f, t) = (saved[3].dim(1)?, saved[3].dim(2)?);
        let (x, xt) = self.transformer(&x, &xt, f, t)?;
        // Tokens (L, 513) back to channels first through the downsamplers.
        let x = mm(&self.down, &x.t()?)?.reshape((384, f, t))?;
        let xt = mm(&self.down_t, &xt.t()?)?.reshape((384, 1, ()))?;
        mark(&xt, || "channel downsamplers".into());
        self.decoders(&x, &xt, saved, saved_t, lengths)
    }

    /// x (L, 512) tokens in (frequency, time) order, xt (T', 512). The results carry a last
    /// column of ones for the downsamplers' bias.
    fn transformer(&self, x: &Tensor, xt: &Tensor, f: usize, t: usize) -> Result<(Tensor, Tensor)> {
        let dev = x.device();
        let c = x.dim(1)?;
        let tt = xt.dim(0)?;
        let pos = match self.pos.get() {
            Some(p) if p.dim(0)? == f * t => p.clone(),
            _ => {
                let p = Tensor::from_vec(super::pos2(f, t, c), (t, f, c), dev)?.transpose(0, 1)?.contiguous()?.reshape((f * t, c))?;
                let _ = self.pos.set(p.clone());
                p
            }
        };
        let pos_t = match self.pos_t.get() {
            Some(p) if p.dim(0)? == tt => p.clone(),
            _ => {
                let p = Tensor::from_vec(super::pos1(tt, c), (tt, c), dev)?;
                let _ = self.pos_t.set(p.clone());
                p
            }
        };
        let mut x = add(&ln_rows(x, &self.norm_in, false)?, &pos)?;
        let mut xt = add(&ln_rows(xt, &self.norm_in_t, false)?, &pos_t)?;
        mark(&xt, || "transformer input norms".into());
        for i in 0..5 {
            let last = i == 4;
            let kind = if i % 2 == 0 { "self" } else { "cross" };
            if i % 2 == 0 {
                x = self.layers[i].forward(&x, None, last)?;
                mark(&x, || format!("transformer {i} ({kind}), spectrogram tokens"));
                xt = self.layers_t[i].forward(&xt, None, last)?;
            } else {
                let old = x;
                x = self.layers[i].forward(&old, Some(&xt), last)?;
                mark(&x, || format!("transformer {i} ({kind}), spectrogram tokens"));
                xt = self.layers_t[i].forward(&xt, Some(&old), last)?;
            }
            mark(&xt, || format!("transformer {i} ({kind}), time tokens"));
        }
        Ok((x, xt))
    }

    /// Both encoders. Returns the transformer inputs as tokens (F*T, 512) and (T', 512), the
    /// skips and the time-branch lengths.
    #[allow(clippy::type_complexity)]
    fn encoders(&self, x: &Tensor, xt: &Tensor) -> Result<(Tensor, Tensor, Vec<Tensor>, Vec<Tensor>, Vec<usize>)> {
        let (mut x, mut xt) = (x.clone(), xt.clone());
        let (mut saved, mut saved_t, mut lengths) = (vec![], vec![], vec![]);
        for i in 0..4 {
            lengths.push(xt.dim(2)?);
            xt = self.encode(&self.tenc[i], &xt, false, i == 3)?;
            mark(&xt, || format!("time encoder {i}"));
            x = self.encode(&self.enc[i], &x, true, i == 3)?;
            if i == 0 {
                let (c, f, t) = x.dims3()?;
                x = run("add_cf", &x, &[&self.freq_emb], [c, f, t].map(|v| v as u32).to_vec(), &[c, f, t], (t, f, c))?;
            }
            mark(&x, || format!("spectrogram encoder {i}"));
            // The deepest layer carries a channel of ones for the upsampler's bias.
            let c = x.dim(0)? - (i == 3) as usize;
            saved.push(x.narrow(0, 0, c)?);
            saved_t.push(xt.narrow(0, 0, c)?);
        }
        let (c1, f, t) = x.dims3()?;
        let x = mm(&x.reshape((c1, f * t))?.t()?, &self.up)?;
        let xt = mm(&xt.reshape((c1, ()))?.t()?, &self.up_t)?;
        mark(&xt, || "channel upsamplers".into());
        Ok((x, xt, saved, saved_t, lengths))
    }

    /// Both decoders from the downsampled transformer outputs x (384, 8, 336), xt (384, 1, T').
    /// Returns the spectrogram branch (16, 2048, 336) and the time branch (8, 1, L).
    fn decoders(&self, x: &Tensor, xt: &Tensor, mut saved: Vec<Tensor>, mut saved_t: Vec<Tensor>, mut lengths: Vec<usize>) -> Result<(Tensor, Tensor)> {
        let (mut x, mut xt) = (x.clone(), xt.clone());
        for i in 0..4 {
            let f = x.dim(1)?;
            x = self.decode(&self.dec[i], &x, &saved.pop().unwrap(), true, f * 4, i == 3)?;
            mark(&x, || format!("spectrogram decoder {i}"));
            xt = self.decode(&self.tdec[i], &xt, &saved_t.pop().unwrap(), false, lengths.pop().unwrap(), i == 3)?;
            mark(&xt, || format!("time decoder {i}"));
        }
        Ok((x, xt))
    }
}

#[cfg(test)]
mod probe {
    //! Measurements behind the choices in this module (ignored tests, run by hand).
    use super::*;
    use candle_core::Device;

    thread_local! {
        /// Forces the fused dconv kernel on or off on this thread (None: as configured).
        pub(super) static FUSED: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    }

    const MPP: &str = r#"
#include <metal_stdlib>
#include <metal_tensor>
#include <MetalPerformancePrimitives/MetalPerformancePrimitives.h>
using namespace metal;
using namespace mpp::tensor_ops;

// c (M, N) = a (M, K) * b (K, N), row-major; p = [M, N, K].
template <typename T, int TM, int TN, int SG, int TK>
static inline void mm_tile(device T *a, device T *b, device float *c, constant uint *p, uint2 tgid) {
    int M = p[0], N = p[1], K = p[2];
    auto A = tensor<device T, dextents<int32_t, 2>, tensor_inline>(a, dextents<int32_t, 2>(K, M));
    auto B = tensor<device T, dextents<int32_t, 2>, tensor_inline>(b, dextents<int32_t, 2>(N, K));
    auto C = tensor<device float, dextents<int32_t, 2>, tensor_inline>(c, dextents<int32_t, 2>(N, M));
    if (TK == 0) {
        constexpr auto desc = matmul2d_descriptor(TM, TN, static_cast<int>(dynamic_extent));
        matmul2d<desc, execution_simdgroups<SG>> op;
        auto mA = A.slice(0, tgid.y * TM);
        auto mB = B.slice(tgid.x * TN, 0);
        auto mC = C.slice(tgid.x * TN, tgid.y * TM);
        op.run(mA, mB, mC);
    } else {
        constexpr auto desc = matmul2d_descriptor(TM, TN, TK, false, false, false, matmul2d_descriptor::mode::multiply_accumulate);
        matmul2d<desc, execution_simdgroups<SG>> op;
        auto cT = op.template get_destination_cooperative_tensor<decltype(A), decltype(B), float>();
        #pragma clang loop unroll(full)
        for (uint16_t i = 0; i < cT.get_capacity(); ++i) { if (cT.is_valid_element(i)) cT[i] = 0; }
        int k = 0;
        for (; k + TK <= K; k += TK) {
            auto mA = A.slice(k, tgid.y * TM);
            auto mB = B.slice(tgid.x * TN, k);
            op.run(mA, mB, cT);
        }
        if (k < K) {
            constexpr auto dtail = matmul2d_descriptor(TM, TN, static_cast<int>(dynamic_extent), false, false, false, matmul2d_descriptor::mode::multiply_accumulate);
            matmul2d<dtail, execution_simdgroups<SG>> tail;
            auto mA = A.slice(k, tgid.y * TM);
            auto mB = B.slice(tgid.x * TN, k);
            tail.run(mA, mB, cT);
        }
        auto mC = C.slice(tgid.x * TN, tgid.y * TM);
        cT.store(mC);
    }
}
#define MM(name, T, TM, TN, SG, TK) \
kernel void name(device T *a [[buffer(0)]], device T *b [[buffer(1)]], device float *c [[buffer(2)]], \
                 constant uint *p [[buffer(3)]], uint2 tgid [[threadgroup_position_in_grid]]) { mm_tile<T, TM, TN, SG, TK>(a, b, c, p, tgid); }
MM(mpp_f32_64_32_4, float, 64, 32, 4, 0)
MM(mpp_f32_64_64_4, float, 64, 64, 4, 0)
MM(mpp_f32_128_64_4, float, 128, 64, 4, 0)
MM(mpp_f32_64_128_4, float, 64, 128, 4, 0)
MM(mpp_f32_128_128_8, float, 128, 128, 8, 0)
MM(mpp_f32_128_64_8, float, 128, 64, 8, 0)
MM(mpp_f32_32_128_4, float, 32, 128, 4, 0)
MM(mpp_f32_64_64_4_k32, float, 64, 64, 4, 32)
MM(mpp_f32_128_64_4_k32, float, 128, 64, 4, 32)
MM(mpp_f32_64_32_4_k32, float, 64, 32, 4, 32)
MM(mpp_f16_64_32_4, half, 64, 32, 4, 0)
"#;

    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((self.0 >> 40) as f32 / (1u64 << 23) as f32) - 1.0
        }
    }

    fn time(dev: &Device, reps: usize, f: &mut dyn FnMut() -> Tensor) -> f64 {
        let _ = f();
        dev.synchronize().unwrap();
        // Outputs are dropped at once so the pool hands the same (warm) buffers out again, as
        // in a separation; keeping them would time first-touch page faults instead.
        let t = std::time::Instant::now();
        for _ in 0..reps {
            drop(f());
        }
        dev.synchronize().unwrap();
        t.elapsed().as_secs_f64() / reps as f64
    }

    /// Relative RMS error of rows `rows` of c against an f64 product on the CPU.
    fn rel_err(a: &[f32], b: &[f32], c: &[f32], (m, k, n): (usize, usize, usize), rows: usize) -> f64 {
        let (mut e, mut s) = (0f64, 0f64);
        for i in (0..m).step_by((m / rows).max(1)) {
            for j in 0..n {
                let r: f64 = (0..k).map(|q| a[i * k + q] as f64 * b[q * n + j] as f64).sum();
                e += (c[i * n + j] as f64 - r).powi(2);
                s += r * r;
            }
        }
        (e / s).sqrt()
    }

    /// Candle's matrix multiply against Metal 4's tensor-op multiply (the M5's neural
    /// accelerators) on the shapes of this network, in f32 (several tile shapes) and with
    /// half inputs.
    #[test]
    fn reads_gpu_generation() {
        assert_eq!(gpu_generation("applegpu_g17s"), Some(17));
        assert_eq!(gpu_generation("applegpu_g13g"), Some(13));
        assert_eq!(gpu_generation("intel"), None);
    }

    #[test]
    #[ignore]
    fn matmul_speed_and_precision() {
        let Ok(dev) = Device::new_metal(0) else { return };
        if let Device::Metal(m) = &dev {
            println!("GPU architecture {}, tensor-op multiply used: {}", m.metal_device().architecture_name(), mpp_ok(m));
        }
        let shapes = [(2688, 513, 2048), (2688, 2049, 512), (2688, 513, 1536), (2688, 513, 512), (1344, 513, 2048), (96, 433, 172032), (768, 3457, 2688), (192, 865, 43008), (384, 1729, 10752), (4096, 4096, 4096)];
        let variants: [(&'static str, usize, usize, usize); 10] = [
            ("mpp_f32_64_32_4", 64, 32, 4),
            ("mpp_f32_64_64_4", 64, 64, 4),
            ("mpp_f32_128_64_4", 128, 64, 4),
            ("mpp_f32_64_128_4", 64, 128, 4),
            ("mpp_f32_128_128_8", 128, 128, 8),
            ("mpp_f32_128_64_8", 128, 64, 8),
            ("mpp_f32_32_128_4", 32, 128, 4),
            ("mpp_f32_64_64_4_k32", 64, 64, 4),
            ("mpp_f32_128_64_4_k32", 128, 64, 4),
            ("mpp_f32_64_32_4_k32", 64, 32, 4),
        ];
        let mut rng = Lcg(5);
        for (m, k, n) in shapes {
            let av: Vec<f32> = (0..m * k).map(|_| rng.next()).collect();
            let bv: Vec<f32> = (0..k * n).map(|_| rng.next()).collect();
            let a = Tensor::from_vec(av.clone(), (m, k), &dev).unwrap();
            let b = Tensor::from_vec(bv.clone(), (k, n), &dev).unwrap();
            let flops = 2.0 * (m * k * n) as f64;
            let reps = (2e11 / flops).clamp(3.0, 50.0) as usize;
            let rows = if m * k * n > 1 << 33 { 4 } else { 16 };
            let t_candle = time(&dev, reps, &mut || mm_candle(&a, &b).unwrap());
            let c_candle = read(&mm_candle(&a, &b).unwrap()).unwrap();
            println!("{m}x{k} * {k}x{n}: candle {:.3} ms {:.2} TFLOPS err {:.1e}", t_candle * 1e3, flops / t_candle / 1e12, rel_err(&av, &bv, &c_candle, (m, k, n), rows));
            let params = vec![m as u32, n as u32, k as u32];
            for (name, tm, tn, sg) in variants {
                let groups = (n.div_ceil(tn), m.div_ceil(tm), 1);
                let f = || run_groups(MPP, name, &a, &[&b], params.clone(), &[m, n], groups, (32 * sg, 1, 1));
                match f() {
                    Err(e) => println!("MM  {name}: {e}"),
                    Ok(_) => {
                        let t = time(&dev, reps, &mut || f().unwrap());
                        let c = read(&f().unwrap()).unwrap();
                        let same = c.iter().zip(&c_candle).filter(|(x, y)| x == y).count() as f64 / c.len() as f64;
                        println!("MM  {name:<22} {:.3} ms {:.2} TFLOPS err {:.1e} ({:.0} % bit-equal to candle)", t * 1e3, flops / t / 1e12, rel_err(&av, &bv, &c, (m, k, n), rows), same * 100.0);
                    }
                }
            }
            let (ah, bh) = (a.to_dtype(DType::F16).unwrap(), b.to_dtype(DType::F16).unwrap());
            let groups = (n.div_ceil(32), m.div_ceil(64), 1);
            let f = || run_groups(MPP, "mpp_f16_64_32_4", &ah, &[&bh], params.clone(), &[m, n], groups, (128, 1, 1));
            let t = time(&dev, reps, &mut || f().unwrap());
            let c = read(&f().unwrap()).unwrap();
            println!("MM  half inputs (64x32)    {:.3} ms {:.2} TFLOPS err {:.1e}", t * 1e3, flops / t / 1e12, rel_err(&av, &bv, &c, (m, k, n), rows));
        }
    }

    fn rand(dev: &Device, shape: &[usize], rng: &mut Lcg) -> Tensor {
        let n: usize = shape.iter().product();
        Tensor::from_vec((0..n).map(|_| rng.next()).collect::<Vec<f32>>(), shape, dev).unwrap()
    }

    /// GPU time of this module's other kernels at the network's shapes (each run many times
    /// back to back, one wait at the end).
    #[test]
    #[ignore]
    fn kernel_times() {
        let Ok(dev) = Device::new_metal(0) else { return };
        let mut rng = Lcg(9);
        let mut row = |label: &str, bytes: usize, f: &mut dyn FnMut() -> Tensor| {
            let t = time(&dev, 30, f);
            println!("KT {label:<44} {:8.3} ms {:7.1} GB/s", t * 1e3, bytes as f64 / t / 1e9);
        };
        for l in [2688usize, 1344] {
            let x = rand(&dev, &[l, 512], &mut rng);
            let cst = rand(&dev, &[1024], &mut rng);
            row(&format!("ln_rows {l}x512 (+ones)"), l * 512 * 8, &mut || ln_rows(&x, &cst, true).unwrap());
            let st = run("row_stats", &x, &[], [l, 1, 512, 512, 1].map(|v| v as u32).to_vec(), &[2, l], (1, l, 1)).unwrap();
            row(&format!("row_stats {l}x512"), l * 512 * 4, &mut || run("row_stats", &x, &[], [l, 1, 512, 512, 1].map(|v| v as u32).to_vec(), &[2, l], (1, l, 1)).unwrap());
            row(&format!("  + candle sum 2x{l}"), l * 8, &mut || st.sum_keepdim(1).unwrap());
            let sts = st.sum_keepdim(1).unwrap().flatten_all().unwrap();
            let params = vec![512u32, 1, ((l * 512) as f32).to_bits(), 1e-5f32.to_bits()];
            row(&format!("row_norm {l}x512"), l * 512 * 8, &mut || run("row_norm", &x, &[&sts, &cst], params.clone(), &[l, 513], (513, l, 1)).unwrap());
            let p = rand(&dev, &[l, 1537], &mut rng);
            row(&format!("split_heads x3 {l}"), l * 1536 * 8, &mut || {
                let _ = split_heads(&p, 0, 512).unwrap();
                let _ = split_heads(&p, 512, 512).unwrap();
                split_heads(&p, 1024, 512).unwrap()
            });
            let q = split_heads(&p, 0, 512).unwrap();
            row(&format!("sdpa {l}x{l} (8 heads of 64)"), 0, &mut || candle_nn::ops::sdpa(&q, &q, &q, None, false, 1.0, 1.0).unwrap());
            let a = candle_nn::ops::sdpa(&q, &q, &q, None, false, 1.0, 1.0).unwrap();
            row(&format!("merge_heads {l}"), l * 512 * 8, &mut || run("merge_heads", &a, &[], [l, HEADS, 64].map(|v| v as u32).to_vec(), &[l, 513], (513, l, 1)).unwrap());
            let h = rand(&dev, &[l, 2048], &mut rng);
            row(&format!("row_gelu {l}x2048"), l * 2048 * 8, &mut || run("row_gelu", &h, &[], vec![2048], &[l, 2049], (2049, l, 1)).unwrap());
            row(&format!("add2 {l}x512"), l * 512 * 12, &mut || add(&x, &x).unwrap());
        }
        // Convolution path at the largest levels.
        for (c, f, t, label) in [(48usize, 512usize, 336usize, "spec level 0"), (48, 1, 85995, "time level 0"), (96, 128, 336, "spec level 1")] {
            let x = rand(&dev, &[c, f, t], &mut rng);
            let n = c * f * t;
            row(&format!("im2col k3 {label} -> [{}, {}]", 3 * c + 1, f * t), (3 * c + 1) * f * t * 4 + n * 4, &mut || im2col(&x, Geo { k: (1, 3), s: (1, 1), p: (0, 1), d: (1, 1) }, (f, t)).unwrap());
            let h = c / 8;
            let y = rand(&dev, &[h, f, t], &mut rng);
            let n1 = rand(&dev, &[2 * h], &mut rng);
            row(&format!("norm_act gelu per-f {label} [{h}]"), h * f * t * 8, &mut || norm_act(&y, Some(Norm { cst: &n1, per_f: f > 1 }), 1, None, true).unwrap());
            let z = rand(&dev, &[2 * c, f, t], &mut rng);
            let n2 = rand(&dev, &[5 * c], &mut rng);
            row(&format!("norm_act glu+res per-f {label} [{}]", 2 * c), 2 * c * f * t * 4 + 2 * n * 4, &mut || norm_act(&z, Some(Norm { cst: &n2, per_f: f > 1 }), 2, Some(&x), false).unwrap());
            row(&format!("  stats only {label} [{}]", 2 * c), 2 * c * f * t * 4, &mut || stats(&z, f > 1).unwrap());
            let w = rand(&dev, &[h, 3 * c + 1], &mut rng);
            let cols = im2col(&x, Geo { k: (1, 3), s: (1, 1), p: (0, 1), d: (1, 1) }, (f, t)).unwrap();
            row(&format!("gemm {h}x{} * cols {label}", 3 * c + 1), (3 * c + 1) * f * t * 4, &mut || mm(&w, &cols).unwrap());
            let e = rand(&dev, &[2 * c, h + 1], &mut rng);
            let yy = rand(&dev, &[h + 1, f * t], &mut rng);
            row(&format!("gemm {}x{} * {}x{} {label}", 2 * c, h + 1, h + 1, f * t), 2 * c * f * t * 4, &mut || mm(&e, &yy).unwrap());
        }
    }

    /// Candle's multiply against the tensor-op one on every multiply shape of the network
    /// (one segment), to choose between them per shape.
    #[test]
    #[ignore]
    fn gemm_policy() {
        let Ok(dev) = Device::new_metal(0) else { return };
        let Device::Metal(md) = &dev else { return };
        if !mpp_ok(md) {
            println!("tensor-op multiply not available here");
            return;
        }
        // (m, k, n, calls per segment); "t" marks the transposed operand layouts used.
        let shapes: &[(usize, usize, usize, usize, &str)] = &[
            (2688, 513, 2048, 5, ""), (2688, 2049, 512, 5, ""), (1344, 2049, 512, 5, ""), (1344, 513, 2048, 5, ""), (2688, 513, 512, 7, ""), (2688, 513, 1536, 3, ""),
            (1344, 513, 512, 7, ""), (2688, 513, 1024, 2, ""), (1344, 513, 1536, 3, ""), (1344, 513, 1024, 2, ""),
            (192, 865, 43008, 1, ""), (96, 433, 172032, 1, ""), (384, 1729, 10752, 1, ""), (768, 3457, 2688, 1, ""),
            (48, 17, 85995, 1, ""), (6, 145, 172032, 4, ""), (24, 577, 10752, 4, ""), (384, 25, 10752, 4, ""), (12, 289, 43008, 4, ""), (96, 7, 172032, 4, ""),
            (6, 145, 85995, 4, ""), (768, 192, 10752, 1, ""), (24, 577, 5375, 4, ""), (192, 13, 43008, 4, ""), (1536, 384, 2688, 1, ""), (96, 7, 85995, 4, ""),
            (48, 33, 172032, 1, ""), (12, 289, 21499, 4, ""), (48, 1153, 2688, 4, ""), (768, 1153, 1344, 1, ""), (48, 1153, 1344, 4, ""), (384, 25, 5375, 4, ""),
            (192, 13, 21499, 4, ""), (384, 577, 5375, 1, ""), (768, 49, 2688, 4, ""), (128, 48, 172032, 1, ""), (384, 96, 43008, 1, ""), (192, 289, 21499, 1, ""),
            (768, 49, 1344, 4, ""), (384, 1537, 2688, 1, ""), (96, 385, 43008, 1, ""), (192, 769, 10752, 1, ""), (768, 192, 5375, 1, ""), (96, 145, 85995, 1, ""),
            (96, 49, 172032, 1, ""), (384, 96, 21499, 1, ""), (192, 97, 43008, 1, ""), (96, 49, 85995, 1, ""), (384, 193, 10752, 1, ""), (1536, 384, 1344, 1, ""),
            (384, 1537, 1344, 1, ""), (96, 385, 21499, 1, ""), (768, 385, 2688, 1, ""), (192, 769, 5375, 1, ""), (384, 513, 2688, 1, "nt"), (2688, 385, 512, 1, "tn"),
            (192, 97, 21499, 1, ""), (64, 48, 85995, 1, ""), (768, 385, 1344, 1, ""), (384, 513, 1344, 1, "nt"), (1344, 385, 512, 1, "tn"),
        ];
        let mut rng = Lcg(11);
        let (mut t_c, mut t_m, mut t_best) = (0.0, 0.0, 0.0);
        for &(m, k, n, calls, lay) in shapes {
            let a = if lay == "tn" { rand(&dev, &[k, m], &mut rng).t().unwrap() } else { rand(&dev, &[m, k], &mut rng) };
            let b = if lay == "nt" { rand(&dev, &[n, k], &mut rng).t().unwrap() } else { rand(&dev, &[k, n], &mut rng) };
            let reps = 20;
            let tc = time(&dev, reps, &mut || mm_candle(&a, &b).unwrap());
            let tm = time(&dev, reps, &mut || a.apply_op2_no_bwd(&b, &MppGemm).unwrap());
            t_c += tc * calls as f64;
            t_m += tm * calls as f64;
            t_best += tc.min(tm) * calls as f64;
            println!("GP {m:>5}x{k:<5}x{n:<7}{lay:>3} x{calls}  candle {:7.3} ms  mpp {:7.3} ms  {}", tc * 1e3, tm * 1e3, if tm < tc { "mpp" } else { "candle" });
        }
        println!("GP per segment: candle {:.1} ms, mpp {:.1} ms, best of both {:.1} ms", t_c * 1e3, t_m * 1e3, t_best * 1e3);
    }

    /// Whether two independent chains of small kernels overlap on the GPU (they would if
    /// separate compute passes on unrelated buffers run concurrently).
    #[test]
    #[ignore]
    fn independent_chains_overlap() {
        let Ok(dev) = Device::new_metal(0) else { return };
        let mut rng = Lcg(13);
        for (c, f, t) in [(384usize, 8usize, 336usize), (192, 32, 336), (384, 1, 5375)] {
            let a = rand(&dev, &[c, f, t], &mut rng);
            let b = rand(&dev, &[c, f, t], &mut rng);
            let w = rand(&dev, &[c, c + 1], &mut rng);
            let chain = |x: &Tensor| -> Tensor {
                let mut y = x.clone();
                for _ in 0..4 {
                    let z = norm_act(&y, None, 1, None, true).unwrap();
                    y = mm(&w, &z.reshape((c + 1, f * t)).unwrap()).unwrap().reshape((c, f, t)).unwrap();
                }
                y
            };
            let ta = time(&dev, 20, &mut || chain(&a));
            let tb = time(&dev, 20, &mut || chain(&b));
            let tab = time(&dev, 20, &mut || {
                let x = chain(&a);
                let _ = chain(&b);
                x
            });
            println!("OV [{c}, {f}, {t}]: a {:.3} ms, b {:.3} ms, a then b {:.3} ms ({:.0} % of the sum)", ta * 1e3, tb * 1e3, tab * 1e3, 100.0 * tab / (ta + tb));
        }
    }

    /// One dconv layer of the spectrogram branch, fused against unfused, at the levels the
    /// fused kernel covers; and how close the two results are.
    #[test]
    #[ignore]
    fn dconv_fused_vs_unfused() {
        let Ok(dev) = Device::new_metal(0) else { return };
        let mut rng = Lcg(17);
        for (c, f, t) in [(48usize, 512usize, 336usize), (96, 128, 336), (48, 1, 85995), (96, 1, 21499)] {
            let h = c / 8;
            let mut layers = vec![];
            for dil in [1, 2] {
                let s1 = (1.0 / (3 * c) as f32).sqrt();
                let s2 = (1.0 / h as f32).sqrt();
                let conv = (rand(&dev, &[h, 3 * c + 1], &mut rng) * s1 as f64).unwrap();
                let expand = (rand(&dev, &[2 * c, h + 1], &mut rng) * s2 as f64).unwrap();
                let norm1 = Tensor::cat(&[rand(&dev, &[h], &mut rng).affine(0.2, 1.0).unwrap(), (rand(&dev, &[h], &mut rng) * 0.1).unwrap()], 0).unwrap();
                let norm2 = Tensor::cat(&[rand(&dev, &[2 * c], &mut rng).affine(0.2, 1.0).unwrap(), (rand(&dev, &[2 * c], &mut rng) * 0.1).unwrap(), rand(&dev, &[c], &mut rng).affine(0.1, 0.3).unwrap()], 0).unwrap();
                layers.push(DconvLayer { conv, norm1, expand, norm2, dilation: dil });
            }
            let d = Dconv(layers);
            let x = rand(&dev, &[c, f, t], &mut rng);
            let mut go = |fused: bool| {
                FUSED.with(|v| v.set(Some(fused)));
                let tm = time(&dev, 20, &mut || d.forward(&x, f > 1, false).unwrap());
                (tm, read(&d.forward(&x, f > 1, false).unwrap()).unwrap())
            };
            let (t0, a) = go(false);
            let (t1, b) = go(true);
            FUSED.with(|v| v.set(None));
            let (mut e, mut sig) = (0f64, 0f64);
            for (p, q) in a.iter().zip(&b) {
                sig += (*p as f64).powi(2);
                e += ((*p - *q) as f64).powi(2);
            }
            println!("DF [{c}, {f}, {t}] two layers: unfused {:.3} ms, fused {:.3} ms, agreement {:.1} dB", t0 * 1e3, t1 * 1e3, 10.0 * (sig / e.max(1e-30)).log10());
        }
    }

    /// How long compiling this module's Metal sources takes when the system's shader cache
    /// does not have them (a unique comment makes every run a cache miss).
    #[test]
    #[ignore]
    fn kernel_compile_time() {
        let Ok(Device::Metal(dev)) = Device::new_metal(0) else { return };
        let tag = format!("\n// {:?}\n", std::time::SystemTime::now());
        for (name, src) in [("kernels", KERNELS), ("tensor-op multiply", MPP_KERNELS)] {
            let src = format!("{src}{tag}");
            let t = std::time::Instant::now();
            let lib = dev.metal_device().new_library_with_source(&src, None);
            println!("KC {name}: {:.2} s ({})", t.elapsed().as_secs_f64(), if lib.is_ok() { "ok" } else { "failed" });
        }
    }
}
