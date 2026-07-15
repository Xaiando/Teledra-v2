import os

def play_sound(path, loop=False):
    try:
        # Simulate sound playback function
        print(f"Playing sound: {path}")
        if loop:
            print("Looping sound")
    except FileNotFoundError as e:
        print(f"Error: {e}. Fallback to silent sound.")
        print("Playing silent sound.")
    except NameError as e:
        print(f"Error: {e}. Fallback to default sound path.")
        print("Playing fallback sound from resources/silent.mp3")

def main():
    # Example usage
    play_sound("nonexistent_path.mp3", loop=True)
    play_sound("supported/path.mp3")
    try:
        play_sound("unsupported/path.mp4")  # This should raise an error
    except Exception as e:
        print(f"Unexpected error: {e}")

if __name__ == "__main__":
    main()