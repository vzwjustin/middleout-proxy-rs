pub fn get_dashboard_html() -> String {
    r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>MiddleOut Proxy</title>
<style>
*,*::before,*::after{box-sizing:border-box;margin:0;padding:0}
:root{
  --bg:#0f172a;--surface:#1e293b;--surface-2:#334155;--border:#475569;--border-2:#64748b;
  --text:#f8fafc;--sub:#cbd5e1;--muted:#94a3b8;
  --blue:#3b82f6;--blue-soft:rgba(59,130,246,0.15);--blue-line:rgba(59,130,246,0.5);
  --green:#22c55e;--green-soft:rgba(34,197,94,0.15);
  --red:#ef4444;--red-soft:rgba(239,68,68,0.15);
  --amber:#f59e0b;--cyan:#06b6d4;--violet:#8b5cf6;
  --sans:system-ui,-apple-system,'Segoe UI',Inter,sans-serif;
  --mono:ui-monospace,'SF Mono','JetBrains Mono',monospace;
  --ease:cubic-bezier(.2,.7,.2,1);
  --shadow-sm:0 1px 2px rgba(0,0,0,0.2);
}
html,body{background:var(--bg);color:var(--text);font-family:var(--sans);font-size:16px;line-height:1.6;min-height:100vh;padding:48px 48px 72px;max-width:1320px;margin:0 auto;-webkit-font-smoothing:antialiased;}
.hdr{position:relative;display:flex;align-items:center;justify-content:space-between;padding:22px 26px 24px;margin:-12px -22px 40px;border-radius:16px;background:linear-gradient(180deg,rgba(59,130,246,0.1) 0%,rgba(59,130,246,0) 70%),var(--surface);border:1px solid var(--border);overflow:hidden;box-shadow:var(--shadow-sm)}
.hdr::after{content:"";position:absolute;left:0;right:0;bottom:-1px;height:1px;background:linear-gradient(90deg,transparent 0%,var(--blue-line) 20%,var(--blue-line) 80%,transparent 100%);opacity:.7;}
.brand{display:flex;align-items:center;gap:12px;font-size:19px;font-weight:600;letter-spacing:-.01em}
.glyph{font-family:var(--mono);font-size:15px;font-weight:700;color:var(--blue);background:var(--blue-soft);padding:4px 10px;border-radius:7px;border:1px solid var(--blue-line);line-height:1}
.brand small{font-size:12.5px;font-weight:500;color:var(--sub);letter-spacing:.06em;text-transform:uppercase;margin-left:2px}
.hdr-right{display:flex;align-items:center;gap:16px}
.pill{display:flex;align-items:center;gap:9px;font-size:14px;color:var(--text);font-weight:500}
.dot{width:9px;height:9px;border-radius:50%;background:var(--green);box-shadow:0 0 8px rgba(34,197,94,.45)}
.dot.err{background:var(--red);box-shadow:0 0 8px rgba(239,68,68,.45)}
section{margin-bottom:42px}
.sh{display:flex;align-items:center;gap:14px;font-size:13px;text-transform:uppercase;letter-spacing:.12em;color:var(--sub);margin-bottom:20px;font-weight:700}
.sh::after{content:"";flex:1;height:1px;background:linear-gradient(90deg,var(--border) 0%,transparent 100%)}
.grid{display:grid;grid-template-columns:repeat(3,1fr);gap:1px;background:var(--border);border:1px solid var(--border);border-radius:16px;overflow:hidden;box-shadow:var(--shadow-sm)}
.cell{position:relative;background:var(--surface);padding:22px 26px;transition:background .2s var(--ease)}
.cell:hover{background:var(--surface-2)}
.cl{font-size:14px;color:var(--sub);margin-bottom:12px;letter-spacing:.01em;font-weight:500}
.cv{font-family:var(--mono);font-size:32px;font-weight:600;line-height:1;letter-spacing:-.01em;color:var(--text);font-variant-numeric:tabular-nums}
.card{background:var(--surface);border:1px solid var(--border);border-radius:16px;padding:26px 28px;box-shadow:var(--shadow-sm);margin-bottom:20px}
.brow{display:flex;justify-content:space-between;align-items:flex-start;gap:24px;margin-bottom:20px}
.bpct{font-family:var(--mono);font-size:26px;font-weight:600;color:var(--text);font-variant-numeric:tabular-nums}
.bpct.small{font-size:20px}
/* Engines */
.eng-grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(210px,1fr));gap:10px}
.eng-badge{display:flex;align-items:center;justify-content:space-between;gap:10px;padding:14px 18px;border-radius:12px;border:1px solid var(--border);background:var(--surface);transition:background .15s,border-color .15s}
.eng-badge:hover{background:var(--surface-2)}
.eng-badge.on{border-color:rgba(34,197,94,0.35);background:rgba(34,197,94,0.05)}
.eng-badge.off{opacity:.55}
.eng-name{font-size:13.5px;font-weight:700;color:var(--text);font-family:var(--mono)}
.eng-sub{font-size:11.5px;color:var(--muted);margin-top:2px}
.eng-pill{display:inline-flex;align-items:center;padding:3px 9px;border-radius:6px;font-size:11.5px;font-weight:700;letter-spacing:.04em;white-space:nowrap;flex-shrink:0}
.eng-pill.on{background:var(--green-soft);color:var(--green);border:1px solid rgba(34,197,94,0.3)}
.eng-pill.off{background:var(--red-soft);color:var(--red);border:1px solid rgba(239,68,68,0.25)}
.eng-pill.cache{background:rgba(59,130,246,0.1);color:var(--blue);border:1px solid rgba(59,130,246,0.3)}
/* Table */
.tbl-wrap{overflow-x:auto;border:1px solid var(--border);border-radius:14px;background:var(--surface);box-shadow:var(--shadow-sm)}
.tbl{width:100%;border-collapse:collapse;font-size:14.5px}
.tbl thead th{background:var(--surface-2);font-weight:700;color:var(--sub);text-align:left;padding:14px 18px;font-size:12px;text-transform:uppercase;letter-spacing:.10em;border-bottom:1px solid var(--border)}
.tbl td{padding:14px 18px;border-bottom:1px solid var(--border);vertical-align:top;font-family:var(--mono);font-variant-numeric:tabular-nums}
.tbl tr:last-child td{border-bottom:none}
.tbl tr.rrow:hover{background:var(--surface-2)}
.st-2xx{color:var(--green);font-weight:600}
.st-5xx{color:var(--red);font-weight:600}
.nodata{text-align:center;color:var(--muted);padding:32px;font-family:var(--sans);font-size:15px;font-style:italic}
.cost-bars{display:flex;flex-direction:column;gap:12px;min-height:88px;margin-top:12px}
.cb{display:grid;grid-template-columns:200px 1fr auto;gap:14px;align-items:center;font-size:14.5px}
.cb .cbn{font-family:var(--mono);color:var(--text);overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-weight:600}
.cb .cbbar{height:10px;background:var(--border);border-radius:5px;overflow:hidden}
.cb .cbbarfill{height:100%;background:linear-gradient(90deg,var(--blue),var(--cyan));border-radius:5px;transition:width .35s var(--ease);width:0%}
.cb .cbv{font-family:var(--mono);color:var(--text);font-variant-numeric:tabular-nums;font-weight:600}
@media(max-width:640px){body{padding:24px 16px}.grid{grid-template-columns:1fr}.eng-grid{grid-template-columns:1fr 1fr}}
</style>
</head>
<body>

