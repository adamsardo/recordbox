//! The app's side of a separation in progress (`stems::Live`): pointing it at its deck's
//! playhead, ending it, and the progress ranges the overview draws.

use std::sync::Arc;

impl crate::app::App {
    /// Keep the running separation pointed at its deck's playhead: the stretch there is made
    /// next (after the one under way).
    pub(crate) fn poll_live_stems(&mut self) {
        let Some((id, live)) = self.stem.live.clone() else { return };
        if live.complete() {
            return;
        }
        let want = (0..self.deck_count()).find(|&n| self.deck_of(n).track == Some(id)).and_then(|n| {
            let ps = self.player.decks[n as usize].lock();
            let sr = ps.audio.as_ref()?.sr;
            Some((ps.pos.max(0.0) * crate::stems::htdemucs::SAMPLE_RATE as f64 / sr.max(1) as f64) as usize)
        });
        live.want(want);
        // PREPDECK_BENCH runs: when the parts under the playhead first became playable.
        static NOTED: std::sync::Mutex<Option<(usize, std::time::Instant, bool)>> = std::sync::Mutex::new(None);
        if std::env::var_os("PREPDECK_BENCH").is_some() {
            if let (Ok(mut n), Some(p)) = (NOTED.lock(), want) {
                let key = Arc::as_ptr(&live) as usize;
                if n.as_ref().is_none_or(|e| e.0 != key) {
                    *n = Some((key, std::time::Instant::now(), false));
                }
                if let Some(e) = n.as_mut().filter(|e| !e.2 && live.ready_span(p, p + crate::stems::READY_BLOCK)) {
                    e.2 = true;
                    eprintln!("bench: stems playable at the playhead {:.2} s after the separation started ({:.0} % of the track ready)", e.1.elapsed().as_secs_f32(), live.fraction() * 100.0);
                }
            }
        }
    }

    /// A separation ended. Complete stems stay on their deck (the caller installs the final
    /// ones); after a stop or a failure the deck's part-finished stems are dropped, as if it
    /// had not started.
    pub(crate) fn finish_live_stems(&mut self, id: i64, ok: bool) {
        let Some((_, live)) = self.stem.live.take_if(|l| l.0 == id) else { return };
        for d in &self.player.decks {
            let mut ps = d.lock();
            if ps.stems_ready.as_ref().is_some_and(|r| Arc::ptr_eq(r, &live)) {
                ps.stems_ready = None;
                if !ok {
                    ps.stems = None;
                }
            }
        }
    }

    /// rekordbox's status message when a hot cue is called into a stretch whose parts are not
    /// separated yet: the deck plays the original there (`UiPlayer::showTrackSeparationCautionIfNeeded`,
    /// after every successful hot cue call: STEMS on, the original readable at the cue, the
    /// track separable and not separated at the cue's sample).
    pub(crate) fn stem_caution_at(&mut self, secs: f64) {
        const MSG: &str = "The track before STEMS analysis is played since STEMS analysis is not completed.";
        if !self.prefs.show_stems {
            return;
        }
        let (live, has_stems) = {
            let ps = self.player.state.lock();
            (ps.stems_ready.clone(), ps.stems.is_some())
        };
        let frame = (secs.max(0.0) * crate::stems::htdemucs::SAMPLE_RATE as f64) as usize;
        let not_ready = match live {
            Some(l) => !l.complete() && !l.ready_at(frame),
            // No stems yet but being separated (or loaded from the kept file).
            None => !has_stems && self.deck.track.is_some_and(|t| self.stem.separating.is_some_and(|s| s.0 == t) || self.stem.loading == Some(t)),
        };
        if not_ready {
            self.toast(MSG);
        }
    }

    /// The stretches of track `id` whose parts are ready, as fractions of the track, for the
    /// blue progress line: from the separation in progress, else from the start to `progress`.
    pub(crate) fn stem_ready_ranges(&self, id: i64, progress: f32) -> Vec<(f32, f32)> {
        match &self.stem.live {
            Some((lid, live)) if *lid == id => live.ranges(),
            _ => vec![(0.0, progress.clamp(0.0, 1.0))],
        }
    }
}
