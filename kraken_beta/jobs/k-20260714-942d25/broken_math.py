def apply_discount(price, discount_rate):
    return price * (1 - discount_rate / 100)

if __name__ == "__main__":
    test_price = 100
    test_discount_rate = 20
    discounted_price = apply_discount(test_price, test_discount_rate)
    print(f"Discounted Price: {discounted_price}")
