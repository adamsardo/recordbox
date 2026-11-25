# Recordbox

<!-- project-badges:start -->
<p align="center">
  <a href="https://github.com/fantuz/recordbox/actions/workflows/build.yml?query=branch%3Amain"><img src="https://github.com/fantuz/recordbox/actions/workflows/build.yml/badge.svg?branch=main" alt="Build and package on main" height="20"></a>
  <a href="https://github.com/fantuz/recordbox/actions/workflows/github-code-scanning/codeql?query=branch%3Amain"><img src="https://github.com/fantuz/recordbox/actions/workflows/github-code-scanning/codeql/badge.svg?branch=main" alt="CodeQL analysis on main" height="20"></a>
  <a href="https://github.com/fantuz/recordbox/releases/latest"><img src="https://img.shields.io/github/v/release/fantuz/recordbox?label=release" alt="Latest stable release" height="20"></a>
  <a href="https://github.com/fantuz/recordbox/releases"><img src="https://img.shields.io/github/downloads/fantuz/recordbox/total?label=downloads" alt="GitHub release downloads" height="20"></a>
  <br>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/fantuz/recordbox" alt="MIT license" height="20"></a>
  <a href="app/Cargo.toml"><img src="https://img.shields.io/badge/built%20with-Rust-555555?logo=rust" alt="Built with Rust" height="20"></a>
  <a href="#running-it"><img src="https://img.shields.io/badge/platforms-macOS%20%7C%20Windows%20%7C%20Linux-507269" alt="macOS, Windows and experimental Linux packages" height="20"></a>
</p>
<!-- project-badges:end -->

Recordbox is an unofficial, open-source DJ preparation app for macOS and Windows, with an experimental Linux build. Organise your music library, analyse tracks, set cues and build playlists, then export them to USB for players that read the classic Device Library format.

> **Not affiliated with AlphaTheta or Pioneer DJ.** rekordbox is a trademark of AlphaTheta Corporation. This is an independent project that uses rekordbox's preparation workflow as the reference for how things should behave.

