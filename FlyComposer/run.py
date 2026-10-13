from pathlib import Path
import argparse
import json


def main():
    parser=argparse.ArgumentParser(description='Fly Composer — standalone connectome music experiment')
    parser.add_argument('--compose',type=int,metavar='SEED',help='Render a blind comparison and exit without opening audio')
    parser.add_argument('--develop',type=int,metavar='SEED',help='Render three contrasting musical directions using the preferred palette')
    parser.add_argument('--perform',type=int,metavar='SEED',help='Record independent Fly and local Organist performances')
    parser.add_argument('--seconds',type=int,default=75,help='Independent capture limit, 30–120 seconds; Organist may end sooner')
    parser.add_argument('--ensemble',choices=('keys','chamber','luminous'),default=None)
    parser.add_argument('--port',type=int,default=8789)
    parser.add_argument('--check',action='store_true')
    args=parser.parse_args()
    root=Path(__file__).resolve().parent
    from flycomposer.experiment import Studio
    studio=Studio(root)
    if args.check:
        from flycomposer.render import instrument_catalog
        actual=instrument_catalog()
        if {v['id'] for v in actual}!={v['id'] for v in studio.catalog}:
            raise RuntimeError('Packaged instrument bank does not match the saved catalog')
        print(json.dumps({'ready':True,'graph':studio.state()['graph'],'instruments':len(studio.catalog),
                          'packaged_synth_verified':True,'model_service_required_for':'Organist independent performances only','court_required':False}));return
    if args.compose is not None:
        print(json.dumps(studio.compose(args.compose,args.ensemble or studio.state()['preferences']['preferred_ensemble'])));return
    if args.develop is not None:
        print(json.dumps(studio.develop(args.develop,args.ensemble)));return
    if args.perform is not None:
        print(json.dumps(studio.perform(args.perform,args.seconds)));return
    from flycomposer.server import make_server
    server=make_server(studio,args.port)
    print(f'Fly Composer is ready at http://127.0.0.1:{server.server_port}/',flush=True)
    print('Playback starts only when you press Play. Ctrl+C closes this standalone server.',flush=True)
    try:server.serve_forever()
    except KeyboardInterrupt:pass
    finally:server.server_close()


if __name__=='__main__':main()
