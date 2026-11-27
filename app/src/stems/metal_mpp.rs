//! Kernels built on Metal 4's tensor operations (Metal Performance Primitives), for GPUs
//! where `mpp_ok` holds (the M5 generation): a fused attention that reads the heads straight
//! out of the projection's output and writes the merged heads in the layout the output
//! projection wants. All f32; the multiplies are full precision (`relaxed_precision` off).

use super::*;

/// Matrix multiplies with the step after them done on the result while it is still in
/// registers: GELU with a column of ones appended (the feed-forward's first layer), or a
/// residual added (the attention's output projection and the feed-forward's second layer).
/// The product itself is the one `mpp_mm_nn` computes (same tiles, same descriptor), and the
/// step after it the same arithmetic as the separate `row_gelu` / `add2` kernels.
pub(super) const GEMM_KERNELS: &str = r#"
#include <metal_stdlib>
#include <metal_tensor>
#include <MetalPerformancePrimitives/MetalPerformancePrimitives.h>
using namespace metal;
using namespace mpp::tensor_ops;

static inline float erf_as(float x) {
    const float a1 = 0.254829592, a2 = -0.284496736, a3 = 1.421413741, a4 = -1.453152027, a5 = 1.061405429, p = 0.3275911;
    float s = x < 0 ? -1.0 : 1.0;
    x = fabs(x);
    float t = 1.0 / (1.0 + p * x);
    float y = 1.0 - (((((a5 * t + a4) * t) + a3) * t + a2) * t + a1) * t * exp(-x * x);
    return s * y;
}
static inline float gelu_erf(float x) { return x * (1 + erf_as(x * M_SQRT1_2_F)) / 2; }

