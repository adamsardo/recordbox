# Recordbox

An unofficial, open-source macOS app for getting ready for a DJ set: organise your music library, analyse tracks, set cues and build playlists, then export them to a USB drive for Pioneer CDJ/XDJ players.

> **Not affiliated with AlphaTheta or Pioneer DJ.** rekordbox is a trademark of AlphaTheta Corporation. This is an independent project that uses rekordbox's preparation workflow as the reference for how things should behave.

## Status

**A first version is ready to try**

So far: 70 source files, 57,441 lines of Rust.

This is being built in the open. An AI coding agent (Claude Code, run through Craft Anything) is writing the code, and this repo syncs from its build folder about every 15 minutes, so you're watching it come together in real time. Expect gaps and the occasional broken build until a first release. Star or watch the repo to follow along.

## What it's aiming for

- **Library:** import, browse, search and organise your tracks
- **Analysis:** tempo, beat grid, key and waveforms
- **Preparation:** cues, loops, playlists and the rest of the pre-set workflow
- **USB export:** drives that CDJ/XDJ players can read, the same way rekordbox does

## Scope

The project focuses on preparing music *before* the set. To rehearse transitions, PERFORMANCE offers two- and four-deck horizontal layouts with an on-screen mixer. Lighting and video are outside that scope.

Once the first version has landed, extended features can be added if enough people ask for them. For example:

- streaming service support
- connecting CDJs and Pioneer DJ controllers

If there's something you'd use, [open an issue](https://github.com/adamsardo/recordbox/issues) or give an existing one a 👍 so it's easy to see what people want most.

## Privacy and safety

- Runs entirely on your Mac. No account, no cloud.
- Design rule: it never modifies your existing rekordbox library or your original audio files.

## Running it

You need [Rust](https://rustup.rs). Then:

```bash
git clone https://github.com/adamsardo/recordbox
cd recordbox/app
cargo run --release
```

This is a work in progress: it may not build at every moment while the agent is mid-change.

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

## Community downloads for macOS

The independently maintained **fantuz/recordbox** fork builds on Adam Sardo’s MIT-licensed project and includes additional features. It publishes ready-to-run app bundles for Apple Silicon and Intel Macs.

Choose the ZIP for your processor from the fork’s **Downloads** table, verify it against `SHA256SUMS`, then extract and open `Recordbox.app`. These community builds are unsigned and ship without Apple notarization. [Fork downloads](https://github.com/fantuz/recordbox/releases/latest) · [Fork installation guide](https://github.com/fantuz/recordbox#running-it)

## Feedback

Ideas and bug reports are welcome in [Issues](https://github.com/adamsardo/recordbox/issues). The code changes quickly while the agent is working, so please open an issue rather than a pull request for now.

For bugs in a community package or a feature added in the fork, please use the [fork’s issue tracker](https://github.com/fantuz/recordbox/issues).

## Licence

MIT. See [LICENSE](LICENSE).
