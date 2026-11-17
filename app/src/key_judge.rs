//! Final vote over the per-segment key candidates.
//!
//! Port of `BeatAnalyzer_2_0::Key_judge_with_keys` (0x101beb14c), `KeyJudge_step_1`
//! (0x101beb9f0) and `KeyJudge_step_2toFinal` (0x101becfb0).
//!
//! Keys are A-based indices: 0..=11 major (A, Bb, B, C, ...), 12..=23 the minors in the
//! same order. The vote works on the *best* key of every segment only; the second-best
//! key of each segment and of the whole track never influence the result.
//!
//! Outline:
//! 1. [`rank_segment_keys`]: histogram of the segment keys, five most frequent keys
//!    ("top 5"). If the whole-track key is not the most frequent one, the leader and the
//!    runner-up each absorb the count of their relative key.
//! 2. [`Judge::new`] (step 1): harmonic-support scores for the three leading candidates,
//!    statistics on how the segments move between them, and a provisional vote.
//! 3. [`Judge::decide`] + [`Judge::whole_track_override`] (step 2): a cascade of rules picks
//!    the final key and finally checks whether the whole-track key should win after all.
//!
//! Numeric conventions of the original that matter for exact agreement: ratios are
//! computed in `f32`; comparisons against decimal thresholds are done after widening to
//! `f64` (so 6/5 computed in `f32` is *not* `<= 1.2`). Ratios are therefore stored here as
//! `f64` holding an `f32`-rounded value, see [`ratio`].

/// Index used to pad the candidate list after a relative key has been merged away.
const PAD_KEY: i32 = 30;
/// Count the original stores for a rank that has no segments at all (its key index is 0).
/// Almost certainly a slip in the original (30 was meant as the key), reproduced as is.
const EMPTY_RANK_COUNT: i32 = 30;

/// The original needs a beat grid and at least this many segments to vote at all.
const MIN_SEGMENTS: usize = 9;

/// `f32` division, widened to `f64` for threshold comparisons.
fn ratio(num: f32, den: f32) -> f64 {
    f64::from(num / den)
}

fn ratio_i(num: i32, den: i32) -> f64 {
    ratio(num as f32, den as f32)
}

fn is_minor(key: i32) -> bool {
    key >= 12
}

/// Relative key (same key signature): major -> minor a minor third below, and back.
fn relative(key: i32) -> i32 {
    if is_minor(key) {
        let k = key - 9;
        if k > 11 { k - 12 } else { k }
    } else {
        let k = key + 21;
        if k > 23 { k - 12 } else { k }
    }
}

/// Parallel key (same tonic, other mode).
fn parallel(key: i32) -> i32 {
    let k = if is_minor(key) { key - 12 } else { key + 12 };
    if k > 24 { k - 12 } else { k }
}

/// The six keys considered harmonically close to `key`, with the weight each one
/// contributes when it is also present among the candidates:
/// parallel (1), fifth up (2), fifth down (2), relative (3), and the dominant and
/// subdominant of the relative key (3 each).
fn related_keys(key: i32) -> [(i32, i32); 6] {
    const WEIGHTS: [i32; 6] = [1, 2, 2, 3, 3, 3];
    // (offset, largest valid index before wrapping down an octave); table at 0x10380CF4C.
    const FROM_MINOR: [(i32, i32); 6] = [(-12, 24), (7, 23), (5, 23), (-9, 11), (-2, 11), (-4, 11)];
    const FROM_MAJOR: [(i32, i32); 6] = [(12, 24), (7, 11), (5, 11), (21, 23), (16, 23), (14, 23)];
    let table = if is_minor(key) { &FROM_MINOR } else { &FROM_MAJOR };
    let mut out = [(0, 0); 6];
    for (slot, (&(offset, limit), &weight)) in out.iter_mut().zip(table.iter().zip(&WEIGHTS)) {
        let k = key + offset;
        *slot = (if k > limit { k - 12 } else { k }, weight);
    }
    out
}

fn occurrences(keys: &[i32], key: i32) -> i32 {
    keys.iter().filter(|&&k| k == key).count() as i32
}

/// The five leading segment keys and their segment counts, most frequent first.
struct Ranking {
    keys: [i32; 5],
    counts: [i32; 5],
}