// c = a (M, K) * b (K, N), both row-major; p = [M, N, K]; M and N multiples of the tile.
// EP 1: c (M, N + 1) = GELU(a b) with ones in column N. EP 2: c (M, N) = res (M, N) + a b.
template <int TM, int TN, int EP>
static inline void mm_ep(device float *a, device float *b, device const float *res, device float *c, constant uint *p, uint2 tgid, uint tid) {
    int M = p[0], N = p[1], K = p[2];
    auto A = tensor<device float, dextents<int32_t, 2>, tensor_inline>(a, dextents<int32_t, 2>(K, M));
    auto B = tensor<device float, dextents<int32_t, 2>, tensor_inline>(b, dextents<int32_t, 2>(N, K));
    constexpr auto desc = matmul2d_descriptor(TM, TN, static_cast<int>(dynamic_extent), false, false, false);
    matmul2d<desc, execution_simdgroups<4>> op;
    auto mA = A.slice(0, tgid.y * TM);
    auto mB = B.slice(tgid.x * TN, 0);
    auto acc = op.template get_destination_cooperative_tensor<decltype(A), decltype(B), float>();
    op.run(mA, mB, acc);
    if (EP == 1) {
        #pragma clang loop unroll(full)
        for (uint16_t i = 0; i < acc.get_capacity(); ++i) { if (acc.is_valid_element(i)) { acc[i] = gelu_erf(acc[i]); } }
        auto C = tensor<device float, dextents<int32_t, 2>, tensor_inline>(c, dextents<int32_t, 2>(N, M), array<int, 2>{1, N + 1});
        acc.store(C.slice(tgid.x * TN, tgid.y * TM));
        if (tgid.x == 0) {
            for (uint r = tid; r < uint(TM); r += 128) { c[(tgid.y * TM + r) * (N + 1) + N] = 1.0; }
        }
    } else {
        auto R = tensor<device float, dextents<int32_t, 2>, tensor_inline>((device float *)res, dextents<int32_t, 2>(N, M));
        auto r = op.template get_destination_cooperative_tensor<decltype(A), decltype(B), float>();
        r.load(R.slice(tgid.x * TN, tgid.y * TM));
        #pragma clang loop unroll(full)
        for (uint16_t i = 0; i < acc.get_capacity(); ++i) { if (acc.is_valid_element(i)) { acc[i] = r[i] + acc[i]; } }
        auto C = tensor<device float, dextents<int32_t, 2>, tensor_inline>(c, dextents<int32_t, 2>(N, M));
        acc.store(C.slice(tgid.x * TN, tgid.y * TM));
    }
}
// c (Ch [+1], N) = GLU of a b, a (2 Ch, K): rows [0, Ch) times sigmoid of rows [Ch, 2 Ch) (the
// encoders' rewrite and its GLU in one pass); p = [2 Ch, N, K, ones]. Any M and N.
static inline float sigmoid(float x) { return 1.0 / (1 + exp(-x)); }
template <int TM, int TN>
static inline void mm_glu(device float *a, device float *b, device float *c, constant uint *p, uint2 tgid, uint tid) {
    int M = p[0], N = p[1], K = p[2], ones = p[3];
    int Ch = M / 2;
    auto A = tensor<device float, dextents<int32_t, 2>, tensor_inline>(a, dextents<int32_t, 2>(K, M));
    auto B = tensor<device float, dextents<int32_t, 2>, tensor_inline>(b, dextents<int32_t, 2>(N, K));
    constexpr auto desc = matmul2d_descriptor(TM, TN, static_cast<int>(dynamic_extent), false, false, false);
    matmul2d<desc, execution_simdgroups<4>> op;
    auto mB = B.slice(tgid.x * TN, 0);
    auto acc = op.template get_destination_cooperative_tensor<decltype(A), decltype(B), float>();
    auto acb = op.template get_destination_cooperative_tensor<decltype(A), decltype(B), float>();
    auto mA = A.slice(0, tgid.y * TM);
    op.run(mA, mB, acc);
    auto mA2 = A.slice(0, Ch + tgid.y * TM);
    op.run(mA2, mB, acb);
    #pragma clang loop unroll(full)
    for (uint16_t i = 0; i < acc.get_capacity(); ++i) {
        if (acc.is_valid_element(i)) {
            auto ix = acc.get_multidimensional_index(i);
            int n = int(tgid.x) * TN + ix[0], m = int(tgid.y) * TM + ix[1];
            if (m < Ch && n < N) { c[m * N + n] = acc[i] * sigmoid(acb[i]); }
        }
    }
    if (ones != 0 && tgid.y == 0) {
        for (int n = int(tid); n < TN; n += 128) { if (int(tgid.x) * TN + n < N) { c[Ch * N + int(tgid.x) * TN + n] = 1.0; } }
    }
}
kernel void mm_glu_32_128(device float *a [[buffer(0)]], device float *b [[buffer(1)]], device float *c [[buffer(2)]],
                          constant uint *p [[buffer(3)]], uint2 tgid [[threadgroup_position_in_grid]], uint tid [[thread_index_in_threadgroup]]) {
    mm_glu<32, 128>(a, b, c, p, tgid, tid);
}
#define MMEP(name, TM, TN, EP) \
kernel void name(device float *a [[buffer(0)]], device float *b [[buffer(1)]], device const float *res [[buffer(2)]], device float *c [[buffer(3)]], \
                 constant uint *p [[buffer(4)]], uint2 tgid [[threadgroup_position_in_grid]], uint tid [[thread_index_in_threadgroup]]) { \
    mm_ep<TM, TN, EP>(a, b, res, c, p, tgid, tid); \
}
MMEP(mm_gelu_32_128, 32, 128, 1)
MMEP(mm_res_32_128, 32, 128, 2)
MMEP(mm_res_64_64, 64, 64, 2)
"#;

/// a (M, K) * b (K, N), then GELU, with a last column of ones: (M, N + 1). None when the
/// shapes do not fit the tiles (the caller does it in two steps).
pub(super) fn mm_gelu(a: &Tensor, b: &Tensor) -> Result<Option<Tensor>> {
    let ((m, k), (k2, n)) = (a.dims2()?, b.dims2()?);
    if k != k2 || m % 32 != 0 || n % 128 != 0 || !a.is_contiguous() || !b.is_contiguous() {
        return Ok(None);
    }
    let params = [m, n, k].map(|v| v as u32).to_vec();
    Ok(Some(run_groups(GEMM_KERNELS, "mm_gelu_32_128", a, &[b, a], params, &[m, n + 1], (n / 128, m / 32, 1), (128, 1, 1))?))
}

/// GLU of a (2 Ch, K) * b (K, N): (Ch [+ a row of ones], N), the same values as the
/// multiply followed by `norm_act` with GLU and no norm.
pub(super) fn mm_glu(a: &Tensor, b: &Tensor, ones: bool) -> Result<Tensor> {
    let ((m, k), (k2, n)) = (a.dims2()?, b.dims2()?);
    if k != k2 || m % 2 != 0 || !a.is_contiguous() || !b.is_contiguous() {
        candle_core::bail!("mm_glu: shapes")
    }
    let params = [m, n, k, ones as usize].map(|v| v as u32).to_vec();
    run_groups(GEMM_KERNELS, "mm_glu_32_128", a, &[b], params, &[m / 2 + ones as usize, n], (n.div_ceil(128), (m / 2).div_ceil(32), 1), (128, 1, 1))
}

