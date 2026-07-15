import json

def CurateRecipe(technique, rule, **kwargs):
    if technique == "L-System":
        recipe = {"type": "l_system", "rule": rule, "iterations": kwargs.get("iterations", 3)}
        return json.dumps(recipe)
    
print(CurateRecipe("L-System", "F -> F+F--F-F", iterations=5))