"""Reproducible population graph from the official MaleCNS Cell Type Explorer.

No synthetic wiring fallback. Local integer IDs index named biological cell types;
they are NOT neuPrint neuron body IDs. Published per-type summed connections are
used, rather than rounded per-neuron means. Only observed table edges are modeled.
"""
from __future__ import annotations

import concurrent.futures
import hashlib
import json
from pathlib import Path
import re
from urllib.parse import quote

import numpy as np

DATA = Path(__file__).resolve().parents[1]/'data'
BASE = 'https://reiserlab.github.io/celltype-explorer-drosophila-male-cns/'
SIGNED = {'ACh':1.,'GABA':-1.,'Glu':-1.}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load_circuit(path=None):
    path = Path(path) if path else DATA/'circuit.npz'
    metadata = json.loads(path.with_suffix('.json').read_text(encoding='utf-8'))
    if digest(path) != metadata['npz_sha256']:
        raise ValueError('Circuit file does not match its provenance manifest')
    with np.load(path,allow_pickle=False) as data:
        result = {k:data[k].copy() for k in ('node_ids','src','dst','weights','signs')}
    n = len(result['node_ids'])
    if n != metadata['nodes'] or len(result['src']) != metadata['edges'] or len(result['signs']) != n:
        raise ValueError('Circuit shape mismatch')
    if not (len(result['src'])==len(result['dst'])==len(result['weights'])):
        raise ValueError('Circuit edge table is incomplete')
    if min(result['src'].min(),result['dst'].min())<0 or max(result['src'].max(),result['dst'].max())>=n:
        raise ValueError('Circuit node index outside range')
    if not np.all(np.isfinite(result['weights'])) or not np.all(result['weights']>0):
        raise ValueError('Invalid observed connection counts')
    if not set(result['signs'].tolist()) <= {-1.,1.}:
        raise ValueError('Invalid explicitly modeled signs')
    result['metadata'] = metadata
    return result


def table_edges(html):
    from bs4 import BeautifulSoup
    soup = BeautifulSoup(html,'html.parser')
    table = soup.find(id='downstream-table')
    if table is None:
        raise ValueError('Official type page has no downstream table')
    # HTML omits closing td/tr tags; html.parser nests them. Split at row starts
    # before parsing each row so labels/values never include subsequent rows.
    raw = str(table)
    rows=[]
    for fragment in re.split(r'<tr\b',raw)[2:]:
        row=BeautifulSoup('<tr'+fragment.split('</tr>')[0]+'</tr>','html.parser')
        link=row.find('a')
        total=row.find(attrs={'title':re.compile(r'^∑ connections:')})
        nt=row.find('abbr')
        if link is None or total is None: continue
        count=int(re.search(r'[\d,]+',total['title']).group().replace(',',''))
        cells=row.find_all('td')
        population=re.match(r'\d+',cells[1].get_text(' ',strip=True)) if len(cells)>1 else None
        rows.append({'target':link.get_text(strip=True),'weight':count,
                     'transmitter':nt.get_text(strip=True) if nt else 'unc',
                     'population':int(population.group()) if population else None})
    if not rows: raise ValueError('No exact connection counts were parsed')
    return rows


