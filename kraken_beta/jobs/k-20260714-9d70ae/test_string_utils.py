import string_utils
import pytest

def test_empty_string():
    assert string_utils.clean_string("") == ""

def test_all_vowels():
    assert string_utils.clean_string("aeiou") == ""

def test_normal_string():
    assert string_utils.clean_string("hello world") == "hll wrld"

def test_punctuation():
    assert string_utils.clean_string("hello, world!") == "hll wrld"

def test_numbers():
    assert string_utils.clean_string("123abc") == "abc"

def test_special_characters():
    assert string_utils.clean_string("@#$%&*()") == ""

def test_mixed_case():
    assert string_utils.clean_string("Hello World!") == "Hl Wrld!"

def test_whitespace():
    assert string_utils.clean_string("   leading and trailing   ") == "leading and trailing"
