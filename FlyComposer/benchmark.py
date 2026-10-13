"""Run the frozen three-seed local composer benchmark with every trial retained."""
import argparse
from pathlib import Path
import sys

from flycomposer.benchmark import run

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--persona', choices=('orator', 'organist'), default='orator')
    args = parser.parse_args()
    if hasattr(sys.stdout, 'reconfigure'):
        sys.stdout.reconfigure(encoding='utf-8', errors='replace')
    run(Path(__file__).resolve().parent, args.persona)
