def mix_fractals(pattern1, pattern2):
    combined_args = " ".join([pattern1, pattern2])
    return f"--type mandala --iterations 300 --palette neon_sunset {combined_args}"

print(mix_fractals("--c-real -0.75 --c-imag 0.1", "--c-real -0.8 +j0.1"))