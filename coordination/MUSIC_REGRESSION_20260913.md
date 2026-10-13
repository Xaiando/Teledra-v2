# Music regression correction — 2026-09-13

The operator reported that the music was extremely annoying and had regressed
after the instrument expansion. Availability checks had established that the
72 patches rendered; they had not established pleasant composition or listening
quality. This follow-up investigates the actual current score and audio.

## Evidence

`target/music-regression/reported-score.json` and `reported-audio.wav` preserve
the reported movement, Dorian Echoes: Rondo Experiment, revision 4432. Celesta
and Arp duplicate the same 32-note ascending/descending scale, written at MIDI
79–113 with identical durations and dynamics. Zero development leaves that
material literal. The ledger previously awarded it 100/100.

Per-note octave folding reversed four melodic steps in each part and created
leaps up to ten semitones where the written maximum was four. Register fitting
now moves the whole line by one constant octave offset. The same snapshot has
zero inserted direction reversals; chord voicings retain their separate logic.

Equal-level detuned oscillator stacks canceled the fundamental in several
voices. At MIDI 48, the measured fundamental share in the harmonic series of
Brass Ensemble was about 1.3%; preserving a central oscillator raises it to
about 70%. Triangle corner correction reduces one measured folded harmonic
by 8.16 dB. PCM, the measurement script and JSON are under the same directory.

The master previously boosted every piece to peak 0.89, undoing a quiet mix.
It now only attenuates excessive peaks. The reported 80-second score rendered
through the corrected engine measures peak 0.287 and RMS 0.0535, versus peak
0.890 and RMS 0.1434 before, an 8.56 dB RMS reduction. Energy above 2 kHz falls
from 4.64% to 0.29% in the sampled comparison. These measurements describe
specific signal changes; they are not a listening verdict.

## Composition and continuity

- Native evolution preserves instruments in explicitly composed ensembles.
  All 72 remain available to deliberate composition and Studio's library.
- Automatic composer guidance prioritizes range, independent accompaniment,
  recognizable phrases and actual gaps before optional experiments. There is
  no requirement to rotate through unused instruments.
- Score diagnostics expose range overflow, exact pitched doubling and notes
  that sustain through nominal rest slots. A form label alone does not prove
  contrasting musical material. Studio labels its percentage SCORE CHECKS and
  makes no concert-readiness claim.
- The observed combination of a wholly duplicated, out-of-range melodic
  ensemble receives the existing bounded repair in automatic composition.
  Failed repair retains the previous piece; ordinary loading still permits
  intentionally doubled lines.
- Renaming an explicitly selected track is no longer mistaken for new music.
  Form and phrase changes still remain musical identity because they can
  restart patterns and the progression even at zero development.
- The operator's rejection and factual findings were recorded in the live
  music feedback directory for the Organist's next composition context.
- Startup requests one repair of a saved score matching the known ensemble
  failure, without waiting for the regular composition timer or overwriting it.
- Musical identity uses the renderer's 0.34 sidechain default; explicitly
  disabling that default ducking is correctly recognized as a mix change.

The saved scores were preserved. `corrected-audio.wav` is an isolated rerender
of the exact reported score, not a newly invented replacement composition.
No claim of listening or restored subjective appeal is made.

## Verification

The combined Music and Teledra release test suites pass, including all 72
instrument renders, contour preservation, gain response, truthful diagnostics,
composer repair/fallback and bounded startup repair. Eleven hidden Studio
tests and the muted real-device render/transport/persistence/stop/play test
pass. Strict release Clippy, formatting and diff whitespace checks pass.
The normal release executable was rebuilt, and `teledra.exe --check` passes.
Use the normal launcher to activate the updated executable.

An isolated live repair initially fixed register and duplicated parts, but then
wrote sixteen continuous cello attacks per bar. The musical repair test failed
despite a technically valid 40-second WAV. That evidence is retained under
`target/music-regression/model-repair/`; it was never installed or played.
This exposed a separate contract hole: the composer could promise attacks the
native patch could not articulate. Automatic acceptance now checks the catalog's
minimum onset spacing and gives a precise bounded repair error. The schema and
composer guidance also clarify that held duration does not advance the onset
grid. This check does not ban fast parts on instruments that support them.

One fresh bounded live request after that acceptance change still authored
continuous cello sixteenths. The new guard rejected the result after the normal
single repair: written onset spacing 0.25 beats versus Cello's required 1.00.
`target/music-regression/model-repair-attack-spacing/generation-error.txt`
preserves the failure. No score or WAV was accepted from this second request,
and no further model attempts were made. The evidence proves that this known
unplayable output is now stopped; it does not prove reliable model phrasing.
Final combined deterministic release suites, strict Clippy and environment
check passed with the acceptance change. The rebuilt executable includes it.