/// Removes `key` from the candidate list if it sits at `pos`, shifting the tail up and
/// padding the end.
fn drop_if_at(keys: &mut [i32; 6], counts: &mut [i32; 6], pos: usize, key: i32) {
    if keys[pos] == key {
        keys.copy_within(pos + 1.., pos);
        counts.copy_within(pos + 1.., pos);
        keys[5] = PAD_KEY;
        counts[5] = 0;
    }
}

/// Stage 0 (`Key_judge_with_keys`): rank the segment keys by how many segments chose them.
///
/// If the whole-track key agrees with the most frequent segment key, the plain top five
/// are used. Otherwise the leader absorbs the segments of its relative key, the next
/// remaining key does the same, and the two are ordered by their merged counts.
fn rank_segment_keys(whole_key: i32, seg_keys: &[i32]) -> Ranking {
    let mut histogram = [0i32; 24];
    for &k in seg_keys {
        histogram[k as usize] += 1;
    }

    // Seven most frequent keys; ties go to the lower index.
    let mut ranked_keys = [0i32; 7];
    let mut ranked_counts = [0i32; 7];
    for rank in 0..7 {
        let mut best = 0;
        for k in 1..24 {
            if histogram[k] > histogram[best] {
                best = k;
            }
        }
        ranked_keys[rank] = best as i32;
        ranked_counts[rank] = if histogram[best] == 0 { EMPTY_RANK_COUNT } else { histogram[best] };
        histogram[best] = 0;
    }

    if whole_key == ranked_keys[0] {
        let mut ranking = Ranking { keys: [0; 5], counts: [0; 5] };
        ranking.keys.copy_from_slice(&ranked_keys[..5]);
        ranking.counts.copy_from_slice(&ranked_counts[..5]);
        return ranking;
    }

    let leader = ranked_keys[0];
    let leader_relative = relative(leader);
    let leader_count = ranked_counts[0] + occurrences(seg_keys, leader_relative);

    let mut rest_keys = [0i32; 6];
    let mut rest_counts = [0i32; 6];
    rest_keys.copy_from_slice(&ranked_keys[1..]);
    rest_counts.copy_from_slice(&ranked_counts[1..]);
    for pos in 0..4 {
        drop_if_at(&mut rest_keys, &mut rest_counts, pos, leader_relative);
    }

    let challenger = rest_keys[0];
    let challenger_relative = relative(challenger);
    let challenger_count = rest_counts[0] + occurrences(seg_keys, challenger_relative);
    for pos in 1..4 {
        drop_if_at(&mut rest_keys, &mut rest_counts, pos, challenger_relative);
    }

    let (first, second) = if leader_count >= challenger_count {
        ((leader, leader_count), (challenger, challenger_count))
    } else {
        // The original takes the raw second-ranked key here, not `challenger`. They only
        // differ when the leader's relative was ranked second and a padded rank overtook.
        ((ranked_keys[1], challenger_count), (leader, leader_count))
    };
    Ranking {
        keys: [first.0, second.0, rest_keys[1], rest_keys[2], rest_keys[3]],
        counts: [first.1, second.1, rest_counts[1], rest_counts[2], rest_counts[3]],
    }
}

/// Which of the candidates the provisional vote of step 1 favours.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lean {
    Key1,
    Key2,
    Key3,
    /// The key detected on the whole track.
    WholeTrack,
}

/// Everything step 1 derives and step 2 consumes (the original keeps these in globals of
/// the same names, given in the comments).
struct Judge {
    /// Best key of the whole track (`key1_current`).
    whole_key: i32,
    bpm: f32,
    /// `top5`.
    ranking: Ranking,
    /// The three candidates: most frequent segment keys (`key1`, `key2`, `key3`).
    key1: i32,
    key2: i32,
    key3: i32,

    /// Harmonic support of candidate i from the other two candidates (`scores`).
    support_top3: [i32; 3],
    /// Same, also counting the 4th and 5th ranked keys (`scores5`).
    support_top5: [i32; 3],
    /// Support from the other two candidates plus the whole-track key (`scores_c`'s
    /// neighbour at 0x1056cc678).
    support_with_whole: [i32; 3],
    /// Support of the whole-track key from the three candidates, plus one (0x1056cc684).
    whole_key_support: i32,
    /// Candidate (1..=3) with the largest summed support (`scores_total_max`).
    strongest: i32,
    /// Largest and runner-up summed support (`scores_total_max_v`, `scores_total_2nd_v`).
    strongest_total: f32,
    runner_up_total: f32,
    /// Summed-support ratios key1/key2, key1/key3, key2/key1 (`r_ks12`, `r_ks13`, `r_ks21`).
    support_ratio_12: f64,
    support_ratio_13: f64,
    support_ratio_21: f64,