def acquire():
    """Download small official published pages; retain each raw source and hash."""
    import requests
    DATA.mkdir(exist_ok=True)
    pages=DATA/'type_pages'; pages.mkdir(exist_ok=True)
    catalog_path=DATA/'neurons.json'
    if not catalog_path.exists():
        response=requests.get(BASE+'data/neurons.json',timeout=(10,25));response.raise_for_status()
        catalog_path.write_bytes(response.content)
    catalog=json.loads(catalog_path.read_text())
    selected=[n for n in catalog['names'] if n in ('Delta7','EPG','EPGt','PEG','PEN_a(PEN1)','PEN_b(PEN2)')
              or n.startswith(('ER','ExR','PFN','PFL'))]
    urls={n:next(x['urls']['combined'] for x in catalog['neurons'] if x['name']==n) for n in selected}

    def fetch(name):
        filename=urls[name]
        path=pages/filename
        if not path.exists():
            last=None
            for attempt in range(3):
                try:
                    response=requests.get(BASE+'types/'+quote(filename),timeout=(10,18))
                    response.raise_for_status()
                    table_edges(response.content.decode('utf-8'))
                    path.write_bytes(response.content); break
                except (OSError,requests.RequestException,ValueError) as error: last=error
            else: raise RuntimeError(f'Could not acquire {name}: {last}')
        return name,table_edges(path.read_text(encoding='utf-8'))
    with concurrent.futures.ThreadPoolExecutor(max_workers=6) as pool:
        results={}
        for name,edges in pool.map(fetch,selected):
            results[name]=edges
            print(f'Acquired {name}: {len(edges)} published output rows',flush=True)
    transmitters={}; populations={}
    for rows in results.values():
        for edge in rows:
            if edge['target'] in selected:
                transmitters.setdefault(edge['target'],set()).add(edge['transmitter'])
                if edge['population']: populations[edge['target']]=edge['population']
    # Neuromodulator/uncertain/mixed types need additional physiology. This first
    # population model explicitly restricts signs to the three declared classes.
    names=sorted(n for n in selected if len(transmitters.get(n,set()))==1 and next(iter(transmitters[n])) in SIGNED)
    index={n:i for i,n in enumerate(names)}
    observed=[(index[n],index[e['target']],e['weight']) for n in names for e in results[n]
              if e['target'] in index and e['weight']>0]
    if len(observed)<len(names):raise ValueError('Insufficient observed circuit recurrence')
    src,dst,weights=map(np.array,zip(*observed))
    filename=DATA/'circuit.npz'
    np.savez_compressed(filename,node_ids=np.arange(len(names),dtype=np.int64),
                        src=src.astype(np.int32),dst=dst.astype(np.int32),weights=weights.astype(float),
                        signs=np.array([SIGNED[next(iter(transmitters[n]))] for n in names]))
    edgeset=set(zip(src.tolist(),dst.tolist()))
    metadata={'version':1,'dataset':'male-cns:v1.0','region':'Central complex: heading, ring and fan-shaped-body pathways',
              'node_kind':'cell-type populations; local IDs are not individual neuron body IDs',
              'nodes':len(names),'edges':len(src),'represented_neurons':sum(populations.get(n,0) for n in names),
              'observed_synapse_count':int(weights.sum()),'reciprocal_directed_edges':sum((b,a) in edgeset for a,b in edgeset),
              'selection_rule':'EPG/EPGt/PEG/PEN/Delta7 and named ER/ExR/PFN/PFL types, restricted to consistent ACh/GABA/Glu table annotations; both sides combined',
              'edge_definition':'Exact summed type-to-type connections from downstream-table title attributes, not rounded displayed means',
              'limitations':'Aggregate published table excerpt; unreported edges are excluded, not proven absent. Type aggregation loses cell-specific and bilateral structure. This is not a whole-brain or individual-neuron simulation.',
              'sign_assumptions':{'ACh':'excitatory +1','GABA':'inhibitory -1','Glu':'assumed inhibitory -1; receptor-dependent physiology is not established by connectivity'},
              'excluded_types':[n for n in selected if n not in index],
              'neuron_types':[{'index':index[n],'name':n,'transmitter':next(iter(transmitters[n])),
                               'population':populations.get(n),'url':BASE+'types/'+quote(urls[n])} for n in names],
              'source_snapshot':catalog['metadata'],'provenance_url':BASE,
              'dataset_url':'https://male-cns.janelia.org/download/',
              'license':'CC BY 4.0','license_url':'https://creativecommons.org/licenses/by/4.0/',
              'attribution':'FlyEM (HHMI Janelia), University of Cambridge, MRC Laboratory of Molecular Biology, Google Research; MaleCNS Cell Type Explorer published by Reiser Lab.',
              'sources':[{'url':BASE+'types/'+quote(urls[n]),'file':'type_pages/'+urls[n],
                          'sha256':digest(pages/urls[n]),'bytes':(pages/urls[n]).stat().st_size} for n in selected],
              'catalog_sha256':digest(catalog_path),'npz_sha256':digest(filename)}
    filename.with_suffix('.json').write_text(json.dumps(metadata,indent=2),encoding='utf-8')
    print(json.dumps({k:metadata[k] for k in ('nodes','edges','represented_neurons','observed_synapse_count')}),flush=True)
    return load_circuit()


if __name__=='__main__':acquire()
