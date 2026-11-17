//! Rule-based refinement of the key candidates (back half of the per-buffer key stage,
//! 0x10114c3c0 … 0x101156104 of the reference routine).
//!
//! Input is the 12 pitch-class energies of one buffer ("chroma", index 0 = A) and, for buffers
//! long enough to hold 30-second windows, the raw constant-Q magnitude matrix. Keys are
//! 0..11 = major on A, Bb, … Ab and 12..23 = the minors in the same order.
//!
//! Everything here works on *ranks*: the position (0 = strongest) of a pitch class in the
//! chroma ordering. The chroma values themselves are never compared again after ranking.
//!
//! Stages, in the order of the original:
//!  1. first-stage table scores → best and second-best key (`first_stage`);
//!  2. short buffers only: a major/minor pair on the same tonic is re-ordered by the ranks of
//!     the two thirds, sixths and sevenths (`settle_parallel_pair`);
//!  3. the second key replaces the first when it is a minor whose tonic is among the three
//!     strongest pitch classes and its scale notes rank better (`prefers_second`);
//!  4. long buffers only: the same analysis on every 30-second window (10-second step) of the
//!     raw matrix; windows whose two internal opinions agree vote for a key, and the most
//!     voted key may override the result (`window_vote`, `final_choice`).
//!
//! Not ported because unreachable in this application: the other analysis modes (the routine's
//! mode argument is always 1, so only table rows 2 and 3 are used), the 4000 Hz rate, and
//! segment modes ≥ 2 (which zero the three lowest octave weights).


/// Semitone offsets of the seven scale degrees: major, natural minor.
const SCALE: [[usize; 7]; 2] = [[0, 2, 4, 5, 7, 9, 11], [0, 2, 3, 5, 7, 8, 10]];

const TONIC: usize = 0;
const SECOND: usize = 1;
const THIRD: usize = 2;
const FIFTH: usize = 4;
const SIXTH: usize = 5;
const SEVENTH: usize = 6;

const ALL_DEGREES: [usize; 7] = [0, 1, 2, 3, 4, 5, 6];
const TRIAD: [usize; 3] = [TONIC, THIRD, FIFTH];
const TRIAD_AND_SECOND: [usize; 4] = [TONIC, SECOND, THIRD, FIFTH];
const UPPER: [usize; 2] = [SIXTH, SEVENTH];

/// "No key": one past the last key index.
const NONE: i32 = 24;

/// Position of each key on the harmonic-mixing wheel: minors 0..11, majors 12..23, a step of
/// one being a fifth; a major and its relative minor are 12 apart.
const WHEEL: [i32; 24] = [22, 17, 12, 19, 14, 21, 16, 23, 18, 13, 20, 15, 7, 2, 9, 4, 11, 6, 1, 8, 3, 10, 5, 0];

fn is_minor(key: i32) -> bool {
    key >= 12
}

/// Pitch class (0 = A) of scale degree `n` (0 = tonic … 6 = seventh) of `key`.
pub fn degree(key: i32, n: usize) -> usize {
    (key as usize % 12 + SCALE[is_minor(key) as usize][n]) % 12
}

/// Major and minor on the same tonic.
fn parallel(a: i32, b: i32) -> bool {
    b - 12 == a || b + 12 == a
}

/// Indices of `v` by descending value, equal values in ascending index order.
fn order_desc(v: &[f32]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..v.len()).collect();
    idx.sort_by(|&a, &b| v[b].partial_cmp(&v[a]).unwrap_or(std::cmp::Ordering::Equal).then(a.cmp(&b)));
    idx
}

/// Pitch classes ordered by energy, and the inverse map.
struct Ranking {
    /// Pitch classes, strongest first.
    order: [usize; 12],
    /// Rank (0 = strongest) of each pitch class.
    rank: [i32; 12],
}

impl Ranking {
    fn new(chroma: &[f32; 12]) -> Ranking {
        let o = order_desc(chroma);
        let mut r = Ranking { order: [0; 12], rank: [0; 12] };
        for (pos, &pc) in o.iter().enumerate() {
            r.order[pos] = pc;
            r.rank[pc] = pos as i32;
        }
        r
    }

