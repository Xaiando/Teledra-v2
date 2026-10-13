'use strict';
const $ = id => document.getElementById(id);
const player = $('audio-player');
let state = null, selected = null, librarySignature = '', comparisonSignature = '', currentId = null, queue = [], polling = false;
let pollErrorText = null;
const libraryDrafts = new Map();
const comparisonDrafts = new Map();
const playbackBus=typeof BroadcastChannel==='function'?new BroadcastChannel('fly-composer-playback'):null;
if(playbackBus)playbackBus.onmessage=()=>{queue=[];player.pause();};
player.addEventListener('play',()=>playbackBus?.postMessage('pause-other-room-players'));
const names = {connectome:'Real wiring', shuffled:'Shuffled wiring', baseline:'Musical baseline', hidden:'Identity hidden'};
const presets = {
  keys:{lead:'classic:keys',answer:'classic:bell',harmony:'classic:pad',bass:'classic:bass',texture:'classic:pluck'},
  chamber:{lead:'piano',answer:'clarinet',harmony:'violin',bass:'double_bass',texture:'harp'},
  luminous:{lead:'electric_piano',answer:'theremin',harmony:'cello',bass:'classic:bass',texture:'vibraphone'}
};
const formatTime = seconds => {const total=Math.round(seconds);return `${Math.floor(total/60)}:${String(total%60).padStart(2,'0')}`;};
const comparisonTitle = c => c.performance_mode==='study'?`${c.title} · ${new Date(c.created_at).toLocaleTimeString([], {hour:'2-digit',minute:'2-digit'})}`:c.title;
function el(tag, text, cls) { const n=document.createElement(tag); if(text!==undefined)n.textContent=text; if(cls)n.className=cls; return n; }
function button(text, cls, action) { const n=el('button',text,cls);n.type='button';n.addEventListener('click',()=>Promise.resolve().then(action).catch(showError));return n; }
function showError(error) { $('global-error').textContent=error.message || String(error);$('global-error').hidden=false; }
function toast(message) { $('toast').textContent=message;$('toast').hidden=false;setTimeout(()=>$('toast').hidden=true,3500); }
async function api(path, body) {
  const response=await fetch(path,body===undefined?{cache:'no-store'}:{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(body)});
  if(!response.ok){let message=`Request failed (${response.status})`;try{message=(await response.json()).error||message;}catch{}throw new Error(message);}
  return response.json();
}
function playingState() {
  $('transport-play').textContent=player.paused?'Play':'Pause';
  $('transport-play').setAttribute('aria-label',player.paused?'Play current piece':'Pause current piece');
  document.querySelectorAll('[data-track-id]').forEach(n=>n.classList.toggle('is-playing',n.dataset.trackId===currentId&&!player.paused));
  document.querySelectorAll('[data-play-id]').forEach(n=>{
    const active=n.dataset.playId===currentId&&!player.paused;
    n.textContent=n.classList.contains('library-play')?(active?'Ⅱ':'▶'):(active?'Pause sample':'Play sample →');
    n.setAttribute('aria-label',`${active?'Pause':'Play'} ${n.dataset.playTitle}`);
  });
}
async function play(track, keepQueue=false, ensurePlaying=false) {
  if(!keepQueue)queue=[];
  if(currentId===track.id&&player.getAttribute('src')===track.audio_url){
    if(player.paused)await player.play();else if(!ensurePlaying)player.pause();
  }else{
    player.pause();currentId=track.id;player.src=track.audio_url;player.load();
    $('now-playing-title').textContent=track.title||`Sample ${track.label}`;
    $('player-dock').hidden=false;document.body.classList.add('player-open');
    await player.play();
  }
  playingState();
}
function stop() { queue=[];player.pause();player.removeAttribute('src');player.load();currentId=null;$('player-dock').hidden=true;document.body.classList.remove('player-open');playingState(); }
player.addEventListener('play',playingState);player.addEventListener('pause',playingState);
player.addEventListener('ended',()=>{if(queue.length)play(queue.shift(),true,true).catch(showError);else playingState();});
player.addEventListener('error',()=>{if(player.getAttribute('src'))showError(new Error('Audio could not load. Check that the local server is running.'));});
$('stop-audio').addEventListener('click',stop);
$('transport-play').addEventListener('click',()=>{if(player.paused)player.play().catch(showError);else player.pause();});
$('transport-mute').addEventListener('click',()=>{player.muted=!player.muted;$('transport-mute').textContent=player.muted?'Unmute':'Mute';$('transport-mute').setAttribute('aria-label',player.muted?'Unmute audio':'Mute audio');});
$('seek').addEventListener('input',()=>{if(Number.isFinite(player.duration))player.currentTime=Number($('seek').value);});
function showPosition(){
  const duration=Number.isFinite(player.duration)?player.duration:0;
  $('seek').max=duration||1;$('seek').value=player.currentTime||0;
  $('elapsed').textContent=formatTime(player.currentTime||0);$('total-time').textContent=formatTime(duration);
}
player.addEventListener('loadedmetadata',showPosition);player.addEventListener('timeupdate',showPosition);

