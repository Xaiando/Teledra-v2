import json
from typing import NamedTuple

class GateVoltage(NamedTuple):
    value: float


def generate_pattern(voltage: float) -> str:
    # Simplified example of pattern generation logic
    palette = "viridis"
    iterations = 200 + int(voltage * 100)
    return f'--type trilayer --iterations {iterations} --palette {palette}'


gate_voltage = GateVoltage(value=0.5)

pattern_string = generate_pattern(gate_voltage.value)
print(pattern_string)