    /// Rank of one scale degree of `key`.
    fn of(&self, key: i32, n: usize) -> i32 {
        self.rank[degree(key, n)]
    }

    /// Sum of the ranks of the given degrees of `key` (lower = better supported).
    fn sum(&self, key: i32, degrees: &[usize]) -> i32 {
        degrees.iter().map(|&n| self.of(key, n)).sum()
    }

    /// How much worse `second` is supported than `first` on the given degrees
    /// (negative = `second` fits better).
    fn deficit(&self, first: i32, second: i32, degrees: &[usize]) -> i32 {
        self.sum(second, degrees) - self.sum(first, degrees)
    }
}

// ------------------------------------------------------------------------------------------
// First stage: table scores
// ------------------------------------------------------------------------------------------

/// Score tables of the first stage: tonic by rank 0..4, third/fourth/fifth by rank 0..6.
pub struct Tables {
    pub tonic_major: [f32; 5],
    pub tonic_minor: [f32; 5],
    pub third: [f32; 7],
    pub fourth: [f32; 7],
    pub fifth: [f32; 7],
}

/// Whole buffer, 30 s or longer (row 2 of table_1st_*, table_3rd, table_4th2, table_5th).
pub const TABLES_LONG: Tables = Tables {
    tonic_major: [3.2, 2.7, 2.1, 1.6, 1.1],
    tonic_minor: [3.781, 2.7, 2.1, 1.6, 1.1],
    third: [2.4, 1.92, 1.42, 1.229, 0.84, 0.4, 0.0],
    fourth: [0.2, 0.16, 0.128, 0.102, 0.072, 0.043, 0.0],
    fifth: [2.4, 1.92, 1.536, 1.229, 0.8, 0.48, 0.0],
};

/// Buffer shorter than 30 s (row 3 of the same tables).
pub const TABLES_SHORT: Tables = Tables {
    tonic_major: [3.2, 2.7, 2.1, 1.6, 1.1],
    tonic_minor: [3.67, 2.7, 2.1, 1.6, 1.1],
    third: [2.4, 2.0, 1.6, 1.2, 0.8, 0.4, 0.0],
    fourth: [0.2, 0.17, 0.13, 0.1, 0.07, 0.03, 0.0],
    fifth: [2.4, 2.0, 1.6, 1.2, 0.8, 0.4, 0.0],
};

/// 30-second windows (row 2 of table_1st_*2, table_3rd2, table_4th2, table_5th2).
const TABLES_WINDOW: Tables = Tables {
    tonic_major: [3.2, 2.7, 2.1, 1.6, 1.1],
    tonic_minor: [3.781, 2.7, 2.1, 1.6, 1.1],
    third: [2.4, 1.92, 1.536, 1.229, 0.8, 0.4, 0.0],
    fourth: [0.2, 0.16, 0.128, 0.102, 0.072, 0.043, 0.0],
    fifth: [2.4, 1.92, 1.536, 1.229, 0.8, 0.48, 0.0],
};

/// Each of the five strongest pitch classes is tried as a major and as a minor tonic; a
/// candidate scores by its tonic's rank plus the ranks of its third, fourth and fifth among
/// the seven strongest. Returns the two best-scoring keys.
fn first_stage(r: &Ranking, t: &Tables) -> (i32, i32) {
    let mut scores = [0f32; 10];
    for pos in 0..5 {
        let tonic = r.order[pos];
        let mut major = t.tonic_major[pos];
        let mut minor = t.tonic_minor[pos];
        for m in 0..7 {
            match (r.order[m] + 12 - tonic) % 12 {
                4 => major += t.third[m],
                3 => minor += t.third[m],
                5 => {
                    major += t.fourth[m];
                    minor += t.fourth[m];
                }
                7 => {
                    major += t.fifth[m];
                    minor += t.fifth[m];
                }
                _ => {}
            }
        }
        scores[2 * pos] = major;
        scores[2 * pos + 1] = minor;
    }
    let by_score = order_desc(&scores);
    let key = |s: usize| (r.order[s / 2] + if s % 2 == 1 { 12 } else { 0 }) as i32;
    (key(by_score[0]), key(by_score[1]))
}

