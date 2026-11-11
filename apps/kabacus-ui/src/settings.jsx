// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
import React,{useState} from "react";
import { t } from "./i18n";
export function Settings({data,auth,book}){
 const [error,setError]=useState("");const settings=book?.settings||{reopen:true,automation:false,control_port:7979};
 async function change(values){try{await book?.onSettings?.(values);setError("");}catch(e){setError(String(e.message||e));}}
 return <section className="panel"><h2>{t("When Kabacus starts")}</h2><label className="checkbox"><input type="checkbox" checked={settings.reopen} onChange={e=>change({reopen:e.target.checked})}/>{t("Reopen my windows")}</label><p className="muted">{t("Show Welcome when this is off.")}</p>
 {window.__TAURI_INTERNALS__&&<><h2>{t("Automation")}</h2><label className="checkbox"><input type="checkbox" checked={settings.automation} onChange={e=>change({automation:e.target.checked})}/>{t("Let agents drive this window")}</label><label>{t("Control port")}<input type="number" min="1" max="65535" defaultValue={settings.control_port||7979} onBlur={e=>change({control_port:Number(e.target.value)})}/></label></>}
 {error&&<p role="alert" className="banner error">{error}</p>}
 <h2>{t("Account")}</h2><p>{book?.local?t("On this computer"):auth.identity||book?.location}</p>{auth.signOut&&<button onClick={auth.signOut}>{t("Sign out")}</button>}
 <h2>{t("About")}</h2><p>Kabacus {__APP_VERSION__}</p><a href={data.version.source_url||"https://github.com/BrannenGH/kabacus"} target="_blank" rel="noreferrer">{t("Open the source code")}</a></section>;
}
