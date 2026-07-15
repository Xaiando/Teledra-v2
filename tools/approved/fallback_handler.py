import json

def handle_fallback(action, default_action="burning_ship"):
    """Handle unsupported actions by providing a fallback."""
    try:
        action = json.loads(action)
        return action['type'] if 'type' in action else default_action
    except (json.JSONDecodeError, KeyError):
        print(f"Unsupported action: {action}. Fallback to {default_action}.")
        return default_action

if __name__ == "__main__":
    fallback_action = handle_fallback('{"type": "julia"}')  # Example usage
    print(f"Using fallback action: {fallback_action}")