pub fn get_dashboard_html() -> String {
    r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>MiddleOut</title>
<style>
*,*::before,*::after{box-sizing:border-box;margin:0;padding:0}
:root{
  --bg:#0f172a;--surface:#1e293b;--surface-2:#334155;--border:#475569;--border-2:#64748b;
  --text:#f8fafc;--sub:#cbd5e1;--muted:#94a3b8;
  --blue:#3b82f6;--blue-soft:rgba(59,130,246,0.15);--blue-line:rgba(59,130,246,0.5);
  --green:#22c55e;--red:#ef4444;--amber:#f59e0b;--cyan:#06b6d4;--violet:#8b5cf6;
  --sans:system-ui,-apple-system,'Segoe UI',Inter,sans-serif;
  --mono:ui-monospace,'SF Mono','JetBrains Mono',monospace;
  --ease:cubic-bezier(.2,.7,.2,1);
  --shadow-sm:0 1px 2px rgba(0,0,0,0.2);
  --shadow-md:0 4px 14px rgba(0,0,0,0.3);
}
html,body{background:var(--bg);color:var(--text);font-family:var(--sans);font-size:16px;line-height:1.6;min-height:100vh;padding:48px 48px 72px;max-width:1320px;margin:0 auto;-webkit-font-smoothing:antialiased;}
.hdr{position:relative;display:flex;align-items:center;justify-content:space-between;padding:22px 26px 24px;margin:-12px -22px 40px;border-radius:16px;background:linear-gradient(180deg,rgba(59,130,246,0.1) 0%,rgba(59,130,246,0) 70%),var(--surface);border:1px solid var(--border);overflow:hidden;box-shadow:var(--shadow-sm)}
.hdr::after{content:"";position:absolute;left:0;right:0;bottom:-1px;height:1px;background:linear-gradient(90deg,transparent 0%,var(--blue-line) 20%,var(--blue-line) 80%,transparent 100%);opacity:.7;}
.brand{display:flex;align-items:center;gap:12px;font-size:19px;font-weight:600;letter-spacing:-.01em}
.glyph{font-family:var(--mono);font-size:15px;font-weight:700;color:var(--blue);background:var(--blue-soft);padding:4px 10px;border-radius:7px;border:1px solid var(--blue-line);line-height:1}
.brand small{font-size:12.5px;font-weight:500;color:var(--sub);letter-spacing:.06em;text-transform:uppercase;margin-left:2px}
.hdr-right{display:flex;align-items:center;gap:16px}
.pill{display:flex;align-items:center;gap:9px;font-size:14px;color:var(--text);font-weight:500}
.dot{position:relative;width:9px;height:9px;border-radius:50%;background:var(--green);box-shadow:0 0 8px rgba(34,197,94,.45)}
.dot.err{background:var(--red);box-shadow:0 0 8px rgba(239,68,68,.45)}
section{margin-bottom:42px}
.sh{display:flex;align-items:center;gap:14px;font-size:13px;text-transform:uppercase;letter-spacing:.12em;color:var(--sub);margin-bottom:20px;font-weight:700}
.sh::after{content:"";flex:1;height:1px;background:linear-gradient(90deg,var(--border) 0%,transparent 100%)}
.grid{display:grid;grid-template-columns:repeat(3,1fr);gap:1px;background:var(--border);border:1px solid var(--border);border-radius:16px;overflow:hidden;box-shadow:var(--shadow-sm)}
.cell{position:relative;background:var(--surface);padding:22px 26px;transition:background .2s var(--ease)}
.cell:hover{background:var(--surface-2)}
.cl{font-size:14px;color:var(--sub);margin-bottom:12px;letter-spacing:.01em;font-weight:500;display:flex;align-items:center;gap:6px}
.cv{font-family:var(--mono);font-size:32px;font-weight:600;line-height:1;letter-spacing:-.01em;color:var(--text);font-variant-numeric:tabular-nums}
.card{position:relative;background:var(--surface);border:1px solid var(--border);border-radius:16px;padding:26px 28px;box-shadow:var(--shadow-sm);margin-bottom:20px}
.brow{display:flex;justify-content:space-between;align-items:flex-start;gap:24px;margin-bottom:20px}
.bpct{font-family:var(--mono);font-size:26px;font-weight:600;letter-spacing:-.01em;color:var(--text);font-variant-numeric:tabular-nums}
.bpct.small{font-size:20px}
.track{position:relative;height:8px;background:var(--border);border-radius:4px;margin-bottom:18px;overflow:hidden}
.fill{height:100%;background:linear-gradient(90deg,var(--blue) 0%,#60a5fa 100%);border-radius:4px;transition:width .6s var(--ease);width:0%}
.bsub{display:flex;justify-content:space-between;align-items:center;gap:20px;font-size:14.5px;color:var(--sub);flex-wrap:wrap}
.bsub b{font-family:var(--mono);color:var(--text);font-weight:600;font-variant-numeric:tabular-nums}
.badge{display:inline-flex;align-items:center;gap:7px;padding:6px 12px;border-radius:8px;font-size:13.5px;font-family:var(--mono);background:var(--surface-2);color:var(--text);border:1px solid var(--border-2);font-weight:600}
.tbl-wrap{overflow-x:auto;border:1px solid var(--border);border-radius:14px;background:var(--surface);box-shadow:var(--shadow-sm)}
.tbl{width:100%;border-collapse:collapse;font-size:14.5px}
.tbl thead th{position:sticky;top:0;background:var(--surface-2);font-weight:700;color:var(--sub);text-align:left;padding:14px 18px;font-size:12px;text-transform:uppercase;letter-spacing:.10em;border-bottom:1px solid var(--border)}
.tbl td{padding:14px 18px;border-bottom:1px solid var(--border);vertical-align:top;font-family:var(--mono);font-variant-numeric:tabular-nums}
.tbl tr:last-child td{border-bottom:none}
.tbl tr.rrow:hover{background:var(--surface-2)}
.st-2xx{color:var(--green);font-weight:600}
.st-5xx{color:var(--red);font-weight:600}
.nodata{text-align:center;color:var(--muted);padding:32px;font-family:var(--sans);font-size:15px;font-style:italic}
.cost-bars{display:flex;flex-direction:column;gap:12px;min-height:88px;margin-top:12px}
.cb{display:grid;grid-template-columns:200px 1fr auto;gap:14px;align-items:center;font-size:14.5px}
.cb .cbn{font-family:var(--mono);color:var(--text);overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-weight:600}
.cb .cbbar{position:relative;height:10px;background:var(--border);border-radius:5px;overflow:hidden}
.cb .cbbarfill{height:100%;background:linear-gradient(90deg,var(--blue),var(--cyan));border-radius:5px;transition:width .35s var(--ease);width:0%}
.cb .cbv{font-family:var(--mono);color:var(--text);font-variant-numeric:tabular-nums;font-weight:600}
@media(max-width:640px){
  body{padding:24px 16px}
  .grid{grid-template-columns:1fr}
}
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
  <div class="sh">Response Caching</div>
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
      <div style="text-align:right"><div class="cl" style="justify-content:flex-end">Attributed requests</div><div class="bpct small" id="b-creq">-</div></div>
    </div>
    <div class="cost-bars" id="b-cost-bars">
      <div class="nodata">No costed traffic yet.</div>
    </div>
  </div>
</section>

<section>
  <div class="sh">Recent requests</div>
  <div class="tbl-wrap">
    <table class="tbl" id="o-recent">
      <thead><tr><th>Path</th><th>Status</th><th>ms</th><th>Bytes in/out</th><th>Model</th></tr></thead>
      <tbody id="o-recent-body"><tr><td class="nodata" colspan="5">No requests logged yet.</td></tr></tbody>
    </table>
  </div>
</section>

<script>
function fmt(n){if(n==null||isNaN(n))return'-';if(n>=1e9)return(n/1e9).toFixed(1)+'B';if(n>=1e6)return(n/1e6).toFixed(1)+'M';if(n>=1e3)return(n/1e3).toFixed(1)+'k';return''+n}
function fup(s){if(s==null)return'-';if(s<60)return s.toFixed(0)+'s';if(s<3600)return Math.floor(s/60)+'m '+Math.floor(s%60)+'s';if(s<86400)return Math.floor(s/3600)+'h '+Math.floor((s%3600)/60)+'m';return Math.floor(s/86400)+'d '+Math.floor((s%86400)/3600)+'h'}

async function refreshStats(){
  try{
    const d = await fetch('/stats').then(r=>r.json());
    document.getElementById('m-total').textContent = fmt(d.requests_total);
    document.getElementById('m-comp').textContent = fmt(d.compressed_requests);
    document.getElementById('m-err').textContent = fmt(d.upstream_errors);
    document.getElementById('m-cin').textContent = fmt(d.chars_saved_in);
    document.getElementById('m-cout').textContent = fmt(d.chars_saved_out);
    document.getElementById('m-up').textContent = fup(d.uptime_s);
    document.getElementById('ch-hits').textContent = fmt(d.cache_hits);
    document.getElementById('ch-misses').textContent = fmt(d.cache_misses);
    document.getElementById('ch-prot').textContent = fmt(d.protected_blocks);
    
    // Cost stats
    const c = await fetch('/cost').then(r=>r.json());
    document.getElementById('b-cost').textContent = '$' + c.total_usd.toFixed(4);
    document.getElementById('b-creq').textContent = c.total_requests;

    const bars = document.getElementById('b-cost-bars');
    const models = Object.keys(c.by_model);
    if(models.length === 0){
      bars.innerHTML = '<div class="nodata">No costed traffic yet.</div>';
    } else {
      let maxCost = 0.000001;
      for(const m of models) {
        maxCost = Math.max(maxCost, c.by_model[m].usd);
      }
      bars.innerHTML = models.map(m => {
        const item = c.by_model[m];
        const width = (item.usd / maxCost * 100).toFixed(0);
        return `<div class="cb">
          <div class="cbn">${m}</div>
          <div class="cbbar"><div class="cbbarfill" style="width:${width}%"></div></div>
          <div class="cbv">$${item.usd.toFixed(4)}</div>
        </div>`;
      }).join('');
    }

    // Recent requests
    const recent = d.recent || [];
    const tbody = document.getElementById('o-recent-body');
    if(recent.length === 0){
      tbody.innerHTML = '<tr><td class="nodata" colspan="5">No requests logged yet.</td></tr>';
    } else {
      tbody.innerHTML = recent.map(r => {
        const is2xx = r.status_code >= 200 && r.status_code < 300;
        const cl = is2xx ? 'st-2xx' : 'st-5xx';
        return `<tr class="rrow">
          <td>${r.path}</td>
          <td class="${cl}">${r.status_code || 'error'}</td>
          <td>${r.ms}ms</td>
          <td>${fmt(r.bytes_in)} / ${fmt(r.bytes_out)}</td>
          <td>${r.model || 'unknown'}</td>
        </tr>`;
      }).join('');
    }
  }catch(e){
    console.error(e);
  }
}

setInterval(refreshStats, 3000);
refreshStats();
</script>
</body>
</html>"#.to_string()
}
