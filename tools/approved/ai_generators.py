import collections

# List of generative AI applications from Wikipedia
ai_applications = ["ChatGPT", "Claude", "Copilot", "DeepSeek", "Doubao", "Google Gemini", "Grok", "Qwen", 
                   "DALL-E", "Firefly", "Stable Diffusion", "Midjourney", "Veo", "LTX", "Sora"]

# Count the frequency of application types
applications_by_type = collections.defaultdict(int)
for app in ai_applications:
    if "chatbot" in app.lower():
        applications_by_type["Chatbots"] += 1
    elif "text-to-" in app.lower() and ("image" in app.lower() or "video" in app.lower()):
        applications_by_type["Text-to-Image/Video"] += 1

# Print the statistics summary
print("Generative AI Applications Statistics:")
for type, count in applications_by_type.items():
    print(f"{type}: {count} ({count / len(ai_applications) * 100:.2f}%)")