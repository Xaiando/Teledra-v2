import numpy as np

def fm_synth(freq_modulator=1000, freq_carrier=5000, mod_index=2):
    t = np.linspace(0, 1, 48000, endpoint=False)
    carrier = np.sin(2 * np.pi * freq_carrier * t)
    modulator = np.sin(2 * np.pi * freq_modulator * t) * mod_index
    signal = (carrier + modulator).astype(np.float32)
    return signal

fm_signal = fm_synth()
print("Generated FM synthesis signal with parameters: Modulator Freq = 1000 Hz, Carrier Freq = 5000 Hz, Index = 2")