// ------------------------------------------------------------------------------------------
// Pair rules: should the second candidate replace the first?
// ------------------------------------------------------------------------------------------

/// Short buffers, candidates on the same tonic (0x10114c3c0): the minor is taken as the
/// first candidate unless its third ranks below the three strongest pitch classes *and* more
/// than two places below the major third *and* the major's third, sixth and seventh together
/// rank better than the minor's. The second candidate is left as it was.
fn settle_parallel_pair(r: &Ranking, k1: i32, k2: i32) -> i32 {
    let (major, minor) = if is_minor(k1) { (k2, k1) } else { (k1, k2) };
    let minor_third = r.of(minor, THIRD);
    if minor_third < 3 {
        return minor;
    }
    let major_third = r.of(major, THIRD) + 2;
    if major_third >= minor_third {
        return minor;
    }
    let major_side = major_third + r.of(major, SIXTH) + r.of(major, SEVENTH);
    let minor_side = r.of(minor, SIXTH) + minor_third + r.of(minor, SEVENTH);
    if major_side >= minor_side {
        minor
    } else {
        major
    }
}

/// Neighbours on the wheel within one mode: a fifth up or down. The explicit key tests stop
/// a step between the last minor position (11, key 16) and the first major one (12, key 2).
fn fifth_apart(k1: i32, k2: i32) -> bool {
    let (h1, h2) = (WHEEL[k1 as usize], WHEEL[k2 as usize]);
    (k1 != 16 && h2 - 1 == h1)
        || (k1 != 2 && h2 + 1 == h1)
        || (h2 + 11 == h1 && (h1 == 11 || h1 == 23))
        || (h2 - 11 == h1 && (h1 == 0 || h1 == 12))
}

/// Whole-buffer version of "closely related" (0x10114c9e0): a fifth apart, or a major and a
/// minor whose wheel positions differ by one step (the relative key of a fifth-neighbour).
fn closely_related(k1: i32, k2: i32) -> bool {
    if fifth_apart(k1, k2) {
        return true;
    }
    let (h1, h2) = (WHEEL[k1 as usize], WHEEL[k2 as usize]);
    if is_minor(k1) && !is_minor(k2) {
        if (h2 - 13 == h1 && k1 != 16) || h2 - 11 == h1 {
            return true;
        }
        if h2 - 1 == h1 && (h1 == 11 || h1 == 23) {
            return true;
        }
        h2 - 23 == h1 && k1 == 23
    } else if !is_minor(k1) && is_minor(k2) {
        if (h1 != 23 && h2 - 1 == h1 - 12) || h2 == h1 - 13 {
            return true;
        }
        if h2 == h1 - 23 && h1 == 23 {
            return true;
        }
        h2 == h1 - 1 && h1 == 12
    } else {
        false
    }
}

/// The two flavours of the pair rules: the whole buffer and one 30-second window.
#[derive(Clone, Copy, PartialEq)]
enum Scope {
    Buffer,
    Window,
}

