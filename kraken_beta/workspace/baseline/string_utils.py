def reverse_vowels(s):
    vowels = 'aeiouAEIOU'
    v = [c for c in s if c in vowels]
    return ''.join([v.pop() if c in vowels else c for c in s])