/// res (M, N) + a (M, K) * b (K, N), or None when the shapes do not fit the tiles.
pub(super) fn mm_res(a: &Tensor, b: &Tensor, res: &Tensor, tile: (&'static str, usize, usize)) -> Result<Option<Tensor>> {
    let ((m, k), (k2, n)) = (a.dims2()?, b.dims2()?);
    let (name, tm, tn) = tile;
    if k != k2 || res.dims() != [m, n] || m % tm != 0 || n % tn != 0 || !a.is_contiguous() || !b.is_contiguous() || !res.is_contiguous() {
        return Ok(None);
    }
    let params = [m, n, k].map(|v| v as u32).to_vec();
    Ok(Some(run_groups(GEMM_KERNELS, name, a, &[b, res], params, &[m, n], (n / tn, m / tm, 1), (128, 1, 1))?))
}

pub(super) const ATTN_KERNELS: &str = r#"
#include <metal_stdlib>
#include <metal_tensor>
#include <MetalPerformancePrimitives/MetalPerformancePrimitives.h>
using namespace metal;
using namespace mpp::tensor_ops;

// Attention for one head of 64 channels, 32 queries per SIMD group, keys in blocks of BK with
// the running maximum and sum of the softmax (flash attention). Scores and the weighted sum are
// tensor-op multiplies; the softmax weights go through threadgroup memory to the second one.
// q (L, wq): head h's queries at columns oq + 64 h; kv (S, wk): its keys at ok + 64 h and values
// at ov + 64 h; out (L, wo) gets head h at columns 64 h and, when wo > 512, ones in column 512.
// p = [L, S, wq, wk, oq, ok, ov, wo]. Thread groups of NSG SIMD groups; grid (L / (32 NSG), 8).
template <int BK, int NSG>
static inline void attn(device float *q, device float *kv, device float *out, constant uint *p, uint2 tg, uint sg, uint lane,
                        threadgroup float *tp, threadgroup float *trow, threadgroup float *tmax) {
    const int L = p[0], S = p[1], wq = p[2], wk = p[3], oq = p[4], ok = p[5], ov = p[6], wo = p[7];
    const int h = tg.y;
    const int q0 = (tg.x * NSG + sg) * 32;
    if (q0 >= L) { return; }
    tp += sg * (32 * BK);
    trow += sg * 32;
    tmax += sg * 32;
    auto Q = tensor<device float, dextents<int32_t, 2>, tensor_inline>(q + q0 * wq + oq + 64 * h, dextents<int32_t, 2>(64, 32), array<int, 2>{1, wq});
    auto Ka = tensor<device float, dextents<int32_t, 2>, tensor_inline>(kv + ok + 64 * h, dextents<int32_t, 2>(64, S), array<int, 2>{1, wk});
    auto Va = tensor<device float, dextents<int32_t, 2>, tensor_inline>(kv + ov + 64 * h, dextents<int32_t, 2>(64, S), array<int, 2>{1, wk});
    auto P = tensor<threadgroup float, dextents<int32_t, 2>, tensor_inline>(tp, dextents<int32_t, 2>(BK, 32));
    constexpr auto d1 = matmul2d_descriptor(32, BK, 64, false, true, false);
    constexpr auto d2 = matmul2d_descriptor(32, 64, BK, false, false, false, matmul2d_descriptor::mode::multiply_accumulate);
    matmul2d<d1, execution_simdgroup> op1;
    matmul2d<d2, execution_simdgroup> op2;
    auto s = op1.template get_destination_cooperative_tensor<decltype(Q), decltype(Ka), float>();
    auto o = op2.template get_destination_cooperative_tensor<decltype(P), decltype(Va), float>();
    auto m = op1.template get_row_reduction_destination_cooperative_tensor<decltype(Q), decltype(Ka), float>();
    auto l = op1.template get_row_reduction_destination_cooperative_tensor<decltype(Q), decltype(Ka), float>();
    auto r = op1.template get_row_reduction_destination_cooperative_tensor<decltype(Q), decltype(Ka), float>();
    #pragma clang loop unroll(full)
    for (uint16_t i = 0; i < o.get_capacity(); ++i) { if (o.is_valid_element(i)) { o[i] = 0; } }
    #pragma clang loop unroll(full)
    for (uint16_t i = 0; i < m.get_capacity(); ++i) { if (m.is_valid_element(i)) { m[i] = -INFINITY; l[i] = 0; } }
    for (int k0 = 0; k0 < S; k0 += BK) {
        auto Kb = Ka.slice(0, k0);
        op1.run(Q, Kb, s);
        reduce_rows(s, r, reduction_operation::max, -INFINITY);
        // New running maximum; r becomes the factor for what was summed so far.
        #pragma clang loop unroll(full)
        for (uint16_t i = 0; i < m.get_capacity(); ++i) {
            if (m.is_valid_element(i)) {
                float mn = max(m[i], r[i]);
                r[i] = exp(m[i] - mn);
                m[i] = mn;
                int row = m.get_multidimensional_index(i)[0];
                trow[row] = r[i];
                tmax[row] = mn;
            }
        }
        simdgroup_barrier(mem_flags::mem_threadgroup);
        #pragma clang loop unroll(full)
        for (uint16_t i = 0; i < s.get_capacity(); ++i) {
            if (s.is_valid_element(i)) { s[i] = exp(s[i] - tmax[s.get_multidimensional_index(i)[1]]); }
        }
        auto bs = op1.template get_row_reduction_destination_cooperative_tensor<decltype(Q), decltype(Ka), float>();
        reduce_rows(s, bs, reduction_operation::sum, 0.0f);
        #pragma clang loop unroll(full)
        for (uint16_t i = 0; i < l.get_capacity(); ++i) { if (l.is_valid_element(i)) { l[i] = l[i] * r[i] + bs[i]; } }
        s.store(P);
        simdgroup_barrier(mem_flags::mem_threadgroup);
        #pragma clang loop unroll(full)
        for (uint16_t i = 0; i < o.get_capacity(); ++i) {
            if (o.is_valid_element(i)) { o[i] *= trow[o.get_multidimensional_index(i)[1]]; }
        }
        auto Vb = Va.slice(0, k0);
        op2.run(P, Vb, o);
        simdgroup_barrier(mem_flags::mem_threadgroup);
    }
    #pragma clang loop unroll(full)
    for (uint16_t i = 0; i < l.get_capacity(); ++i) {
        if (l.is_valid_element(i)) { trow[l.get_multidimensional_index(i)[0]] = 1.0f / l[i]; }
    }
    simdgroup_barrier(mem_flags::mem_threadgroup);
    #pragma clang loop unroll(full)
    for (uint16_t i = 0; i < o.get_capacity(); ++i) {
        if (o.is_valid_element(i)) {
            auto ix = o.get_multidimensional_index(i);
            out[(q0 + ix[1]) * wo + 64 * h + ix[0]] = o[i] * trow[ix[1]];
        }
    }
    if (h == 0 && wo > 512) { out[(q0 + lane) * wo + 512] = 1.0f; }
}
#define ATTN(BK, NSG) \
kernel void attn_##BK##_##NSG(device float *q [[buffer(0)]], device float *kv [[buffer(1)]], device float *out [[buffer(2)]], \
                              constant uint *p [[buffer(3)]], uint2 tg [[threadgroup_position_in_grid]], \
                              uint sg [[simdgroup_index_in_threadgroup]], uint lane [[thread_index_in_simdgroup]]) { \
    threadgroup float tp[NSG * 32 * BK]; \
    threadgroup float trow[NSG * 32]; \
    threadgroup float tmax[NSG * 32]; \
    attn<BK, NSG>(q, kv, out, p, tg, sg, lane, tp, trow, tmax); \
}
ATTN(32, 4)
ATTN(64, 2)
ATTN(64, 1)
ATTN(32, 1)
ATTN(128, 1)
"#;

