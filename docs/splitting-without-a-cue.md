# Splitting without a CUE sheet

A single `.dsf` or `.dff` that has no CUE sheet (a live recording, a needle-drop of a
record side, a whole-album rip) can be cut in the app or on the command line.

## In the app

1. **Add audio file…** (or drop the file onto the window). The file becomes an album with
   one track, and the **Waveform** tab opens.
2. **Analyse audio.** The file is read once in the background (the first time takes a
   little while for a long side) and drawn as a waveform: the pale bars are the peaks, the
   darker core is the loudness.
3. **Add cuts at the gaps.** Quiet passages that last long enough are shaded; the button
   places a track boundary in each one. The three numbers above the waveform decide what
   counts as a gap:
   - *Silence below* (default −60 dB): the level under which audio is treated as silent.
   - *for at least* (default 1.5 s): shorter quiet moments are ignored.
   - *Shortest track* (default 10 s): no track shorter than this is created.
4. **Refine by eye.**
   - Drag a marker to move a cut. **Hold Ctrl** to snap it to the middle of the nearest gap.
   - **Double-click** empty waveform to add a cut; **right-click** a marker to remove it
     (the track then continues into the next one).
   - Scroll to zoom around the pointer, drag empty space to pan.
   - A marker with ⚠ lands in loud audio and may be audible: move it into a quiet spot.
5. Edit titles and tags as usual, then split.

Cuts are kept to the sample; **Export CUE sheet…** (Album & output tab) saves them as a
CUE sheet if you want one. CUE times are rounded to 1/75 s (about 13 ms), so save a
**project** to keep the cut points exactly.

## On the command line

```sh
dsd-workbench live.dsf --auto --out out --verify        # find the gaps and split there
dsd-workbench live.dsf --auto --dry-run --write-cue live.cue   # only write a CUE sheet to review
dsd-workbench take.dsf --at 2:30 --at 7:12.5 --out out   # cut at times you choose
```

| Option | Meaning |
|--------|---------|
| `--auto` | Detect silent gaps and cut there. |
| `--at TIME` | Add a track at this time (`90`, `1:30`, `1:02:03.5`); repeatable; one file. |
| `--silence-db DB` | Level that counts as silence (default `-60`). |
| `--min-silence S` | Shortest gap, in seconds (default `1.5`). |
| `--min-track S` | Shortest track, in seconds (default `10`). |
| `--cut-at middle\|before` | Cut in the middle of a gap (default) or just before the music. |
| `--write-cue FILE` | Write the resulting CUE sheet. |

Several audio files can be given; with `--auto` each is analysed separately.

## How detection works

The 1-bit stream is filtered down to a normal audio rate with a short low-pass filter and
measured in 10 ms steps (peak and RMS, relative to full scale). That is deliberately
cheaper than the audition filter: it only has to see the music, not play it. A real
sigma-delta recording of a tone is measured to within 1.5 dB, and a modulator's idle noise
sits far below the −60 dB default, which the tests check.

Silence at the very start or end of the file is a lead-in or fade-out, not a gap between
tracks, so it never produces a cut.

If a file has no gaps (a gapless live album), `--auto` leaves it as one track; use `--at`
or the waveform to place cuts by hand.
