// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import { Api, ApiError } from "./api";
import { t } from "./i18n";
const invoke = (name,args) => window.__TAURI_INTERNALS__.invoke(name,args);
let runtime;
async function wasm() {
  runtime ||= import(/* @vite-ignore */ new URL("./wasm/kabacus_wasm.js", document.baseURI).href).then(async (module) => { await module.default(); return module; });
  return runtime;
}
export async function database() {
  return new Promise((resolve,reject) => {
    const request=indexedDB.open("kabacus-local-books",1);
    request.onupgradeneeded=()=>{request.result.createObjectStore("books",{keyPath:"id"});request.result.createObjectStore("state");};
    request.onsuccess=()=>resolve(request.result);request.onerror=()=>reject(request.error);
  });
}
export async function stored(store,operation,key,value) {
  const db=await database();
  try { return await new Promise((resolve,reject)=>{
    const tx=db.transaction(store,operation==="get"||operation==="getAll"?"readonly":"readwrite");
    const object=tx.objectStore(store);const request=operation==="put"?(key===undefined?object.put(value):object.put(value,key)):key===undefined?object[operation]():object[operation](key);
    let result;request.onsuccess=()=>{result=request.result;};tx.oncomplete=()=>resolve(result);tx.onerror=()=>reject(tx.error);tx.onabort=()=>reject(tx.error);
  }); } finally {db.close();}
}
export async function permission(handle) {
  if (!handle) return true;
  return (await handle.queryPermission({mode:"readwrite"}))==="granted" || (await handle.requestPermission({mode:"readwrite"}))==="granted";
}
export function downloadBytes(bytes,filename) {
  const url=URL.createObjectURL(new Blob([bytes],{type:"application/x-kabacus"}));const link=document.createElement("a");link.href=url;link.download=filename;link.click();setTimeout(()=>URL.revokeObjectURL(url),1000);
}
function decode(response) {const bytes=Uint8Array.from(response.body);const text=new TextDecoder().decode(bytes);if(response.status>=400){let body;try{body=JSON.parse(text);}catch{body={message:text};}throw new ApiError(response.status,body);}return {bytes,text};}
export class NativeApi extends Api {
  constructor(){super();}
  async raw(path,{method="GET",body,head}={}) {try{return await invoke("books_request",{request:{method,path,body:body??null,head:head??null}});}catch(error){if(/unauthorized|signed out/i.test(String(error)))throw new ApiError(401,{message:t("Your session ended. Sign in again.")});throw error;}}
  async request(path,options){return JSON.parse(decode(await this.raw(path,options)).text);}
  async text(path,options){return decode(await this.raw(path,options)).text;}
  async download(path,filename){const response=await this.raw(path);downloadBytes(decode(response).bytes,response.filename||filename);}
}
export class BrowserApi extends Api {
  constructor(record,books){super();this.record=record;this.books=books;this.stamp=record.stamp;}
  static async open(record){
    if(record.handle && !(await permission(record.handle)))throw new Error(t("Allow editing again"));
    const bytes=record.handle?new Uint8Array(await (await record.handle.getFile()).arrayBuffer()):record.bytes;
    const module=await wasm();return new BrowserApi(record,await module.BrowserBooks.open(bytes,record.username||"local"));
  }
  static async create({name,currency,account,handle,username="local"}){
    const module=await wasm();const books=await module.BrowserBooks.create(name,currency,account,username);
    const bytes=await books.bytes();
    const record={id:crypto.randomUUID(),name,filename:handle?.name||`${name}.kabacus`,handle,bytes,username,stamp:Date.now()};
    const api=new BrowserApi(record,books);await api.persist(bytes);return api;
  }
  async persist(bytes){
    if(this.record.handle){const writable=await this.record.handle.createWritable({mode:"exclusive"});try{await writable.write(bytes);await writable.close();}catch(error){await writable.abort().catch(()=>{});throw error;}}
    this.record={...this.record,bytes,stamp:Date.now()};this.stamp=this.record.stamp;await stored("books","put",undefined,this.record);
  }
  async reload(){
    const latest=await stored("books","get",this.record.id);
    let bytes,changed=false;
    if(this.record.handle){const file=await this.record.handle.getFile();changed=file.lastModified!==this.modified;this.modified=file.lastModified;if(changed)bytes=new Uint8Array(await file.arrayBuffer());}
    else if(latest&&latest.stamp!==this.stamp){bytes=latest.bytes;this.stamp=latest.stamp;changed=true;}
    if(changed&&bytes?.length){const module=await wasm();const old=this.books;this.books=await module.BrowserBooks.open(bytes,this.record.username||"local");old.free();}
  }
  async raw(path,{method="GET",body,head,signal}={}){
    signal?.throwIfAborted();
    const execute=async()=>{
      if(method!=="GET"||path==="/version")await this.reload();
      const before=method!=="GET"?await this.books.bytes():null;
      const response=JSON.parse(await this.books.request(JSON.stringify({method,path,body:body??null,head:head??null})));
      if(method!=="GET"&&response.status<400&&body?.dry_run!==true){
        try{await this.persist(await this.books.bytes());}
        catch(error){const module=await wasm();const old=this.books;this.books=await module.BrowserBooks.open(before,this.record.username||"local");old.free();throw new ApiError(507,{message:t("Can't save these books. Check the folder's permissions and available space."),details:{reason:String(error)}});}
      }
      return response;
    };
    return navigator.locks?navigator.locks.request(`kabacus:${this.record.id}`,execute):execute();
  }
  async request(path,options){return JSON.parse(decode(await this.raw(path,options)).text);}
  async text(path,options){return decode(await this.raw(path,options)).text;}
  async download(path,filename){const response=await this.raw(path);downloadBytes(decode(response).bytes,response.filename||filename);}
  async saveCopy(){
    await this.reload();const bytes=await this.books.bytes();
    if(window.showSaveFilePicker){
      const handle=await window.showSaveFilePicker({suggestedName:this.record.filename.replace(/\.kabacus$/i," copy.kabacus"),types:[{description:"Kabacus",accept:{"application/x-kabacus":[".kabacus"]}}]});
      if(this.record.handle&&await handle.isSameEntry(this.record.handle))throw new Error(t("Choose a new file name; these books already exist."));
      const writable=await handle.createWritable();try{await writable.write(bytes);await writable.close();}catch(error){await writable.abort().catch(()=>{});throw error;}
    }else downloadBytes(bytes,this.record.filename);
  }
}
export async function importFile(file,handle){
  if(handle){
    for(const existing of await stored("books","getAll")){
      if(existing.handle && await existing.handle.isSameEntry(handle)) return BrowserApi.open(existing);
    }
  }
  const record={id:crypto.randomUUID(),name:file.name.replace(/\.kabacus$/i,""),filename:file.name,bytes:new Uint8Array(await file.arrayBuffer()),handle,stamp:Date.now(),username:"local"};
  const api=await BrowserApi.open(record);await stored("books","put",undefined,record);return api;
}