/// Stride-1 convolutions without an im2col matrix: the input is copied once into a zero-padded
/// buffer whose rows are `tp = T + 2 pt` long, and each tap of the kernel is then a plain
/// matrix (C rows of `F * tp` values, one row per channel) at a fixed offset into it, so the
/// convolution is a sum of `kf * kt` multiplies (Co, C) x (C, F * tp) accumulated in place. The
/// two padding columns of each output row are computed and dropped.
pub(super) const CONV_KERNELS: &str = r#"
#include <metal_stdlib>
#include <metal_tensor>
#include <MetalPerformancePrimitives/MetalPerformancePrimitives.h>
using namespace metal;
using namespace mpp::tensor_ops;

// xp (C, F + 2 pf, T + 2 pt) [+ slack] = x (+ skip) zero-padded. Grid (Tp, Fp, C + 1); the extra
// channel zeroes the slack after the last plane. p = [C, F, T, pf, pt, has_skip, slack].
kernel void pad_add(device const float *x [[buffer(0)]], device const float *skip [[buffer(1)]], device float *xp [[buffer(2)]],
                    constant uint *p [[buffer(3)]], uint3 gid [[thread_position_in_grid]]) {
    uint C = p[0], F = p[1], T = p[2], pf = p[3], pt = p[4], has_skip = p[5], slack = p[6];
    uint Tp = T + 2 * pt, Fp = F + 2 * pf;
    uint t = gid.x, f = gid.y, c = gid.z;
    if (t >= Tp || f >= Fp) { return; }
    uint o = (c * Fp + f) * Tp + t;
    if (c == C) { if (o - C * Fp * Tp < slack) { xp[o] = 0.0; } return; }
    int fi = int(f) - int(pf), ti = int(t) - int(pt);
    float v = 0.0;
    if (fi >= 0 && fi < int(F) && ti >= 0 && ti < int(T)) {
        uint i = (c * F + uint(fi)) * T + uint(ti);
        v = x[i];
        if (has_skip != 0) { v += skip[i]; }
    }
    xp[o] = v;
}

