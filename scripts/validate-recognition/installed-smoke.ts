// Run real CPU inference with the helper extracted from the application package.
import {join, resolve, dirname} from 'node:path';
import {mkdir, rename} from 'node:fs/promises';
const helper=process.argv[2];
if(!helper)throw new Error('Use installed-smoke.ts INSTALLED-HELPER');
const root=resolve(import.meta.dir,'../..');
const catalog=await Bun.file(join(root,'crates/models/catalog.json')).json();
const model=catalog.models.find((entry:any)=>entry.id==='sortformer-v2-q80');
const target=join(root,'target/recognition-ci',model.filename);
await mkdir(dirname(target),{recursive:true});
if(!await Bun.file(target).exists()){
 const download=Bun.spawn(['curl','--fail','--location','--retry','3','--output',target+'.part',model.url],{stdout:'inherit',stderr:'inherit'});
 if(await download.exited!==0)throw new Error('Speaker model download failed');
 await rename(target+'.part',target);
}
const hash=new Bun.CryptoHasher('sha256');const file=Bun.file(target);
for await(const chunk of file.stream())hash.update(chunk);
if(file.size!==model.size_bytes||hash.digest('hex')!==model.sha256)throw new Error('Speaker artifact hash mismatch');
const env={...process.env};
delete env.DYLD_LIBRARY_PATH;delete env.DYLD_FALLBACK_LIBRARY_PATH;delete env.LD_LIBRARY_PATH;
// Restrict search paths so developer-installed SDKs cannot satisfy missing DLLs.
env.PATH=process.platform==='win32'?join(process.env.SystemRoot!,'System32'):'/usr/bin:/bin';
const child=Bun.spawn([process.execPath,join(root,'scripts/test-nemo.ts'),'--helper',resolve(helper),'--speaker',target,'--route','separate','--wav',join(root,'crates/vad/tests/fixtures/ami-sdm-speech.wav'),'--gpu','-1'],{env,stdout:'inherit',stderr:'inherit'});
if(await child.exited!==0)throw new Error('Installed helper CPU inference failed');