    /// Segment-count ratios between the candidates (`top12r`, `top13r`, `top23r`).
    count_ratio_12: f64,
    count_ratio_13: f64,
    count_ratio_23: f64,
    /// Share of segments on key1 or key2 (`r_t12all`).
    share_key12: f64,

    /// Runs of consecutive segments per isolated excursion (`rdp` = `dt2` / `pt`):
    /// large = the candidates alternate in long blocks, small = short flickers.
    runs_per_excursion: f64,
    /// Isolated one-segment visits "key2, key1, not key2" and "key1, key2, not key1"
    /// (`p_cnt21`, `p_cnt12`).
    visits_2_to_1: i32,
    visits_1_to_2: i32,
    /// Candidate that is visited most from the others, relative to their size (`key_no1`).
    most_visited: i32,

    /// key2 / key3 is the parallel key of key1 (`doshu12_flag`, `doshu13_flag`).
    parallel_12: bool,
    parallel_13: bool,
    /// key2 / key3 is the relative key of key1 (`heiko12_flag`, `heiko13_flag`).
    relative_12: bool,
    relative_13: bool,

    /// Provisional vote before the whole-track / support refinements (`pre_final_0`).
    first_lean: Lean,
    /// Provisional vote (`pre_final`) and its rule code (`pre_judge_t`).
    lean: Lean,
    lean_rule: i32,
}

