import os

def main():
    if not os.path.exists("D:\Teledra\tools"):
        print("Directory does not exist.")
        return
    
    try:
        # Simulate some operation that might fail
        result = 1 / 0
        print(result)
    except ZeroDivisionError as e:
        print(f"Caught an error: {e}")

if __name__ == "__main__":
    main()