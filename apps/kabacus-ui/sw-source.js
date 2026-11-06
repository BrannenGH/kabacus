// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
// SPDX-License-Identifier: GPL-3.0-or-later
const ASSETS = __KABACUS_ASSETS__;
const CACHE = "kabacus-" + __KABACUS_VERSION__;
const urls = ASSETS.map(path => new URL(path, self.registration.scope).href);
self.addEventListener("install",event=>{event.waitUntil(caches.open(CACHE).then(cache=>cache.addAll(urls)).then(()=>self.skipWaiting()));});
self.addEventListener("activate",event=>{event.waitUntil(caches.keys().then(keys=>Promise.all(keys.filter(key=>key.startsWith("kabacus-")&&key!==CACHE).map(key=>caches.delete(key)))).then(()=>self.clients.claim()));});
self.addEventListener("fetch",event=>{
 const url=new URL(event.request.url);
 if(event.request.method!=="GET"||url.origin!==self.location.origin)return;
 // Only the generated application assets are cached. Book data and API calls stay in the page.
 const index=new URL("index.html",self.registration.scope).href;
 const asset=urls.includes(url.href)?url.href:event.request.mode==="navigate"&&url.href.startsWith(self.registration.scope)?index:null;
 if(!asset)return;
 event.respondWith(caches.open(CACHE).then(async cache=>(await cache.match(asset))||fetch(event.request)));
});