/// Decides whether the second candidate `k2` should replace the first, `k1`.
///
/// Three independent reasons, any of which is enough:
///  * `k2` is the parallel minor of a major `k1` and the major's third / sixth / seventh rank
///    worse than the minor's;
///  * `k2` is a minor on the strongest pitch class and its scale is not much worse supported
///    than `k1`'s;
///  * `k2` is a minor on the second or third strongest pitch class and its scale is clearly
///    better supported than `k1`'s.
fn prefers_second(r: &Ranking, k1: i32, k2: i32, scope: Scope) -> bool {
    // Parallel minor of a major first candidate (0x10114c6d4 / window 0x10114fa5c).
    let mut parallel_minor_wins = false;
    if k1 + 12 == k2 {
        let worse = [THIRD, SIXTH, SEVENTH].map(|n| r.of(k1, n) > r.of(k2, n));
        let count = worse.iter().filter(|&&w| w).count();
        parallel_minor_wins = match scope {
            Scope::Buffer => count > 1 || (count == 1 && r.deficit(k1, k2, &[THIRD, SIXTH, SEVENTH]) < 0),
            Scope::Window => count > 0,
        };
    }

    let minor_tonic = k2 - 12;
    let top = |pos: usize| r.order[pos] as i32;
    let triad = r.deficit(k1, k2, &TRIAD);
    let upper = r.deficit(k1, k2, &UPPER);
    let whole = r.deficit(k1, k2, &ALL_DEGREES);

    let mut scale_fits = false;
    if top(0) == minor_tonic {
        // Minor on the strongest pitch class (0x10114cb14 / window 0x10114fd50).
        let limit = if is_minor(k1) { 8 } else { 6 };
        let first_triad = r.sum(k1, &TRIAD);
        let triad_slack = if scope == Scope::Buffer { 4 } else { 5 };
        if r.deficit(k1, k2, &TRIAD_AND_SECOND) < limit && (first_triad > 7 || triad <= triad_slack) {
            let tonic_rank = r.of(k1, TONIC);
            let over = |tol: i32| [whole > tol, triad > tol, upper > tol].iter().filter(|&&o| o).count();
            scale_fits = match scope {
                Scope::Buffer => {
                    let tol = if parallel(k1, k2) {
                        0
                    } else if tonic_rank < 2 {
                        2
                    } else if tonic_rank == 2 {
                        3
                    } else {
                        4
                    };
                    // Related keys must pass all three comparisons, unrelated ones any.
                    if closely_related(k1, k2) {
                        over(tol) == 0
                    } else {
                        over(tol) < 3
                    }
                }
                Scope::Window => {
                    let tol = if tonic_rank <= 2 {
                        2
                    } else if tonic_rank == 3 {
                        3
                    } else {
                        4
                    };
                    if fifth_apart(k1, k2) {
                        over(tol) < 2
                    } else {
                        over(tol) < 3
                    }
                }
            };
        }
    } else if top(1) == minor_tonic || top(2) == minor_tonic {
        // Minor on the second or third strongest pitch class (0x10114d0a0 and 0x101155c9c /
        // window 0x101150a60).
        if whole < 0 && triad < 0 && upper < 0 {
            return true;
        }
        let total = triad + whole + upper;
        scale_fits = if whole >= 1 && triad >= 1 && upper > 0 {
            false
        } else if whole > 4 || upper > 4 {
            (upper < 1 || whole < 1) && triad <= -6 && total < 4
        } else {
            total < 4 && triad <= 0 && (triad < -4 || r.deficit(k1, k2, &TRIAD_AND_SECOND) < -4)
        };
    }
    parallel_minor_wins || scale_fits
}

// ------------------------------------------------------------------------------------------
// Second opinion: weighted scale fit of all 24 keys
// ------------------------------------------------------------------------------------------

// rank × 0.9 and rank × 0.6 as the original's literal constants (not products).
const TIMES_0_9: [f32; 12] = [0.9, 1.8, 2.7, 3.6, 4.5, 5.4, 6.3, 7.2, 8.1, 9.0, 9.9, 10.8];
const TIMES_0_6: [f32; 12] = [0.6, 1.2, 1.8, 2.4, 3.0, 3.6, 4.2, 4.8, 5.4, 6.0, 6.6, 7.2];

/// All 24 keys ordered by how well the chroma ranking fits their scale (0x10114d5dc).
struct ScaleFit {
    /// Keys, best fit (lowest weighted rank sum) first.
    keys: [i32; 24],
    /// Per key: sum of the 1-based ranks of tonic, third and fifth.
    triad: [f32; 24],
    /// Per key: sum of the 1-based ranks of all seven degrees.
    whole: [f32; 24],
}

