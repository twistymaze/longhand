# VAD fixture

`ami-sdm-speech.wav` — 33.96 s of far-field meeting speech, mono 16 kHz PCM16.

- **Source:** AMI Meeting Corpus, single distant microphone condition, via the
  Hugging Face dataset `diarizers-community/ami`, config `sdm`, split `test`,
  row 2 (speakers `FEE013`, `FEE016`), 969.08–1003.04 s.
- **Licence:** CC BY 4.0. © University of Edinburgh and the AMI Consortium.
  Cut from the original without other modification.
- **Why this excerpt:** its annotated speech run starts and ends at pauses of at
  least 1.6 s, so the file begins and ends at a real onset and offset.
  Reproduce with `scripts/diarization/fetch_corpora.py`, then
  `ffmpeg -ss 969.08 -to 1003.04 -i ami_sdm_test_0002.wav -ac 1 -ar 16000 -c:a pcm_s16le`.
