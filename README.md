# recordbox

An unofficial, open-source macOS app for getting ready for a DJ set: organise your music library, analyse tracks, set cues and build playlists, then export them to a USB drive for Pioneer CDJ/XDJ players.

> **Not affiliated with AlphaTheta or Pioneer DJ.** rekordbox is a trademark of AlphaTheta Corporation. This is an independent project that uses rekordbox's preparation workflow as the reference for how things should behave.

## Status

**Studying how rekordbox behaves (no app code yet)**

No app code has been written yet.

This is being built in the open. An AI coding agent (Claude Code, run through Craft Anything) is writing the code, and this repo syncs from its build folder about every 15 minutes, so you're watching it come together in real time. Expect gaps and the occasional broken build until a first release. Star or watch the repo to follow along.

## What it's aiming for

- **Library:** import, browse, search and organise your tracks
- **Analysis:** tempo, beat grid, key and waveforms
- **Preparation:** cues, loops, playlists and the rest of the pre-set workflow
- **USB export:** drives that CDJ/XDJ players can read, the same way rekordbox does

Performance features (mixing live, effects, lighting, video, streaming) are out of scope. The focus is everything you do *before* the set.

## Privacy and safety

- Runs entirely on your Mac. No account, no cloud.
- Design rule: it never modifies your existing rekordbox library or your original audio files.

## Running it

Nothing to run yet. Instructions will appear here once the first version builds.

## The brief

This is the request the agent is building from, word for word:

> build a high-fidelity, fully functional recreation of rekordbox 7, focused specifically on its export mode, music library management, track analysis and complete dj preparation workflow.
>
> i want the experience of using rekordbox to organise my music, analyse tracks and prepare for dj sets, without the additional features i don't use.
>
> this must be a genuinely working application, not a visual prototype or collection of mock interfaces.
>
> use rekordbox's existing behaviour as the reference when deciding how preparation features should work.
>
> the application should run locally on macos, keep my music and preparation data private, and never modify or corrupt my existing rekordbox library or original audio files.
>
> Exporting playlists to a USB drive must produce a drive that Pioneer CDJ/XDJ players can read, the same way rekordbox does.
>
> the goal is to replace rekordbox for everything i do before a dj set.
>
> The full app bundle is at /Applications/rekordbox 7/rekordbox.app; use its Contents/Resources folder (skins, layouts, text) as the reference for how screens look and are labelled.

## Feedback

Ideas and bug reports are welcome in [Issues](https://github.com/adamsardo/recordbox/issues). The code changes quickly while the agent is working, so please open an issue rather than a pull request for now.

## Licence

No licence has been chosen yet, so all rights are reserved for the moment.
