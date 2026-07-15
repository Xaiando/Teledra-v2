import random

def mutate_sigtron(arg_string):
    split_args = arg_string.split(',')
    mutated_args = []
    for arg in split_args:
        if 'alpha' in arg:
            value_range = (0.1, 0.9)
        elif 'beta' in arg:
            value_range = (-0.5, -0.1)
        else:
            continue
        new_value = random.uniform(*value_range)
        mutated_args.append(f"{arg.split('=')[0]}={new_value}")
    return ','.join(mutated_args)

if __name__ == "__main__":
    original_arg_string = "alpha=0.25,beta=-0.3,learning_rate=0.1"
    print(mutate_sigtron(original_arg_string))