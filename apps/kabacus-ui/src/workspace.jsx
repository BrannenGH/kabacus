// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import React,{useEffect,useRef,useState} from "react";
import { t } from "./i18n";
import { Api } from "./api";
import { BrowserApi, NativeApi, importFile, stored } from "./local-books";
import icon from "./icon.svg";
const native=()=>!!window.__TAURI_INTERNALS__;
const invoke=(command,args)=>window.__TAURI_INTERNALS__.invoke(command,args);
const localAuth={signedIn:true,identity:"",token:async()=>null};
export function Workspace({Application,hosted}){
 const [book,setBook]=useState(null),[recent,setRecent]=useState([]),[dialog,setDialog]=useState(null),[error,setError]=useState(""),[busy,setBusy]=useState(false),[settings,setSettings]=useState({reopen:true,automation:false,control_port:7979}),[activity,setActivity]=useState(null);
 const [nativeSignedIn,setNativeSignedIn]=useState(true);
 const previous=useRef(null);
 const input=useRef(),windowId=useRef(window.__KABACUS_WINDOW_ID__||((window.opener&&performance.getEntriesByType("navigation")[0]?.type!=="reload")?crypto.randomUUID():sessionStorage.getItem("kabacus.window"))||crypto.randomUUID());
 const channel=useRef();
 useEffect(()=>{if(native()||!window.BroadcastChannel)return;const bus=new BroadcastChannel("kabacus-windows");channel.current=bus;bus.onmessage=e=>{if(e.data.type==="focus"&&e.data.book===book?.id&&e.data.window!==windowId.current){bus.postMessage({type:"present",request:e.data.request,book:book.id});window.focus();}};return()=>{channel.current=null;bus.close();};},[book?.id]);
 async function focusExisting(id){
  if(!channel.current||book?.id===id)return false;
  const bus=channel.current,request=crypto.randomUUID();
  const present=await new Promise(resolve=>{let timer;const reply=e=>{if(e.data.type==="present"&&e.data.request===request){clearTimeout(timer);bus.removeEventListener("message",reply);resolve(true);}};bus.addEventListener("message",reply);timer=setTimeout(()=>{bus.removeEventListener("message",reply);resolve(false);},150);bus.postMessage({type:"focus",book:id,window:windowId.current,request});});
  if(present)window.open("",`kabacus-${id}`)?.focus();return present;
 }
 sessionStorage.setItem("kabacus.window",windowId.current);
 async function list(){const state=native()?await invoke("workspace_state"):await stored("state","get","workspace");setSettings(state||{reopen:true});setRecent(native()?(state?.recent||[]):[...await stored("books","getAll"),...(state?.recent||[]).filter(s=>s.kind==="remote")]);return state;}
 function current(metadata,api,state){if(book&&book.id!==metadata.id)previous.current=book;const value={...metadata,api,state,activity,remember:async(view)=>{
   const record={id:windowId.current,source:metadata.source,bookId:metadata.id,...view,width:window.innerWidth,height:window.innerHeight};
   if(native())await invoke("remember_window",{record});
   else{const workspace=await stored("state","get","workspace")||{reopen:true,windows:[]};workspace.windows=(workspace.windows||[]).filter(w=>w.id!==record.id);workspace.windows.push(record);await stored("state","put","workspace",workspace);}
 }};document.title=`${metadata.name} — Kabacus`;if(!native())window.name=`kabacus-${metadata.id}`;setNativeSignedIn(!metadata.needs_sign_in);setBook(value);setDialog(null);setError("");}
 async function open(source,state){
  setBusy(true);setError("");try{
   if(native()){const metadata=await invoke("open_books",{source});if(metadata.focused)return;current(metadata,new NativeApi(),state);}
   else if(source?.kind==="remote"){
    const url=new URL(source.url);
    if(hosted&&url.origin===location.origin){
     current({id:location.origin,name:location.host,filename:location.host,location:hosted.identity,local:false,source:{...source,url:url.origin}},new Api(hosted.token),state);
    }else{location.assign(`${url.origin}/app/?connect=1`);return;}
   }
   else{const record=typeof source==="string"?await stored("books","get",source):source;if(await focusExisting(record.id))return;const api=await BrowserApi.open(record);current({id:record.id,name:record.name,filename:record.filename,local:true,location:record.handle?record.filename:t("Browser storage"),source:{kind:"browser",id:record.id}},api,state);}
   await list();
  }catch(e){setError(String(e.message||e));}finally{setBusy(false);}
 }
 useEffect(()=>{list().then(async state=>{
   const params=new URLSearchParams(location.search);
   const selected=params.get("books");
   if(!native()&&selected){await open(selected,(state?.windows||[]).find(w=>w.bookId===selected));return;}
   if(!native()&&params.has("welcome"))return;
   const own=(state?.windows||[]).find(w=>w.id===windowId.current);
   if(!native()&&hosted&&(params.has("connect")||sessionStorage.getItem("kabacus.connect")||state?.reopen&&own?.source?.kind==="remote")){
    sessionStorage.removeItem("kabacus.connect");
    const record={id:location.origin,name:location.host,filename:location.host,location:hosted.identity,local:false,source:{kind:"remote",url:location.origin,name:location.host}};
    current(record,new Api(hosted.token),own);return;
   }
   if(state?.reopen&&state.windows?.length){
    if(native()){const restored=await invoke("restore_windows");if(restored?.source)await open(restored.source,restored);}
    else if(own?.source?.kind==="browser")await open(own.bookId,own);
    else setDialog({kind:"reopen",windows:state.windows});
   }
 }).catch(e=>setError(String(e.message||e)));
 if("launchQueue"in window)window.launchQueue.setConsumer(async params=>{for(const handle of params.files||[]){const api=await importFile(await handle.getFile(),handle);current({id:api.record.id,name:api.record.name,filename:api.record.filename,local:true,source:{kind:"browser",id:api.record.id}},api);}});
 },[]);
 useEffect(()=>{if(!native())return;const timer=setInterval(()=>invoke("agent_activity").then(setActivity).catch(()=>{}),500);return()=>clearInterval(timer);},[]);
 async function chooseFile(create=false){
  setError("");try{
   if(native()){const path=await invoke("choose_books",{save:create});if(!path)return;if(create)setDialog({kind:"new",path});else await open({kind:"local",path,name:path.split(/[\\/]/).pop().replace(/\.kabacus$/i,"")});}
   else if(create){let handle;if(window.showSaveFilePicker){handle=await window.showSaveFilePicker({suggestedName:"Household.kabacus",types:[{description:"Kabacus",accept:{"application/x-kabacus":[".kabacus"]}}]});if((await handle.getFile()).size)throw new Error(t("Choose a new file name; these books already exist."));}setDialog({kind:"new",handle});}
   else if(window.showOpenFilePicker){const[handle]=await window.showOpenFilePicker({multiple:false,types:[{description:"Kabacus",accept:{"application/x-kabacus":[".kabacus"]}}]});const api=await importFile(await handle.getFile(),handle);await open(api.record);}
   else input.current.click();
  }catch(e){if(e.name!=="AbortError")setError(String(e.message||e));}
 }
 async function fileAction(id){
  if(id==="welcome"){
   if(native()&&previous.current?.source){await open(previous.current.source,previous.current.state);return;}
   const target=sessionStorage.getItem("kabacus.return");
   if(!native()&&target){sessionStorage.removeItem("kabacus.return");location.assign(target);return;}
   setBook(null);sessionStorage.removeItem("kabacus.connect");return;
  }
  if(["new","open","recent","connect","close"].includes(id)&&document.querySelector('.bookkeeping-dialog form[data-dirty="true"]')&&!window.confirm(t("Discard this change?")))return;
  if(id==="new-window"){if(native())await invoke("new_window");else window.open(location.pathname,"_blank");}
  else if(id==="new")await chooseFile(true);
  else if(id==="open")await chooseFile();
  else if(id==="recent"||id==="connect")setDialog({kind:id});
  else if(id==="save-copy"){if(native())await invoke("save_copy");else await book.api.saveCopy();}
  else if(id==="close"){if(native())await invoke("close_window");else{const state=await stored("state","get","workspace")||{reopen:true,windows:[]};state.windows=(state.windows||[]).filter(w=>w.id!==windowId.current);await stored("state","put","workspace",state);setBook(null);document.title="Kabacus";if(window.opener)window.close();}}
  else if(id==="quit"){if(native())await invoke("quit");else{const state=await stored("state","get","workspace")||{reopen:true,windows:[]};state.windows=(state.windows||[]).filter(w=>w.id!==windowId.current);await stored("state","put","workspace",state);setBook(null);document.title="Kabacus";if(window.opener)window.close();}}
 }
 useEffect(()=>{
   if(!native())return;
   invoke("configure_menu",{labels:Object.fromEntries([["file","File"],["new-window","New Window"],["new","New Books…"],["open","Open Books…"],["recent","Open Recent"],["save-copy","Save a Copy As…"],["close","Close Window"],["connect","Connect to a Server…"],["quit","Quit"]].map(([id,label])=>[id,t(label)]))}).catch(()=>{});
   const action=e=>fileAction(e.detail).catch(e=>setError(String(e.message||e)));const quit=()=>{const dirty=document.querySelector('.bookkeeping-dialog form[data-dirty="true"]');invoke("approve_quit",{approved:!dirty||window.confirm(t("Discard this change?"))}).catch(e=>setError(String(e.message||e)));};window.addEventListener("kabacus-file-action",action);window.addEventListener("kabacus-quit-check",quit);return()=>{window.removeEventListener("kabacus-file-action",action);window.removeEventListener("kabacus-quit-check",quit);};
 },[book]);
 useEffect(()=>{const keyboard=e=>{if(!(e.ctrlKey||e.metaKey))return;const key=e.key.toLowerCase();let action;if(key==="o")action="open";if(e.shiftKey&&key==="n")action="new-window";if(e.shiftKey&&key==="s")action="save-copy";if(e.shiftKey&&key==="w")action="close";if(key==="q")action="quit";if(action){e.preventDefault();fileAction(action).catch(e=>setError(String(e.message||e)));}};window.addEventListener("keydown",keyboard);return()=>window.removeEventListener("keydown",keyboard);},[book]);
 async function setup(event){event.preventDefault();const form=Object.fromEntries(new FormData(event.currentTarget));setBusy(true);setError("");try{
  if(native()){const metadata=await invoke("new_books",{path:dialog.path,name:form.name,currency:form.currency,account:form.account});current(metadata,new NativeApi());}
  else{const api=await BrowserApi.create({...form,handle:dialog.handle});current({id:api.record.id,name:form.name,filename:api.record.filename,local:true,location:dialog.handle?api.record.filename:t("Browser storage"),source:{kind:"browser",id:api.record.id}},api);}
  await list();
 }catch(e){setError(String(e.message||e));}finally{setBusy(false);}}
 async function connect(event){event.preventDefault();const form=Object.fromEntries(new FormData(event.currentTarget));try{
  const url=new URL(form.url);if(url.protocol!=="https:"||url.username||url.password||url.pathname!=="/"||url.search||url.hash)throw new Error(t("The server address must start with https://"));
  if(native()){const metadata=await invoke("connect_server",{origin:url.origin,privateCa:form.ca||""});current(metadata,new NativeApi());await list();}
  else{const state=await stored("state","get","workspace")||{reopen:true,windows:[]};state.recent=[{kind:"remote",url:url.origin,name:url.host},...(state.recent||[]).filter(s=>s.url!==url.origin)].slice(0,20);await stored("state","put","workspace",state);const back=new URL(location.href);back.search="";if(book?.local)back.searchParams.set("books",book.id);const target=new URL(`${url.origin}/app/`);target.searchParams.set("connect","1");target.searchParams.set("returnTo",back.href);sessionStorage.setItem("kabacus.connect","1");location.assign(target.href);}
 }catch(e){setError(String(e.message||e));}}
 return <>
  <input ref={input} hidden type="file" accept=".kabacus" onChange={async e=>{const file=e.target.files[0];e.target.value="";if(file)try{const api=await importFile(file);await open(api.record);}catch(e){setError(String(e.message||e));}}}/>
  {book?<Application key={book.id||book.name} auth={!book.local&&hosted?hosted:!book.local&&native()?{...localAuth,signedIn:nativeSignedIn,identity:book.location,signOut:book.oauth===false?undefined:async()=>{await invoke("sign_out");setNativeSignedIn(false);},signIn:async()=>{const source=book.source;const metadata=await invoke("connect_server",{origin:source.url,privateCa:source.private_ca||""});current(metadata,new NativeApi(),book.state);}}:localAuth} book={{...book,activity,recent,openRecent:open,settings,onSettings:async change=>{const state={...settings,...change};if(native())await invoke("save_workspace_settings",{settings:state});else await stored("state","put","workspace",state);setSettings(state);}}} onFileAction={id=>fileAction(id).catch(e=>setError(String(e.message||e)))}/>:<main className="welcome"><img src={icon} alt=""/><h1>{t("Welcome to Kabacus")}</h1><p>{t("Your books, on this computer. Every change is saved.")}</p><div className="welcome-actions"><button onClick={()=>chooseFile(true)} disabled={busy}>{t("New Books…")}</button><button className="secondary" onClick={()=>chooseFile()} disabled={busy}>{t("Open Books…")}</button><button className="quiet" onClick={()=>setDialog({kind:"connect"})}>{t("Connect to a Server…")}</button></div><h2>{t("Open Recent")}</h2>{recent.map((source,i)=><div className="recent-row" key={source.id||i}><button className="quiet" onClick={()=>open(source)}>{source.name||source.filename||source.url||source.path}</button><button className="quiet" aria-label={t("New Window")} onClick={()=>native()?invoke("new_window",{source}):window.open(`?books=${source.id}`,`kabacus-${source.id}`)}>↗</button></div>)}</main>}
  {dialog&&<div className="workspace-dialog" role="dialog" aria-modal="true" aria-labelledby="workspace-dialog-title"><div className="workspace-dialog-body"><button className="quiet close-dialog" onClick={()=>{setDialog(null);setError("");}}>{t("Cancel")} Esc</button>
   {dialog.kind==="new"&&<><h2 id="workspace-dialog-title">{t("New Books…")}</h2><form onSubmit={setup}><label>{t("Name")}<input name="name" defaultValue="Household" required autoFocus/></label><label>{t("Currency")}<input name="currency" defaultValue="USD" required pattern="[A-Za-z0-9]+"/></label><label>{t("First account")}<input name="account" defaultValue="assets:bank:checking" required/></label><button disabled={busy}>{t("Create books")}</button></form></>}
   {dialog.kind==="connect"&&<><h2 id="workspace-dialog-title">{t("Connect to a Server…")}</h2><form onSubmit={connect}><label>{t("Server address")}<input name="url" type="url" placeholder="https://books.example.org" required autoFocus/></label>{native()&&<label>{t("Private CA")}<input name="ca"/></label>}<button>{t("Connect")}</button><button type="button" className="secondary" onClick={async(e)=>{const form=e.currentTarget.form;try{if(native())await invoke("test_server",{origin:form.elements.url.value,privateCa:form.elements.ca?.value||""});else{const response=await fetch(`${new URL(form.elements.url.value).origin}/version`,{signal:AbortSignal.timeout(10000)});if(!response.ok)throw new Error(response.statusText);}setError(t("Connected"));}catch(e){setError(String(e.message||e));}}}>{t("Test connection")}</button></form></>}
   {dialog.kind==="recent"&&<><h2 id="workspace-dialog-title">{t("Open Recent")}</h2>{recent.map((source,i)=><button key={i} className="recent-row quiet" onClick={()=>open(source)}>{source.name||source.filename||source.path||source.url}</button>)}</>}
   {dialog.kind==="reopen"&&<><h2 id="workspace-dialog-title">{t("Pick up where you left off")}</h2><p>{t("Allow editing again")}</p><button onClick={async()=>{const windows=dialog.windows;for(let i=0;i<windows.length;i++){const item=windows[i];if(i===0)await open(item.source?.kind==="remote"?item.source:item.bookId,item);else if(item.source?.kind==="remote")window.open(`${item.source.url}/app/?connect=1`,`kabacus-${item.source.url}`);else window.open(`?books=${item.bookId}`,`kabacus-${item.bookId}`);}}}>{t("Reopen")}</button></>}
   {error&&<p role="alert" className="banner error">{error}</p>}
  </div></div>}
  {error&&!dialog&&<div className="save-toast" role="alert">{error}<button className="quiet" onClick={()=>setError("")}>{t("Close")}</button></div>}
 </>;
}