// out (Co, F, T) = bias + sum over taps of w[tap] (Co, C) x xp at the tap's offset, where tap
// = i * kt + j sits at i * df * Tp + j * dt. p = [Co, C, F, T, Tp, plane, kf, kt, df, dt].
// GLU: out (Co / 2, F, T) = a * sigmoid(b) of the two halves of the channels (the decoders'
// rewrite and its GLU in one pass; the same arithmetic as `norm_act` without a norm).
static inline float sigmoid(float x) { return 1.0 / (1 + exp(-x)); }
template <int TM, int TN, bool GLU>
static inline void conv_taps(device float *w, device float *xp, device const float *bias, device float *out, constant uint *p, uint2 tgid) {
    const int Co = p[0], C = p[1], F = p[2], T = p[3], Tp = p[4], plane = p[5], kf = p[6], kt = p[7], df = p[8], dt = p[9];
    const int N = F * Tp;
    const int Ch = GLU ? Co / 2 : Co;
    constexpr auto desc = matmul2d_descriptor(TM, TN, static_cast<int>(dynamic_extent), false, false, false, matmul2d_descriptor::mode::multiply_accumulate);
    matmul2d<desc, execution_simdgroups<4>> op;
    auto A0 = tensor<device float, dextents<int32_t, 2>, tensor_inline>(w, dextents<int32_t, 2>(C, Co));
    auto B0 = tensor<device float, dextents<int32_t, 2>, tensor_inline>(xp, dextents<int32_t, 2>(N, C), array<int, 2>{1, plane});
    auto acc = op.template get_destination_cooperative_tensor<decltype(A0), decltype(B0), float>();
    auto acb = op.template get_destination_cooperative_tensor<decltype(A0), decltype(B0), float>();
    #pragma clang loop unroll(full)
    for (uint16_t i = 0; i < acc.get_capacity(); ++i) { if (acc.is_valid_element(i)) { acc[i] = 0; acb[i] = 0; } }
    for (int a = 0; a < kf; a++) {
        for (int b = 0; b < kt; b++) {
            auto A = tensor<device float, dextents<int32_t, 2>, tensor_inline>(w + (a * kt + b) * Co * C, dextents<int32_t, 2>(C, Co));
            auto B = tensor<device float, dextents<int32_t, 2>, tensor_inline>(xp + a * df * Tp + b * dt, dextents<int32_t, 2>(N, C), array<int, 2>{1, plane});
            auto mA = A.slice(0, tgid.y * TM);
            auto mB = B.slice(tgid.x * TN, 0);
            op.run(mA, mB, acc);
            if (GLU) {
                auto mA2 = A.slice(0, Ch + tgid.y * TM);
                op.run(mA2, mB, acb);
            }
        }
    }
    #pragma clang loop unroll(full)
    for (uint16_t i = 0; i < acc.get_capacity(); ++i) {
        if (acc.is_valid_element(i)) {
            auto ix = acc.get_multidimensional_index(i);
            int n = int(tgid.x) * TN + ix[0], m = int(tgid.y) * TM + ix[1];
            if (m < Ch && n < N) {
                int f = n / Tp, t = n - f * Tp;
                if (t < T) {
                    float v = acc[i] + bias[m];
                    if (GLU) { v = v * sigmoid(acb[i] + bias[m + Ch]); }
                    out[(m * F + f) * T + t] = v;
                }
            }
        }
    }
}
#define CONV(TM, TN) \
kernel void conv_taps_##TM##_##TN(device float *w [[buffer(0)]], device float *xp [[buffer(1)]], device const float *bias [[buffer(2)]], \
                                  device float *out [[buffer(3)]], constant uint *p [[buffer(4)]], uint2 tgid [[threadgroup_position_in_grid]]) { \
    conv_taps<TM, TN, false>(w, xp, bias, out, p, tgid); \
} \
kernel void conv_glu_##TM##_##TN(device float *w [[buffer(0)]], device float *xp [[buffer(1)]], device const float *bias [[buffer(2)]], \
                                 device float *out [[buffer(3)]], constant uint *p [[buffer(4)]], uint2 tgid [[threadgroup_position_in_grid]]) { \
    conv_taps<TM, TN, true>(w, xp, bias, out, p, tgid); \
}
CONV(32, 128)
CONV(64, 64)
CONV(64, 32)
CONV(32, 64)
"#;

