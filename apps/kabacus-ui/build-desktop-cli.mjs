// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import {execFileSync} from 'node:child_process';
import {copyFileSync,mkdirSync} from 'node:fs';
import {dirname,resolve,join} from 'node:path';
import {fileURLToPath} from 'node:url';
if(!['android','ios'].includes(process.env.TAURI_ENV_PLATFORM)){
 const app=dirname(fileURLToPath(import.meta.url)),root=resolve(app,'../..');
 const host=execFileSync('rustc',['-vV'],{encoding:'utf8'}).match(/^host: (.+)$/m)[1];
 const target=process.env.CARGO_BUILD_TARGET||process.env.TAURI_ENV_TARGET_TRIPLE||host;
 execFileSync('cargo',['build','--release','--locked','--target',target,'-p','kabacus-cli'],{cwd:root,stdio:'inherit'});
 const suffix=target.includes('windows')?'.exe':'';
 const source=join(resolve(root,process.env.CARGO_TARGET_DIR||'target'),target,'release',`kabacus-cli${suffix}`);
 const binaries=join(app,'src-tauri','binaries');mkdirSync(binaries,{recursive:true});
 copyFileSync(source,join(binaries,`kabacus-cli-${target}${suffix}`));
 const artifacts=join(app,'src-tauri','target','release');mkdirSync(artifacts,{recursive:true});
 copyFileSync(source,join(artifacts,`kabacus-cli${suffix}`));
}
