import re

def extract_ai_type(text):
    pattern = r"Chatbot|Text-to-Image|Text-to-Video"
    match = re.search(pattern, text)
    if match:
        return match.group()
    else:
        return "Unknown"

if __name__ == "__main__":
    text = """Here is a concise, source-backed factual note: Generative AI applications include chatbots such as ChatGPT, Claude, Copilot, DeepSeek, Doubao, Google Gemini, Grok and Qwen; text-to-image models such as DALL-E, Firefly, Stable Diffusion, and Midjourney; and text-to-video models such as Veo, LTX and Sora. (Source: arXiv.org)"""
    print("AI application type:", extract_ai_type(text))