/// Attention of the 8 heads of 64: queries from columns [oq, oq + 512) of q (L, wq), keys and
/// values from columns [ok, ok + 512) and [ov, ov + 512) of kv (S, wk) (the 1/sqrt(d) scale is
/// already in the queries). Returns (L, 513) with a last column of ones. L and S must be
/// multiples of 32 and of the key block.
#[allow(clippy::too_many_arguments)]
pub(super) fn attention_with(name: &'static str, nsg: usize, q: &Tensor, kv: &Tensor, oq: usize, ok: usize, ov: usize) -> Result<Tensor> {
    let (l, wq) = q.dims2()?;
    let (s, wk) = kv.dims2()?;
    let params = [l, s, wq, wk, oq, ok, ov, 513].map(|v| v as u32).to_vec();
    run_groups(ATTN_KERNELS, name, q, &[kv], params, &[l, 513], (l.div_ceil(32 * nsg), HEADS, 1), (32 * nsg, 1, 1))
}

/// A stride-1 convolution's weights as one (Co, C) matrix per tap: (kf * kt, Co, C) from
/// (Co, C, kf, kt) or (Co, C, kt).
pub(super) fn tap_weights(w: &Tensor) -> Result<Tensor> {
    match w.rank() {
        4 => {
            let (co, c, kf, kt) = w.dims4()?;
            w.permute((2, 3, 0, 1))?.contiguous()?.reshape((kf * kt, co, c))
        }
        3 => w.permute((2, 0, 1))?.contiguous(),
        _ => candle_core::bail!("tap_weights: convolution weight expected"),
    }
}

