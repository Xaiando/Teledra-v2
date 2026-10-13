"""Start or resume bounded, fully local Buzzart practice."""
import argparse
import os
import sys
from pathlib import Path

# Every worker receives one BLAS thread; the CPU budget is the worker count.
for key in ('OPENBLAS_NUM_THREADS','OMP_NUM_THREADS','MKL_NUM_THREADS','NUMEXPR_NUM_THREADS'):
    os.environ[key]='1'

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run',help='New local training directory name')
    parser.add_argument('--hours',type=float,default=8.)
    parser.add_argument('--workers',type=int,default=4)
    parser.add_argument('--population',type=int,default=24)
    parser.add_argument('--max-generations',type=int,default=20000)
    parser.add_argument('--validate-every',type=int,default=32)
    parser.add_argument('--preview-minutes',type=float,default=30.)
    parser.add_argument('--max-candidates',type=int,default=16)
    parser.add_argument('--resume',action='store_true',help='Use the saved configuration and original deadline unchanged')
    parser.add_argument('--parent-wide',help='Starting brain for the wide lineage (default brains/buzzart-wide-candidate.npz)')
    parser.add_argument('--parent-heritage',help='Starting brain for the heritage lineage (default brains/buzzart-indie-long.npz)')
    parser.add_argument('--render',type=Path,help=argparse.SUPPRESS)
    args=parser.parse_args()
    frozen_flags=('--hours','--workers','--population','--max-generations','--validate-every','--preview-minutes','--max-candidates')
    if args.resume and any(token.split('=')[0] in frozen_flags for token in sys.argv[1:]):
        parser.error('--resume restores the saved settings; do not supply new training bounds')
    if os.name=='nt':
        # This process and its children yield CPU to interactive work.
        import ctypes
        ctypes.windll.kernel32.SetPriorityClass(ctypes.windll.kernel32.GetCurrentProcess(),0x00004000)
    if args.render:
        from flycomposer.buzzart.autonomy_render import render_request
        render_request(args.render)
    else:
        if not args.run or Path(args.run).name!=args.run or args.run in ('.','..'):
            parser.error('--run must be one directory name')
        root=Path(__file__).resolve().parent
        import json
        from flycomposer.buzzart import autonomy_train
        from flycomposer.buzzart.autonomy_train import configuration,run
        run_dir=root/'training/buzzart'/args.run
        if args.resume:
            # A run continues from the parents it recorded, whatever the defaults are now.
            if args.parent_wide or args.parent_heritage:
                parser.error('--resume restores the saved parents; do not supply new ones')
            saved=json.loads((run_dir/'protocol.json').read_text(encoding='utf-8')).get('parents')
            if saved:
                autonomy_train.PARENTS.update(saved)
        else:
            for lane,path in (('wide',args.parent_wide),('heritage',args.parent_heritage)):
                if path:
                    if not (root/path).is_file() or not (root/path).with_suffix('.json').is_file():
                        parser.error(f'{path} needs both its .npz and .json beside this script')
                    autonomy_train.PARENTS[lane]=Path(path).as_posix()
        config=None if args.resume else configuration(hours=args.hours,workers=args.workers,population=args.population,
                    max_generations=args.max_generations,validate_every=args.validate_every,
                    preview_minutes=args.preview_minutes,max_candidates=args.max_candidates)
        run(root,run_dir,config=config,resume=args.resume)