This is the [fantuz/recordbox](https://github.com/fantuz/recordbox) fork of
[adamsardo/recordbox](https://github.com/adamsardo/recordbox), with the original
authors and MIT licence preserved.

<p align="center">
  <a href="https://github.com/fantuz/recordbox/releases/latest"><img src="https://img.shields.io/badge/Download-latest%20release-2e7d32?style=for-the-badge&logo=github" alt="Download the latest Recordbox release" height="32"></a>
  <br>
  <a href="#running-it">Installation details</a> · <a href="https://github.com/fantuz/recordbox/releases">All releases</a>
</p>

Choose your operating system and processor from the release's **Downloads**
table. Packages are unsigned; macOS builds are not notarized. Linux packages are experimental.

## Status

**A first version is ready to try**

So far: 70 source files, 57,441 lines of Rust.

The upstream project is being built in the open with an AI coding agent (Claude Code, run through Craft Anything), and its repository syncs from the build folder about every 15 minutes. This fork adds validated native packages and release automation. Expect gaps and the occasional broken development build. Star or watch the repo to follow along.

## What it's aiming for

- **Library:** import, browse, search and organise your tracks
- **Analysis:** tempo, optional high-precision beat grids for steady and changing tempos, key and waveforms
- **Preparation:** cues, loops, playlists and the rest of the pre-set workflow
- **USB export** for CDJ/XDJ models that read the classic Device Library.

Import integrity checks include signed PQTZ beatgrid shifts and known PSSI
phrase versions, with generated parser regressions. See
[ANLZ import integrity](docs/ANLZ-IMPORT-INTEGRITY.md) for the exact scope and
remaining compatibility limits.

Right-click **Collection → All** and choose **Review collection cleanup…** to see how many references point
to each Windows drive or macOS volume, including disconnected disks and cloud
folders. Linux offers a read-only review. Windows and macOS can move a reviewed
selection of eligible references into Recordbox’s own Removed Tracks.
[Cleanup criteria and recovery](docs/COLLECTION-CLEANUP.md)

## Scope

The first version is deliberately limited to preparation: everything you do *before* the set. Performance features such as live mixing, effects, lighting and video aren't part of it.

Once the first version has landed, extended features can be added if enough people ask for them. For example:

- streaming service support
- connecting CDJs and Pioneer DJ controllers

If there's something you'd use, [open an issue](https://github.com/adamsardo/recordbox/issues) or give an existing one a 👍 so it's easy to see what people want most.

## Privacy and safety

- Library preparation and STEMS processing run locally on your computer. You can use both without an account.
- Audio files are read-only during library sync. Routine edits and analysis are saved in Recordbox’s own library.

The lower-left sync arrows offer two directions.

- **Import · AlphaTheta Rekordbox → recordbox** reads the installed AlphaTheta library and imports tracks into your active recordbox collection.
- **Write back · recordbox → AlphaTheta Rekordbox** reviews selected edits and saves a recovery copy before updating the original desktop library.

**Device Sync Manager** handles classic USB exports. Check your player model and the catalogs already on the drive before exporting. [Desktop library sync](docs/LIBRARY-SYNC.md) · [USB formats and player compatibility](docs/USB-COMPATIBILITY.md)

## Running it

Downloads for the `fantuz/recordbox` fork are available from
[the latest release](https://github.com/fantuz/recordbox/releases/latest).
Choose the macOS app ZIP matching your processor, the Windows x64 ZIP,
or the experimental Linux x64 archive. Prebuilt packages do not require Rust; see the release
notes and [installation guide](docs/BUILD-AND-PACKAGING.md) for requirements,
checksums and known limits.

For NVIDIA STEMS separation on Windows or Linux x64, choose the **CUDA 12**
preview archive. It targets the Quadro P2000 and other compatible GPUs.
[CUDA setup and single-track comparison](docs/CUDA12.md)

For a Radeon Vega 8 or another AMD adapter with a DirectX 12 driver, choose
the **Windows x64 Radeon DirectML** preview ZIP. It includes the inference runtime. Vega 8 driver 26.20.14048.2 and older use manual CPU separation after a full-model GPU hang was reported.
[Check your adapter and compare one song](docs/DIRECTML.md)

For **experimental Linux x64 Vulkan STEMS**, choose the Linux x64 Vulkan archive
from the release downloads. It runs the model's transformer linear layers on the GPU and
the remaining stages on the CPU. [Linux Vulkan setup and comparison](docs/VULKAN.md)

To build from source, install [Rust through rustup](https://rustup.rs). The
repository selects the same toolchain as CI. Install your platform's native
prerequisites from the [build guide](docs/BUILD-AND-PACKAGING.md), then:

```bash
git clone https://github.com/fantuz/recordbox.git
cd recordbox/app
cargo run --release --locked
```

See [Contributing](CONTRIBUTING.md) for the pinned toolchain, checks, dependency
policy and the limits of CI coverage.

This is a work in progress: it may not build at every moment while the agent is mid-change.

The **Build and package** GitHub Actions workflow also produces unsigned macOS
app ZIPs (Apple Silicon and Intel), a Windows x64 portable ZIP and
an experimental Linux x64 archive. Download the artifact for your platform from
a successful workflow run; artifacts include source/runtime manifests and
checksum sidecars. See [build checks and packaging](docs/BUILD-AND-PACKAGING.md)
for installation, system requirements, current platform limitations and the
checks maintainers can enable. Version tags publish validated packages to GitHub
Releases; the application packages remain unsigned.

On Linux, run `./recordbox --disable-load` to keep tracks out of the decks, or
`./recordbox --disable-play` to inspect a loaded track with audio disabled.
Both switches may be combined. For automatic STEMS separation, analysis workers
and one-deck CPU measurements, see the [performance guide](docs/PLAYBACK-PERFORMANCE.md).

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

MIT. See [LICENSE](LICENSE).
