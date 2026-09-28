// Exercise Longhand's ASR adapter with each catalog artifact. Models and output stay local.
import {join, resolve} from 'node:path';
import {mkdir} from 'node:fs/promises';
const [pathsFile,wav,outputDirectory]=process.argv.slice(2);
if(!pathsFile||!wav||!outputDirectory)throw new Error('Use asr-matrix.ts MODEL-PATHS.json WAV OUTPUT-DIRECTORY');
const root=resolve(import.meta.dir,'../..');
const paths: {id:string,path:string,sha256:string}[]=await Bun.file(pathsFile).json();
const catalog=await Bun.file(join(root,'crates/models/catalog.json')).json();
const output=resolve(outputDirectory);await mkdir(output,{recursive:true});
const results:object[]=[];let failures=0;
for(const model of catalog.models.filter((item:any)=>item.kind==='asr')){
 const artifact=paths.find(item=>item.id===model.id);
 if(!artifact)throw new Error(`Missing artifact: ${model.id}`);
 const file=Bun.file(artifact.path);const hash=new Bun.CryptoHasher('sha256');
 for await(const chunk of file.stream())hash.update(chunk);
 if(file.size!==model.size_bytes||hash.digest('hex')!==model.sha256)throw new Error(`Artifact mismatch: ${model.id}`);
 const modes=['off'];if(model.streaming)modes.push('live');
 if(['moss-transcribe-diarize-q80','granite-speech-4.1-2b-plus-q5km','multitalker-parakeet-streaming-0.6b-v1-q80'].includes(model.id))modes.push('native');
 for(const mode of modes){
  const id=`${model.id}--${mode}`;const seconds=mode==='native'?300:30;const started=performance.now();
  const child=Bun.spawn(['cargo','test','-p','asr','--test','native_attribution','--','--ignored','--nocapture'],{cwd:root,env:{...process.env,LONGHAND_ASR_MODEL:artifact.path,LONGHAND_ASR_WAV:resolve(wav),LONGHAND_ASR_MODE:mode,LONGHAND_ASR_SECONDS:String(seconds),LONGHAND_ASR_RESULT:join(output,`${id}.json`)},stdout:'pipe',stderr:'pipe'});
  const [stdout,stderr,code]=await Promise.all([new Response(child.stdout).text(),new Response(child.stderr).text(),child.exited]);
  await Bun.write(join(output,`${id}.log`),stdout+stderr);
  if(code!==0)failures++;
  results.push({id,sha256:model.sha256,mode,seconds,exitCode:code,elapsedMs:performance.now()-started});
  await Bun.write(join(output,'summary.json'),JSON.stringify({componentOnly:true,results,failures},null,2)+'\n');
  console.log(`${code===0?'PASS':'FAIL'} ${id}`);
 }
}
if(failures)process.exitCode=1;
