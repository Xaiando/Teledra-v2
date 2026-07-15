import sys
import re
from youtube_transcript_api import YouTubeTranscriptApi

def get_video_id(url):
    pattern = r'(?:https?:\/\/)?(?:www\.)?(?:youtube\.com\/(?:[^\/\n\s]+\/\S+\/|(?:v|e(?:mbed)?)\/|\S*?[?&]v=)|youtu\.be\/)([a-zA-Z0-9_-]{11})'
    match = re.search(pattern, url)
    return match.group(1) if match else None

def main():
    if len(sys.argv) < 2:
        print("Error: No URL provided", file=sys.stderr)
        sys.exit(1)
        
    url = sys.argv[1]
    video_id = get_video_id(url)
    if not video_id:
        # Check if they just passed the 11-char ID directly
        if len(url) == 11 and re.match(r'^[a-zA-Z0-9_-]{11}$', url):
            video_id = url
        else:
            print(f"Error: Invalid YouTube URL or Video ID: {url}", file=sys.stderr)
            sys.exit(1)
        
    try:
        transcript = YouTubeTranscriptApi().fetch(video_id)
        # Combine text into a single paragraph
        combined_text = " ".join([entry.text for entry in transcript])
        print(combined_text)
    except Exception as e:
        print(f"Error retrieving transcript: {e}", file=sys.stderr)
        sys.exit(1)

if __name__ == "__main__":
    main()