function renderComparison() {
  const c=state.comparisons.find(c=>c.id===selected);
  $('comparison-empty').hidden=!!c;$('comparison-content').hidden=!c;
  if(!c)return;
  $('comparison-title').textContent=comparisonTitle(c);
  const development=c.kind==='development',revision=c.kind==='revision',performance=c.kind==='performance',buzzart=c.kind==='buzzart',benchmark=c.kind==='benchmark',longform=!!c.longform,named=development||revision||performance||buzzart;
  const studyName=track=>track.display_name||track.direction_name;
  const trackName=track=>benchmark?(c.revealed?studyName(track):`Sample ${track.label}`):(named?studyName(track):(c.revealed?names[track.condition]:`Sample ${track.label}`));
  $('comparison-meta').textContent=revision?`Seed ${c.seed} · same piece and instruments · phrasing revision`:(development?`Seed ${c.seed} · contrasting forms · shared instruments`:`Seed ${c.seed} · matched ensemble and RMS`);
  $('blind-status').textContent=revision?'Original and revision':(development?'Development studies':(c.revealed?'Identities revealed':'Blind listening'));
  $('session-explanation').textContent=revision?'Two versions of the same piece, using the same seed and instruments. This revision focuses on continuity between phrases. Compare them at your own pace; only one version plays at a time.':(development?'These studies share the instruments you chose, with different rhythms, pacing and layer entrances. Keep the ones that deserve another listen.':'Three controllers share one arrangement and ensemble to test the wiring. Musical differences can be subtle. Only one piece plays at a time.');
  $('comparison-hint').textContent=revision?'Choosing Revised or keeping it supports using this phrasing in future Crossing Sparks studies. Choosing Original counts against it. Later ratings can update that preference.':(development?'Your choice helps order future studies; Keep also contributes to the starting instrument palette. These are musical directions, not a test of which wiring wins.':'Choose a preference before revealing the identities. Technical completion is separate from musical quality.');
  $('reveal-button').hidden=named;
  $('play-session').hidden=!performance;$('play-session').disabled=!c.tracks.length;
  $('retry-session').hidden=!performance||!Object.values(c.performer_status||{}).includes('failed');
  $('retry-session').disabled=state.busy||(c.performance_mode==='study'&&!state.capabilities?.organist_study_paging);
  $('retry-session').textContent=c.performance_mode==='study'?'Retry Organist study':'Retry missing recording';
  $('study-organist').hidden=!performance||c.tracks.length!==2;
  $('study-organist').disabled=state.busy||!state.capabilities?.organist_study_paging;
  $('extend-session').hidden=!performance||c.tracks.length!==2||c.tracks.every(t=>t.duration>=118);
  $('extend-session').textContent=c.performance_mode==='continuation'?'Continue toward 2 minutes':'Extend both to 2 minutes';
  $('extend-session').disabled=state.busy;
  if(performance){
    $('comparison-meta').textContent=`Seed ${c.seed} · independently authored · recorded through PyMusic`;
    $('blind-status').textContent='Independent performances';
    $('session-explanation').textContent='Fly uses an evolving circuit and musical memory. The Organist persona writes its own sequence with the configured local model. These are finite recordings; each chose its own musical material and instruments.';
    $('comparison-hint').textContent=c.recording_errors?.length?c.recording_errors.join(' · '):'Download either recording, or play the session in sequence. One shared player switches between them. New recording sessions can develop in the background.';
    if(c.performance_mode==='continuation'){
      $('comparison-meta').textContent=`Seed ${c.seed} · continuation target ${formatTime(c.recording_seconds)} · PyMusic`;
      $('session-explanation').textContent='Each performer develops its own earlier music. Fly resumes its circuit state and memory; Organist continues its own phrases across successive turns. Every original note is retained. The original recordings remain in their earlier session.';
      const partial=Object.entries(c.continuation_status||{}).filter(([,v])=>v.status==='partial');
      if(partial.length)$('comparison-hint').textContent=partial.map(([name,v])=>`${name==='fly'?'Fly':'Organist'} reached ${formatTime(v.actual_seconds)} of the ${formatTime(v.target_seconds)} target. Its valid continuation is saved.${v.stop_reason?.startsWith('invalid_authoring:')?' A later passage failed note validation.':(v.stop_reason?.startsWith('inference_failed:')?' The local model stopped responding.':'')}`).join(' ');
    }
    if(c.performance_mode==='study'){
      $('comparison-meta').textContent=`Seed ${c.seed} · Organist feedback study · PyMusic`;
      $('session-explanation').textContent='Organist writes a fresh piece from its own plan, reviews the actual notes and can revise them. Its instruments and tempo stay the same. The saved Fly recording remains here for comparison; earlier takes and listening notes are preserved.';
      $('comparison-hint').textContent=c.recording_errors?.length?c.recording_errors.join(' · '):`Study direction: ${c.study_context?.feedback||'Develop the composition from listening feedback.'} Musical improvement is for you to judge.`;
    }
  }
  if(buzzart){
    $('comparison-meta').textContent=`Seed ${c.seed} · same instruments · matched RMS`;
    $('blind-status').textContent='Buzzart training check';
    $('session-explanation').textContent=c.scope||'The original Fly and Buzzart perform the same seed with the same instruments. Buzzart keeps the real fly wiring; training changed only its input and readout connections, cell excitability and timing, guided by a designed music critic.';
    $('comparison-hint').textContent='Critic scores are measurements, not taste. Keep or Reject what you hear: your verdicts are the real test.';
    $('play-session').hidden=false;
  }
  if(longform){
    $('comparison-meta').textContent=`Seed ${c.seed} · ${c.longform.bpm} BPM · original and three-minute development`;
    $('blind-status').textContent='Three-minute development';
    $('session-explanation').textContent=c.scope;
    $('comparison-hint').textContent='Listen for continuing bass drive, connected phrases and developing layers. Rendering evidence confirms note delivery; your listening decides the musical result.';
  }
  if(benchmark){
    const musicSeconds=c.benchmark?.music_seconds??40,bpm=c.benchmark?.bpm??96;
    $('comparison-meta').textContent=`Seed ${c.seed} · ${musicSeconds}-second brief · ${bpm} BPM · shared ${c.benchmark?.palette||'chip'} voices`;
    $('blind-status').textContent=c.revealed?'Composer identities revealed':'Blind composer benchmark';
    $('session-explanation').textContent=`Both composers receive the same ${musicSeconds}-second brief, ${bpm} BPM and instrument palette. Each writes its own music. Recordings use the same synthesizer and matched RMS; a short release tail follows the music. Only one sample plays at a time.`;
    const incomplete=c.tracks.length!==2;
    $('comparison-hint').textContent=(incomplete?`${c.tracks.length} of 2 recordings completed. This round cannot support an A/B preference. `:'Listen to A and B before Reveal. Completion and continuity measurements do not rank musical quality. ')+(c.recording_errors?.length?c.recording_errors.join(' '):'');
    $('play-session').hidden=false;
  }
  $('reveal-button').disabled=c.revealed;
  $('comparison-tracks').classList.toggle('track-grid-pair',revision||performance||benchmark||longform);
  $('comparison-tracks').replaceChildren();
  if(benchmark&&!c.tracks.length)$('comparison-tracks').append(el('p','No recording completed in this round. The failure details remain available through Reveal.','empty-state'));
  for(const track of c.tracks){
    const full=state.library.find(t=>t.id===track.id),metrics=full.metrics;
    const card=el('article',undefined,'track-card');card.dataset.trackId=track.id;
    const top=el('div',undefined,'track-top');top.append(el('span',track.label,'track-letter'),el('span',formatTime(track.duration),'track-duration'));
    card.append(top,el('h3',trackName(track)),el('p',benchmark?(c.revealed?track.description:'Identity hidden until you choose to reveal it.'):(named?track.description:(c.revealed?'Same musical rules, synthesis and seed.':'Identity hidden until you choose to reveal it.')),'track-description'));
    const facts=el('div',undefined,'track-facts');facts.append(el('span',`${metrics.layers} parts`),el('span',buzzart&&!longform&&full.critic?`Critic ${full.critic.reward.toFixed(2)}`:(performance?'Independent recording':(benchmark?'Matched composition brief':`${metrics.sections} sections`))));card.append(facts);
    const playButton=button('Play sample →','button track-play',()=>play({...track,title:named||benchmark?`${trackName(track)} · ${c.seed}`:`${c.title} · ${track.label}`}));playButton.dataset.playId=track.id;playButton.dataset.playTitle=trackName(track);card.append(playButton);
    if(performance||buzzart||benchmark){const download=el('a','Download WAV','text-link recording-download');download.href=track.audio_url;download.download=benchmark?`benchmark-${c.seed}-sample-${track.label}-${Math.round(track.duration)}s.wav`:`${full.performer}-${full.performance_mode==='study'?'study':(full.performance_mode==='continuation'?'continued':'performance')}-${c.seed}-${Math.round(track.duration)}s.wav`;card.append(download);}
    const details=el('details');details.append(el('summary','Rendering evidence'));
    const checks=el('ul',undefined,'check-list');
    const benchmarkFacts=[];
    if(benchmark||longform){
      const measured=full.benchmark_metrics||{};
      if(Number.isFinite(measured.occupied_fraction))benchmarkFacts.push(['Scheduled-note coverage',`${(measured.occupied_fraction*100).toFixed(1)}%`]);
      if(Number.isFinite(measured.longest_gap_seconds))benchmarkFacts.push(['Longest scheduled silence',`${measured.longest_gap_seconds.toFixed(2)} s`]);
      if(Number.isFinite(measured.pitched_parts?.mean))benchmarkFacts.push(['Mean pitched parts together',measured.pitched_parts.mean.toFixed(2)]);
    }
    for(const [label,value] of [['Notes placed',`${metrics.notes_mixed} / ${metrics.notes_scheduled}`],...(!performance&&!buzzart&&!benchmark?[['Developed phrases',metrics.unique_lead_phrases]]:[]),...benchmarkFacts,...(buzzart&&!longform&&full.critic?Object.entries(full.critic.components).map(([name,value])=>[`Critic · ${name}`,value.toFixed(2)]):[]),['Peak',metrics.peak.toFixed(3)],['RMS',metrics.rms.toFixed(4)]]){
      const row=el('li');row.append(el('span',label),el('strong',String(value)));checks.append(row);
    }
    details.append(checks,el('p','Placement checks establish technical completion. Your listening decides quality.','evidence-note'));card.append(details);$('comparison-tracks').append(card);
  }
  const feedback=comparisonDrafts.get(c.id)||c.feedback||{};$('preference-options').replaceChildren();
  $('preference-options').closest('fieldset').disabled=benchmark&&c.tracks.length!==2;
  for(const [value,label] of [...c.tracks.map(t=>[t.id,named||benchmark?trackName(t):t.label]),['','No preference']]){
    const choice=el('label',undefined,'preference-option'),input=el('input');input.type='radio';input.name='preferred_track_id';input.value=value;input.checked=(feedback.preferred_track_id||'')===value;choice.append(input,document.createTextNode(label));$('preference-options').append(choice);
  }
  const form=$('comparison-feedback-form');for(const key of ['coherence','motion','fatigue'])form.elements[key].value=feedback[key]||'';
  if($('comparison-note').value!==(feedback.note||''))$('comparison-note').value=feedback.note||'';
  $('feedback-state').textContent=comparisonDrafts.has(c.id)?'Unsaved changes':(Object.keys(feedback).length?'Saved locally':'');
  playingState();
}
function renderLibrary() {
  const focused=document.activeElement;
  const editing=focused?.tagName==='TEXTAREA'&&$('library-list').contains(focused)?{id:focused.id,start:focused.selectionStart,end:focused.selectionEnd}:null;
  const filter=$('library-filter').value;
  const tracks=state.library.filter(t=>filter==='all'||(filter==='unrated'?!t.verdict:t.verdict===filter));
  $('library-empty').hidden=tracks.length>0;$('library-list').replaceChildren();
  $('play-kept').disabled=!state.library.some(t=>t.verdict==='keep');
  for(const track of tracks){
    const row=el('article',undefined,'library-row');row.dataset.trackId=track.id;
    const playButton=button('▶','library-play',()=>play(track));playButton.dataset.playId=track.id;playButton.dataset.playTitle=track.title;
    const info=el('div',undefined,'library-info'),meta=el('div',undefined,'library-meta');
    const preset=presets[track.ensemble],palette=preset&&Object.keys(preset).every(role=>preset[role]===track.voices?.[role])?track.ensemble:'custom voices';
    info.append(el('h3',track.title));for(const value of [formatTime(track.duration),palette,track.longform?'Three-minute development':(track.kind==='benchmark'?'Composer benchmark':(track.kind==='buzzart'?'Buzzart training check':track.kind==='performance'?'independent recording':(track.kind==='revision'?'revised study':(track.kind==='development'?'development study':names[track.condition]))))])meta.append(el('span',value));info.append(meta);
    const verdicts=el('div',undefined,'library-verdict');
    for(const verdict of ['keep','reject']){
      const b=button(verdict==='keep'?'Keep':'Reject',`verdict-button${track.verdict===verdict?' active':''}`,async()=>{await api('/api/feedback',{track_id:track.id,verdict:track.verdict===verdict?'clear':verdict,note:libraryDrafts.get(track.id)?.note??track.note});await poll();});
      b.dataset.verdict=verdict;b.setAttribute('aria-label',`${verdict==='keep'?'Keep':'Reject'} ${track.title}`);verdicts.append(b);
    }
    const draft=libraryDrafts.get(track.id);
    const noteForm=el('form',undefined,'library-note-form');noteForm.hidden=!draft?.open;
    const note=el('textarea');note.id=`track-note-${track.id}`;note.value=draft?.note??track.note??'';note.maxLength=2000;note.rows=2;note.setAttribute('aria-label',`Notes for ${track.title}`);
    note.addEventListener('input',()=>libraryDrafts.set(track.id,{note:note.value,open:!noteForm.hidden}));
    const save=el('button','Save note','button secondary');save.type='submit';noteForm.append(note,save);
    noteForm.addEventListener('submit',async event=>{event.preventDefault();try{await api('/api/feedback',{track_id:track.id,verdict:track.verdict||'clear',note:note.value});toast('Note saved');await poll();}catch(error){showError(error);}});
    row.append(playButton,info,verdicts,button(track.note?'Edit note':'Add note','library-note-toggle',()=>{noteForm.hidden=!noteForm.hidden;libraryDrafts.set(track.id,{note:note.value,open:!noteForm.hidden});if(!noteForm.hidden)note.focus();}),noteForm);$('library-list').append(row);
  }
  if(editing){const restored=$(editing.id);if(restored){restored.focus({preventScroll:true});restored.setSelectionRange(editing.start,editing.end);}}
  playingState();
}
function renderCatalog() {
  const groups=new Map();for(const voice of state.catalog){if(!groups.has(voice.group))groups.set(voice.group,[]);groups.get(voice.group).push(voice.name);}
  $('catalog-summary').textContent=`${state.catalog.length} homemade instruments + 7 classic PyMusic timbres`;
  $('catalog-list').replaceChildren();for(const [name,voices] of groups){const group=el('div',undefined,'catalog-group');group.append(el('h4',name),el('p',voices.join(' · ')));$('catalog-list').append(group);}
  $('voice-options').replaceChildren();
  const voices=[...['sine','triangle','pluck','bell','pad','keys','bass'].map(v=>({id:`classic:${v}`,name:`Classic ${v}`})),...state.catalog];
  for(const role of Object.keys(presets.keys)){
    const label=el('label',role[0].toUpperCase()+role.slice(1)),select=el('select');select.id=`voice-${role}`;
    for(const voice of voices){const option=el('option',voice.name);option.value=voice.id;select.append(option);}label.append(select);$('voice-options').append(label);
  }
  setPreset();
}
function setPreset(){const ensemble=new FormData($('compose-form')).get('ensemble');for(const [role,voice] of Object.entries(presets[ensemble]))$(`voice-${role}`).value=voice;}
async function poll() {
  if(polling)return;polling=true;
  try{
    const next=await api('/api/state'),first=!state;state=next;
    $('connection-status').className='status connected';$('connection-status').lastElementChild.textContent=state.busy?'Composing locally':'Local server ready';
    $('compose-button').disabled=state.busy;$('retry-session').disabled=state.busy;$('extend-session').disabled=state.busy;$('compose-progress').hidden=!state.busy;$('progress-label').textContent=state.progress;
    const studyReady=state.capabilities?.organist_study_paging===true;
    $('study-organist').disabled=state.busy||!studyReady;
    $('study-organist').textContent=studyReady?'Rework Organist from these notes':'Restart Fly Composer to enable reworking';
    if(!studyReady&&state.comparisons.find(c=>c.id===selected)?.performance_mode==='study')$('retry-session').disabled=true;
    if(state.error){pollErrorText=state.error;showError(new Error(state.error));}
    else if(pollErrorText){if($('global-error').textContent===pollErrorText)$('global-error').hidden=true;pollErrorText=null;}
    const preference=state.preferences;
    if(preference){
      const paletteSummary=preference.supporting_batches?`Starting palette: ${preference.preferred_label}, from your saved verdicts in ${preference.supporting_batches} listening ${preference.supporting_batches===1?'session':'sessions'}. Related comparison samples count as one session. You can choose any instruments below.`:'Starting palette: Luminous. Keep pieces you like to guide future sessions.';
      $('preference-summary').textContent=paletteSummary+(preference.preferred_revisions?.sparks==='connected_phrases_v1'?' Crossing Sparks will use your preferred connected phrasing.':'');
    }
    if(first){
      $('graph-badge').textContent='MaleCNS · v1.0';$('graph-nodes').textContent=state.graph.nodes.toLocaleString();$('graph-edges').textContent=state.graph.edges.toLocaleString();$('graph-region').textContent=`${state.graph.region}. ${state.graph.represented_neurons} neurons aggregated into cell types.`;$('graph-source').href=state.graph.provenance_url;
      if(preference)document.querySelectorAll('input[name="ensemble"]').forEach(input=>input.checked=input.value===preference.preferred_ensemble);
      renderCatalog();if(preference)for(const [role,voice] of Object.entries(preference.preferred_voices))$(`voice-${role}`).value=voice;
    }
    const cs=JSON.stringify(state.comparisons),ls=JSON.stringify(state.library);
    if(cs!==comparisonSignature){
      const previouslySelected=selected;
      if(!state.comparisons.some(c=>c.id===selected))selected=(state.comparisons.find(c=>c.tracks.length>=(c.kind==='performance'?2:1))||state.comparisons[0])?.id||null;
      $('comparison-select').replaceChildren();for(const c of state.comparisons){const option=el('option',comparisonTitle(c));option.value=c.id;$('comparison-select').append(option);}
      if(!state.comparisons.length)$('comparison-select').append(el('option','No comparisons yet'));
      $('comparison-select').value=selected||'';
      // New background pieces do not erase listening notes in a different session.
      const old=JSON.parse(comparisonSignature||'[]').find(c=>c.id===selected),current=state.comparisons.find(c=>c.id===selected);
      if(first||previouslySelected!==selected||JSON.stringify(old)!==JSON.stringify(current))renderComparison();
      comparisonSignature=cs;
    }
    if(ls!==librarySignature){renderLibrary();librarySignature=ls;}
  }catch(error){$('connection-status').className='status offline';$('connection-status').lastElementChild.textContent='Server unavailable';pollErrorText=error.message||String(error);showError(error);}
  finally{polling=false;}
}
$('comparison-select').addEventListener('change',()=>{selected=$('comparison-select').value;renderComparison();});
$('library-filter').addEventListener('change',renderLibrary);
function feedbackPayload() {
  const form=new FormData($('comparison-feedback-form')),data={comparison_id:selected,preferred_track_id:form.get('preferred_track_id')||null,note:form.get('note')};
  for(const key of ['coherence','motion','fatigue'])data[key]=form.get(key)?Number(form.get(key)):null;
  return data;
}
for(const event of ['input','change'])$('comparison-feedback-form').addEventListener(event,()=>{if(selected){comparisonDrafts.set(selected,feedbackPayload());$('feedback-state').textContent='Unsaved changes';}});
function acknowledgeSavedFeedback(data){if(JSON.stringify(comparisonDrafts.get(data.comparison_id))===JSON.stringify(data))comparisonDrafts.delete(data.comparison_id);if(selected===data.comparison_id&&!comparisonDrafts.has(selected))$('feedback-state').textContent='Saved locally';}
$('reveal-button').addEventListener('click',async()=>{
  const data=feedbackPayload();$('reveal-button').disabled=true;
  try{await api('/api/comparison-feedback',data);acknowledgeSavedFeedback(data);await api('/api/reveal',{comparison_id:data.comparison_id});await poll();}
  catch(error){showError(error);$('reveal-button').disabled=false;}
});
$('comparison-feedback-form').addEventListener('submit',async event=>{
  event.preventDefault();const data=feedbackPayload();
  try{await api('/api/comparison-feedback',data);acknowledgeSavedFeedback(data);toast('Listening notes saved locally');await poll();}catch(error){showError(error);}
});
$('compose-form').addEventListener('submit',async event=>{
  event.preventDefault();const form=new FormData(event.target),voices={};for(const role of Object.keys(presets.keys))voices[role]=$(`voice-${role}`).value;
  $('compose-button').disabled=true;$('global-error').hidden=true;
  const independent=form.get('generation')==='performance';
  try{await api(independent?'/api/perform':(form.get('generation')==='wiring'?'/api/compose':'/api/develop'),independent?{seed:Number(form.get('seed')),seconds:75}:{seed:Number(form.get('seed')),ensemble:form.get('ensemble'),voices,blind:true});toast('Composing in the background. You can keep listening.');await poll();}catch(error){showError(error);$('compose-button').disabled=false;}
});
function showCreationMode(){
  const wiring=$('creation-mode').value==='wiring',independent=$('creation-mode').value==='performance';
  $('arrangement-options').hidden=independent;
  $('compose-button').firstElementChild.textContent=independent?'Record Fly and Organist':(wiring?'Compose a wiring comparison':'Develop three studies');
  $('generation-help').textContent=independent?'Each performer chooses its sequence and instruments. Record up to 75 seconds each; the Organist may end sooner. The local model may take a few minutes. Existing audio remains available.':(wiring?'One shared arrangement, real wiring, shuffled wiring and baseline. Identities stay hidden until Reveal.':'A seed makes each session repeatable. Development varies the form, meter and layers while keeping your chosen instruments.');
}
$('creation-mode').addEventListener('change',showCreationMode);showCreationMode();
document.querySelectorAll('input[name="ensemble"]').forEach(n=>n.addEventListener('change',setPreset));
function newSeed(){const values=new Uint32Array(1);crypto.getRandomValues(values);$('seed').value=values[0]&0x7fffffff;}
$('random-seed').addEventListener('click',newSeed);newSeed();
$('play-kept').addEventListener('click',()=>{queue=state.library.filter(t=>t.verdict==='keep');if(queue.length)play(queue.shift(),true,true).catch(showError);});
$('play-session').addEventListener('click',()=>{const c=state.comparisons.find(c=>c.id===selected);queue=(c?.tracks||[]).map(t=>({...t,title:t.display_name||t.direction_name||`Sample ${t.label}`}));if(queue.length)play(queue.shift(),true,true).catch(showError);});
$('retry-session').addEventListener('click',async()=>{
  if(state.comparisons.find(c=>c.id===selected)?.performance_mode==='study'&&!state.capabilities?.organist_study_paging){showError(new Error('Restart Fly Composer to load the updated Organist study workflow.'));return;}
  $('retry-session').disabled=true;$('global-error').hidden=true;
  try{const c=state.comparisons.find(c=>c.id===selected);if(c?.performance_mode==='study')await api('/api/organist-study',{source_id:c.study_context.source_comparison_id,feedback:c.study_context.feedback,seconds:c.recording_seconds});else await api('/api/perform',{retry_id:selected});toast('Retrying the missing performer. Your saved recording remains available.');await poll();}
  catch(error){showError(error);$('retry-session').disabled=false;}
});
$('study-organist').addEventListener('click',async()=>{
  if(!state.capabilities?.organist_study_paging){showError(new Error('Restart Fly Composer to load the updated Organist study workflow.'));return;}
  const feedback=$('comparison-note').value;
  if(!feedback.trim()){showError(new Error('Write what Organist should change in your listening notes first.'));$('comparison-note').focus();return;}
  $('study-organist').disabled=true;$('global-error').hidden=true;
  try{await api('/api/organist-study',{source_id:selected,feedback,seconds:75});toast('Organist is planning a fresh take from your notes. Saved music remains available.');await poll();}
  catch(error){showError(error);$('study-organist').disabled=false;}
});
$('extend-session').addEventListener('click',async()=>{
  $('extend-session').disabled=true;$('global-error').hidden=true;
  try{await api('/api/perform',{source_id:selected,seconds:120});toast('Both performers are continuing their own music. Your originals remain saved.');await poll();}
  catch(error){showError(error);$('extend-session').disabled=false;}
});
poll();setInterval(poll,2000);
