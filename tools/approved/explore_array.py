import array

def main():
    # Create an array of integers
    arr = array.array('i', [1, 2, 3, 4, 5])
    
    # Print the type and elements of the array
    print(f"Array type: {type(arr)}")
    print("Array elements:", list(arr))

if __name__ == "__main__":
    main()