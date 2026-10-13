"""Train a separate Buzzart candidate across musical styles and drawn grammars."""
import argparse
from pathlib import Path
import os

os.environ.setdefault('OPENBLAS_NUM_THREADS', '1')
os.environ.setdefault('OMP_NUM_THREADS', '1')

if __name__ == '__main__':
    from flycomposer.buzzart.breadth_training import run
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run', default='listener-width-20260917')
    parser.add_argument('--generations', type=int, default=72)
    parser.add_argument('--population', type=int, default=32)
    parser.add_argument('--workers', type=int, default=6)
    args = parser.parse_args()
    if Path(args.run).name != args.run:
        parser.error('--run must be a directory name')
    root = Path(__file__).resolve().parent
    run(root, root/'training/buzzart'/args.run, generations=args.generations,
        population=args.population, workers=args.workers)
