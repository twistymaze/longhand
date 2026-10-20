// Build and stage the private helper before an application bundle is made.
import {spawn, spawnSync} from 'node:child_process';
import {readFileSync} from 'node:fs';
import {join, resolve} from 'node:path';
const root=resolve(import.meta.dirname,'..');
const backend=process.env.LONGHAND_NEMO_BACKEND ?? (process.platform==='darwin'?'metal':process.platform==='win32'?'vulkan':'cpu');
const helperBuild=process.env.LONGHAND_NEMO_HELPER_BUILD ?? join(process.env.LONGHAND_NEMO_CACHE ?? join(root,'target/nemo'),`helper-${backend}`);
const config=JSON.parse(readFileSync(join(root,'src-tauri/tauri.conf.json'),'utf8'));
const env:NodeJS.ProcessEnv={...process.env,LONGHAND_NEMO_BACKEND:backend,LONGHAND_NEMO_HELPER_BUILD:helperBuild};
if(process.platform==='darwin')env.LONGHAND_NEMO_SIGNING_IDENTITY=process.env.LONGHAND_NEMO_SIGNING_IDENTITY ?? process.env.APPLE_SIGNING_IDENTITY ?? config.bundle.macOS.signingIdentity;
async function run(command:string[]){
 const [program,...args]=command;
 const code=await new Promise<number|null>((done,fail)=>{
  const child=spawn(program,args,{cwd:root,env,stdio:'inherit'});
  child.on('error',fail);child.on('close',done);
 });
 if(code!==0)throw new Error(`${program} failed with code ${code}`);
}
if(process.platform==='win32' && !env.LONGHAND_VULKAN_RUNTIME_DIR){
 const result=spawnSync('powershell.exe',['-NoProfile','-ExecutionPolicy','Bypass','-File',join(root,'scripts/prepare-vulkan-runtime.ps1'),'-OutputDirectory',join(helperBuild,'../vulkan-runtime')],{cwd:root,env,encoding:'utf8'});
 if(result.status!==0)throw new Error(`Vulkan runtime preparation failed: ${result.error?.message ?? result.stderr}`);
 env.LONGHAND_VULKAN_RUNTIME_DIR=result.stdout.trim();
 if(!env.LONGHAND_VULKAN_RUNTIME_DIR)throw new Error('Vulkan runtime preparation returned no loader directory');
}
await run(['bash',join(root,'scripts/build-nemo.sh')]);
await run([process.execPath,join(root,'scripts/package-nemo.ts'),join(helperBuild,'build-output.json')]);
