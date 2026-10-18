# Playback

Listen to tracks straight from the source file before splitting. Nothing is written.

## Controls

Each track row has three buttons:

| Button | Plays |
|--------|-------|
| **Track** | The whole track, from its start time to the next track's start (or end of file). |
| **Start** | 2 s before to 3 s after the track's start. The cut is 2 s in. |
| **End** | 3 s before to 2 s after the track's end. The cut is 3 s in. |

Playback is continuous across the cut, so you hear what sits on each side of it. Edit the start time and press **Start** again to compare. Buttons are disabled while the source file is unreadable or the start time is invalid.

The player bar above the tabs has Pause/Resume, Stop, a seek slider, volume, and a **Spectrum** toggle. Seeking restarts playback at that position. Starting a split, removing an album or clearing the project stops playback.

## How it sounds

DSD cannot be played by operating systems directly, so audio is converted to PCM for listening: a 20 kHz low-pass filter, decimated to 88.2 kHz (96 kHz for the 48 kHz family). This is for auditioning cut points; it is not a mastering-grade converter and not bit-perfect output. Only the first two channels are played; other channels are ignored. Playback may start up to 8 DSD samples (about 3 µs at DSD64) earlier than requested; splitting itself stays sample-exact.

## Spectrum

Log-spaced bars from 30 Hz to 20 kHz, computed from the audio actually leaving the player. Peaks rise instantly and fall slowly.

## Requirements

Windows (WASAPI) and macOS (CoreAudio) need nothing. Linux builds need ALSA development headers (`libasound2-dev`, `alsa-lib-devel`) and run on ALSA, PulseAudio or PipeWire. With no output device the app still runs; pressing a listen button shows an error.

## Audio settings

The **Audio** menu in the player bar applies the next time you press a listen button.

| Setting | Options | Effect |
|---------|---------|--------|
| Output sample rate | Standard 88.2/96 kHz, High 176.4/192 kHz, Maximum 352.8/384 kHz | Rate sent to your device. Limited to the DSD rate ÷ 8 (DSD64 reaches 352.8 kHz). |
| Filter | Fast, Balanced, Precise, Maximum | Filter length: longer means a steeper cut-off above 20 kHz and more CPU. |

CPU cost depends on the filter only, not on the output rate. If your device does not support the chosen rate, the audio library resamples it with a basic resampler; check your DAC before using High or Maximum. Settings are not saved between sessions.

Sources can be DSF or DSDIFF. All file access, filter design and device opening run on a dedicated audio thread, so the interface never waits on the sound system. While a device is being opened the status line says "opening…".