/// Stride-1 convolution of x (+ skip) (C, F, T) with zero padding (kf - 1) * df / 2 along
/// frequency and (kt - 1) * dt / 2 along time: (Co, F, T). `taps` (kf * kt, Co, C) from
/// `tap_weights`, `bias` (Co).
#[allow(clippy::too_many_arguments)]
pub(super) fn conv_with(tile: (&'static str, usize, usize), x: &Tensor, skip: Option<&Tensor>, taps: &Tensor, bias: &Tensor, k: (usize, usize), d: (usize, usize)) -> Result<Tensor> {
    let (c, f, t) = x.dims3()?;
    let (ntap, co, c2) = taps.dims3()?;
    if c2 != c || ntap != k.0 * k.1 {
        candle_core::bail!("conv_with: weights do not fit the input")
    }
    let (pf, pt) = ((k.0 - 1) * d.0 / 2, (k.1 - 1) * d.1 / 2);
    let (fp, tp) = (f + 2 * pf, t + 2 * pt);
    let plane = fp * tp;
    let slack = 2 * pt + 64;
    let params = [c, f, t, pf, pt, skip.is_some() as usize, slack].map(|v| v as u32).to_vec();
    let xp = x.apply_op1_no_bwd(&Kernel { name: "pad_add", src: CONV_KERNELS, extra: vec![skip.unwrap_or(x).clone()], params, out: vec![c * plane + slack], grid: (tp, fp, c + 1), group: None })?;
    let (name, tm, tn) = tile;
    // `conv_glu_*`: the GLU of the two channel halves, done in the same pass.
    let rows = if name.starts_with("conv_glu") { co / 2 } else { co };
    let params = [co, c, f, t, tp, plane, k.0, k.1, d.0, d.1].map(|v| v as u32).to_vec();
    run_groups(CONV_KERNELS, name, taps, &[&xp, bias], params, &[rows, f, t], ((f * tp).div_ceil(tn), rows.div_ceil(tm), 1), (128, 1, 1))
}

#[cfg(test)]
mod probe {
    use super::*;
    use candle_core::Device;

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
        let t = std::time::Instant::now();
        for _ in 0..reps {
            drop(f());
        }
        dev.synchronize().unwrap();
        t.elapsed().as_secs_f64() / reps as f64
    }

    /// The fused tensor-op attention against split heads + Candle's sdpa + merged heads, on
    /// the network's four shapes: time per call and agreement.
    #[test]
    #[ignore]
    fn attention_variants() {
        let Ok(dev) = Device::new_metal(0) else { return };
        let Device::Metal(md) = &dev else { return };
        if !mpp_ok(md) {
            println!("tensor-op multiply not available here");
            return;
        }
        let mut rng = Lcg(21);
        for (l, s) in [(2688usize, 2688usize), (2688, 1344), (1344, 2688), (1344, 1344)] {
            // Projections as the layers make them: queries scaled like in_q, plus the ones column.
            let qv: Vec<f32> = (0..l * 1537).map(|_| rng.next() * 0.6).collect();
            let kvv: Vec<f32> = (0..s * 1537).map(|_| rng.next() * 0.6).collect();
            let q = Tensor::from_vec(qv, (l, 1537), &dev).unwrap();
            let kv = Tensor::from_vec(kvv, (s, 1537), &dev).unwrap();
            let reference = || {
                let (qh, kh, vh) = (split_heads(&q, 0, 512).unwrap(), split_heads(&kv, 512, 512).unwrap(), split_heads(&kv, 1024, 512).unwrap());
                let a = candle_nn::ops::sdpa(&qh, &kh, &vh, None, false, 1.0, 1.0).unwrap();
                run("merge_heads", &a, &[], [l, HEADS, 64].map(|v| v as u32).to_vec(), &[l, 513], (513, l, 1)).unwrap()
            };
            let t_ref = time(&dev, 20, &mut || reference());
            let want = read(&reference()).unwrap();
            println!("AT {l}x{s}: split + sdpa + merge {:.3} ms", t_ref * 1e3);
            for (name, nsg) in [("attn_32_4", 4), ("attn_64_2", 2), ("attn_64_1", 1), ("attn_32_1", 1), ("attn_128_1", 1)] {
                match attention_with(name, nsg, &q, &kv, 0, 512, 1024) {
                    Err(e) => println!("AT   {name}: {e}"),
                    Ok(_) => {
                        let t = time(&dev, 20, &mut || attention_with(name, nsg, &q, &kv, 0, 512, 1024).unwrap());
                        let got = read(&attention_with(name, nsg, &q, &kv, 0, 512, 1024).unwrap()).unwrap();
                        let (mut e, mut sig) = (0f64, 0f64);
                        for (a, b) in want.iter().zip(&got) {
                            sig += (*a as f64).powi(2);
                            e += ((*a - *b) as f64).powi(2);
                        }
                        println!("AT   {name:<12} {:.3} ms, agreement {:.1} dB", t * 1e3, 10.0 * (sig / e.max(1e-30)).log10());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod conv_probe {
    use super::*;
    use candle_core::Device;

    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((self.0 >> 40) as f32 / (1u64 << 23) as f32) - 1.0
        }
    }

    fn rand(dev: &Device, shape: &[usize], rng: &mut Lcg, k: f32) -> Tensor {
        let n: usize = shape.iter().product();
        Tensor::from_vec((0..n).map(|_| rng.next() * k).collect::<Vec<f32>>(), shape, dev).unwrap()
    }

    fn time(dev: &Device, reps: usize, f: &mut dyn FnMut() -> Tensor) -> f64 {
        let _ = f();
        dev.synchronize().unwrap();
        let t = std::time::Instant::now();
        for _ in 0..reps {
            drop(f());
        }
        dev.synchronize().unwrap();
        t.elapsed().as_secs_f64() / reps as f64
    }

    fn agree(a: &[f32], b: &[f32]) -> f64 {
        let (mut e, mut s) = (0f64, 0f64);
        for (x, y) in a.iter().zip(b) {
            s += (*x as f64).powi(2);
            e += ((*x - *y) as f64).powi(2);
        }
        10.0 * (s / e.max(1e-30)).log10()
    }

    /// The decoders' 3x3 / 1x3 convolutions (with the skip added) and a dilated dconv
    /// convolution: im2col + multiply against the tap-wise multiply, time and agreement.
    #[test]
    #[ignore]
    fn conv_variants() {
        let Ok(dev) = Device::new_metal(0) else { return };
        let Device::Metal(md) = &dev else { return };
        if !mpp_ok(md) {
            println!("tensor-op multiply not available here");
            return;
        }
        let mut rng = Lcg(23);
        // (C, F, T, Co, kf, kt, dt, skip)
        let cases = [
            (384usize, 8usize, 336usize, 768usize, 3usize, 3usize, 1usize, true),
            (192, 32, 336, 384, 3, 3, 1, true),
            (96, 128, 336, 192, 3, 3, 1, true),
            (48, 512, 336, 96, 3, 3, 1, true),
            (384, 1, 1344, 768, 1, 3, 1, true),
            (192, 1, 5375, 384, 1, 3, 1, true),
            (96, 1, 21499, 192, 1, 3, 1, true),
            (48, 1, 85995, 96, 1, 3, 1, true),
            (192, 32, 336, 24, 1, 3, 2, false),
            (384, 8, 336, 48, 1, 3, 1, false),
            (192, 1, 5375, 24, 1, 3, 2, false),
        ];
        for (c, f, t, co, kf, kt, dt, with_skip) in cases {
            let x = rand(&dev, &[c, f, t], &mut rng, 1.0);
            let skip = rand(&dev, &[c, f, t], &mut rng, 1.0);
            let w = rand(&dev, &[co, c, kf, kt], &mut rng, (1.0 / (c * kf * kt) as f32).sqrt());
            let bias = rand(&dev, &[co], &mut rng, 0.1);
            let matrix = Tensor::cat(&[w.reshape((co, ())).unwrap(), bias.reshape((co, 1)).unwrap()], 1).unwrap().contiguous().unwrap();
            let geo = Geo { k: (kf, kt), s: (1, 1), p: ((kf - 1) / 2, (kt - 1) * dt / 2), d: (1, dt) };
            let old = || {
                let y = if with_skip { add(&x, &skip).unwrap() } else { x.clone() };
                mm(&matrix, &im2col(&y, geo, (f, t)).unwrap()).unwrap().reshape((co, f, t)).unwrap()
            };
            let taps = tap_weights(&w).unwrap();
            let t_old = time(&dev, 20, &mut || old());
            let want = read(&old()).unwrap();
            print!("CV [{c}, {f}, {t}] -> {co} k{kf}x{kt} d{dt}: im2col + mm {:.3} ms", t_old * 1e3);
            for tile in [("conv_taps_32_128", 32, 128), ("conv_taps_64_64", 64, 64), ("conv_taps_64_32", 64, 32), ("conv_taps_32_64", 32, 64)] {
                let new = || conv_with(tile, &x, with_skip.then_some(&skip), &taps, &bias, (kf, kt), (1, dt)).unwrap();
                let tn = time(&dev, 20, &mut || new());
                let got = read(&new()).unwrap();
                print!(" | {} {:.3} ms {:.0} dB", &tile.0[10..], tn * 1e3, agree(&want, &got));
            }
            println!();
        }
    }
}

#[cfg(test)]
mod tile_probe {
    use super::*;
    use candle_core::Device;

    /// Whether the residual multiply gives the same bits with 64 x 64 and 32 x 128 tiles, and
    /// both the same as the plain multiply plus `add`, on the transformer's shapes.
    #[test]
    #[ignore]
    fn residual_tiles_agree() {
        let Ok(dev) = Device::new_metal(0) else { return };
        let Device::Metal(md) = &dev else { return };
        if !mpp_ok(md) {
            return;
        }
        let mut seed = 5u64;
        let mut rnd = |n: usize| -> Vec<f32> {
            (0..n)
                .map(|_| {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    ((seed >> 40) as f32 / (1u64 << 23) as f32) - 1.0
                })
                .collect()
        };
        for (m, k, n) in [(2688usize, 513usize, 512usize), (2688, 2049, 512), (1344, 513, 512), (1344, 2049, 512)] {
            let a = Tensor::from_vec(rnd(m * k), (m, k), &dev).unwrap();
            let b = Tensor::from_vec(rnd(k * n), (k, n), &dev).unwrap();
            let r = Tensor::from_vec(rnd(m * n), (m, n), &dev).unwrap();
            let plain = read(&add(&r, &mm(&a, &b).unwrap()).unwrap()).unwrap();
            let t1 = read(&mm_res(&a, &b, &r, ("mm_res_32_128", 32, 128)).unwrap().unwrap()).unwrap();
            let t2 = read(&mm_res(&a, &b, &r, ("mm_res_64_64", 64, 64)).unwrap().unwrap()).unwrap();
            let same = |x: &[f32], y: &[f32]| x.iter().zip(y).filter(|(p, q)| p.to_bits() == q.to_bits()).count() as f64 / x.len() as f64 * 100.0;
            println!("RT {m}x{k}x{n}: 32x128 vs plain {:.2} % bit-equal, 64x64 vs plain {:.2} %", same(&t1, &plain), same(&t2, &plain));
        }
    }
}