impl ScaleFit {
    fn new(r: &Ranking) -> ScaleFit {
        let mut weighted = [0f32; 24];
        let mut triad = [0f32; 24];
        let mut whole = [0f32; 24];
        for key in 0..24 {
            let rk = |n: usize| r.of(key as i32, n) as usize; // 0-based
            let one = |n: usize| (rk(n) + 1) as f32;
            // Weights 4, 0.9, 3, 0.9, 3.5, 0.6, 2 for tonic … seventh, added in this order.
            let mut s = 0f32;
            s += 4.0 * one(0);
            s += TIMES_0_9[rk(1)];
            s += 3.0 * one(2);
            s += TIMES_0_9[rk(3)];
            s += 3.5 * one(4);
            s += TIMES_0_6[rk(5)];
            s += 2.0 * one(6);
            weighted[key] = s;
            triad[key] = (one(0) + one(2)) + one(4);
            let mut w = 0f32;
            for n in 0..7 {
                w += one(n);
            }
            whole[key] = w;
        }
        // Ascending, obtained as the original does: descending sort (equal values by
        // ascending key), reversal, then one bubble pass over equal neighbours.
        let mut idx = order_desc(&weighted);
        idx.reverse();
        for i in 0..23 {
            if weighted[idx[i]] == weighted[idx[i + 1]] && idx[i] > idx[i + 1] {
                idx.swap(i, i + 1);
            }
        }
        let mut keys = [0i32; 24];
        for (i, &k) in idx.iter().enumerate() {
            keys[i] = k as i32;
        }
        ScaleFit { keys, triad, whole }
    }
}

/// Support a candidate gets from a set of well-fitting keys: each key a fifth away counts 1
/// (for a minor candidate the first such key counts 1.6), each relative major/minor 1.5, the
/// parallel key 3.
fn wheel_support(candidate: i32, others: &[i32]) -> f32 {
    let h = WHEEL[candidate as usize];
    let mut score = 0f32;
    let mut seen_fifth = false;
    for &o in others {
        let ho = WHEEL[o as usize];
        let wraps = (ho + 11 == h && (h == 11 || h == 23)) || (ho - 11 == h && (h == 0 || h == 12));
        let fifth = if is_minor(candidate) {
            (candidate != 16 && ho - 1 == h) || ho + 1 == h || wraps
        } else {
            ho - 1 == h || (candidate != 2 && ho + 1 == h) || wraps
        };
        if fifth {
            score += if is_minor(candidate) && !seen_fifth { 1.6 } else { 1.0 };
            seen_fifth = true;
        } else if ho + 12 == h || ho - 12 == h {
            score += 1.5;
        } else if parallel(candidate, o) {
            score += 3.0;
        }
    }
    score
}

/// The five best-fitting keys re-ranked by wheel support from `others` plus a bonus for
/// their fit position; returns positions into the five, best first.
fn rank_by_support(top: &[i32], others: &[i32], bonus: [f32; 5]) -> Vec<usize> {
    let mut scores = [0f32; 5];
    for i in 0..5 {
        scores[i] = wheel_support(top[i], others) + bonus[i];
    }
    order_desc(&scores)
}

// ------------------------------------------------------------------------------------------
// 30-second windows
// ------------------------------------------------------------------------------------------

/// Sum in the original's order: four interleaved partial sums, combined pairwise, then the
/// remaining one to three values.
fn sum_lanes(v: &[f32]) -> f32 {
    let mut lane = [0f32; 4];
    let quads = v.len() / 4;
    for q in 0..quads {
        for k in 0..4 {
            lane[k] += v[4 * q + k];
        }
    }
    let mut s = if quads > 0 { (lane[0] + lane[1]) + (lane[2] + lane[3]) } else { 0.0 };
    for &x in &v[4 * quads..] {
        s += x;
    }
    s
}

/// Octave weights, lowest octave first (mode 1, segment modes 0 and 1).
pub const OCTAVE_WEIGHTS: [f32; 6] = [0.65, 0.5, 1.0, 1.0, 1.0, 1.0];

