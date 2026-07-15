import json

def generate_diplomacy_prompt(model_name, topic):
    prompt = f"Please evaluate the capabilities of {model_name} in terms of its understanding of {topic}. Provide a concise assessment."
    return prompt

if __name__ == "__main__":
    model_name = "Galactica"
    topic = "advanced AI language models"
    print(generate_diplomacy_prompt(model_name, topic))