impl Judge {
    /// Step 1 (`KeyJudge_step_1`): scores, transition statistics and the provisional vote.
    fn new(whole_key: i32, seg_keys: &[i32], bpm: f32) -> Judge {
        let ranking = rank_segment_keys(whole_key, seg_keys);
        let [key1, key2, key3, key4, key5] = ranking.keys;
        let candidates = [key1, key2, key3];

        // --- Harmonic support -------------------------------------------------------
        // A candidate gains the weight of each of its related keys that is itself present
        // among the other leading keys.
        let mut support_top3 = [0i32; 3];
        let mut support_top5 = [0i32; 3];
        let mut support_with_whole = [0i32; 3];
        for (i, &candidate) in candidates.iter().enumerate() {
            let (other_a, other_b) = match i {
                0 => (key2, key3),
                1 => (key1, key3),
                _ => (key1, key2),
            };
            for (related, weight) in related_keys(candidate) {
                let among_top3 = related == other_a || related == other_b;
                if among_top3 {
                    support_top3[i] += weight;
                }
                if among_top3 || related == key4 || related == key5 {
                    support_top5[i] += weight;
                }
                if among_top3 || related == whole_key {
                    support_with_whole[i] += weight;
                }
            }
        }
        let whole_key_support = 1 + related_keys(whole_key)
            .iter()
            .filter(|(related, _)| candidates.contains(related))
            .map(|&(_, weight)| weight)
            .sum::<i32>();

        // --- How the segments move between the candidates -----------------------------
        // Number of runs (maximal blocks of equal consecutive segments) on the candidates.
        let mut runs = 0;
        if candidates.contains(&seg_keys[0]) {
            runs += 1;
        }
        for pair in seg_keys.windows(2) {
            let (prev, next) = (pair[0], pair[1]);
            if candidates.iter().any(|&c| next == c && prev != c) {
                runs += 1;
            }
        }
        // "from, to, not back": a segment on `to` right after one on `from`, where the
        // track does not return to `from` immediately. Each scan of the original counts
        // two patterns, the first taking precedence (`shadowed_by`); that only matters
        // when two candidates are the same (padded) key.
        let visits = |from: i32, to: i32, shadowed_by: Option<i32>| -> i32 {
            let hit = |t: &[i32], from: i32| t[0] == from && t[1] == to && t[2] != from;
            seg_keys
                .windows(3)
                .filter(|t| hit(t, from) && !shadowed_by.is_some_and(|first| hit(t, first)))
                .count() as i32
        };
        let visits_2_to_1 = visits(key2, key1, None);
        let visits_3_to_1 = visits(key3, key1, Some(key2));
        let visits_1_to_2 = visits(key1, key2, None);
        let visits_3_to_2 = visits(key3, key2, Some(key1));
        let visits_1_to_3 = visits(key1, key3, None);
        let visits_2_to_3 = visits(key2, key3, Some(key1));

        let nonzero = |n: i32| if n != 0 { n } else { 1 };
        let [count1, count2, count3, _, _] = ranking.counts;
        let (size1, size2, size3) = (nonzero(count1), nonzero(count2), nonzero(count3));
        let most_visited;
        let second_most_visited;
        if ratio_i(visits_2_to_1, size2) >= ratio_i(visits_1_to_2, size1) {
            if ratio_i(visits_3_to_1, size3) < ratio_i(visits_1_to_3, size1) {
                (most_visited, second_most_visited) = (3, 1);
            } else {
                (most_visited, second_most_visited) = (1, 2);
            }
        } else if ratio_i(visits_3_to_2, size3) < ratio_i(visits_2_to_3, size2) {
            (most_visited, second_most_visited) = (3, 2);
        } else {
            (most_visited, second_most_visited) = (2, 3);
        }

        let visits_into_1 = visits_3_to_1 + visits_2_to_1;
        let visits_into_2 = visits_3_to_2 + visits_1_to_2;
        let visits_into_3 = visits_2_to_3 + visits_1_to_3;
        let excursions = nonzero(visits_into_1 + visits_into_2 + visits_into_3);
        let runs_per_excursion = ratio_i(runs, excursions);

        let count_ratio_12 = if count2 != 0 { ratio_i(count1, count2) } else { 0.0 };
        let (count_ratio_13, count_ratio_23) = if count3 != 0 {
            (ratio_i(count1, count3), ratio_i(count2, count3))
        } else {
            (0.0, 0.0)
        };
        let segments = seg_keys.len() as f32;
        // Share of segments on key1 (`r_t1all`).
        let share_key1 = ratio(count1 as f32, segments);
        let share_key12 = ratio(count2 as f32 + count1 as f32, segments);

        // --- Summed support and its ratios -------------------------------------------
        let total = |i: usize| support_top5[i] + support_top3[i] + support_with_whole[i];
        let (total1, total2, total3) = (total(0), total(1), total(2));
        let (t1, t2, t3) = (total1 as f32, total2 as f32, total3 as f32);
        // A zero denominator leaves the plain numerator in place.
        let over = |num: f32, den_total: i32, den: f32| {
            if den_total != 0 { ratio(num, den) } else { f64::from(num) }
        };
        let support_ratio_21 = over(t2, total1, t1);
        let support_ratio_31 = over(t3, total1, t1);
        let support_ratio_12 = over(t1, total2, t2);
        let support_ratio_32 = over(t3, total2, t2);
        let support_ratio_13 = over(t1, total3, t3);

        let strongest;
        let strongest_total;
        let mut runner_up_total;
        let mut runner_up_is_key1 = false;
        let mut runner_up_is_key2 = false;
        if total1 >= total2 && total1 >= total3 {
            strongest = 1;
            strongest_total = t1;
            runner_up_is_key2 = total2 >= total3;
            runner_up_total = if runner_up_is_key2 { t2 } else { t3 };
        } else if total1 >= total2 {
            strongest = 3;
            strongest_total = t3;
            runner_up_is_key1 = true;
            runner_up_total = t1;
        } else if total2 < total3 {
            strongest = 3;
            strongest_total = t3;
            runner_up_is_key2 = true;
            runner_up_total = t2;
        } else {
            strongest = 2;
            strongest_total = t2;
            runner_up_is_key1 = total1 >= total3;
            runner_up_total = if runner_up_is_key1 { t1 } else { t3 };
        }

        // --- Provisional vote ----------------------------------------------------------
        // Rule codes: 101 long blocks -> key1; 131 default key1; 133 key3 has clearly the
        // best support; 122/112 key2 has the best support; 111 key2 only reached through
        // rare excursions -> key1. +1000 whole-track key takes over, +2000 back to key2,
        // +3000 key2 by overwhelming support, +4000 key3 by support. The adjustments add
        // up (e.g. 3112 = 112 + 1000 + 2000), as in the original.
        let rdp = runs_per_excursion;
        let (first_lean, mut lean_rule) = if rdp >= 4.0 {
            (Lean::Key1, 101)
        } else {
            match strongest {
                3 => {
                    let third_clearly_ahead = (runner_up_is_key1 && support_ratio_31 > 1.4)
                        || (runner_up_is_key2 && support_ratio_32 > 1.4);
                    if rdp < 2.6 && third_clearly_ahead {
                        (Lean::Key3, 133)
                    } else {
                        (Lean::Key1, 131)
                    }
                }
                2 => {
                    if most_visited != 2 && second_most_visited != 2 {
                        if support_ratio_21 > 1.2 && support_top5[1] > support_top5[0] {
                            (Lean::Key2, 122)
                        } else {
                            (Lean::Key1, 131)
                        }
                    } else {
                        let into_1 = nonzero(visits_into_1);
                        // Integer division in the original.
                        let few_visits_to_2 = f64::from((visits_into_2 / into_1) as f32) < 1.1;
                        if runner_up_is_key1 && support_ratio_21 < 1.2 && few_visits_to_2 && into_1 > 5 {
                            (Lean::Key1, 111)
                        } else {
                            (Lean::Key2, 112)
                        }
                    }
                }
                _ => (Lean::Key1, 131),
            }
        };

        // The whole-track key takes over when it is the rival candidate, nearly as frequent
        // as the leaning one and comparably supported.
        let whole_takes_over = match first_lean {
            Lean::Key1 => {
                (whole_key == key2 && count_ratio_12 < 1.1 && support_ratio_21 > 0.75)
                    || (whole_key == key3 && count_ratio_13 < 4.0 && support_ratio_31 > 1.6)
            }
            Lean::Key2 => whole_key == key1 && count_ratio_12 < 1.1 && support_ratio_12 > 0.75,
            Lean::Key3 => whole_key == key1 && count_ratio_13 < 4.0 && support_ratio_13 > 1.6,
            Lean::WholeTrack => false,
        };
        let mut lean = first_lean;
        if whole_takes_over {
            lean = Lean::WholeTrack;
            lean_rule += 1000;
        }

        if runner_up_total == 0.0 {
            runner_up_total = 1.0;
        }
        let lead_over_runner_up = ratio(strongest_total, runner_up_total);

        // key2 has the best support by a margin and nearly as many segments as key1.
        let key2_could_reclaim =
            lean == Lean::Key1 || (lean == Lean::WholeTrack && whole_key == key1);
        if strongest == 2 && key2_could_reclaim && lead_over_runner_up >= 1.2 && count_ratio_12 < 1.1 {
            lean = Lean::Key2;
            lean_rule += 2000;
        }

        let total1 = nonzero(total1);
        let lead_over_key1 = ratio(strongest_total, total1 as f32);
        let whole_is_key1 = whole_key == key1;
        // key2 much better supported than key1 while the track alternates in medium-sized
        // blocks and key1 covers only about a quarter of it.
        let key2_supported = strongest == 2
            && (lean == Lean::Key1 || (lean == Lean::WholeTrack && whole_is_key1))
            && lead_over_key1 >= 1.2
            && count_ratio_12 < 1.55
            && rdp > 3.0
            && rdp <= 4.0
            && share_key1 < 0.25;
        let key2_triples_key1 = runner_up_is_key2
            && (lean == Lean::Key1 || (lean == Lean::WholeTrack && whole_is_key1))
            && total2 / total1 >= 3
            && count_ratio_12 < 1.55
            && rdp > 3.0
            && rdp <= 5.0
            && share_key1 < 0.26;
        if key2_supported || key2_triples_key1 {
            lean = Lean::Key2;
            lean_rule += 3000;
        }

        if strongest == 3
            && (lean == Lean::Key1 || (lean == Lean::WholeTrack && whole_is_key1))
            && lead_over_runner_up >= 1.5
            && count_ratio_13 < 4.0
            && rdp < 4.2
            && count_ratio_12 < 1.7
        {
            lean = Lean::Key3;
            lean_rule += 4000;
        }

        Judge {
            whole_key,
            bpm,
            key1,
            key2,
            key3,
            support_top3,
            support_top5,
            support_with_whole,
            whole_key_support,
            strongest,
            strongest_total,
            runner_up_total,
            support_ratio_12,
            support_ratio_13,
            support_ratio_21,
            count_ratio_12,
            count_ratio_13,
            count_ratio_23,
            share_key12,
            runs_per_excursion,
            visits_2_to_1,
            visits_1_to_2,
            most_visited,
            parallel_12: parallel(key1) == key2,
            parallel_13: parallel(key1) == key3,
            relative_12: relative(key1) == key2,
            relative_13: relative(key1) == key3,
            first_lean,
            lean,
            lean_rule,
            ranking,
        }
    }

