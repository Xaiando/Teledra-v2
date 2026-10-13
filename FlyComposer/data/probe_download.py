import concurrent.futures
import time
from urllib.parse import quote

import requests

p = 'v1.0/connectome-data/flat-connectome/body-annotations-male-cns-v1.0-minconf-0.5.feather'
urls = [
    'https://storage.googleapis.com/flyem-male-cns/' + p,
    'https://flyem-male-cns.storage.googleapis.com/' + p,
    'https://storage.googleapis.com/download/storage/v1/b/flyem-male-cns/o/' + quote(p, safe='') + '?alt=media',
]


def probe(url):
    start = time.monotonic()
    try:
        result = requests.get(url, headers={'Range': 'bytes=0-1048575'}, timeout=(10, 25))
        print(url[:75], result.status_code, len(result.content), time.monotonic() - start, flush=True)
    except Exception as error:
        print(url[:75], type(error).__name__, str(error)[:100], flush=True)


list(concurrent.futures.ThreadPoolExecutor(3).map(probe, urls))
