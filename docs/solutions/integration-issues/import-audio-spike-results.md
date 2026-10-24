---
title: Import audio spike results
module: audio-import
tags: [import, decoder, symphonia, aac, mp3, icloud, tcc]
problem_type: integration_issue
---

# Import audio spike results

The spike (U1 of `docs/plans/2026-10-24-0928-feat-import-audio-recordings-plan.md`) checked the assumptions of the audio import before the code depended on them. The spike code was throwaway; only these results stay.

## Decoder: Symphonia 0.6.1 works on real recordings

Symphonia 0.6.1 (`default-features = false`, features `aac`, `isomp4`, `mp3`, `wav`, `pcm`) decoded each test file with no decode error:

| File | Result |
|---|---|
| The private 3 h sample (AAC-LC, 2 ch, 16 kHz, 66 MB) | 10838.7 s in 1.8 s; `afinfo` gives 10838.5 s (the difference is the encoder priming) |
| A 9 h m4a (the sample three times, 197 MB) | 32515.7 s in 5.5 s, 6 MB maximum resident memory |
| AAC-LC mono and stereo from `afconvert` | correct length |
| An m4a with `moov` after `mdat` (the `ffmpeg` default) | correct length |
| mp3 from `lame` | correct length |
| wav (16-bit) | correct length |
| HE-AAC from `afconvert -d aach` | decodes, but only the AAC-LC base layer: the output rate is half of the file rate (11025 Hz for a 22050 Hz file) |

So KTD1 stays: Symphonia, and no fallback to the platform decoders. HE-AAC loses the upper band. Voice Memos and the usual recorder apps write AAC-LC, so this is accepted.

The 0.6 API differs from 0.5: `get_probe().probe(&hint, mss, FormatOptions, MetadataOptions)` returns the format reader, `default_track(TrackType::Audio)`, `codec_params` is `Option<CodecParameters>` with an `Audio` variant, `get_codecs().make_audio_decoder(&params, &AudioDecoderOptions)`, `next_packet()` returns `Result<Option<Packet>>`, and `Packet::track_id` is a field.

## The decoder seeks inside m4a files

`ffmpeg` writes `ftyp free mdat moov`, so the reader must seek forward to `moov` and back to `mdat`. A hash of the decoder's reads is therefore not the hash of the file. KTD3 copies the file in one sequential pass with its hash, and decodes the copy.

## Recording time

`ffmpeg -metadata creation_time=…` writes the time into `mvhd`, and `ffprobe` reads it back. Symphonia does not expose `mvhd`, so U2 reads it with a small atom reader. A Voice Memo from an iPhone was not available to the spike; the manual test M1 compares its start time with the time on the phone.

## An older Longhand and `.longhand/imports/`

No code in the current release reads `.longhand/` by listing it: the library scan reads only the `library` marker, and the shared settings read `settings.json`, `background.md` and `prompts/` by name. The library watcher routes only those names and skips the rest. So an older build ignores `.longhand/imports/`.

The agent skill's `check.py` (`plugins/longhand/skills/longhand-notes/scripts/check.py`, `longhand_files`) makes a digest of every file in `.longhand/`. A watched folder that imports during an agent session would show as a change there, so U9 excludes `imports/`.

## Not checked in the spike

These need a signed build or other hardware, and the manual tests cover them:

- An online-only iCloud file (`SF_DATALESS`) and its download: M2. A sandboxed shell cannot list iCloud Drive (macOS refuses with "Operation not permitted").
- Access to an iCloud Drive or `~/Library/CloudStorage` watched folder after a relaunch: M2.
- An older release on a storage folder with imported notes: M5.
- A OneDrive placeholder on Windows 11: M6.