    /// key1 has at least as much harmonic support as key2 on all three measures, and a
    /// solid amount of it.
    fn key1_well_supported(&self) -> bool {
        let measures = [&self.support_top3, &self.support_top5, &self.support_with_whole];
        measures.iter().all(|m| m[0] >= m[1] && m[0] >= 5)
    }

    /// key2 beats key1 on all three support measures and is not beaten by key3 on any.
    fn key2_best_supported(&self) -> bool {
        let measures = [&self.support_top3, &self.support_top5, &self.support_with_whole];
        measures.iter().all(|m| m[1] > m[0] && m[1] >= m[2])
    }

    /// Step 2, main cascade (`KeyJudge_step_2toFinal` up to LABEL_67). Returns the chosen
    /// key and the rule code (`judge_type`).
    fn decide(&self) -> (i32, i32) {
        let (key1, key2, key3) = (self.key1, self.key2, self.key3);
        let c12 = self.count_ratio_12;
        let rdp = self.runs_per_excursion;

        // 11: key1 has at least 1.8 times the segments of key2.
        if c12 >= 1.8 {
            return (key1, 11);
        }

        // 21/22: key1 is well supported; only its relative key can still win, when the
        // whole-track analysis says so and key1 does not lead clearly.
        if self.key1_well_supported() {
            let rel = relative(key1);
            if rel == self.whole_key && c12 < 1.39 {
                return (rel, 22);
            }
            return (key1, 21);
        }

        // 31..33: key1 and key2 are parallel keys (major/minor on the same tonic).
        if self.parallel_12 {
            let (mut key, mut rule) = (key1, 31);
            if c12 <= 1.2 && matches!(self.first_lean, Lean::Key2 | Lean::Key3) {
                (key, rule) = (key2, 32);
            }
            // Flip to the other mode if the whole track says so and the two are close;
            // how close they must be depends on how blocky the alternation is.
            let other_mode = parallel(key);
            if other_mode == self.whole_key && c12 < 1.23 && rdp < 3.5 {
                let keep = (c12 <= 1.06 || rdp <= 2.5 || rdp >= 3.0)
                    && (c12 <= 1.1 || rdp <= 2.0 || rdp >= 3.0)
                    && (c12 <= 1.2 || rdp < 3.0)
                    && (c12 > 1.06 || rdp >= 2.0);
                if !keep {
                    return (other_mode, 33);
                }
            }
            return (key, rule);
        }

        // 41..43: key1 and key3 are parallel keys.
        if self.parallel_13 {
            // key2 a fourth above key1 and visited more often than the reverse.
            if key2 - key1 == 5 && self.visits_2_to_1 < self.visits_1_to_2 && rdp < 2.0 {
                return (key2, 42);
            }
            if self.strongest == 3
                && ratio(self.strongest_total, self.runner_up_total) > 1.4
                && self.share_key12 < 0.64
            {
                return (key3, 43);
            }
            return (key1, 41);
        }

        let [_, _, _, key4, _] = self.ranking.keys;
        let [count1, _, _, count4, count5] = self.ranking.counts;

        // 51: key1 best supported and key2 far ahead of key3 (a two-key track).
        if self.strongest == 1 && self.count_ratio_23 >= 3.0 {
            return (key1, 51);
        }
        // 61: key2 is the relative key of key1 -> the more frequent one wins.
        if self.relative_12 {
            return (key1, 61);
        }
        // 71..74: key3 is the relative key of key1.
        if self.relative_13 {
            if self.strongest == 2 && self.support_ratio_21 > 1.1 {
                return (key2, 72);
            }
            // key2 is itself backed by its relative / parallel key in fourth place.
            let key1_vs_key4 = ratio_i(count1, count4);
            if relative(key2) == key4 {
                if c12 >= 1.5 || key1_vs_key4 >= 2.1 {
                    return (key1, 71);
                }
                return (key2, 73);
            }
            if parallel(key2) == key4 && c12 < 1.2 && key1_vs_key4 < 2.0 {
                return (key2, 74);
            }
            return (key1, 71);
        }

        // 781: key1 and key2 dominate a calm track, the tail is negligible.
        if self.support_ratio_12 >= 0.6
            && self.share_key12 >= 0.64
            && ratio_i(count1, count5) > 8.4
            && rdp < 2.5
        {
            return (key1, 781);
        }

        // 82..84: key2 is the best supported candidate.
        if self.key2_best_supported() {
            if rdp < 1.6 && c12 > 1.2 && self.most_visited == 1 && self.support_ratio_12 >= 0.5 {
                return (key1, 83);
            }
            if rdp < 2.7 && c12 > 1.3 && self.support_ratio_12 > 0.7 && self.support_ratio_13 > 1.0 {
                return (key1, 84);
            }
            return (key2, 82);
        }

        // 93: key3 is the whole-track key and the best supported candidate.
        if key3 == self.whole_key
            && self.strongest == 3
            && ratio(self.strongest_total, self.runner_up_total) > 1.25
            && self.count_ratio_13 < 2.0
            && rdp < 6.5
        {
            return (key3, 93);
        }

        // Otherwise the provisional vote of step 1 stands.
        let key = match self.lean {
            Lean::Key1 => key1,
            Lean::Key2 => key2,
            Lean::Key3 => key3,
            Lean::WholeTrack => self.whole_key,
        };
        (key, self.lean_rule)
    }

