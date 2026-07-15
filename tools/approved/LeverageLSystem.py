import json

def levy_curve(axiom="F F", rules={"F F": "-F"}, angle=45):
    """
    Generates a string representing the L-system for the Lévy curve.
    """
    stack = [axiom]
    for _ in range(3):  # Simple depth to demonstrate
        new_stack = []
        for symbol in stack:
            if symbol in rules:
                new_stack.extend(rules[symbol])
            else:
                new_stack.append(symbol)
        stack = new_stack
    
    result = "".join(stack).replace("F", "s()")
    
    return result

print(json.dumps({"pattern": levy_curve(), "angle": 45}))