/// Squared weighted sum over octaves of the per-octave pitch-class sums.
pub fn fold_octaves(octave_sum: impl Fn(usize, usize) -> f32) -> [f32; 12] {
    let mut out = [0f32; 12];
    for pc in 0..12 {
        let mut v = octave_sum(0, pc) * OCTAVE_WEIGHTS[0] + octave_sum(1, pc) * OCTAVE_WEIGHTS[1];
        for oct in 2..6 {
            v += octave_sum(oct, pc) * OCTAVE_WEIGHTS[oct];
        }
        out[pc] = v * v;
    }
    out
}

/// Key of one window if its two opinions agree and the key's tonic is one of the window's
/// two strongest pitch classes (0x10114f33c … 0x101155a50).
fn window_key(chroma: &[f32; 12]) -> Option<i32> {
    let r = Ranking::new(chroma);
    // Opinion one: table scores and pair rules.
    let (k1, k2) = first_stage(&r, &TABLES_WINDOW);
    let by_rules = if prefers_second(&r, k1, k2, Scope::Window) { k2 } else { k1 };

    // Opinion two: scale fit.
    let fit = ScaleFit::new(&r);
    let top = &fit.keys[..5];
    let (first, second) = (top[0], top[1]);
    let least = |v: &[f32; 24]| top.iter().map(|&k| v[k as usize]).fold(f32::INFINITY, f32::min);
    let best_triad = least(&fit.triad);
    let by_fit = if is_minor(first)
        && top[1..].contains(&(first - 12))
        && least(&fit.whole) == fit.whole[first as usize]
        && best_triad == fit.triad[first as usize]
        && first == by_rules
    {
        // Best-fitting key is a minor whose parallel major is also in the top five, it has
        // the best triad and whole-scale ranks, and the rules chose it too.
        by_rules
    } else if first == second - 12 && best_triad == fit.triad[second as usize] {
        // Major first, its parallel minor second with the best triad: the minor.
        second
    } else {
        top[rank_by_support(top, top, [5.0, 4.0, 3.0, 2.0, 1.0])[0]]
    };

    let tonic = (by_fit % 12) as usize;
    (by_rules == by_fit && (tonic == r.order[0] || tonic == r.order[1])).then_some(by_fit)
}

/// Most frequent key among the windows that reached a verdict, or `NONE`. `raw` is the
/// constant-Q matrix before normalisation and smoothing, `raw[bin * frames + frame]`.
fn window_vote(raw: &[f32], frames: usize, rate: f32, hop: usize) -> i32 {
    let hop_f = hop as f32;
    let len = (rate * 30.0 / hop_f) as usize; // frames in 30 s
    let step_f = (rate * 10.0 / hop_f).trunc(); // frames in 10 s
    let step = step_f as usize;
    if step == 0 || frames < len {
        return NONE;
    }
    let count = ((frames - len) as f32 / step_f) as usize;
    let mut votes = [0i32; 24];
    for w in 0..count {
        // The first window starts one step in; a window must end before the last frame.
        let start = (w + 1) * step;
        if start + len >= frames {
            break;
        }
        let chroma = fold_octaves(|oct, pc| {
            let row = (oct * 12 + pc) * frames + start;
            sum_lanes(&raw[row..row + len])
        });
        if let Some(key) = window_key(&chroma) {
            votes[key as usize] += 1;
        }
    }
    // First key with the strictly highest count.
    let mut best = NONE;
    let mut most = 0;
    for (key, &n) in votes.iter().enumerate() {
        if n > most {
            most = n;
            best = key as i32;
        }
    }
    best
}

// ------------------------------------------------------------------------------------------
// Final choice
// ------------------------------------------------------------------------------------------