    /// Step 2, tail (LABEL_67 onwards): should the whole-track key replace the decision?
    fn whole_track_override(&self, rule: i32) -> bool {
        let rdp = self.runs_per_excursion;
        let c12 = self.count_ratio_12;
        let [count1, _, _, _, count5] = self.ranking.counts;
        // Integer ratio of the leader to the fifth key (the leader's count if there is none).
        let lead_over_fifth = if count5 != 0 { count1 / count5 } else { count1 };

        // The whole-track key is one of the five leading keys, is better supported by the
        // candidates than any of them (counting it as their partner), the segment vote is
        // not clear-cut, and the tempo is outside 121..=128 BPM.
        let whole_key_fits_best = self.ranking.keys.contains(&self.whole_key)
            && self.support_with_whole.iter().all(|&s| self.whole_key_support > s)
            && rdp <= 3.8
            && c12 <= 1.47
            && lead_over_fifth <= 22
            && (self.bpm < 121.0 || self.bpm > 128.0);
        if whole_key_fits_best {
            // Rules whose decision is kept regardless.
            return !matches!(rule, 31 | 51 | 84 | 112);
        }
        match rule {
            // key2 won on support, but the keys come in long blocks and the tail is heavy.
            82 => rdp > 4.0 && lead_over_fifth < 4,
            // key1 won on count alone, in long blocks, with a lead that is either modest
            // (below 2x) or very large (above 3x).
            11 => rdp > 4.1 && !(c12 > 2.0 && c12 <= 3.0),
            _ => false,
        }
    }
}

