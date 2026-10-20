// Exercise Longhand's ASR adapter with each catalog artifact. Models and output stay local.
import {join, resolve} from 'node:path';
import {readFileSync} from 'node:fs';
import {mkdir, writeFile} from 'node:fs/promises';
import {capture, sha256File} from './process.ts';
const [pathsFile,wav,outputDirectory]=process.argv.slice(2);
if(!pathsFile||!wav||!outputDirectory)throw new Error('Use asr-matrix.ts MODEL-PATHS.json WAV OUTPUT-DIRECTORY');
const root=resolve(import.meta.dirname,'../..');
const paths: {id:string,path:string,sha256:string}[]=JSON.parse(readFileSync(pathsFile,'utf8'));
const catalog=JSON.parse(readFileSync(join(root,'crates/models/catalog.json'),'utf8'));
const output=resolve(outputDirectory);await mkdir(output,{recursive:true});
const results:object[]=[];let failures=0;
for(const model of catalog.models.filter((item:any)=>item.kind==='asr')){
 const artifact=paths.find(item=>item.id===model.id);
 if(!artifact)throw new Error(`Missing artifact: ${model.id}`);
 const file=await sha256File(artifact.path);
 if(file.size!==model.size_bytes||file.sha256!==model.sha256)throw new Error(`Artifact mismatch: ${model.id}`);
 const modes=['off'];if(model.streaming)modes.push('live');
 for(const mode of modes){
  const id=`${model.id}--${mode}`;const seconds=30;const started=performance.now();
  const {stdout,stderr,code}=await capture(['cargo','test','-p','asr','--test','native_attribution','--','--ignored','--nocapture'],{cwd:root,env:{...process.env,LONGHAND_ASR_MODEL:artifact.path,LONGHAND_ASR_WAV:resolve(wav),LONGHAND_ASR_MODE:mode,LONGHAND_ASR_SECONDS:String(seconds),LONGHAND_ASR_RESULT:join(output,`${id}.json`)}});
  await writeFile(join(output,`${id}.log`),stdout+stderr);
  if(code!==0)failures++;
  results.push({id,sha256:model.sha256,mode,seconds,exitCode:code,elapsedMs:performance.now()-started});
  await writeFile(join(output,'summary.json'),JSON.stringify({componentOnly:true,results,failures},null,2)+'\n');
  console.log(`${code===0?'PASS':'FAIL'} ${id}`);
 }
}
if(failures)process.exitCode=1;