/// Reconciles the rule result `best` with the window vote `voted` (0x101155a54).
fn final_choice(r: &Ranking, best: i32, second: i32, voted: i32) -> i32 {
    if voted == NONE {
        // Every path of the original returns the rule result when no window voted.
        return best;
    }
    // Two more candidates from the scale fit of the whole buffer: the five best-fitting keys
    // re-ranked by support from the twelve best-fitting ones.
    let fit = ScaleFit::new(r);
    let by_support = rank_by_support(&fit.keys[..5], &fit.keys[..12], [9.0, 6.0, 5.0, 4.0, 1.0]);
    let fit1 = fit.keys[by_support[0]];
    let fit2 = fit.keys[by_support[1]];

    // And one from the strongest pitch class: `best`, or the major or minor on that pitch
    // class, whichever has the lowest rank sum over its scale.
    let strongest = r.order[0] as i32;
    let (sum_best, sum_major, sum_minor) =
        (r.sum(best, &ALL_DEGREES), r.sum(strongest, &ALL_DEGREES), r.sum(strongest + 12, &ALL_DEGREES));
    let mut on_strongest = if sum_best <= sum_major { best } else { strongest };
    if sum_best.min(sum_major) > sum_minor {
        on_strongest = strongest + 12;
    }

    // Keys whose whole triad consists of tonics of the six candidates.
    let tonics = [best, second, fit1, fit2, on_strongest, voted].map(|k| (k % 12) as usize);
    let triad_of_candidates =
        |key: i32| TRIAD.iter().all(|&n| tonics.contains(&degree(key, n)));
    let supported: Vec<i32> = (0..24).filter(|&k| triad_of_candidates(k)).collect();

    let best_tonic_rank = r.rank[(best % 12) as usize];
    let voted_tonic_rank = r.rank[(voted % 12) as usize];

    let mut pick = if best == voted || ((best == fit1 || best == fit2) && best_tonic_rank < 2) {
        best
    } else if supported.is_empty() {
        return voted_or_fallback(voted, voted_tonic_rank, best, second, fit1, fit2, on_strongest);
    } else if best_tonic_rank <= 2 && supported.contains(&best) {
        best
    } else {
        NONE
    };
    if supported.contains(&voted) {
        pick = voted;
    }
    if pick != NONE {
        return pick;
    }
    voted_or_fallback(voted, voted_tonic_rank, best, second, fit1, fit2, on_strongest)
}

/// The rule result found no support: take the voted key unless its tonic is strong (rank ≤ 3)
/// or it is the second candidate and no other opinion backs it; then fall back to the rule
/// result, or to the second candidate when both other opinions name that one.
fn voted_or_fallback(voted: i32, voted_tonic_rank: i32, best: i32, second: i32, fit1: i32, fit2: i32, on_strongest: i32) -> i32 {
    if voted_tonic_rank > 3 && voted != second {
        return voted;
    }
    if voted == fit1 || voted == fit2 || voted == on_strongest {
        return voted;
    }
    if second == on_strongest && second == fit1 {
        second
    } else {
        best
    }
}

// ------------------------------------------------------------------------------------------
// Entry point
// ------------------------------------------------------------------------------------------

/// Raw constant-Q magnitudes of a long buffer, for the window vote.
pub struct RawMatrix<'a> {
    /// `data[bin * frames + frame]`, 72 bins, before normalisation and smoothing.
    pub data: &'a [f32],
    pub frames: usize,
    pub rate: f32,
    pub hop: usize,
}

/// Best and second-best key of one buffer. `short` = buffer shorter than 30 s.
pub fn keys(chroma: &[f32; 12], short: bool, raw: Option<RawMatrix>) -> (i32, i32) {
    let r = Ranking::new(chroma);
    let (mut k1, k2) = first_stage(&r, if short { &TABLES_SHORT } else { &TABLES_LONG });
    if short && parallel(k1, k2) {
        k1 = settle_parallel_pair(&r, k1, k2);
    }
    let (best, second) = if prefers_second(&r, k1, k2, Scope::Buffer) { (k2, k1) } else { (k1, k2) };
    let voted = match &raw {
        Some(m) => window_vote(m.data, m.frames, m.rate, m.hop),
        None => NONE,
    };
    let result = (final_choice(&r, best, second, voted), second);
    result
}