/// How [`judge_traced`] arrived at its answer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Verdict {
    /// The final key.
    pub key: i32,
    /// Rule code of the deciding rule (the original's `judge_type`); 0 when no vote was
    /// held (no beat grid or fewer than nine segments).
    pub rule: i32,
    /// The whole-track key replaced the rule's choice at the very end.
    pub overridden: bool,
}

/// `whole`: best and second-best key of the whole track; `segs`: the same for each beat
/// segment; `segmented`: false when there is no beat grid (then `segs` has one entry, the
/// whole-track pair). Keys are A-based indices, +12 for minor. Returns the final key.
pub fn judge(whole: (i32, i32), segs: &[(i32, i32)], segmented: bool, bpm: f32) -> i32 {
    judge_traced(whole, segs, segmented, bpm).key
}

/// Same as [`judge`], also reporting which rule decided.
#[allow(dead_code)]
pub fn judge_traced(whole: (i32, i32), segs: &[(i32, i32)], segmented: bool, bpm: f32) -> Verdict {
    let whole_key = whole.0;
    let no_vote = Verdict { key: whole_key, rule: 0, overridden: false };
    // Without a beat grid, or with too few segments, the whole-track key is the answer.
    if !segmented || segs.len() < MIN_SEGMENTS {
        return no_vote;
    }
    // Keys outside 0..24 would index out of bounds in the original; treat as "no vote".
    if segs.iter().any(|s| !(0..24).contains(&s.0)) {
        return no_vote;
    }
    let seg_keys: Vec<i32> = segs.iter().map(|s| s.0).collect();

    let judge = Judge::new(whole_key, &seg_keys, bpm);
    let (key, rule) = judge.decide();
    let overridden = judge.whole_track_override(rule);
    Verdict { key: if overridden { whole_key } else { key }, rule, overridden }
}
