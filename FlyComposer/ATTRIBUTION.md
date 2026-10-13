# Sources and packaged components

## Biological data

Dataset: **male-cns:v1.0**, MaleCNS Cell Type Explorer snapshot generated 2026-09-09.

- [Google Research announcement](https://research.google/blog/a-connectomics-milestone-mapping-the-complete-male-fruit-fly-brain/)
- [Janelia MaleCNS downloads](https://male-cns.janelia.org/download/)
- [Janelia exploration resources](https://male-cns.janelia.org/explore/)
- [Reiser Lab MaleCNS Cell Type Explorer](https://reiserlab.github.io/celltype-explorer-drosophila-male-cns/)
- [License: Creative Commons Attribution 4.0](https://creativecommons.org/licenses/by/4.0/)

Credit: FlyEM at HHMI Janelia, University of Cambridge, MRC Laboratory of Molecular Biology, and Google Research; Cell Type Explorer published by Reiser Lab. The local circuit is a transformed population-level subset, with engineered coupling signs and scaling. This experiment is not endorsed by the data authors.

The 53 preserved type pages include two types excluded because their transmitter class needs additional physiological assumptions. Full per-file provenance is in `data/circuit.json`; the included NPZ SHA-256 is `98442a237277ff9d9419c5271fec59a79dab868cd055c3ac0ba76d695cf4957c`.

## Homemade synthesis

`flycomposer/vendor/teledra_synth.py` is copied from this workspace's PyMusic synthesis library. SHA-256: `71bdaa9494898a1c054ba7e70a5440e3313c0c25b8e30819162e71e78a739c6f`.

`bin/PyMusicSynth.exe` is the existing compiled Teledra renderer copied as an isolated note helper, invoked only with `--music-note-server`. Size: 13,990,400 bytes. SHA-256: `1ab256009150bf05cb0b2488a2cdf8d37afd4e99f19c5ce5bbf908e119bbf006`. This mode returns a catalog and raw note audio without launching the court. The classic seven PyMusic timbres render in Python; the 72 named patches use this packaged helper. No general external synthesizer or soundfont is required.

The prototype is a local fork of the user's existing project; this notice does not grant new redistribution rights to the original project's code or binary. Their original licensing continues to apply. NumPy and optional Requests/Beautiful Soup dependencies retain their own licenses when installed.
