# Fractus

Owns the bounded version-2 scene DSL, validation, native PNG renderer, looping
GIF renderer, artifact verification, manifests, and the persistent Fractus
Studio presentation surface.

The Rust renderer remains the only art generator. After verification, Teledra
sends the trusted artifact path to one supervised Studio process over JSONL.
The same gallery window hot-reloads later scenes, plays the renderer's distinct
GIF phases, and gives static PNGs a subtle reversible pan/zoom presentation.
It provides Play/Pause and Fit controls without changing the persisted source
or allowing arbitrary code. Closing the gallery is safe; the next render opens
it again, and exiting Teledra kills and reaps it.
