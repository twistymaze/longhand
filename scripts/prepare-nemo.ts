// Build and stage the private helper before an application bundle is made.
import {join, resolve} from 'node:path';
const root=resolve(import.meta.dir,'..');
const backend=process.env.LONGHAND_NEMO_BACKEND ?? (process.platform==='darwin'?'metal':process.platform==='win32'?'vulkan':'cpu');
const helperBuild=process.env.LONGHAND_NEMO_HELPER_BUILD ?? join(process.env.LONGHAND_NEMO_CACHE ?? join(root,'target/nemo'),`helper-${backend}`);
const config=await Bun.file(join(root,'src-tauri/tauri.conf.json')).json();
const env={...process.env,LONGHAND_NEMO_BACKEND:backend,LONGHAND_NEMO_HELPER_BUILD:helperBuild};
if(process.platform==='darwin')env.LONGHAND_NEMO_SIGNING_IDENTITY=process.env.LONGHAND_NEMO_SIGNING_IDENTITY ?? process.env.APPLE_SIGNING_IDENTITY ?? config.bundle.macOS.signingIdentity;
async function run(command:string[]){
 const child=Bun.spawn(command,{cwd:root,env,stdout:'inherit',stderr:'inherit'});
 const code=await child.exited;if(code!==0)throw new Error(`${command[0]} failed with code ${code}`);
}
if(process.platform==='win32' && !env.LONGHAND_VULKAN_RUNTIME_DIR){
 const result=Bun.spawnSync(['powershell.exe','-NoProfile','-ExecutionPolicy','Bypass','-File',join(root,'scripts/prepare-vulkan-runtime.ps1'),'-OutputDirectory',join(helperBuild,'../vulkan-runtime')],{cwd:root,env,stdout:'pipe',stderr:'pipe'});
 if(result.exitCode!==0)throw new Error(`Vulkan runtime preparation failed: ${result.stderr.toString()}`);
 env.LONGHAND_VULKAN_RUNTIME_DIR=result.stdout.toString().trim();
 if(!env.LONGHAND_VULKAN_RUNTIME_DIR)throw new Error('Vulkan runtime preparation returned no loader directory');
}
await run(['bash',join(root,'scripts/build-nemo.sh')]);
await run(['bun',join(root,'scripts/package-nemo.ts'),join(helperBuild,'build-output.json')]);
