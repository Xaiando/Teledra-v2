def build_pattern():
    layers = [
        's("bd ~ sn ~ hh*2 oh").gain(0.48)',
        'note("c2 eb2 g2 bb2").s("triangle").gain(0.34).slow(1.5)',
        'note("c4 g4 bb4 eb5").s("sawtooth").gain(0.20).slow(2)',
        'note("g5 eb5 c5 bb4").s("sine").gain(0.18).fast(1.5)',
    ]
    return "stack(\\n" + ",\\n".join(layers) + "\\n)"


print("Gothic Clockwork Pulse")
print(build_pattern())