<div class="hdr">
  <div class="brand">
    <span class="glyph">M/O</span>
    <span>MiddleOut Rust Proxy <small>v0.2.0</small></span>
  </div>
  <div class="hdr-right">
    <div class="pill"><div class="dot" id="dot"></div><span id="stxt">online</span></div>
  </div>
</div>

<section>
  <div class="sh">Traffic metrics</div>
  <div class="grid">
    <div class="cell"><div class="cl">Total requests</div><div class="cv" id="m-total">-</div></div>
    <div class="cell"><div class="cl">Compressed requests</div><div class="cv" id="m-comp">-</div></div>
    <div class="cell"><div class="cl">Upstream errors</div><div class="cv" id="m-err">-</div></div>
    <div class="cell"><div class="cl">Chars saved (in)</div><div class="cv" id="m-cin">-</div></div>
    <div class="cell"><div class="cl">Chars saved (out)</div><div class="cv" id="m-cout">-</div></div>
    <div class="cell"><div class="cl">Uptime</div><div class="cv" id="m-up">-</div></div>
  </div>
</section>

<section>
  <div class="sh">Compression engines</div>
  <div class="eng-grid" id="eng-grid">
    <div class="nodata" style="grid-column:1/-1">Loading&hellip;</div>
  </div>
</section>

