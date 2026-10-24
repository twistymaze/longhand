# Audio import test files

Synthetic tones for the tests of `crates/audio-import`. No file holds speech or a real recording. Each file was made with `ffmpeg` 8 and `lame`:

| File | Content | Command |
|---|---|---|
| `tone-mono.m4a` | 5 s, 440 Hz, mono AAC-LC, 22050 Hz; `moov` before `mdat`; `mvhd` time 2026-10-20 14:30:00 UTC | `ffmpeg -f lavfi -i "sine=frequency=440:sample_rate=22050:duration=5" -c:a aac -b:a 32k -ac 1 -movflags +faststart -metadata creation_time=2026-10-20T14:30:00Z tone-mono.m4a` |
| `tone-moov-last.m4a` | 5 s, stereo AAC-LC (440 Hz left, 880 Hz right); `moov` after `mdat`; same `mvhd` time | `ffmpeg -f lavfi -i "sine=frequency=440:sample_rate=22050:duration=5" -f lavfi -i "sine=frequency=880:sample_rate=22050:duration=5" -filter_complex "[0][1]amerge=inputs=2" -c:a aac -b:a 48k -metadata creation_time=2026-10-20T14:30:00Z tone-moov-last.m4a` |
| `tone-no-time.m4a` | as `tone-mono.m4a`, with `mvhd` time zero | `ffmpeg -f lavfi -i "sine=frequency=440:sample_rate=22050:duration=5" -c:a aac -b:a 32k -ac 1 -movflags +faststart -fflags +bitexact tone-no-time.m4a` |
| `tone-stereo.mp3` | 5 s stereo mp3: 440 Hz left, silence right; ID3v2.4 `TDRC` 2026-10-20T14:30:00 | `ffmpeg -f lavfi -i "sine=frequency=440:sample_rate=22050:duration=5" -f lavfi -i "anullsrc=r=22050:cl=mono:d=5" -filter_complex "[0][1]amerge=inputs=2" -c:a libmp3lame -b:a 64k -id3v2_version 4 -metadata date=2026-10-20T14:30:00 tone-stereo.mp3` |
| `tone.wav` | 5 s, 440 Hz, mono 16-bit PCM, 22050 Hz | `ffmpeg -f lavfi -i "sine=frequency=440:sample_rate=22050:duration=5" -c:a pcm_s16le -ac 1 -fflags +bitexact tone.wav` |
