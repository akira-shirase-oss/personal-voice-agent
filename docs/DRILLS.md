# Manual drills

Some behaviour needs a person and real hardware: macOS permission prompts
cannot be granted by an agent, and live microphone/speaker timing is not
reproducible in CI. These drills cover exactly the thin live paths the
automated tests cannot. Record each run under `perf/results/` with date,
machine, commit, macOS version and `uptime` load average.

The drill steps are written when the live paths they exercise land
(V1 capture/playback, V2 conversation).