<section>
  <div class="sh">Response caching</div>
  <div class="grid">
    <div class="cell"><div class="cl">Cache hits</div><div class="cv" id="ch-hits">-</div></div>
    <div class="cell"><div class="cl">Cache misses</div><div class="cv" id="ch-misses">-</div></div>
    <div class="cell"><div class="cl">Protected blocks</div><div class="cv" id="ch-prot">-</div></div>
  </div>
</section>

<section>
  <div class="sh">Financial cost tracking</div>
  <div class="card">
    <div class="brow">
      <div><div class="cl">Cumulative spend</div><div class="bpct" id="b-cost">-</div></div>
      <div style="text-align:right"><div class="cl">Attributed requests</div><div class="bpct small" id="b-creq">-</div></div>
    </div>
    <div class="cost-bars" id="b-cost-bars"><div class="nodata">No costed traffic yet.</div></div>
  </div>
</section>

<section>
  <div class="sh">Recent requests</div>
  <div class="tbl-wrap">
    <table class="tbl">
      <thead><tr><th>Path</th><th>Status</th><th>ms</th><th>Bytes in/out</th><th>Model</th></tr></thead>
      <tbody id="o-recent-body"><tr><td class="nodata" colspan="5">No requests logged yet.</td></tr></tbody>
    </table>
  </div>
</section>

<script>
function fmt(n){if(n==null||isNaN(n))return'-';if(n>=1e9)return(n/1e9).toFixed(1)+'B';if(n>=1e6)return(n/1e6).toFixed(1)+'M';if(n>=1e3)return(n/1e3).toFixed(1)+'k';return''+n}
function fup(s){if(s==null)return'-';if(s<60)return s.toFixed(0)+'s';if(s<3600)return Math.floor(s/60)+'m '+Math.floor(s%60)+'s';if(s<86400)return Math.floor(s/3600)+'h '+Math.floor((s%3600)/60)+'m';return Math.floor(s/86400)+'d '+Math.floor((s%86400)/3600)+'h'}

const ENGINES = [
  {key:'input_compression',   label:'Input compression',  sub:'compress request payloads'},
  {key:'output_compression',  label:'Output compression', sub:'compress response payloads'},
  {key:'jl_dedupe',           label:'JL dedup',           sub:'random projection sketches'},
  {key:'caveman_enabled',     label:'Caveman',            sub:'terse prose compactor',    levelKey:'caveman_level'},
  {key:'rtk_enabled',         label:'RTK',                sub:'phrase abbreviation dict', levelKey:'rtk_level'},
  {key:'json_aware_enabled',  label:'JSON aware',         sub:'structural JSON collapse', levelKey:'json_aware_level'},
  {key:'lsh_enabled',         label:'LSH dedup',          sub:'locality-sensitive hashing',levelKey:'lsh_level'},
  {key:'auto_insert_cache_wall',label:'Cache wall',       sub:'auto cache-control fence'},
  {key:'l1_cache_enabled',    label:'L1 cache',           sub:'SQLite exact match',       isCache:true},
  {key:'l2_cache_enabled',    label:'L2 cache',           sub:'semantic vector store',    isCache:true},
  {key:'rate_limit_enabled',  label:'Rate limiter',       sub:'token bucket per client'},
  {key:'adaptive_enabled',    label:'Adaptive mode',      sub:'auto-tune compression'},
];

