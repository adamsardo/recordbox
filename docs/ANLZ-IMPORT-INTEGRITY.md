# ANLZ import integrity

The importer materializes effective PQTZ beat times by adding the signed
big-endian 16-bit header shift at section offset `0x12` to each raw millisecond
timestamp exactly once. Negative shifts are supported when the resulting beat
times stay nonnegative. A grid with any effective timestamp before zero is
left unimported: the audible grid model drops preroll beats, which would change
native PSSI's positional beat indices. It is not clamped or wrapped into an
incorrect imported grid. Supporting that case requires explicit preservation
of the source beat indexing. Workbench can still report those signed times as
read-only evidence. This change does not add PQT2 fractional
timestamp import; the existing unchanged-grid export path preserves paired
PQTZ/PQT2 sections. A newly generated PQTZ can continue
to write a zero shift because its timestamps are already materialized.

Native phrase reuse also requires every source PQTZ record to match the
audible grid at the same positional index, including bar beat and tempo,
within the documented millisecond quantization tolerance. A valid positive
shift can make the player extrapolate an additional audible beat before the
first source record. Its corrected grid is still imported, but native phrases
are then held rather than attached to the wrong beats; the existing phrase
analysis fallback remains available. Anchor equality alone is insufficient.

PSSI parsing recognizes versions 0 and 1 with 24-byte entries, validates the
declared section bounds and entry count, and rotates the XOR mask by the
version before interpreting masked payloads. Invalid framing or mood values
are rejected rather than becoming plausible phrase/cue metadata. Unsupported
sections retain the existing raw-section preservation behavior; this does not
add a writer for a new native format or establish native STEMS readiness.

The regressions use independently generated binary layouts. They cover
positive and negative grid shifts, zero-time boundaries, unsupported preroll,
unchanged zero-shift behavior, phrase-index mismatches after extrapolation,
masked and unmasked phrase versions, and malformed or unsupported headers.
They exercise the application parsers, not just a second copy of the masking
arithmetic. Existing broader application-test and advisory baselines are
described in [build checks](BUILD-AND-PACKAGING.md).

Public sources inspected on 19 November 2026:

- [Signed PQTZ shift discovery](https://github.com/teknopaul/vdj-ui/commit/7b33c57344c4cb6bb745d8f84dea445c732bc2a1)
  and [public timing regression](https://github.com/teknopaul/vdj-ui/blob/01a81a475ef1c4cebeba55325862741a82234891/tests/test_trakcrate_deckedits.cpp#L68-L77).
- [PSSI version/stride schema](https://github.com/dylanljones/pyrekordbox/blob/50b4604d98dfd52740aa9e76d80c3a32dfa451ba/pyrekordbox/anlz/structs.py#L246-L260)
  and [version-dependent mask](https://github.com/dylanljones/pyrekordbox/blob/50b4604d98dfd52740aa9e76d80c3a32dfa451ba/pyrekordbox/anlz/file.py#L123-L142).

Only format facts informed the independently written implementation. No
external implementation, proprietary executable, personal library or audio
fixture was copied. These tests do not certify every device or firmware.
