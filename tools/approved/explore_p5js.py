import random

def main():
    # Generate a random example URL from the list
    examples = [
        "https://p5js.org/examples/circles-intersecting/",
        "https://p5js.org/examples/gradient-colors/",
        "https://p5js.org/examples/sketch-as-object/",
        "https://p5js.org/examples/motion-arc-sin/",
    ]
    example_url = random.choice(examples)

    # Print the URL and a vivid description
    print(f"Teledra's court is intrigued by the {example_url.split('/')[-2]} phenomenon. Let us explore its wonders.")
    print(f"{example_url}")

if __name__ == "__main__":
    main()