function renderEngines(h){
  const grid = document.getElementById('eng-grid');
  grid.innerHTML = ENGINES.map(e => {
    const on = !!h[e.key];
    const level = e.levelKey ? h[e.levelKey] : null;
    const pillCls = on ? (e.isCache ? 'cache' : 'on') : 'off';
    const pillTxt = on ? (level || 'ON') : 'OFF';
    return `<div class="eng-badge ${on?'on':'off'}">
      <div><div class="eng-name">${e.label}</div><div class="eng-sub">${e.sub}</div></div>
      <span class="eng-pill ${pillCls}">${pillTxt}</span>
    </div>`;
  }).join('');
}

async function refresh(){
  try{
    const [d,h,c] = await Promise.all([
      fetch('/stats').then(r=>r.json()),
      fetch('/healthz').then(r=>r.json()),
      fetch('/cost').then(r=>r.json()),
    ]);

    document.getElementById('dot').className='dot';
    document.getElementById('stxt').textContent='online';

    document.getElementById('m-total').textContent=fmt(d.requests_total);
    document.getElementById('m-comp').textContent=fmt(d.compressed_requests);
    document.getElementById('m-err').textContent=fmt(d.upstream_errors);
    document.getElementById('m-cin').textContent=fmt(d.chars_saved_in);
    document.getElementById('m-cout').textContent=fmt(d.chars_saved_out);
    document.getElementById('m-up').textContent=fup(d.uptime_s);
    document.getElementById('ch-hits').textContent=fmt(d.cache_hits);
    document.getElementById('ch-misses').textContent=fmt(d.cache_misses);
    document.getElementById('ch-prot').textContent=fmt(d.protected_blocks);

    renderEngines(h);

    document.getElementById('b-cost').textContent='$'+c.total_usd.toFixed(4);
    document.getElementById('b-creq').textContent=c.total_requests;
    const bars=document.getElementById('b-cost-bars');
    const models=Object.keys(c.by_model||{});
    if(!models.length){bars.innerHTML='<div class="nodata">No costed traffic yet.</div>';}
    else{
      let mx=0.000001;for(const m of models)mx=Math.max(mx,c.by_model[m].usd);
      bars.innerHTML=models.map(m=>{
        const it=c.by_model[m],w=(it.usd/mx*100).toFixed(0);
        return `<div class="cb"><div class="cbn">${m}</div><div class="cbbar"><div class="cbbarfill" style="width:${w}%"></div></div><div class="cbv">$${it.usd.toFixed(4)}</div></div>`;
      }).join('');
    }

    const recent=d.recent||[];
    const tbody=document.getElementById('o-recent-body');
    if(!recent.length){tbody.innerHTML='<tr><td class="nodata" colspan="5">No requests logged yet.</td></tr>';}
    else{
      tbody.innerHTML=recent.map(r=>{
        const ok=r.status_code>=200&&r.status_code<300;
        return `<tr class="rrow"><td>${r.path}</td><td class="${ok?'st-2xx':'st-5xx'}">${r.status_code||'error'}</td><td>${r.ms}ms</td><td>${fmt(r.bytes_in)} / ${fmt(r.bytes_out)}</td><td>${r.model||'unknown'}</td></tr>`;
      }).join('');
    }
  }catch(e){
    console.error(e);
    document.getElementById('dot').className='dot err';
    document.getElementById('stxt').textContent='offline';
  }
}

setInterval(refresh,3000);
refresh();
</script>
</body>
</html>"#.to_string()
}
